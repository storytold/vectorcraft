//! The command palette: fuzzy search over the commands a pick runs, the tools, the panels and the
//! Effect menu's items.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
use crate::{VectorcraftApp, menus};

mod search;
pub(crate) mod words;

/// Engine commands the palette leaves out, each with the UI command of the same name that runs it
/// the way the panels and menus do: (engine id, UI command id). The Layers panel's Delete
/// Selection asks before deleting a layer that holds art, and the other UI commands open the
/// command's dialog or file chooser (the four Other Library… commands ask for the library file).
/// Batch… and Flattener Preview open the dialog or panel that runs the engine command. Scripts and
/// the control channel still run the engine ids.
const WRAPPED: &[(&str, &str)] = &[
    ("layer.delete", "ui.layerDelete"),
    ("brush.new", "ui.newBrush"),
    ("brush.options", "ui.brushOptions"),
    ("brush.objectOptions", "ui.brushObjectOptions"),
    ("symbol.options", "ui.symbolOptions"),
    ("svgFilter.import", "ui.importSvgFilter"),
    ("text.convertListToText", "type.convertListToText"),
    ("action.batch", "ui.batchDialog"),
    ("flattener.preview", "ui.flattenerPreview"),
    ("swatch.library.load", "window.swatchLibrary.other"),
    ("graphicStyle.loadLibrary", "window.graphicStyleLibrary.other"),
    ("symbol.loadLibrary", "window.symbolLibrary.other"),
    ("brush.loadLibrary", "window.brushLibrary.other"),
];

/// Engine commands that [`crate::shortcut_editor::needs_a_value`] marks and that a pick, which
/// gives no params, still runs: Stop Script stops the running script, Clear Script Console empties
/// the Script Console, Clear Plug-in Cache deletes every plug-in's cache, Reset Preferences resets
/// every preference, Forget Import Matches forgets every saved match, Begin Undo Group and End
/// Undo Group begin and end a group, the six Save … Preset commands save a new preset (Save
/// Gradient Preset the selection's gradient), Delete Gradient Preset deletes the saved preset the
/// selection's gradient matches, Pixel Snapping Options…
/// opens its dialog, the three Asset Export commands open the Asset Export panel (#4864), New
/// Symbol makes a symbol of the selection, Gradient Vector gives the selection the default
/// gradient, Define Perspective Grid makes the document's grid, Select Slice deselects the
/// slices, and Find Next goes on from the last Find and Replace search or opens its dialog, as
/// its menu item does.
const PICK_RUNS: &[&str] = &[
    "script.stop",
    "script.console.clear",
    "plugin.clearCache",
    "prefs.reset",
    "importMatches.reset",
    "edit.undoGroup.begin",
    "edit.undoGroup.end",
    "imageTrace.presets.save",
    "flattener.presets.save",
    "pdf.preset.save",
    "print.presets.save",
    "perspective.presets.save",
    "gradient.presets.save",
    "gradient.presets.delete",
    "view.snapToPixel.options",
    "assets.export",
    "assets.export3D",
    "assets.settings.set",
    "symbol.new",
    "paint.setGradientGeom",
    "perspective.grid.set",
    "object.slice.select",
    "edit.findNext",
];

/// The menu path of a panel's result: a pick runs Show Panel (`window.panel`) for the panel.
const PANEL_PATH: &str = "Window › Show Panel";

/// The menu path ([`context`]) a result takes when another result has its name: (id, English
/// path), for commands that no item of the menu bar runs and whose id names no parent command.
const CONTEXTS: &[(&str, &str)] = &[
    ("addon.pen.close", "Pen"),
    ("addon.pen.reshape", "Pen"),
    ("addon.pen.togglePoint", "Pen"),
    ("charStyle.setAttrs", "Character Styles"),
    ("file.exportInBackground", "Export"),
    ("file.openInBackground", "Open"),
    ("imageTrace.hideRuns", "Image Trace"),
    ("links.reveal", "Links"),
    ("livePaint.fill", "Live Paint"),
    ("object.blend.spine.convertAnchor", "Blend"),
    ("paraStyle.setAttrs", "Paragraph Styles"),
    ("path.close", "Path"),
    ("path.convertAnchor", "Path"),
    ("path.knife", "Path"),
    ("path.reshapeSegment", "Path"),
    ("plugin.hideRuns", "Plug-ins"),
    ("text.editRange", "Text"),
    ("ui.layersPanelOptions", "Layers"),
    ("ui.navigatorPanelOptions", "Navigator"),
];

/// A palette result before its name is made distinct ([`distinct_names`]).
struct Draft {
    /// The English menu path, `›`-separated; each segment translates.
    path: String,
    /// The English label.
    label: String,
    /// Does the label translate? An installed filter plug-in's name does not.
    translates: bool,
    id: String,
    shortcut: String,
    /// Has [`distinct_names`] given the result its [`context`]?
    in_context: bool,
    /// Does the result show its id after its name?
    keyed: bool,
}

impl Draft {
    fn new(path: &str, label: &str, id: String, shortcut: &str) -> Draft {
        Draft {
            path: path.to_string(),
            label: label.to_string(),
            translates: true,
            id,
            shortcut: shortcut.to_string(),
            in_context: false,
            keyed: false,
        }
    }

    /// The name in `lang`: the path's segments and the label translate on their own, and the id
    /// follows in parentheses when the result shows it.
    fn name(&self, lang: crate::i18n::Lang) -> String {
        // A built-in effect's label translates by its effect id first, as the Effect menu's does.
        let label = match self.id.strip_prefix("effect:") {
            _ if !self.translates => self.label.as_str(),
            Some(effect) => crate::i18n::tr_id(lang, effect, &self.label),
            None => crate::i18n::tr(lang, &self.label),
        };
        let name = if self.path.is_empty() { label.to_string() } else { format!("{} › {label}", shown_label(lang, &self.path)) };
        if self.keyed { format!("{name} ({})", self.id) } else { name }
    }
}

/// The palette's results before their names are made distinct: every listed engine command and
/// plug-in command, UI command, tool, panel and Effect menu item.
fn drafts() -> Vec<Draft> {
    let mut out = vec![];
    // Aliases kept for older scripts run a command that is listed already, a wrapped engine
    // command is listed as its UI command, and an engine command with a UI command of the same id
    // (Place…, Print…) is listed once, as the UI command. An engine command that needs a value a
    // pick doesn't give, or a query whose answer a pick drops ([`crate::shortcut_editor::needs_a_value`]),
    // is left out unless it does something besides answering ([`PICK_RUNS`]).
    let plugin_commands = vectorcraft_engine::cmd::plugin::command_specs();
    let ui_ids: std::collections::HashSet<&str> = menus::UI_COMMANDS.iter().map(|c| c.0).collect();
    let listed = |c: &vectorcraft_engine::CommandSpec| {
        !vectorcraft_engine::cmd::is_alias(c.id)
            && !WRAPPED.iter().any(|(engine, _)| *engine == c.id)
            && !ui_ids.contains(c.id)
            && (!crate::shortcut_editor::needs_a_value(c.id) || PICK_RUNS.contains(&c.id))
    };
    for c in vectorcraft_engine::command_specs().iter().filter(|c| listed(c)).chain(plugin_commands) {
        out.push(Draft::new(&c.menu.join(" › "), c.label, c.id.to_string(), menus::shortcut_of(c.id).unwrap_or("")));
    }
    // `plugin.dialog` runs nothing without an id: [`build`] lists each filter instead. A script of
    // the Scripts folder runs from File › Scripts, by its path. A UI command that wraps an engine
    // command, or has an engine command's id, takes that command's menu path, so a search for the
    // path still finds it.
    for c in menus::UI_COMMANDS.iter().filter(|c| c.0 != "plugin.dialog" && c.0 != crate::scripts_menu::COMMAND) {
        let engine = WRAPPED.iter().find(|(_, ui)| *ui == c.0).map_or(c.0, |(engine, _)| *engine);
        let engine = vectorcraft_engine::find_command(engine);
        let path = engine.map(|e| e.menu.join(" › ")).unwrap_or_default();
        out.push(Draft::new(&path, c.1, c.0.to_string(), menus::shortcut_of(c.0).unwrap_or("")));
    }
    for tool in vectorcraft_tools::catalog::all_tools() {
        out.push(Draft::new("", tool.label, format!("tool:{}", tool.id), crate::shortcut_editor::tool_shortcut(tool.id).unwrap_or("")));
    }
    // A panel's result shows the panel, as `window.panel` does without `toggle`, under Window ›
    // Show Panel.
    for (id, label) in crate::state::all_panels() {
        out.push(Draft::new(PANEL_PATH, label, format!("panel:{id}"), crate::shortcut_editor::panel_shortcut(id).unwrap_or("")));
    }
    // An Effect menu item runs what its item runs; a live-effect plug-in's name does not translate.
    for e in crate::shortcut_editor::effect_items().iter() {
        out.push(Draft {
            translates: vectorcraft_plugins::effect::plugin_id(&e.effect).is_none(),
            ..Draft::new(&e.group, &e.label, format!("effect:{}", e.effect), crate::shortcut_editor::effect_shortcut(&e.effect).unwrap_or(""))
        });
    }
    out
}

/// Everything the palette searches: (label, command id or `tool:<id>`, shortcut). Labels are the
/// English names before [`build`] makes them distinct.
pub fn items() -> Vec<(String, String, String)> {
    drafts().into_iter().map(|d| (d.name(crate::i18n::Lang::EN), d.id, d.shortcut)).collect()
}

/// The menu path a result shows when its name is another result's: its [`CONTEXTS`] path, Tools
/// for a tool, the menu path of the item of the menu bar that runs it (the four Other Library…
/// commands go under Window › Swatch Libraries, Brush Libraries, Graphic Style Libraries and Symbol
/// Libraries), or the label of the command whose id is its id without the last part (Reset Panel
/// goes under Character Style Options and Paragraph Style Options).
fn context(id: &str) -> Option<String> {
    if let Some((_, path)) = CONTEXTS.iter().find(|(c, _)| *c == id) {
        return Some(path.to_string());
    }
    if id.starts_with("tool:") {
        return Some("Tools".to_string());
    }
    // A plug-in's, panel's or effect's result already shows its menu path.
    if ["plugin:", "panel:", "effect:"].iter().any(|p| id.starts_with(p)) {
        return None;
    }
    if let Some(path) = crate::shortcut_editor::menu_bar_path(id) {
        return Some(path.to_string());
    }
    let (parent, _) = id.rsplit_once('.')?;
    let label =
        vectorcraft_engine::find_command(parent).map(|c| c.label).or_else(|| menus::UI_COMMANDS.iter().find(|c| c.0 == parent).map(|c| c.1))?;
    Some(label.trim_end_matches('…').to_string())
}

/// Gives results for different ids different names in each of `langs`. Results that share a name
/// in one of them take their [`context`], when they have one, as their menu path; results that
/// still share one show their id after it. An installed plug-in whose name is another
/// result's name with its id ("Blur (plugin:a)") shows its id too.
fn distinct_names(drafts: &mut [Draft], langs: &[crate::i18n::Lang]) {
    // The results whose name, in one of `langs`, a result with another id has.
    let shared = |drafts: &[Draft]| -> Vec<usize> {
        let mut out = std::collections::BTreeSet::new();
        for &lang in langs {
            let mut by_name: std::collections::HashMap<String, Vec<usize>> = std::collections::HashMap::new();
            for (i, d) in drafts.iter().enumerate() {
                by_name.entry(d.name(lang)).or_default().push(i);
            }
            let differ = |list: &Vec<usize>| {
                let ids: Vec<&str> = list.iter().filter_map(|&i| drafts.get(i)).map(|d| d.id.as_str()).collect();
                ids.iter().any(|id| Some(id) != ids.first())
            };
            out.extend(by_name.into_values().filter(differ).flatten());
        }
        out.into_iter().collect()
    };
    for i in shared(drafts) {
        if let Some(d) = drafts.get_mut(i).filter(|d| !d.in_context)
            && let Some(path) = context(&d.id)
        {
            d.path = path;
            d.in_context = true;
        }
    }
    // A pass gives every result of a shared name its id, so a name is shared again only by a result
    // whose label ends with another result's id; one pass per result is enough.
    for _ in 0..drafts.len() {
        let mut changed = false;
        for i in shared(drafts) {
            if let Some(d) = drafts.get_mut(i).filter(|d| !d.keyed) {
                d.keyed = true;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// A palette label in `lang`: each `›`-separated menu segment and the command label translate on
/// their own.
fn shown_label(lang: crate::i18n::Lang, label: &str) -> String {
    label.split(" › ").map(|s| crate::i18n::tr(lang, s)).collect::<Vec<_>>().join(" › ")
}

/// The label of the item of the menu bar that runs `e`, in `lang`, as the menu bar draws it
/// ([`menus::display_label`]). A built-in effect's item translates by its effect id first, as the
/// Effect menu's does. Any other item translates by the id of the command it runs first (File ›
/// Close is Cerrar archivo in Spanish), and a name the menu shows as written ([`menus::shows_a_name`]:
/// a filter or live-effect plug-in's) does not translate.
fn item_label(lang: crate::i18n::Lang, e: &Entry) -> &'static str {
    if let Some(effect) = e.id.strip_prefix("effect:") {
        return match vectorcraft_plugins::effect::plugin_id(effect) {
            Some(_) => e.menu_item,
            None => crate::i18n::tr_id(lang, effect, e.menu_item),
        };
    }
    // The command the menu item runs: `plugin.dialog` for a filter, `window.panel` for a panel and
    // the engine command a UI command wraps.
    let id = match e.id.as_str() {
        id if id.starts_with("plugin:") => "plugin.dialog",
        id if id.starts_with("panel:") => "window.panel",
        id => WRAPPED.iter().find(|(_, ui)| *ui == id).map_or(id, |(engine, _)| engine),
    };
    if menus::shows_a_name(id, e.menu_item) { e.menu_item } else { crate::i18n::tr_id(lang, id, e.menu_item) }
}

/// One palette entry, with what it shows and what a query matches worked out once.
pub(crate) struct Entry {
    id: String,
    shortcut: String,
    /// The label in the UI language.
    shown: String,
    /// The English label before and after [`distinct_names`] (what agents document), the shown
    /// label and the id (and the id of the engine command a UI command wraps), lowercased, one per
    /// line: a query word (which has no line break) matches within one of them.
    haystack: String,
    /// The English menu path of the first item of the menu bar shown that runs the entry
    /// ([`search::menu_homes`]), if one does.
    home: Option<String>,
    /// The English label of that item of the menu bar, which may differ from the entry's (File ›
    /// Exit runs Quit VectorCraft), or empty.
    menu_item: &'static str,
    /// The entry's kind, in the order [`search::kind`] ranks kinds.
    kind: u8,
    /// Where the row's label starts in `shown`: after the menu path, which the row shows as the
    /// entry's home instead ([`home_text`]), or 0.
    label_at: usize,
    /// Does the row's home name the item of the menu bar after its menu path, since the item's
    /// label differs from the entry's?
    names_item: bool,
}

/// The last query, the recent picks and the pick counts it was ranked with, and its ranked results.
type Ranked = (String, Vec<u32>, CommandUses, Arc<[u32]>);

/// The palette's entries, the index a query is looked up in, and the last query's ranked results.
pub(crate) struct Catalog {
    list: Vec<Entry>,
    index: search::Index,
    ranked: std::sync::Mutex<Option<Ranked>>,
}

impl std::ops::Deref for Catalog {
    type Target = [Entry];

    fn deref(&self) -> &[Entry] {
        &self.list
    }
}

/// What the entries depend on: the language, the installed plug-ins and the shortcuts.
fn cache_key(lang: crate::i18n::Lang) -> (&'static str, u64, u64) {
    (lang.code(), menus::plugin_revision(), crate::shortcut_editor::GENERATION.load(std::sync::atomic::Ordering::Relaxed))
}

/// The palette's entries in `lang` with the in-window menu bar.
#[cfg(test)]
fn entries(ctx: &egui::Context, lang: crate::i18n::Lang) -> Arc<Catalog> {
    entries_in(ctx, lang, false)
}

/// The palette's entries in `lang`, with the native menu bar when `native`. They and their index
/// are built the first time the palette opens after the language, the installed plug-ins, the
/// keyboard shortcuts (shown beside each entry) or the menu bar change, not at start and not on
/// every frame while a query is typed.
fn entries_in(ctx: &egui::Context, lang: crate::i18n::Lang, native: bool) -> Arc<Catalog> {
    type Cached = ((&'static str, u64, u64), bool, Arc<Catalog>);
    let key = cache_key(lang);
    let id = egui::Id::unique("palette-entries");
    if let Some((k, n, v)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && k == key
        && n == native
    {
        return v;
    }
    let built = Arc::new(catalog(lang, native));
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (key, native, built.clone())));
    built
}

/// The entries of [`build_in`] and their index.
fn catalog(lang: crate::i18n::Lang, native: bool) -> Catalog {
    let list = build_in(lang, native);
    let shown_homes: Vec<Option<String>> =
        list.iter().map(|e| e.home.as_deref().map(|h| format!("{} › {}", shown_label(lang, h), item_label(lang, e)))).collect();
    // A group's row in the `search` context gives its words in `lang`; a plain row is not read.
    let search_row = |key: &str| crate::i18n::tr_ctx_row(lang, words::CONTEXT, key).map(str::to_string);
    let index = search::Index::build(&list, &shown_homes, &params_texts(&list), search_row);
    Catalog { list, index, ranked: std::sync::Mutex::new(None) }
}

/// The params text of each entry's command, which the index reads a description from
/// ([`search::description`]): an engine or plug-in command's, a UI command's (the one it wraps
/// keeps its own) and an effect's. A tool, a panel and a filter plug-in have none.
fn params_texts(list: &[Entry]) -> Vec<&'static str> {
    let engine = vectorcraft_engine::command_specs().iter().chain(vectorcraft_engine::cmd::plugin::command_specs());
    let mut by_id: std::collections::HashMap<&str, &'static str> = engine.map(|c| (c.id, c.params)).collect();
    by_id.extend(menus::UI_COMMANDS.iter().map(|c| (c.0, c.3)));
    list.iter()
        .map(|e| match e.id.strip_prefix("effect:") {
            Some(effect) => vectorcraft_effects::effect_info(effect).map_or("", |info| info.params),
            None => by_id.get(e.id.as_str()).copied().unwrap_or(""),
        })
        .collect()
}

/// The palette's entries in `lang` with the in-window menu bar.
#[cfg(test)]
fn build(lang: crate::i18n::Lang) -> Vec<Entry> {
    build_in(lang, false)
}

/// The palette's entries in `lang`: [`items`], then one per filter plug-in's Object › Plug-ins
/// item (`plugin:<id>`), whose name is not translated, each with a name no result for another id
/// shows, in English and in `lang` ([`distinct_names`]). A query still matches the name before
/// it was made distinct. Each entry's menu home is read from the native menu bar when `native`.
fn build_in(lang: crate::i18n::Lang, native: bool) -> Vec<Entry> {
    let homes = search::menu_homes(native);
    let mut drafts = drafts();
    drafts.extend(crate::shortcut_editor::filter_items().iter().map(|f| Draft {
        translates: false,
        ..Draft::new(&f.group, &f.label, format!("plugin:{}", f.id), crate::shortcut_editor::filter_shortcut(&f.id).unwrap_or(""))
    }));
    let plain: Vec<String> = drafts.iter().map(|d| d.name(crate::i18n::Lang::EN)).collect();
    distinct_names(&mut drafts, &[crate::i18n::Lang::EN, lang]);
    drafts
        .into_iter()
        .zip(plain)
        .map(|(d, plain)| {
            let (label, shown) = (d.name(crate::i18n::Lang::EN), d.name(lang));
            let mut haystack = [plain, label, shown.clone(), d.id.clone()].map(|s| s.to_lowercase()).join("\n");
            // A UI command that wraps an engine command is also found by the engine command's id.
            if let Some((engine, _)) = WRAPPED.iter().find(|(_, ui)| *ui == d.id) {
                haystack.push('\n');
                haystack.push_str(&engine.to_lowercase());
            }
            let (home, menu_item) = homes.get(&d.id).map_or((None, ""), |(path, item)| (Some(path.clone()), *item));
            let kind = search::kind(&d.id, home.as_deref());
            // A row with a home shows its label without the menu path in front of it. A path that
            // makes the name distinct ([`distinct_names`]) stays, unless the row's home shows that
            // same path or it is the Tools in front of a tool's name, which the tool's kind and
            // toolbar show.
            let tool = d.id.starts_with("tool:");
            let drop = if d.in_context { home.as_deref() == Some(d.path.as_str()) || (tool && d.path == "Tools") } else { home.is_some() || tool };
            let prefix = format!("{} › ", shown_label(lang, &d.path));
            let label_at = if drop && !d.path.is_empty() && shown.starts_with(&prefix) { prefix.len() } else { 0 };
            let names_item = !menu_item.is_empty() && menu_item != d.label;
            Entry { id: d.id, shortcut: d.shortcut, shown, haystack, home, menu_item, kind, label_at, names_item }
        })
        .collect()
}

/// Runs the palette result `id`: selects a tool, or invokes a command with the params its entry
/// stands for. A command whose menu bar item gives params runs what a click on that item runs
/// ([`crate::shortcut_editor::menu_target_now`]), so Clean Up… and Find and Replace… open their
/// dialogs. While a result is dimmed ([`unavailable`]), its pick runs nothing, opens no dialog or
/// panel, and the status line says why the command can't run. A panel's result shows the panel,
/// and an effect's result runs what its Effect menu item runs ([`crate::keymap::command_of`]).
/// Returns what the pick did.
pub(crate) fn run_pick(app: &mut VectorcraftApp, id: &str) -> Picked {
    let had_dialog = app.ui.dialog.is_some();
    let ran = if let Some(tool) = id.strip_prefix("tool:") {
        match app.try_select_tool(tool) {
            Ok(()) => true,
            Err(e) => {
                app.status(e);
                false
            }
        }
    } else if let Some(panel) = id.strip_prefix("panel:") {
        // A panel that can be seen stays as it is, where `window.panel` would put an open flyout or
        // a floating panel away. In Presentation Mode the pick fails and changes nothing, as
        // `window.panel` does.
        if app.ui.screen_mode >= 3 {
            app.status("panels can't be shown in Presentation Mode");
            false
        } else if let Err(e) = crate::panel_docking::reveal_panel(app, panel) {
            app.status(e);
            false
        } else {
            true
        }
    } else if id.starts_with("plugin:") || id.starts_with("effect:") {
        let command = crate::keymap::command_of(id);
        // While the item is dimmed (an effect with nothing selected), no dialog opens and the
        // status line says why.
        if let Some(why) = crate::shortcut_editor::dimmed(app, &command.id, &command.id) {
            app.status(why);
            false
        } else {
            menus::invoke(app, &command.id, command.params);
            true
        }
    } else {
        match crate::shortcut_editor::menu_target_now(app, id) {
            Some(Ok((id, p))) => {
                menus::invoke(app, &id, p);
                true
            }
            Some(Err(why)) => {
                app.status(why);
                false
            }
            // A dimmed result does not run: the Asset Export commands with no document open show
            // no panel.
            None => match crate::shortcut_editor::dimmed(app, id, id) {
                Some(why) => {
                    app.status(why);
                    false
                }
                None => {
                    menus::invoke(app, id, json!({}));
                    true
                }
            },
        }
    };
    // A dimmed command that still opens its dialog counts as opening it.
    if !had_dialog && app.ui.dialog.is_some() {
        Picked::OpenedDialog
    } else if ran {
        Picked::Ran
    } else {
        Picked::Refused
    }
}

/// What a pick of a palette result did ([`run_pick`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Picked {
    /// The command ran, the tool was chosen or the panel was shown.
    Ran,
    /// The pick opened a dialog, which may still be canceled.
    OpenedDialog,
    /// The pick's menu item is dimmed now, its tool can't be chosen or its panel can't be shown:
    /// the status line says why.
    Refused,
}

/// Why the result `id`, shown as `name`, can't run now, as an English status message, or `None`
/// while it can: the status line a pick of its dimmed menu item gives
/// ([`crate::shortcut_editor::dimmed`]), with the engine's reason when it gives one and `name` in
/// place of the command's id. A panel can't be shown in Presentation Mode, a tool can always be
/// chosen, and every other result asks about the command its pick runs.
fn unavailable(app: &VectorcraftApp, id: &str, name: &str) -> Option<String> {
    if id.starts_with("tool:") {
        return None;
    }
    if id.starts_with("panel:") && app.ui.screen_mode >= 3 {
        return Some("panels can't be shown in Presentation Mode".to_string());
    }
    // The command the pick runs.
    let id = if id.starts_with("panel:") || id.starts_with("plugin:") || id.starts_with("effect:") {
        crate::keymap::command_of(id).id
    } else {
        id.to_string()
    };
    crate::shortcut_editor::dimmed(app, &id, name)
}

/// The drawn results' [`unavailable`] answers for the state `key` ([`availability_key`]) they were
/// read in. An engine check can take tens of milliseconds on a large selection, so each result is
/// asked once per state instead of on every frame.
#[derive(Clone, Default)]
struct Availability {
    key: u64,
    /// Each asked result's answer: why it can't run now, in English, or `None` while it can.
    known: std::collections::HashMap<String, Option<String>>,
    /// How many results were asked since the key was set.
    #[cfg(test)]
    asked: usize,
}

impl Availability {
    /// Why the result `e` can't run now, in the interface language, or `None` while it can.
    fn why(&mut self, app: &VectorcraftApp, e: &Entry) -> Option<String> {
        let why = match self.known.get(&e.id) {
            Some(why) => why.clone(),
            None => {
                #[cfg(test)]
                {
                    self.asked += 1;
                }
                let why = unavailable(app, &e.id, &e.shown);
                self.known.insert(e.id.clone(), why.clone());
                why
            }
        };
        why.map(|why| crate::i18n::msg(&why))
    }
}

/// What the results' availability depends on: what the native menu bar's rows depend on
/// ([`crate::native_menu::state_hash`]: commands run, the document's revision, selection, history
/// and clipboard, the language and more), whether the document has unsaved changes, the screen
/// mode, an open dialog, and the background saves and exports, file reads, plug-in runs and traces
/// that run or show their progress. A click asks again for what it leaves out (a panel shown, a
/// tool chosen) without changing it ([`show`]); typing in the search field does not.
fn availability_key(app: &VectorcraftApp) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    crate::native_menu::state_hash(app, None).hash(&mut h);
    let running = app.background.jobs.iter().filter(|j| j.progress.is_some() && !j.cancelled()).count();
    (app.ui.screen_mode, app.ui.dialog.is_some(), app.background.jobs.len(), running).hash(&mut h);
    (app.opening.pending.len(), crate::opening::any_shown(app)).hash(&mut h);
    // A background save marks the document saved without a new revision.
    app.session.active().map(|d| (d.path.is_some(), d.converted, d.is_dirty())).hash(&mut h);
    let plugins = (crate::plugin_runs::any_running(app), crate::plugin_runs::any_shown(app));
    (plugins, crate::trace_runs::any_running(app), crate::trace_runs::any_shown(app)).hash(&mut h);
    h.finish()
}

/// The entries the query `q` matches, best first ([`search::rank`]), with the commands picked most
/// often (`uses`) first among the entries that match as well, and then the picks that Recent
/// Commands lists for the ids `recent_ids` ([`recent`]). The ranking of the last query is kept, so
/// a frame that draws the same query with the same counts and recent picks does not rank again.
/// [`show`] lists the first [`MAX_RESULTS`] and counts the rest.
fn matching<'a>(catalog: &'a Catalog, q: &str, recent_ids: &[String], uses: &CommandUses) -> Vec<&'a Entry> {
    let q = search::normalize(q);
    let picks: Vec<u32> = recent_places(&catalog.list, recent_ids).into_iter().filter_map(|i| u32::try_from(i).ok()).collect();
    let kept = catalog
        .ranked
        .lock()
        .ok()
        .and_then(|last| last.as_ref().filter(|(k, p, c, _)| *k == q && *p == picks && c == uses).map(|(_, _, _, r)| r.clone()));
    let ranked = kept.unwrap_or_else(|| {
        let ranked: Arc<[u32]> = search::rank(&catalog.list, &catalog.index, &q, &picks, &uses.grades(&catalog.list)).into();
        // A poisoned lock only loses the kept ranking: the next frame ranks again.
        if let Ok(mut last) = catalog.ranked.lock() {
            *last = Some((q, picks, uses.clone(), ranked.clone()));
        }
        ranked
    });
    ranked.iter().filter_map(|&i| catalog.list.get(i as usize)).collect()
}

/// The height of a result row, in points: one line of 13-point body text.
const ROW_HEIGHT: f32 = 20.0;
/// The most result rows the list shows at once; the others scroll into view.
const VISIBLE_ROWS: usize = 12;
/// The most results the list holds. The footer says how many matched in all.
const MAX_RESULTS: usize = 50;
/// The popup's distance from the top of the window, in points.
const TOP: f32 = 90.0;
/// The space kept between the popup and the bottom of the window, in points.
const BOTTOM_GAP: f32 = 8.0;

/// The point size of the kind icon at the start of a result row.
const KIND_ICON: f32 = 14.0;
/// The point size of the home's text after a row's label.
const HOME_TEXT: f32 = 11.0;

/// The kind of result `e` is: the icon its row starts with and the word screen readers read
/// first, in the UI language. A tool shows its own icon.
fn kind_of(e: &Entry) -> (&'static str, &'static str) {
    match e.id.split_once(':') {
        Some(("tool", tool)) => (vectorcraft_tools::tool_info(tool).map_or("square-dashed", |t| crate::icons::tool_icon(t.icon)), tl!("Tool")),
        Some(("panel", _)) => ("layout-grid", tl!("Panel")),
        Some(("effect", _)) => ("dc-fx", tl!("Effect")),
        Some(("plugin", _)) => ("dc-filter", tl!("Filter")),
        _ => ("menu", tl!("Command")),
    }
}

/// The label a result row shows: its name without the menu path its home shows.
fn label_of(e: &Entry) -> &str {
    e.shown.get(e.label_at..).unwrap_or(&e.shown)
}

/// Where result `e` lives, as its row shows it: a tool's place on the toolbars ([`tool_home`]),
/// the menu path of the item of the menu bar that runs it (with the item's label when it differs
/// from the result's), or "No menu item".
fn home_text(app: &VectorcraftApp, e: &Entry) -> String {
    let lang = crate::i18n::current();
    if let Some(tool) = e.id.strip_prefix("tool:") {
        return tool_home(app, tool);
    }
    match &e.home {
        Some(path) if e.names_item => format!("{} › {}", shown_label(lang, path), item_label(lang, e)),
        Some(path) => shown_label(lang, path),
        None => tl!("No menu item").to_string(),
    }
}

/// Where `tool` is in the current layout: "Toolbar" for its button on the Tools panel, or the name
/// of the toolbar made with New Toolbar that holds it, followed by ", in the <tool> group", naming
/// the tool its slot's button shows, when the tool is hidden in that slot's group and the group is
/// not torn off into a strip; "All Tools"
/// when no toolbar holds it. A toolbar shown now comes first: the Tools panel, then the toolbars
/// made with New Toolbar; then a hidden Tools panel, then hidden toolbars made with New Toolbar.
/// Toolbars are hidden in Presentation Mode and while Tab hides the panels.
fn tool_home(app: &VectorcraftApp, tool: &str) -> String {
    let lang = crate::i18n::current();
    let place = |name: &str, shown: Option<&str>| match shown.filter(|s| *s != tool).and_then(vectorcraft_tools::tool_info) {
        // The tool goes in first, so a toolbar named with a placeholder keeps its name.
        Some(t) => crate::i18n::fmt(tl!("{toolbar}, in the {tool} group"), &[("tool", crate::i18n::tr(lang, t.label)), ("toolbar", name)]),
        None => name.to_string(),
    };
    let toolbars_shown = app.ui.screen_mode < 3 && app.ui.tab_hid.is_none();
    let main = crate::toolbar::slots(app).into_iter().map(|(_, s)| s).find(|s| s.contains(&tool));
    // A group torn off into a strip shows each of its tools on its own button.
    let main = main.map(|slot| {
        let torn_off = slot.len() > 1 && crate::toolbar::is_floating(app, None, slot[0]);
        place(tl!("Toolbar"), if torn_off { None } else { Some(crate::toolbar::shown_tool(app, &slot)) })
    });
    let active = app.session.tool_id();
    let custom = |shown: bool| {
        app.ui.custom_toolbars.iter().filter(|b| b.shown == shown).find_map(|bar| {
            let slot = bar.slots.iter().find(|s| s.iter().any(|t| t == tool))?;
            let tools: Vec<&'static str> = slot.iter().filter_map(|id| vectorcraft_tools::tool_info(id).map(|t| t.id)).collect();
            if tools.len() > 1 && tools.first().is_some_and(|first| crate::toolbar::is_floating(app, Some(&bar.name), first)) {
                return Some(place(&bar.name, None));
            }
            Some(place(&bar.name, crate::custom_toolbars::shown_tool(bar, active, &tools)))
        })
    };
    if toolbars_shown {
        if app.ui.toolbar
            && let Some(main) = &main
        {
            return main.clone();
        }
        if let Some(bar) = custom(true) {
            return bar;
        }
    }
    main.or_else(|| custom(true)).or_else(|| custom(false)).unwrap_or_else(|| tl!("All Tools").to_string())
}

/// What screen readers read for a result row, and its hover text: its kind, its label, where it
/// lives (`home`) and its shortcut, after commas.
fn row_label(e: &Entry, home: &str) -> String {
    let shortcut = menus::shown_shortcut(&e.shortcut);
    let kind = kind_of(e).1;
    let start = format!("{kind}, {}, {home}", label_of(e));
    if shortcut.is_empty() { start } else { format!("{start}, {shortcut}") }
}

/// A control the highlighted result's callout can point at ([`note_spot`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum SpotAt {
    /// A title of the in-window menu bar, by its English name.
    Menu(String),
    /// A slot's button on the Tools panel (toolbar none) or on the toolbar made with New Toolbar of
    /// this name, and the ids of the tools its group holds.
    Slot(Option<String>, Vec<String>),
    /// A tool's button on a strip torn off the Tools panel (toolbar none) or off the toolbar made
    /// with New Toolbar of this name, and the tool's id.
    Strip(Option<String>, String),
}

/// Where a control was drawn: the viewport and pass, its layer, the clip rectangle and its own.
#[derive(Clone, Debug)]
struct Spot {
    at: SpotAt,
    viewport: egui::ViewportId,
    pass: u64,
    layer: egui::LayerId,
    clip: egui::Rect,
    rect: egui::Rect,
}

fn spots_id() -> egui::Id {
    egui::Id::unique("palette-spots")
}

/// Where the palette says whether it is open, for [`note_spot`].
fn spots_wanted_id() -> egui::Id {
    egui::Id::unique("palette-spots-wanted")
}

/// While the palette is open, notes that `ui` drew the control `at` at `rect` in this pass, so the
/// highlighted result's callout can point at it. The menu bar and the toolbars draw before the
/// palette in each pass; `at` is built only while the palette is open. A control that is not
/// wholly in view ([`in_view`]) is not noted, so a pass notes no more controls than the window
/// shows.
pub(crate) fn note_spot(ui: &egui::Ui, rect: egui::Rect, at: impl FnOnce() -> SpotAt) {
    let ctx = ui.ctx();
    if !ctx.data(|d| d.get_temp::<bool>(spots_wanted_id())).unwrap_or(false) || !in_view(ctx.content_rect(), ui.clip_rect(), rect) {
        return;
    }
    let viewport = ctx.viewport_id();
    let pass = ctx.cumulative_pass_nr_for(viewport);
    let spot = Spot { at: at(), viewport, pass, layer: ui.layer_id(), clip: ui.clip_rect(), rect };
    ctx.data_mut(|d| add_spot(d.get_temp_mut_or_default::<Vec<Spot>>(spots_id()), spot));
}

/// Adds `spot` to `list`. The first control a viewport notes in a pass drops every control it
/// noted in earlier passes, so the list holds at most one pass of each viewport: no more controls
/// than its windows drew wholly in view.
fn add_spot(list: &mut Vec<Spot>, spot: Spot) {
    if list.iter().rev().find(|s| s.viewport == spot.viewport).is_some_and(|s| s.pass != spot.pass) {
        list.retain(|s| s.viewport != spot.viewport || s.pass == spot.pass);
    }
    list.push(spot);
}

/// Is `rect`, drawn clipped to `clip`, wholly in view: inside `clip` and inside the window `screen`?
fn in_view(screen: egui::Rect, clip: egui::Rect, rect: egui::Rect) -> bool {
    rect.is_finite() && clip.expand(0.5).contains_rect(rect) && screen.expand(0.5).contains_rect(rect)
}

/// The control noted as `at` in this pass of this viewport, when it is wholly in view and no
/// window or toolbar lies over it ([`covered`]).
fn spot(ctx: &egui::Context, at: impl Fn(&SpotAt) -> bool) -> Option<Spot> {
    let viewport = ctx.viewport_id();
    let pass = ctx.cumulative_pass_nr_for(viewport);
    let screen = ctx.content_rect();
    let found = ctx.data_mut(|d| {
        d.get_temp_mut_or_default::<Vec<Spot>>(spots_id())
            .iter()
            .find(|s| s.viewport == viewport && s.pass == pass && at(&s.at) && in_view(screen, s.clip, s.rect))
            .cloned()
    })?;
    (!covered(ctx, &found)).then_some(found)
}

/// Does a shown area above the control `s`'s layer overlap it: a floating panel, a toolbar or a
/// menu? The palette's own backdrop, popup and Recent Commands list do not count (the callout keeps
/// off the popup itself, [`aim`]), nor do tooltips. A layer that is not an area (a docked panel's)
/// is under every area of a higher order.
fn covered(ctx: &egui::Context, s: &Spot) -> bool {
    let own = [egui::Id::unique("palette-backdrop"), egui::Id::unique("palette"), recent_list_id()];
    ctx.memory(|m| {
        let order: Vec<egui::LayerId> = m.layer_ids().collect();
        let at = order.iter().position(|l| *l == s.layer);
        order.iter().enumerate().any(|(i, l)| {
            let above = match at {
                Some(at) => i > at,
                None => l.order > s.layer.order,
            };
            above
                && l.order < egui::Order::Tooltip
                && !own.contains(&l.id)
                && m.areas().is_visible(l)
                && m.area_rect(l.id).is_some_and(|r| r.intersects(s.rect))
        })
    })
}

/// Where the highlighted result lives, as the palette points at it.
enum Pointer {
    /// The control that shows the home, the callout's text, and the footer line that names the
    /// home when the popup covers the control.
    At(Spot, String, String),
    /// No control in view shows the home: the footer shows the text.
    Footer(String),
}

/// The footer line for a home that no control in view shows.
fn unseen_line(home: &str) -> String {
    crate::i18n::fmt(tl!("{home}, not shown now"), &[("home", home)])
}

fn unseen(home: &str) -> Pointer {
    Pointer::Footer(unseen_line(home))
}

/// How the callout at a control keeps off the palette's popup and off the control.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Aim {
    /// Outline the control and show the text in a box beside it.
    Box,
    /// Outline the control and show the text in the footer: a box beside the control would cover
    /// the popup or the control.
    Outline,
    /// The popup covers the control: no callout, and the footer names the home.
    Covered,
}

/// How the callout at `place` with `text` keeps off the popup at `popup` and off the control.
fn aim(ctx: &egui::Context, popup: Option<egui::Rect>, place: &Spot, text: &str) -> Aim {
    if popup.is_some_and(|p| p.intersects(place.rect)) {
        return Aim::Covered;
    }
    let boxed = callout_box(ctx, place, text, popup).0;
    if popup.is_some_and(|p| p.intersects(boxed)) || boxed.intersects(place.rect) { Aim::Outline } else { Aim::Box }
}

/// The callout for the highlighted result `e`. A tool points at its slot's button on a shown
/// toolbar, in the order of [`tool_home`], or at its button on a strip torn off its group; a
/// result with an item of the in-window menu bar points at the menu's title. A native menu bar,
/// a home that is not in view now and a result with no menu item get a footer line instead.
fn pointer(app: &VectorcraftApp, ctx: &egui::Context, e: &Entry) -> Pointer {
    let lang = crate::i18n::current();
    if let Some(id) = e.id.strip_prefix("tool:") {
        let Some(tool) = vectorcraft_tools::tool_info(id) else { return unseen(&tool_home(app, id)) };
        let Ok(found) = crate::panels::learn::locate::find(app, tool.id, true) else { return unseen(&tool_home(app, id)) };
        let bar = match &found.target {
            crate::panels::learn::locate::Target::Tools => None,
            crate::panels::learn::locate::Target::Custom(name) => Some(name.as_str()),
            crate::panels::learn::locate::Target::AllTools => return unseen(&tool_home(app, id)),
        };
        let place = if found.location == "strip" {
            spot(ctx, |at| matches!(at, SpotAt::Strip(b, t) if b.as_deref() == bar && t == tool.id))
        } else {
            spot(ctx, |at| matches!(at, SpotAt::Slot(b, tools) if b.as_deref() == bar && tools.iter().any(|t| t == tool.id)))
        };
        let Some(place) = place else { return unseen(&tool_home(app, id)) };
        let text = match found.shown.filter(|_| found.location == "flyout").and_then(vectorcraft_tools::tool_info) {
            Some(group) => crate::i18n::fmt(tl!("In the {group} group: press and hold to open it"), &[("group", crate::i18n::tr(lang, group.label))]),
            None => crate::i18n::tr(lang, tool.label).to_string(),
        };
        return Pointer::At(place, text, tool_home(app, id));
    }
    let Some(path) = &e.home else {
        let shortcut = menus::shown_shortcut(&e.shortcut);
        let none = tl!("No menu item");
        return Pointer::Footer(if shortcut.is_empty() { none.to_string() } else { format!("{none}, {shortcut}") });
    };
    // The native bar's application menu gives its items no label of their own.
    let full = match e.menu_item {
        "" => shown_label(lang, path),
        _ => format!("{} › {}", shown_label(lang, path), item_label(lang, e)),
    };
    let in_bar = crate::i18n::fmt(tl!("In the menu bar: {path}"), &[("path", &full)]);
    if app.services.native_menu.is_some() {
        return Pointer::Footer(in_bar);
    }
    let title = path.split(" › ").next().unwrap_or_default();
    match spot(ctx, |at| matches!(at, SpotAt::Menu(t) if t == title)) {
        Some(place) => Pointer::At(place, full, in_bar),
        None => unseen(&full),
    }
}

/// The callout drawn in the last pass: the outlined control, the box (none when the footer shows
/// the text) and the text. Only tests keep it.
type Callout = Option<(egui::Rect, Option<egui::Rect>, String)>;

#[cfg(test)]
fn callout_id() -> egui::Id {
    egui::Id::unique("palette-callout")
}

/// The callout's layer, above the palette's backdrop and popup.
fn callout_painter(ctx: &egui::Context) -> egui::Painter {
    ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("palette-callout-layer"))).with_clip_rect(ctx.content_rect())
}

/// Where the box with `text` goes beside the control `place`, and its text laid out: under a menu
/// title, and on the canvas side of a toolbar's button (its right when the button is in the
/// window's left half). Beside a button, the text wraps to at most [`BOX_ROWS`] rows to fit
/// between the button and the window's edge, or the palette's popup at `popup` when the popup is
/// on that side. The box stays inside the window.
fn callout_box(ctx: &egui::Context, place: &Spot, text: &str, popup: Option<egui::Rect>) -> (egui::Rect, Arc<egui::Galley>) {
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let r = place.rect;
    let menu = matches!(place.at, SpotAt::Menu(_));
    let right = r.center().x <= screen.center().x;
    let mut width = screen.width() - 2.0 * BOX_PAD.x - 8.0;
    if !menu {
        // Beside a button, the text wraps to fit between the button and the window's edge.
        // Narrower room than this leaves the box over the button, and the footer shows the text
        // instead ([`aim`]).
        let room = if right { screen.right() - r.right() } else { r.left() - screen.left() } - 10.0 - 2.0 * BOX_PAD.x;
        if room >= 80.0 {
            width = width.min(room);
        }
    }
    if let Some(p) = popup.filter(|_| !menu) {
        let room = if right { p.left() - r.right() } else { r.left() - p.right() } - 10.0 - 2.0 * BOX_PAD.x;
        // Narrower room than this leaves the box where it would cover the popup, and the footer
        // shows the text instead ([`aim`]).
        if room >= 80.0 {
            width = width.min(room);
        }
    }
    let mut job = egui::text::LayoutJob::simple(text.to_owned(), egui::FontId::proportional(12.0), t.text, width.max(0.0));
    job.wrap = egui::text::TextWrapping {
        max_width: width.max(0.0),
        max_rows: if menu { 1 } else { BOX_ROWS },
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let galley = callout_painter(ctx).layout_job(job);
    let size = galley.size() + 2.0 * BOX_PAD;
    let at = if menu {
        egui::pos2(r.left(), r.bottom() + 4.0)
    } else if right {
        egui::pos2(r.right() + 6.0, r.center().y - size.y / 2.0)
    } else {
        egui::pos2(r.left() - 6.0 - size.x, r.center().y - size.y / 2.0)
    };
    let x = at.x.min(screen.right() - 4.0 - size.x).max(screen.left() + 4.0);
    let y = at.y.min(screen.bottom() - 4.0 - size.y).max(screen.top() + 4.0);
    (egui::Rect::from_min_size(egui::pos2(x, y), size), galley)
}

/// How many rows the text of a box beside a toolbar's button wraps to at most.
const BOX_ROWS: usize = 3;

/// The space between the callout box's border and its text.
const BOX_PAD: egui::Vec2 = egui::vec2(6.0, 3.0);

/// Outline the control `place`, and with `text` show it in a box beside it ([`callout_box`], with
/// the popup at `popup`). Returns the box.
fn callout(ctx: &egui::Context, place: &Spot, text: Option<&str>, popup: Option<egui::Rect>) -> Option<egui::Rect> {
    let t = Tokens::get(ctx);
    let painter = callout_painter(ctx);
    crate::panels::learn::locate::outline(&painter.with_clip_rect(place.clip.intersect(ctx.content_rect())), place.rect, t.selection);
    let (rect, galley) = callout_box(ctx, place, text?, popup);
    painter.rect(rect, 4.0, t.panel, egui::Stroke::new(1.0, t.selection), egui::StrokeKind::Inside);
    painter.galley(rect.min + BOX_PAD, galley, t.text);
    Some(rect)
}

/// The line under the results: how many results matched, how many of them the list holds when it
/// can't hold them all, or that none matched.
fn footer(listed: usize, total: usize) -> String {
    if total == 0 {
        tl!("No commands match").to_string()
    } else if listed < total {
        let line = crate::i18n::tn(
            total as u64,
            "{shown} of {n} result. Keep typing to narrow them.",
            "{shown} of {n} results. Keep typing to narrow them.",
        );
        crate::i18n::fmt(&line, &[("shown", &listed.to_string())])
    } else {
        crate::i18n::tn(total as u64, "{n} result", "{n} results")
    }
}

/// The keys that move the highlighted result, in the order they were pressed this frame.
#[derive(Default)]
struct Moves(Vec<egui::Key>);

impl Moves {
    /// The keys that move the highlighted result.
    const KEYS: [egui::Key; 4] = [egui::Key::ArrowDown, egui::Key::ArrowUp, egui::Key::PageDown, egui::Key::PageUp];

    fn any(&self) -> bool {
        !self.0.is_empty()
    }

    /// The highlighted result of `n` after these moves from `sel`, taken in the order they were
    /// pressed, a page being `page` rows: Up and Down wrap at the ends, Page Up and Page Down stop
    /// at them.
    fn apply(&self, sel: usize, n: usize, page: usize) -> usize {
        let Some(last) = n.checked_sub(1) else { return 0 };
        self.0.iter().fold(sel.min(last), |sel, key| match key {
            egui::Key::ArrowDown => (sel + 1) % n,
            egui::Key::ArrowUp => (sel + last) % n,
            egui::Key::PageDown => sel.saturating_add(page).min(last),
            egui::Key::PageUp => sel.saturating_sub(page),
            _ => sel,
        })
    }
}

/// `name` laid out on one line at most `width` points wide; a longer name ends with "…".
fn name_galley(painter: &egui::Painter, name: &str, font: egui::FontId, color: egui::Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    painter.layout_job(job)
}

/// A result row's label and home laid out in `width` points, and whether either was cut short. The
/// home goes after the label in the smaller dim text and is cut first; when the label alone does
/// not fit, the home is left out and the label is cut before the id it shows after its name
/// ([`distinct_names`]), which stays whole unless the id alone is wider than the row.
fn row_galleys(
    painter: &egui::Painter,
    e: &Entry,
    home: &str,
    font: egui::FontId,
    color: egui::Color32,
    dim: egui::Color32,
    width: f32,
) -> (Arc<egui::Galley>, Option<Arc<egui::Galley>>, bool) {
    let label = label_of(e);
    let full = name_galley(painter, label, font.clone(), color, f32::INFINITY);
    let room = width - full.size().x - 10.0;
    if full.size().x <= width {
        if room < HOME_TEXT {
            return (full, None, !home.is_empty());
        }
        let home = name_galley(painter, home, egui::FontId::new(HOME_TEXT, font.family), dim, room);
        let cut = home.elided;
        return (full, Some(home), cut);
    }
    let suffix = format!(" ({})", e.id);
    let (head, suffix) = match label.strip_suffix(suffix.as_str()) {
        Some(head) => (head, suffix),
        None => (label, String::new()),
    };
    let suffix_width = name_galley(painter, &suffix, font.clone(), color, f32::INFINITY).size().x;
    let cut_head = name_galley(painter, head, font.clone(), color, width - suffix_width);
    let shown: String = cut_head.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.chr)).collect();
    (name_galley(painter, &format!("{shown}{suffix}"), font, color, width), None, true)
}

/// The scroll offset that keeps row `sel` in a list `view` points tall that was scrolled to `offset`.
fn offset_showing(sel: usize, offset: f32, view: f32) -> f32 {
    let top = sel as f32 * ROW_HEIGHT;
    offset.min(top).max(top + ROW_HEIGHT - view).max(0.0)
}

/// The row nearest to `sel` among the rows of `n` wholly in view of a list `view` points tall
/// scrolled to `offset`.
fn nearest_in_view(sel: usize, offset: f32, view: f32, n: usize) -> usize {
    let Some(last_row) = n.checked_sub(1) else { return 0 };
    // Half a point of leeway takes in rounding of the offset.
    let first = (((offset - 0.5) / ROW_HEIGHT).ceil().max(0.0) as usize).min(last_row);
    let end = ((offset + view + 0.5) / ROW_HEIGHT).floor().max(0.0) as usize;
    let last = end.saturating_sub(1).clamp(first, last_row);
    sel.clamp(first, last)
}

/// The highlighted result, the query it belongs to, the list's scroll offset, the list's height
/// and the highlighted result's id.
type Highlight = (usize, String, f32, f32, String);

/// How many picks the Recent Commands list beside the search field, and an empty search, show.
pub(crate) const MAX_RECENT_COMMANDS: usize = 8;

/// How many ids the saved Recent Commands list holds at most, unavailable ones included.
const MAX_SAVED_RECENT_COMMANDS: usize = 20;

/// Puts `id` first in the Recent Commands ids `list`, removing its earlier place, and keeps the
/// newest [`MAX_RECENT_COMMANDS`] ids that are `known`. An id that is not `known` now (a plug-in
/// that is disabled or not loaded yet) keeps its place among them, so it shows again when its
/// command comes back. The palette's own toggle, saved before its picks were left out, is dropped
/// so that it does not take a place. The list never holds more than [`MAX_SAVED_RECENT_COMMANDS`] ids.
fn remember(list: &mut Vec<String>, id: &str, known: impl Fn(&str) -> bool) {
    list.retain(|c| c != id && c != PALETTE_TOGGLE);
    list.insert(0, id.to_string());
    let mut kept = 0;
    let end = list.iter().position(|c| {
        if known(c) {
            kept += 1;
        }
        kept > MAX_RECENT_COMMANDS
    });
    list.truncate(end.unwrap_or(list.len()).min(MAX_SAVED_RECENT_COMMANDS));
}

/// The command that opens and closes the palette. A pick of it is not recorded in Recent Commands:
/// as the first recent pick, Enter on an empty search would close the palette and open it again.
const PALETTE_TOGGLE: &str = "help.commandPalette";

/// The entries the Recent Commands list shows for the ids `ids`, most recent first. An id with no
/// entry (a removed plug-in) and the palette's own toggle, saved before its picks were left out,
/// are skipped.
fn recent<'a>(entries: &'a [Entry], ids: &[String]) -> Vec<&'a Entry> {
    recent_places(entries, ids).into_iter().filter_map(|i| entries.get(i)).collect()
}

/// The places in `entries` of the entries [`recent`] lists for the ids `ids`, most recent first.
fn recent_places(entries: &[Entry], ids: &[String]) -> Vec<usize> {
    ids.iter()
        .take(MAX_SAVED_RECENT_COMMANDS)
        .filter(|id| *id != PALETTE_TOGGLE)
        .filter_map(|id| entries.iter().position(|e| e.id == *id))
        .take(MAX_RECENT_COMMANDS)
        .collect()
}

/// How many command ids [`CommandUses`] counts at most.
const MAX_COUNTED_COMMANDS: usize = 200;

/// The count at which [`CommandUses::count`] halves every count.
const HALVING_COUNT: u32 = 1_000;

/// How many times each command was picked in the palette: commands picked often rank first among
/// the results that match a query as well ([`matching`]). It counts at most
/// [`MAX_COUNTED_COMMANDS`] ids. When a count reaches [`HALVING_COUNT`], every count is halved, so
/// a count is at most 999.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct CommandUses {
    /// The number of the last pick; each pick takes the next one.
    pick: u32,
    /// For each command id, its count and the number of its last pick.
    uses: std::collections::BTreeMap<String, (u32, u32)>,
}

impl CommandUses {
    /// Counts a pick of `id`. A new id when [`MAX_COUNTED_COMMANDS`] are counted first removes
    /// one id from the half of the ids picked longest ago: the one with the lowest count, of those
    /// the one picked longest ago. It then counts once. When the pick number is at its limit, the
    /// counted ids' pick numbers are first renumbered from 1 in their order ([`renumbered`]), so
    /// the pick still takes a number above all of them.
    pub(crate) fn count(&mut self, id: &str) {
        if self.pick == u32::MAX {
            let lasts: Vec<u64> = self.uses.values().map(|&(_, last)| u64::from(last)).collect();
            for ((_, last), number) in self.uses.values_mut().zip(renumbered(&lasts)) {
                *last = number;
            }
            self.pick = self.uses.values().map(|&(_, last)| last).max().unwrap_or(0);
        }
        self.pick = self.pick.saturating_add(1);
        let pick = self.pick;
        if let Some((n, last)) = self.uses.get_mut(id) {
            *n = n.saturating_add(1);
            *last = pick;
        } else {
            if self.uses.len() >= MAX_COUNTED_COMMANDS {
                let mut by_age: Vec<(u32, u32, &String)> = self.uses.iter().map(|(k, &(n, last))| (last, n, k)).collect();
                by_age.sort_unstable();
                let older = by_age.len().div_ceil(2);
                if let Some(least) = by_age.iter().take(older).min_by_key(|&&(last, n, _)| (n, last)).map(|&(_, _, k)| k.clone()) {
                    self.uses.remove(&least);
                }
            }
            self.uses.insert(id.to_string(), (1, pick));
        }
        if self.uses.get(id).is_some_and(|&(n, _)| n >= HALVING_COUNT) {
            for (n, _) in self.uses.values_mut() {
                *n /= 2;
            }
            self.uses.retain(|_, (n, _)| *n > 0);
        }
    }

    /// Whether no command is counted.
    pub(crate) fn is_empty(&self) -> bool {
        self.uses.is_empty()
    }

    /// The count of `id`'s picks as a grade from 0 (never picked) to 9: the base-2 logarithm of
    /// the count plus one, so 1 pick gives 1, 3 give 2, 7 give 3, and so on.
    fn grade(&self, id: &str) -> u8 {
        self.uses.get(id).map_or(0, |&(n, _)| u8::try_from(n.min(HALVING_COUNT - 1).saturating_add(1).ilog2()).unwrap_or(u8::MAX))
    }

    /// The grade ([`CommandUses::grade`]) of each of `entries`, in their order, or none when no
    /// command was picked.
    fn grades(&self, entries: &[Entry]) -> Vec<u8> {
        if self.uses.is_empty() {
            return vec![];
        }
        entries.iter().map(|e| self.grade(&e.id)).collect()
    }
}

/// The pick numbers `lasts` renumbered from 1 in their order, in the order of `lasts`: equal
/// numbers stay equal, and the lowest becomes 1.
fn renumbered(lasts: &[u64]) -> Vec<u32> {
    let mut distinct = lasts.to_vec();
    distinct.sort_unstable();
    distinct.dedup();
    lasts.iter().map(|last| u32::try_from(distinct.partition_point(|d| d < last) + 1).unwrap_or(u32::MAX)).collect()
}

/// Serde reader for the saved pick counts ([`crate::UiState::command_uses`]): an entry that is not
/// a pair of counts is dropped, a count above 999 reads as 999, and of more than
/// [`MAX_COUNTED_COMMANDS`] ids the ones picked most recently are kept. When a kept pick number or
/// the last pick number is above the largest the counts hold, the kept pick numbers are
/// renumbered from 1 in their order ([`renumbered`]). Anything else reads as no picks, so an
/// unknown value doesn't lose the rest of the preferences.
pub fn deserialize_command_uses<'de, D: serde::Deserializer<'de>>(d: D) -> Result<CommandUses, D::Error> {
    use std::cmp::Reverse;
    let v = <serde_json::Value as serde::Deserialize>::deserialize(d)?;
    let number = |v: Option<&serde_json::Value>| v.and_then(serde_json::Value::as_u64);
    // The heap holds at most MAX_COUNTED_COMMANDS + 1 entries, the least recent pick on top (of
    // equal picks, the id that sorts last), so a long saved list is not collected whole. Pick
    // numbers are compared as saved, before any is renumbered.
    let mut kept: std::collections::BinaryHeap<Reverse<(u64, Reverse<&String>, u32)>> = std::collections::BinaryHeap::new();
    for (id, u) in v.get("uses").and_then(serde_json::Value::as_object).into_iter().flatten() {
        let (Some(n), Some(last)) = (number(u.get(0)), number(u.get(1))) else { continue };
        if n == 0 || !u.as_array().is_some_and(|a| a.len() == 2) {
            continue;
        }
        let n = u32::try_from(n.min(u64::from(HALVING_COUNT - 1))).unwrap_or(HALVING_COUNT - 1);
        kept.push(Reverse((last, Reverse(id), n)));
        if kept.len() > MAX_COUNTED_COMMANDS {
            kept.pop();
        }
    }
    let kept: Vec<(&String, u32, u64)> = kept.into_iter().map(|Reverse((last, Reverse(id), n))| (id, n, last)).collect();
    // The pick number stays above every saved last pick, so the next pick is the most recent.
    let newest = kept.iter().map(|&(_, _, last)| last).max().unwrap_or(0);
    let pick = number(v.get("pick")).unwrap_or(0).max(newest);
    if let Ok(pick) = u32::try_from(pick) {
        let uses = kept.into_iter().map(|(id, n, last)| (id.clone(), (n, u32::try_from(last).unwrap_or(pick)))).collect();
        return Ok(CommandUses { pick, uses });
    }
    let lasts: Vec<u64> = kept.iter().map(|&(_, _, last)| last).collect();
    let uses: std::collections::BTreeMap<String, (u32, u32)> =
        kept.into_iter().zip(renumbered(&lasts)).map(|((id, n, _), last)| (id.clone(), (n, last))).collect();
    let pick = uses.values().map(|&(_, last)| last).max().unwrap_or(0);
    Ok(CommandUses { pick, uses })
}

/// The id of the Recent Commands list's popup.
fn recent_list_id() -> egui::Id {
    egui::Id::unique("palette-recent-commands")
}

/// Is the palette's Recent Commands list open?
pub(crate) fn recent_list_open(ctx: &egui::Context) -> bool {
    egui::Popup::is_id_open(ctx, recent_list_id())
}

/// Opens the palette with an empty search when it is closed, and closes it when it is open.
pub(crate) fn toggle(app: &mut VectorcraftApp) {
    app.ui.palette_open = !app.ui.palette_open;
    app.ui.palette_query.clear();
    app.ui.palette_toggled = true;
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // The widget that had the keyboard focus when the palette opened takes it back when the palette
    // closes, unless a modal window is up (a pick opened a dialog, which focuses its own first
    // field, or a window that opened closed the palette), or the press that closed it focused
    // another widget, such as a field in a window above the backdrop.
    let focus_id = egui::Id::unique("palette-focus-before");
    let field_id = egui::Id::unique("palette-field");
    let sel_id = egui::Id::unique("palette-selected");
    let availability_id = egui::Id::unique("palette-availability");
    // A toggle can close the palette and open it again before it is drawn closed, as two runs of its
    // command in one step do, so the palette forgets its highlight after any toggle.
    if std::mem::take(&mut app.ui.palette_toggled) {
        ctx.data_mut(|d| d.remove::<Highlight>(sel_id));
    }
    // The Recent Commands rows drawn in the last frame, so a pick with the keyboard, which leaves the
    // focus on its row, gives the focus back too.
    let rows_id = egui::Id::unique("palette-recent-rows");
    // The menu bar and the toolbars note where they draw their controls only while the palette is
    // open; the pass that opens it asks for another, which they draw noting them.
    let wanted = ctx.data_mut(|d| std::mem::replace(d.get_temp_mut_or_default::<bool>(spots_wanted_id()), app.ui.palette_open));
    if app.ui.palette_open && !wanted {
        ctx.request_repaint();
    } else if !app.ui.palette_open && wanted {
        ctx.data_mut(|d| {
            d.remove::<Vec<Spot>>(spots_id());
            #[cfg(test)]
            d.remove::<Callout>(callout_id());
        });
    }
    if !app.ui.palette_open {
        // The palette opens again with its first row highlighted and its list scrolled to the top
        // (a list height of 0 scrolls it to the highlighted row), and asks its results again
        // whether they can run.
        let (field, rows) = ctx.data_mut(|d| {
            d.remove::<Highlight>(sel_id);
            d.remove::<Availability>(availability_id);
            let field = d.get_temp::<egui::Id>(field_id);
            d.remove::<egui::Id>(field_id);
            let rows = d.get_temp::<Vec<egui::Id>>(rows_id).unwrap_or_default();
            d.remove::<Vec<egui::Id>>(rows_id);
            (field, rows)
        });
        let now = ctx.memory(|m| m.focused());
        if let Some(Some(before)) = ctx.data_mut(|d| d.remove_temp::<Option<egui::Id>>(focus_id))
            && !crate::panel_docking::modal_window_up(app, ctx)
            && (now.is_none() || now == field || now.is_some_and(|n| rows.contains(&n)))
        {
            ctx.memory_mut(|m| m.request_focus(before));
        }
        return;
    }
    if ctx.data(|d| d.get_temp::<Option<egui::Id>>(focus_id)).is_none() {
        let before = ctx.memory(|m| m.focused());
        ctx.data_mut(|d| d.insert_temp(focus_id, before));
    }
    crate::shortcuts::hold_back_keys(app, ctx);
    let t = Tokens::get(ctx);
    let q = app.ui.palette_query.to_lowercase();
    let entries = entries_in(ctx, crate::i18n::current(), app.services.native_menu.is_some());
    let recent = recent(&entries.list, &app.ui.recent_commands);
    // An empty search lists the recent picks, most recent first, so Enter runs the last pick again.
    let empty = app.ui.palette_query.trim().is_empty();
    let mut matches = if empty { recent.clone() } else { matching(&entries, &q, &app.ui.recent_commands, &app.ui.command_uses) };
    let total = matches.len();
    // The highlighted result (#1757): Up and Down move it (wrapping), Page Up and Page Down move it
    // by a page, the pointer over a result takes it, and a new query starts again from the first.
    let (mut sel, last_q, mut offset, last_view, last_id) = ctx.data(|d| d.get_temp::<Highlight>(sel_id)).unwrap_or_default();
    let mut view = last_view;
    let new_query = last_q != q;
    // A result that ranks above the highlighted one when the results change under the same query
    // can push it past the last shown result; it then takes the last shown row instead, and
    // `kept` holds its place among every match.
    let mut kept = None;
    if !new_query
        && let Some(at) = matches.iter().position(|e| e.id == last_id)
        && at >= MAX_RESULTS
    {
        let held_result = matches.remove(at);
        matches.truncate(MAX_RESULTS - 1);
        matches.push(held_result);
        kept = Some(at);
    }
    matches.truncate(MAX_RESULTS);
    // While the Recent Commands list is open, the arrow keys and Enter go to its rows, and the
    // search field leaves them the keyboard.
    let list_open = recent_list_open(ctx);
    let mut run: Option<String> = None;
    let mut history_focused = false;
    // The highlight stays on its result when the results change under the same query, as when a
    // plug-in is installed, a shortcut changes or the language changes; it goes to the first
    // result when its result is gone.
    let held = sel;
    if new_query {
        sel = 0;
    } else if matches.get(sel).is_none_or(|e| e.id != last_id) {
        sel = matches.iter().position(|e| e.id == last_id).unwrap_or(0);
    }
    let moved = sel != held;
    // The Recent Commands row with the keyboard: Up and Down move it, wrapping at the ends, so the
    // keys never leave the list for the results below it.
    let rsel_id = egui::Id::unique("palette-recent-selected");
    let mut rsel = if list_open { ctx.data(|d| d.get_temp::<usize>(rsel_id)).unwrap_or(0) } else { 0 };
    if list_open && !recent.is_empty() {
        let rn = recent.len();
        let (down, up) =
            ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)));
        if down {
            rsel = (rsel + 1) % rn;
        }
        if up {
            rsel = (rsel + rn - 1) % rn;
        }
        rsel = rsel.min(rn - 1);
    }
    // Enter or Space runs the row the arrow keys of this frame moved to. The keys are taken here,
    // since a row's button turns them into a click of the row that had the keyboard last frame.
    if list_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter) | i.consume_key(egui::Modifiers::NONE, egui::Key::Space))
        && let Some(e) = recent.get(rsel)
    {
        run = Some(e.id.clone());
    }
    let n = matches.len();
    let mut moves = Moves::default();
    if n > 0 && !list_open {
        ctx.input_mut(|i| {
            i.events.retain(|e| match e {
                egui::Event::Key { key, pressed: true, modifiers, .. }
                    if Moves::KEYS.contains(key) && modifiers.matches_logically(egui::Modifiers::NONE) =>
                {
                    moves.0.push(*key);
                    false
                }
                _ => true,
            });
        });
    }
    let screen = ctx.content_rect();
    // The drawn results' availability, kept while the state it was read in lasts. A frame with a
    // pointer button release asks again, once, for what the state leaves out.
    let key = availability_key(app);
    let mut availability = ctx
        .data_mut(|d| d.remove_temp::<Availability>(availability_id))
        .filter(|a| a.key == key)
        .unwrap_or_else(|| Availability { key, ..Default::default() });
    if ctx.input(|i| i.events.iter().any(|e| matches!(e, egui::Event::PointerButton { pressed: false, .. }))) {
        availability.known.clear();
    }
    // The highlighted result the rows and the field's description are drawn with.
    let mut drawn = sel;
    // The control that shows where the highlighted result lives, the callout's text, and how the
    // footer drawn in this pass keeps the callout off the popup.
    let mut pointed: Option<(Spot, String, Aim)> = None;
    // The popup as the last pass laid it out: the footer is drawn before this pass's popup has its
    // size, and the callout is checked again against that size after the popup is drawn.
    let last_popup = ctx.memory(|m| m.area_rect(egui::Id::unique("palette")));
    // The backdrop covers the canvas, the toolbars, the docked panels and the app bar under the
    // popup, in the order of a modal dialog's backdrop (`dialogs::modal::backdrop`), so the press
    // that closes the palette does not press a control under it. Windows in the Foreground order
    // stay above it. Not focusable (`Sense::click()` is): Tab stays in the popup.
    egui::Area::new(egui::Id::unique("palette-backdrop")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
        ui.allocate_rect(screen, egui::Sense::CLICK);
    });
    // The area keeps itself inside the window (egui's `Area::constrain` is on), so in a window too
    // short for the field, one row and the footer below TOP, the popup moves up.
    let popup_area = egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, TOP]);
    let area = popup_area.show(ctx, |ui| {
        let margin: i8 = 10;
        egui::Frame::popup(ui.style()).fill(t.panel).inner_margin(egui::Margin::same(margin)).show(ui, |ui| {
            ui.set_width(520.0);
            let r = ui
                .horizontal(|ui| {
                    let r = ui.add(
                        egui::TextEdit::singleline(&mut app.ui.palette_query)
                            .align(egui::Align2::LEFT_CENTER)
                            .hint_text(tl!("Search commands, tools, panels and effects…"))
                            .desired_width(470.0),
                    );
                    // Recent Commands: the last picks, most recent first. A choice runs as a pick of
                    // the same result does. The first row takes the keyboard when the list opens, and
                    // the row with the keyboard keeps it while the list is open.
                    let button = crate::widgets::icon_button(ui, "history", tl!("Recent Commands"), false, 24.0);
                    // The search field keeps the keyboard, except while the list is open or Tab has
                    // moved it to the list's button, where Enter or Space opens the list.
                    history_focused = button.has_focus();
                    if !list_open && !history_focused {
                        r.request_focus();
                    }
                    let mut rows = Vec::new();
                    egui::Popup::menu(&button).id(recent_list_id()).show(|ui| {
                        if recent.is_empty() {
                            ui.add_enabled(false, egui::Button::new(tl!("No recent commands")));
                        }
                        for (i, e) in recent.iter().enumerate() {
                            // A pick that can't run now is drawn dimmed, as in the results, and says
                            // why on hover and to screen readers; it can still be chosen.
                            let why = availability.why(app, e);
                            let (name, shortcut) = (egui::RichText::new(e.shown.as_str()), egui::RichText::new(menus::shown_shortcut(&e.shortcut)));
                            let (name, shortcut) = match why {
                                Some(_) => (name.color(t.text_disabled), shortcut.color(t.text_disabled)),
                                None => (name, shortcut),
                            };
                            let row = ui.add(egui::Button::new(name).shortcut_text(shortcut));
                            ui.ctx().accesskit_node_builder(row.id, |node| {
                                if let Some(why) = &why {
                                    node.set_disabled();
                                    node.set_description(why.clone());
                                }
                            });
                            let row = match why {
                                Some(why) => row.on_hover_text(why),
                                None => row,
                            };
                            rows.push(row.id);
                            if i == rsel {
                                row.request_focus();
                            }
                            if row.clicked() {
                                run = Some(e.id.clone());
                            }
                        }
                    });
                    ui.ctx().data_mut(|d| d.insert_temp(rows_id, rows));
                    r
                })
                .inner;
            ui.ctx().data_mut(|d| d.insert_temp(field_id, r.id));
            ui.add_space(6.0);
            if empty && n > 0 {
                ui.label(egui::RichText::new(tl!("Recent")).color(t.text_dim));
            }
            // The list shows as many rows as fit above the footer in the window, up to
            // VISIBLE_ROWS; a page is that many rows.
            let spacing = ui.spacing().item_spacing.y;
            // Under the recent picks the footer line holds only where the highlighted pick's home is,
            // when no callout shows it, so the line's room is kept there too.
            let footer_height = spacing + 6.0 + ui.text_style_height(&egui::TextStyle::Body) + f32::from(margin) + BOTTOM_GAP;
            let room = screen.bottom() - footer_height - ui.cursor().top() - spacing;
            let rows = ((room / ROW_HEIGHT).floor().max(1.0) as usize).min(VISIBLE_ROWS);
            sel = moves.apply(sel, n, rows);
            drawn = sel;
            view = rows.min(n) as f32 * ROW_HEIGHT;
            // When the list's height changes, as when the window is resized, the list scrolls to
            // keep the highlighted result in view.
            let resized = (view - last_view).abs() > 0.5;
            // Screen readers stay on the field: its description names the highlighted result and
            // its place among every match, as the footer counts them, and on a second line why it
            // can't run now when it can't.
            let mut described = None;
            if let Some(e) = matches.get(sel) {
                let rank = if sel + 1 == matches.len() { kept.unwrap_or(sel) } else { sel };
                let pos = (rank + 1).to_string();
                let of = total.to_string();
                let name = row_label(e, &home_text(app, e));
                let mut said = crate::i18n::fmt(tl!("{pos} of {total}: {name}"), &[("pos", &pos), ("total", &of), ("name", &name)]);
                if let Some(why) = availability.why(app, e) {
                    said = format!("{said}\n{why}");
                }
                described = Some(said.clone());
                ui.ctx().accesskit_node_builder(r.id, |node| node.set_description(said));
            }
            if n > 0 {
                ui.scope(|ui| {
                    // `show_rows` adds the item spacing to each row, and rows touch.
                    ui.spacing_mut().item_spacing = egui::Vec2::ZERO;
                    let mut area = egui::ScrollArea::vertical().id_salt("palette-results").max_height(view).auto_shrink([false, true]);
                    if new_query || moves.any() || resized || moved {
                        offset = offset_showing(sel, if new_query { 0.0 } else { offset }, view);
                        area = area.vertical_scroll_offset(offset);
                    }
                    let out = area.show_rows(ui, ROW_HEIGHT, n, |ui, range| {
                        for i in range {
                            let Some(e) = matches.get(i) else { continue };
                            let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ROW_HEIGHT), egui::Sense::click());
                            if resp.hovered() && ui.input(|inp| inp.pointer.delta() != egui::Vec2::ZERO) {
                                sel = i;
                            }
                            let on = i == sel;
                            if on {
                                ui.painter().rect_filled(rect, 4.0, ui.visuals().selection.bg_fill);
                            }
                            // A result that can't run now stays in its place, drawn dimmed, and says
                            // why on hover and to screen readers.
                            let why = availability.why(app, e);
                            let (name_color, shortcut_color) = if why.is_some() { (t.text_disabled, t.text_disabled) } else { (t.text, t.text_dim) };
                            let font = egui::TextStyle::Body.resolve(ui.style());
                            let shortcut = menus::shown_shortcut(&e.shortcut);
                            let home = home_text(app, e);
                            let said = row_label(e, &home);
                            let p = ui.painter();
                            let shortcut_at = rect.right_center() - egui::vec2(8.0, 0.0);
                            let used = p.text(shortcut_at, egui::Align2::RIGHT_CENTER, &shortcut, font.clone(), shortcut_color);
                            let name_end = if shortcut.is_empty() { rect.right() - 8.0 } else { used.left() - 12.0 };
                            let icon = egui::Rect::from_center_size(
                                egui::pos2(rect.left() + 8.0 + KIND_ICON / 2.0, rect.center().y),
                                egui::Vec2::splat(KIND_ICON),
                            );
                            crate::icons::paint(ui, kind_of(e).0, icon, if why.is_some() { t.text_disabled } else { t.icon });
                            let name_at = icon.right() + 6.0;
                            let home_color = if why.is_some() { t.text_disabled } else { t.text_dim };
                            let (label, home, cut) = row_galleys(p, e, &home, font, name_color, home_color, name_end - name_at);
                            let home_at = name_at + label.size().x + 10.0;
                            p.galley(egui::pos2(name_at, rect.center().y - label.size().y / 2.0), label, name_color);
                            if let Some(home) = home {
                                p.galley(egui::pos2(home_at, rect.center().y - home.size().y / 2.0), home, home_color);
                            }
                            // A row whose text is cut short shows all of it while the pointer rests on it.
                            let resp = if cut { resp.on_hover_text(said.clone()) } else { resp };
                            // `WidgetInfo::selected` reaches screen readers as a toggled state, so the
                            // row sets the selected state on its node itself.
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, why.is_none(), said.clone()));
                            ui.ctx().accesskit_node_builder(resp.id, |node| {
                                node.set_selected(on);
                                if let Some(why) = &why {
                                    node.set_description(why.clone());
                                }
                            });
                            let resp = match why {
                                Some(why) => resp.on_hover_text(why),
                                None => resp,
                            };
                            if resp.clicked() {
                                run = Some(e.id.clone());
                            }
                        }
                    });
                    let before = offset;
                    offset = out.state.clamped_offset().y;
                    // After a scroll with the wheel or the scroll bar, by any amount, the highlight
                    // moves to the nearest row in view, so Enter runs a result that can be seen.
                    if !new_query && !moves.any() && (offset - before).abs() > 0.0 {
                        sel = nearest_in_view(sel, offset, view, n);
                    }
                });
            }
            // While the highlighted result can't run now, the footer also says why, so a keyboard
            // user reads it too, in place of the line that names its home.
            let why = matches.get(sel).and_then(|e| availability.why(app, e));
            // The highlighted result's home, after a pointer or a scroll moved the highlight too. In
            // the pass that opens the palette, the menu bar and the toolbars have noted nothing yet,
            // and the next pass shows it.
            let mut hint = None;
            match matches.get(sel).filter(|_| wanted).map(|e| pointer(app, ui.ctx(), e)) {
                Some(Pointer::At(place, text, covered)) => {
                    let how = aim(ui.ctx(), last_popup, &place, &text);
                    hint = match how {
                        Aim::Box => None,
                        Aim::Outline => Some(text.clone()),
                        Aim::Covered => Some(covered),
                    };
                    // Screen readers get the text of a box beside a tool's button, such as how to
                    // open the group that hides the tool, with the field's description, unless the
                    // description already holds it.
                    if how == Aim::Box
                        && !matches!(place.at, SpotAt::Menu(_))
                        && sel == drawn
                        && let Some(said) = &described
                        && !said.contains(text.as_str())
                    {
                        // The text goes on the first line, before the line that says why the
                        // result can't run now.
                        let said = match said.split_once('\n') {
                            Some((first, why)) => format!("{first}, {text}\n{why}"),
                            None => format!("{said}, {text}"),
                        };
                        ui.ctx().accesskit_node_builder(r.id, |node| node.set_description(said));
                    }
                    pointed = Some((place, text, how));
                }
                Some(Pointer::Footer(text)) => hint = Some(text),
                None => {}
            }
            // Under the recent picks no count is shown, only why the highlighted pick can't run now,
            // or where its home is when no callout shows it; with no picks, one dimmed line says
            // what a search finds.
            let line = why.or(hint);
            if !empty || (n > 0 && line.is_some()) {
                ui.add_space(6.0);
                ui.scope(|ui| {
                    // The footer stays one line of text tall, as the list's height counts it.
                    ui.spacing_mut().interact_size.y = 0.0;
                    ui.horizontal(|ui| {
                        if !empty {
                            ui.label(egui::RichText::new(footer(n, total)).color(t.text_dim));
                        }
                        if let Some(line) = line {
                            ui.add(egui::Label::new(egui::RichText::new(line).color(t.text_dim)).truncate());
                        }
                    });
                });
            } else if n == 0 {
                ui.add_space(6.0);
                ui.label(egui::RichText::new(tl!("Type to search commands, tools, panels and effects")).color(t.text_dim));
            }
            if !list_open && !history_focused && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                // Text typed in the frame that pressed Enter changed the query after the results
                // were listed: Enter runs the first result of the new query.
                let typed = app.ui.palette_query.to_lowercase();
                let picked = if typed == q {
                    matches.get(sel).copied()
                } else if typed.trim().is_empty() {
                    recent.first().copied()
                } else {
                    matching(&entries, &typed, &app.ui.recent_commands, &app.ui.command_uses).first().copied()
                };
                if let Some(e) = picked {
                    run = Some(e.id.clone());
                }
            }
        });
    });
    // A press of any pointer button that neither the popup's layer nor its Recent Commands list's
    // layer gets closes it: a press on the backdrop, or on a window above it, which also gets the
    // press, even where that window lies over the popup.
    let presses: Vec<egui::Pos2> = ctx.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::PointerButton { pos, pressed: true, .. } => Some(*pos),
                _ => None,
            })
            .collect()
    });
    let popup = egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("palette"));
    let list = egui::LayerId::new(egui::Order::Foreground, recent_list_id());
    let outside = presses.into_iter().any(|p| !ctx.layer_id_at(p).is_some_and(|l| l == popup || l == list));
    // The callout keeps off the popup as drawn in this pass. When the popup's new size changes
    // which of the three cases applies, the next pass draws the footer to match.
    let now = Some(area.response.rect);
    let shown: Callout = pointed.and_then(|(place, text, before)| {
        let how = aim(ctx, now, &place, &text);
        if how != before {
            ctx.request_repaint();
        }
        match how {
            Aim::Covered => None,
            Aim::Outline => Some((place.rect, callout(ctx, &place, None, now), text)),
            Aim::Box => Some((place.rect, callout(ctx, &place, Some(&text), now), text)),
        }
    });
    #[cfg(test)]
    ctx.data_mut(|d| d.insert_temp::<Callout>(callout_id(), shown));
    #[cfg(not(test))]
    drop(shown);
    // The pointer or a scroll moved the highlight after the rows were drawn: the next frame draws
    // it where it now is.
    if sel != drawn {
        ctx.request_repaint();
    }
    let sel_key = matches.get(sel).map(|e| e.id.clone()).unwrap_or_default();
    // A palette that closes on this frame forgets its highlight and its answers here, as a closed
    // frame does, so a command that opens it again before that frame starts it afresh.
    let closing = run.is_some() || outside;
    ctx.data_mut(|d| {
        if closing {
            d.remove::<Highlight>(sel_id);
            d.remove::<Availability>(availability_id);
        } else {
            d.insert_temp::<Highlight>(sel_id, (sel, q, offset, view, sel_key));
            d.insert_temp(availability_id, availability);
        }
        d.insert_temp(rsel_id, rsel);
    });
    if let Some(id) = run {
        app.ui.palette_open = false;
        // A pick goes first in Recent Commands before it runs, and stays there when it ran or
        // opened a dialog, even when the dialog is then canceled. A refused pick puts the list
        // back as it was. The pick is counted with it, before it runs, so Clear Recent Commands
        // clears its own pick and count, and a refused pick puts the counts back too.
        let before = (id != PALETTE_TOGGLE).then(|| (app.ui.recent_commands.clone(), app.ui.command_uses.clone()));
        if before.is_some() {
            remember(&mut app.ui.recent_commands, &id, |c| entries.list.iter().any(|e| e.id == c));
            app.ui.command_uses.count(&id);
        }
        if run_pick(app, &id) == Picked::Refused
            && let Some((recent, uses)) = before
        {
            app.ui.recent_commands = recent;
            app.ui.command_uses = uses;
        }
    } else if outside {
        app.ui.palette_open = false;
    }
    // The search icon can open the palette again before the next frame draws it, so a palette that
    // closes here forgets its highlight now and opens again at its first row.
    if !app.ui.palette_open {
        ctx.data_mut(|d| d.remove::<Highlight>(sel_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    /// The entries `q` matches, best first, with the recent picks `recent_ids` and no pick counts.
    fn matching<'a>(catalog: &'a Catalog, q: &str, recent_ids: &[String]) -> Vec<&'a Entry> {
        super::matching(catalog, q, recent_ids, &CommandUses::default())
    }

    #[test]
    fn entries_are_built_once_per_language_and_match_english_shown_text_and_ids() {
        let ctx = egui::Context::default();
        let zh = Lang::from_code("zh-hant").unwrap();
        // Tests running alongside edit shortcuts and install plug-ins, which rebuilds the entries:
        // look twice until nothing changed in between.
        let (en, again) = (0..100)
            .find_map(|_| {
                let key = cache_key(Lang::EN);
                let pair = (entries(&ctx, Lang::EN), entries(&ctx, Lang::EN));
                (cache_key(Lang::EN) == key).then_some(pair)
            })
            .unwrap();
        assert!(Arc::ptr_eq(&en, &again), "kept while the language stays");
        let zh_entries = entries(&ctx, zh);
        assert!(!Arc::ptr_eq(&en, &zh_entries), "rebuilt for another language");
        // Editing a shortcut rebuilds them, so the palette shows the new one.
        crate::shortcut_editor::GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        assert!(!Arc::ptr_eq(&zh_entries, &entries(&ctx, zh)), "rebuilt after a shortcut edit");
        let group = zh_entries.iter().find(|e| e.id == "object.group").unwrap();
        assert!(group.shown.ends_with(crate::i18n::tr(zh, "Group")), "{}", group.shown);
        // A query matches the English label, the shown label or the id.
        for word in ["group", crate::i18n::tr(zh, "Group"), "object.group"] {
            assert!(group.haystack.contains(&word.to_lowercase()), "{word}");
        }
    }

    /// Searching for the preference's name, Use Precise Cursors, finds the command that toggles it,
    /// and choosing the first result changes the preference.
    #[test]
    fn use_precise_cursors_palette_result_toggles_the_preference() {
        let ctx = egui::Context::default();
        let query = "use precise cursors";
        let entries = entries(&ctx, Lang::EN);
        let matches = matching(&entries, query, &[]);
        assert!(matches.iter().any(|e| e.id == "view.preciseCursors"), "preference toggle appears for its name");
        assert!(entries.iter().filter(|e| e.id == "view.capsLock").all(|e| e.shown != "Use Precise Cursors"));

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let before = app.session.prefs.use_precise_cursors;
        let first = matches.first().expect("query has a result");
        run_pick(&mut app, &first.id);
        assert_ne!(app.session.prefs.use_precise_cursors, before, "first result toggles the preference");
    }

    /// The palette lists `select.key` once, as Cancel Key Object (#541): a pick runs it with no id,
    /// which cancels the key object. `engine.commands` gives agents the same label.
    #[test]
    fn the_palette_lists_cancel_key_object_once() {
        let items = items();
        let ids: Vec<&str> = items.iter().filter(|(label, ..)| label == "Cancel Key Object").map(|(_, id, _)| id.as_str()).collect();
        assert_eq!(ids, ["select.key"]);
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let commands = crate::control::all_commands(&app);
        let key = commands.as_array().unwrap().iter().find(|c| c["id"] == "select.key").unwrap();
        assert_eq!(key["label"], "Cancel Key Object");
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("select.key", json!({"id": b})).unwrap();
        assert!(app.session.active().unwrap().selection.key.is_some());
        run_pick(&mut app, ids[0]);
        let st = app.session.active().unwrap();
        assert_eq!((st.selection.len(), st.selection.key), (2, None), "both objects stay selected, without a key");
    }

    /// An engine command that a UI command of the same name runs is listed once, as the UI command:
    /// the Layers panel's Delete Selection, New Brush…, Brush Options…, Symbol Options…, Import SVG
    /// Filter… and Convert to Text each have one result.
    #[test]
    fn a_wrapped_engine_command_is_listed_once_as_its_ui_command() {
        let items = items();
        for (engine, ui) in WRAPPED {
            let spec = vectorcraft_engine::command_specs().iter().find(|c| c.id == *engine).unwrap_or_else(|| panic!("{engine} is a command"));
            let wrapper = menus::UI_COMMANDS.iter().find(|c| c.0 == *ui).unwrap_or_else(|| panic!("{ui} is a UI command"));
            assert_eq!(spec.label, wrapper.1, "{engine} and {ui} share a name");
            assert!(items.iter().all(|(_, id, _)| id != engine), "{engine} is left out");
            let listed: Vec<&str> = items.iter().filter(|(_, id, _)| id == ui).map(|(label, ..)| label.as_str()).collect();
            let path: String = spec.menu.iter().map(|m| format!("{m} › ")).collect();
            assert_eq!(listed, [format!("{path}{}", wrapper.1)], "{ui} is listed once, with {engine}'s menu path");
        }
        // A search for Delete Selection, or for its menu path, finds the Layers panel's, and not the
        // engine command that deletes without asking; the path translates.
        let entries = build(Lang::EN);
        for (engine, ui) in WRAPPED {
            assert!(entries.iter().any(|e| e.id == *ui && e.haystack.contains(&engine.to_lowercase())), "a search for {engine} finds {ui}");
        }
        for query in ["delete selection", "layers delete", "layer.delete"] {
            let found: Vec<&str> =
                entries.iter().filter(|e| query.split_whitespace().all(|w| e.haystack.contains(w))).map(|e| e.id.as_str()).collect();
            assert!(found.contains(&"ui.layerDelete") && !found.contains(&"layer.delete"), "{query}: {found:?}");
        }
        let zh = Lang::from_code("zh-hant").unwrap();
        let shown = build(zh).into_iter().find(|e| e.id == "ui.layerDelete").map(|e| e.shown).unwrap();
        let want = ["Window", "Layers", "Delete Selection"].map(|s| crate::i18n::tr(zh, s)).join(" › ");
        assert_eq!(shown, want);
    }

    /// Every result a search for Delete Selection gives asks before deleting a layer that holds art:
    /// Cancel keeps it, and OK deletes it in one undo step. A script's `layer.delete` still deletes
    /// at once.
    #[test]
    fn a_pick_of_delete_selection_asks_before_deleting_a_layer_with_art() {
        let query = "delete selection";
        let picks: Vec<String> =
            build(Lang::EN).into_iter().filter(|e| query.split_whitespace().all(|w| e.haystack.contains(w))).map(|e| e.id).collect();
        assert!(!picks.is_empty(), "the search finds Delete Selection");
        let node = |v: &serde_json::Value| vectorcraft_doc::NodeId(v.as_u64().unwrap());
        let exists = |app: &VectorcraftApp, v: &serde_json::Value| app.session.active().unwrap().doc.node(node(v)).is_some();
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        let asked = |app: &VectorcraftApp| app.ui.dialog.as_ref().is_some_and(|d| d.kind == crate::dialogs::confirm::KIND);
        for pick in &picks {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let layer = app.run("layer.new", json!({})).unwrap()["id"].clone();
            let art = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
            app.run("select.none", json!({})).unwrap();
            let before = undo(&app);
            run_pick(&mut app, pick);
            assert!(asked(&app), "{pick} asks first: {}", app.ui.status);
            assert!(exists(&app, &layer) && exists(&app, &art) && undo(&app) == before, "{pick}: nothing is deleted yet");
            crate::dialogs::cancel(&mut app);
            assert!(exists(&app, &layer) && exists(&app, &art) && undo(&app) == before, "{pick}: Cancel keeps the layer");
            run_pick(&mut app, pick);
            crate::dialogs::confirm(&mut app).unwrap();
            assert!(!exists(&app, &layer) && !exists(&app, &art), "{pick}: OK deletes the layer and its art");
            assert_eq!(undo(&app), before + 1, "{pick}: one undo step");
            app.run("edit.undo", json!({})).unwrap();
            app.run("layer.delete", json!({ "ids": [layer] })).unwrap();
            assert!(app.ui.dialog.is_none() && !exists(&app, &layer), "a script deletes at once");
        }
    }

    /// #4864: a pick of an Asset Export command, which the palette runs with no params, opens the
    /// Asset Export panel instead of failing on the missing params or exporting nowhere.
    #[test]
    fn a_pick_of_an_asset_export_command_opens_its_panel() {
        let ids = ["assets.export", "assets.remove", "assets.rename", "assets.settings.set", "assets.export3D", "assets.set3DFormat"];
        for id in ids {
            assert!(items().iter().any(|(_, item, _)| item == id), "{id} is in the palette");
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.ui.status.clear();
            assert!(!crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}");
            run_pick(&mut app, id);
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: {}", app.ui.status);
            assert!(!app.ui.status.contains("invalid parameters"), "{id}: {}", app.ui.status);
            // A second pick leaves the open panel open, docked or floating.
            run_pick(&mut app, id);
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: picked twice");
            app.run("window.panel.float", json!({"panel": crate::panels::asset_export::ID})).unwrap();
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: floating");
            run_pick(&mut app, id);
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: floating, picked again");
        }
    }

    /// With no document open, the Asset Export commands are dimmed, and a pick of one shows no panel
    /// and says why it can't run, as the row's description does.
    #[test]
    fn a_dimmed_asset_export_command_does_not_open_its_panel() {
        let ids = ["assets.export", "assets.remove", "assets.rename", "assets.settings.set", "assets.export3D", "assets.set3DFormat"];
        for id in ids {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            let why = unavailable(&app, id, id);
            assert!(why.is_some(), "{id} is dimmed with no document open");
            app.ui.status.clear();
            run_pick(&mut app, id);
            assert!(!crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: {}", app.ui.status);
            assert_eq!(Some(app.ui.status.clone()), why, "{id}");
        }
    }

    /// The open dialog's kind and fields, the active document's undo depth and the status line.
    type Outcome = (Option<(String, serde_json::Map<String, serde_json::Value>)>, Option<usize>, String);

    /// The [`Outcome`] `app` shows.
    fn outcome(app: &VectorcraftApp) -> Outcome {
        (
            app.ui.dialog.as_ref().map(|d| (d.kind.clone(), d.fields.clone())),
            app.session.active().map(|st| st.history.undo.len()),
            app.ui.status.clone(),
        )
    }

    /// A pick of a "…" command whose menu bar item gives params opens that item's dialog, Smart
    /// Punctuation opens its dialog and Smooth opens its slider; nothing changes until the dialog
    /// is confirmed.
    #[test]
    fn a_pick_opens_the_dialog_its_menu_item_opens() {
        let expected = [
            ("object.path.cleanUp", "command"),
            ("object.mesh.create", "command"),
            ("type.smartPunctuation", crate::dialogs::smart_punctuation::KIND),
            ("edit.findReplace", crate::dialogs::find_replace::KIND),
            ("object.path.smooth", crate::dialogs::smooth_slider::KIND),
        ];
        for (id, kind) in expected {
            assert!(items().iter().any(|(_, item, _)| item == id), "{id} is in the palette");
            let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
            app.ui.status.clear();
            let before = app.session.active().unwrap().history.undo.len();
            run_pick(&mut app, id);
            let d = app.ui.dialog.as_ref().unwrap_or_else(|| panic!("{id} opens its dialog: {}", app.ui.status));
            assert_eq!(d.kind, kind, "{id}");
            if kind == "command" {
                assert_eq!(d.fields["__command"], json!(id), "{id}: {:?}", d.fields);
            }
            assert_eq!(app.session.active().unwrap().history.undo.len(), before, "{id} changes nothing yet");
            assert!(app.ui.status.is_empty(), "{id}: {}", app.ui.status);
        }
    }

    /// Every palette command whose menu bar items all give it the same params does what a click on
    /// such an item does, as the menu row shows it ([`menus::entry`]): with no document open, in an
    /// empty document and with one or several objects selected, the pick opens the same dialog with
    /// the same fields and leaves the same undo steps and status. While the row is dimmed, the pick
    /// opens no dialog, changes nothing and says why the command can't run.
    #[test]
    fn every_pick_runs_what_its_menu_item_runs() {
        use vectorcraft_testkit::fixtures::Fixture;
        fn walk<'a>(items: &'a [menus::Item], out: &mut Vec<&'a menus::Item>) {
            for it in items {
                match it {
                    menus::Item::Sub(_, children) => walk(children, out),
                    // Items that list an installed plug-in or show a name that is user data come and
                    // go as tests beside this one install plug-ins or add toolbars, and no shortcut or
                    // pick reads them.
                    menus::Item::Cmd(label, id, p) if !menus::lists_plugin(id, p) && !menus::shows_a_name(id, label) => out.push(it),
                    _ => {}
                }
            }
        }
        let tree = menus::menu_tree();
        let mut rows = vec![];
        for (_, items) in &tree {
            walk(items, &mut rows);
        }
        let mut checked: Vec<(String, &menus::Item)> = vec![];
        for (_, id, _) in items() {
            let of_id: Vec<&menus::Item> = rows.iter().copied().filter(|it| matches!(it, menus::Item::Cmd(_, c, _) if *c == id)).collect();
            let params: Vec<&serde_json::Value> =
                of_id.iter().filter_map(|it| if let menus::Item::Cmd(_, _, p) = it { Some(p) } else { None }).collect();
            let Some(first) = params.first() else { continue };
            if first.as_object().is_none_or(serde_json::Map::is_empty) || params.iter().any(|p| p != first) {
                continue;
            }
            checked.push((id, of_id[0]));
        }
        assert!(checked.iter().any(|(id, _)| id == "edit.findReplace"), "{:?}", checked.iter().map(|c| &c.0).collect::<Vec<_>>());
        let state = |n: usize| match n.checked_sub(1) {
            None => vectorcraft_engine::Session::new(),
            Some(k) => Fixture::ALL[k].session(),
        };
        let (mut compared, mut dimmed) = (0, 0);
        for (id, item) in &checked {
            for n in 0..=Fixture::ALL.len() {
                let mut picked = VectorcraftApp::new(state(n), crate::Services::default());
                let mut clicked = VectorcraftApp::new(state(n), crate::Services::default());
                picked.ui.status.clear();
                clicked.ui.status.clear();
                // A row the menu leaves out in this state has no click to compare with.
                let Some(menus::Entry::Item(row)) = menus::entry(&clicked, item) else { continue };
                let (target, enabled) = (row.target().expect("the item is implemented"), row.enabled);
                let before = outcome(&picked);
                run_pick(&mut picked, id);
                if enabled {
                    menus::invoke(&mut clicked, &target.0, target.1);
                    assert_eq!(outcome(&picked), outcome(&clicked), "{id} [state {n}]");
                    compared += 1;
                } else {
                    let after = outcome(&picked);
                    assert_eq!((after.0, after.1), (None, before.1), "{id} [state {n}] is dimmed");
                    assert!(after.2.contains("not available"), "{id} [state {n}]: {}", after.2);
                    dimmed += 1;
                }
            }
        }
        assert!(compared > 0 && dimmed > 0, "{compared} compared, {dimmed} dimmed");
    }

    /// A pick of a "…" command whose menu bar item is dimmed (Remove Background and Object Mosaic
    /// with no image selected, Find and Replace with no document open) opens no dialog, changes
    /// nothing and reports why the command can't run.
    #[test]
    fn a_pick_of_a_dimmed_command_says_why_it_cannot_run() {
        let cases = [
            ("addon.image.removeBackground", vectorcraft_testkit::fixtures::Fixture::Multi.session()),
            ("object.createObjectMosaic", vectorcraft_testkit::fixtures::Fixture::Multi.session()),
            ("edit.findReplace", vectorcraft_engine::Session::new()),
        ];
        for (id, session) in cases {
            assert!(crate::shortcut_editor::menu_target(id).is_some(), "{id} has a dialog");
            let mut app = VectorcraftApp::new(session, crate::Services::default());
            assert!(!menus::enabled(&app, id), "{id}");
            app.ui.status.clear();
            let before = app.session.active().map(|st| st.history.undo.len());
            run_pick(&mut app, id);
            assert!(app.ui.dialog.is_none(), "{id}");
            assert_eq!(app.session.active().map(|st| st.history.undo.len()), before, "{id}");
            assert!(app.ui.status.contains("not available"), "{id}: {}", app.ui.status);
        }
    }

    /// A pick of a command that opens no dialog runs it at once.
    #[test]
    fn a_pick_of_a_command_without_a_dialog_runs_at_once() {
        for id in ["object.group", "paint.swap"] {
            assert!(items().iter().any(|(_, item, _)| item == id), "{id} is in the palette");
            assert_eq!(crate::shortcut_editor::menu_target(id), None, "{id}");
            let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
            let before = app.session.active().unwrap().history.undo.len();
            run_pick(&mut app, id);
            assert!(app.ui.dialog.is_none(), "{id}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1, "{id}: {}", app.ui.status);
        }
    }

    /// #6376: a pick goes first in Recent Commands, and a pick of a command already listed moves it
    /// to the top, so each command takes one place. The list is saved with the UI state.
    #[test]
    fn recent_commands_put_the_latest_pick_first_and_list_each_command_once() {
        let mut list = vec![];
        for id in ["object.group", "paint.swap", "object.group"] {
            remember(&mut list, id, |_| true);
        }
        assert_eq!(list, ["object.group", "paint.swap"]);
        // A pick with Enter in the open palette is remembered.
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        app.ui.recent_commands = list;
        app.ui.palette_open = true;
        app.ui.palette_query = "paint.swap".into();
        let first = build(crate::i18n::current()).into_iter().find(|e| e.haystack.contains("paint.swap")).unwrap().id;
        let ctx = egui::Context::default();
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        ctx.run_ui(egui::RawInput { events: vec![enter], ..Default::default() }, |ui| show(&mut app, ui.ctx())).textures_delta.clear();
        assert!(!app.ui.palette_open);
        assert_eq!(app.ui.recent_commands.first(), Some(&first));
        assert_eq!(app.ui.recent_commands.iter().filter(|c| **c == first).count(), 1, "{:?}", app.ui.recent_commands);
        let restored: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&app.ui).unwrap()).unwrap();
        assert_eq!(restored.recent_commands, app.ui.recent_commands);
    }

    /// Recent Commands keeps the last eight picks.
    #[test]
    fn recent_commands_keep_the_last_eight_picks() {
        let mut list = vec![];
        let ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
        for id in ids {
            remember(&mut list, id, |_| true);
        }
        assert_eq!(list, ["j", "i", "h", "g", "f", "e", "d", "c"]);
        // A palette toggle saved before its picks were left out takes none of the eight places.
        let mut list: Vec<String> = [PALETTE_TOGGLE, "a", "b", "c", "d", "e", "f", "g"].map(str::to_string).into();
        remember(&mut list, "h", |_| true);
        assert_eq!(list, ["h", "a", "b", "c", "d", "e", "f", "g"]);
        let entries = build(Lang::EN);
        let many: Vec<String> = entries.iter().take(MAX_RECENT_COMMANDS + 2).map(|e| e.id.clone()).collect();
        assert_eq!(recent(&entries, &many).len(), MAX_RECENT_COMMANDS);
    }

    /// #6376: a command that does not exist now, such as a plug-in's filter while the plug-in is
    /// not loaded, is left out of Recent Commands. A pick keeps it in the saved list, so it shows
    /// again when the command comes back; eight other commands picked after it push it out.
    #[test]
    fn recent_commands_skip_ids_that_no_longer_exist() {
        let entries = build(Lang::EN);
        let known = |c: &str| entries.iter().any(|e| e.id == c);
        let saved: Vec<String> = ["plugin:test.UI.palette_removed", "object.group", "tool:no-such-tool"].map(str::to_string).into();
        let shown: Vec<&str> = recent(&entries, &saved).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(shown, ["object.group"]);
        let mut list = saved;
        remember(&mut list, "paint.swap", known);
        assert_eq!(list, ["paint.swap", "plugin:test.UI.palette_removed", "object.group", "tool:no-such-tool"]);
        let others: Vec<String> =
            entries.iter().filter(|e| e.id != "paint.swap" && e.id != "object.group").take(MAX_RECENT_COMMANDS - 1).map(|e| e.id.clone()).collect();
        for id in &others {
            remember(&mut list, id, known);
        }
        let expected: Vec<String> =
            others.iter().rev().cloned().chain(["paint.swap".to_string(), "plugin:test.UI.palette_removed".to_string()]).collect();
        assert_eq!(list, expected, "object.group, the ninth known id, and the ids after it go");
        // Unavailable ids are capped too.
        let mut list: Vec<String> = (0..50).map(|i| format!("plugin:test.UI.gone{i}")).collect();
        remember(&mut list, "paint.swap", known);
        assert_eq!(list.len(), MAX_SAVED_RECENT_COMMANDS);
        assert_eq!(list.first().map(String::as_str), Some("paint.swap"));
    }

    /// A palette toggle saved in a full Recent Commands list does not take one of its places: after
    /// another pick the list keeps that pick and the seven newest commands before it, and the
    /// toggle is dropped.
    #[test]
    fn a_saved_palette_toggle_does_not_take_a_recent_place() {
        let entries = build(Lang::EN);
        let known = |c: &str| entries.iter().any(|e| e.id == c);
        assert!(known(PALETTE_TOGGLE), "the toggle has a result");
        let others: Vec<String> =
            entries.iter().filter(|e| e.id != PALETTE_TOGGLE && e.id != "paint.swap").take(MAX_RECENT_COMMANDS).map(|e| e.id.clone()).collect();
        let mut list: Vec<String> = std::iter::once(PALETTE_TOGGLE.to_string()).chain(others.iter().cloned()).collect();
        remember(&mut list, "paint.swap", known);
        let expected: Vec<String> = std::iter::once("paint.swap".to_string()).chain(others.iter().take(MAX_RECENT_COMMANDS - 1).cloned()).collect();
        assert_eq!(list, expected);
        let shown: Vec<&str> = recent(&entries, &list).iter().map(|e| e.id.as_str()).collect();
        assert_eq!(shown.len(), MAX_RECENT_COMMANDS);
    }

    /// The open palette drawn over `app`, after the frame's keys go to the shortcuts as in the app,
    /// with a query no result matches, so the only command buttons are the Recent Commands list's.
    fn palette_harness(mut app: VectorcraftApp) -> egui_kittest::Harness<'static, VectorcraftApp> {
        app.ui.palette_open = true;
        app.ui.palette_query = "zzqqxxjj".into();
        let mut fonts = false;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 700.0)).with_accessibility_check(false).build_ui_state(
            move |ui, app: &mut VectorcraftApp| {
                if !std::mem::replace(&mut fonts, true) {
                    crate::theme::install_fonts(ui.ctx());
                    return;
                }
                crate::shortcuts::handle(app, ui.ctx());
                show(app, ui.ctx());
            },
            app,
        );
        h.run_steps(3);
        h
    }

    /// #6376: the open Recent Commands list has the keyboard: its first row takes it, Down moves to
    /// the next row and Enter runs that row, not the highlighted search result.
    #[test]
    fn the_keyboard_moves_through_recent_commands_and_enter_runs_a_row() {
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        app.ui.recent_commands = vec!["object.group".into(), "paint.swap".into()];
        let mut h = palette_harness(app);
        h.state_mut().ui.palette_query = "zoom".into();
        h.run_steps(1);
        h.get_by_label(tl!("Recent Commands")).click();
        h.run_steps(2);
        h.key_press(egui::Key::ArrowDown);
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert!(!h.state().ui.palette_open, "the palette closes");
        assert_eq!(h.state().ui.recent_commands, ["paint.swap", "object.group"]);
    }

    /// #6376: Down on the last Recent Commands row wraps to the first row, and Up on the first
    /// wraps to the last, so the keys never reach the search results below the list.
    #[test]
    fn the_keys_wrap_within_recent_commands() {
        use egui_kittest::kittest::Queryable;
        // (keys pressed after the list opens, the command Enter then runs)
        let cases = [(vec![egui::Key::ArrowDown, egui::Key::ArrowDown], "object.group"), (vec![egui::Key::ArrowUp], "paint.swap")];
        for (keys, ran) in cases {
            let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
            app.ui.recent_commands = vec!["object.group".into(), "paint.swap".into()];
            let mut h = palette_harness(app);
            h.state_mut().ui.palette_query = "zoom".into();
            h.run_steps(1);
            h.get_by_label(tl!("Recent Commands")).click();
            h.run_steps(2);
            for key in &keys {
                h.key_press(*key);
                h.run_steps(2);
            }
            h.key_press(egui::Key::Enter);
            h.run_steps(2);
            assert!(!h.state().ui.palette_open, "{keys:?}: the palette closes");
            assert_eq!(h.state().ui.recent_commands.first().map(String::as_str), Some(ran), "{keys:?}: {:?}", h.state().ui.recent_commands);
            assert_eq!(h.state().ui.recent_commands.len(), 2, "{keys:?}: no search result ran");
        }
    }

    /// #6376: Down and Enter that arrive in the same frame run the row Down moved to, not the row
    /// that had the keyboard before.
    #[test]
    fn down_and_enter_in_one_frame_run_the_row_down_moved_to() {
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        app.ui.recent_commands = vec!["object.group".into(), "paint.swap".into()];
        let mut h = palette_harness(app);
        h.get_by_label(tl!("Recent Commands")).click();
        h.run_steps(2);
        // `key_press` runs a frame for each event, so both keys go into the next frame's input.
        for key in [egui::Key::ArrowDown, egui::Key::Enter] {
            h.input_mut().events.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE });
        }
        h.run_steps(2);
        assert!(!h.state().ui.palette_open, "the palette closes");
        assert_eq!(h.state().ui.recent_commands, ["paint.swap", "object.group"]);
    }

    /// #6376: Tab moves the keyboard from the search field to the Recent Commands button and it
    /// stays there; Enter then opens the list instead of running the highlighted result.
    #[test]
    fn tab_reaches_the_recent_commands_button_and_enter_opens_the_list() {
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        app.ui.recent_commands = vec!["object.group".into()];
        let mut h = palette_harness(app);
        h.state_mut().ui.palette_query = "zoom".into();
        h.run_steps(2);
        h.key_press(egui::Key::Tab);
        h.run_steps(3);
        assert!(h.get_by_label(tl!("Recent Commands")).is_focused(), "the button keeps the keyboard");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert!(h.state().ui.palette_open, "no search result ran");
        let shown = build(crate::i18n::current()).into_iter().find(|e| e.id == "object.group").unwrap().shown;
        assert!(h.query_by(|n| n.label().is_some_and(|l| l.starts_with(&shown))).is_some(), "the list is open");
        assert_eq!(h.state().ui.recent_commands, ["object.group"]);
    }

    /// #6376: another open popup leaves the palette's keys alone: Enter still runs the highlighted
    /// result. Only the Recent Commands list takes them.
    #[test]
    fn another_popup_leaves_enter_to_the_palette() {
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        app.ui.palette_open = true;
        app.ui.palette_query = "paint.swap".into();
        let first = build(crate::i18n::current()).into_iter().find(|e| e.haystack.contains("paint.swap")).unwrap().id;
        let ctx = egui::Context::default();
        let other = egui::Id::unique("another popup");
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        for events in [vec![], vec![enter]] {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                egui::Popup::open_id(ui.ctx(), other);
                show(&mut app, ui.ctx());
            })
            .textures_delta
            .clear();
        }
        assert!(egui::Popup::is_id_open(&ctx, other) && !recent_list_open(&ctx));
        assert!(!app.ui.palette_open, "Enter runs the highlighted result");
        assert_eq!(app.ui.recent_commands, [first]);
    }

    /// #6376: Escape with the Recent Commands list open closes the list and leaves the palette
    /// open; the next Escape closes the palette.
    #[test]
    fn escape_closes_recent_commands_before_the_palette() {
        use egui_kittest::kittest::Queryable;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.ui.recent_commands = vec!["object.group".into()];
        let mut h = palette_harness(app);
        h.get_by_label(tl!("Recent Commands")).click();
        h.run_steps(2);
        let shown = build(crate::i18n::current()).into_iter().find(|e| e.id == "object.group").unwrap().shown;
        let row = |h: &egui_kittest::Harness<'_, VectorcraftApp>| h.query_by(|n| n.label().is_some_and(|l| l.starts_with(&shown))).is_some();
        assert!(row(&h), "the list is open");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(!row(&h), "the list closes");
        assert!(h.state().ui.palette_open, "the palette stays open");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(!h.state().ui.palette_open, "the next Escape closes the palette");
    }

    /// #6376: a choice in Recent Commands closes the palette and runs what a pick of the same
    /// result runs (here Clean Up… opens its dialog), and moves the command to the top.
    #[test]
    fn a_recent_command_runs_what_its_pick_runs() {
        use egui_kittest::kittest::Queryable;
        let id = "object.path.cleanUp";
        let session = || vectorcraft_testkit::fixtures::Fixture::Single.session();
        let mut picked = VectorcraftApp::new(session(), crate::Services::default());
        picked.ui.status.clear();
        run_pick(&mut picked, id);
        assert!(picked.ui.dialog.is_some(), "{}", picked.ui.status);

        let mut app = VectorcraftApp::new(session(), crate::Services::default());
        app.ui.status.clear();
        app.ui.recent_commands = vec!["object.group".into(), id.into()];
        // A result's button is named by its label and its shortcut.
        let e = build(crate::i18n::current()).into_iter().find(|e| e.id == id).unwrap();
        let name = format!("{} {}", e.shown, menus::shown_shortcut(&e.shortcut));
        let mut h = palette_harness(app);
        h.get_by_label(tl!("Recent Commands")).click();
        h.run_steps(2);
        h.get_by_label(&name).click();
        h.run_steps(2);
        let app = h.state();
        assert!(!app.ui.palette_open, "the palette closes");
        assert_eq!(outcome(app), outcome(&picked));
        assert_eq!(app.ui.recent_commands, [id, "object.group"]);
    }

    /// A result picked recently ranks first among the results that match the query as well as it
    /// does: "rect" lists the Rectangle Tool above Effect › Convert to Shape › Rectangle…, and after
    /// a pick of the effect, the effect comes first, so Enter runs it again.
    #[test]
    fn a_recent_pick_ranks_first_among_results_that_match_as_well() {
        let (tool, effect) = ("tool:rectangle", "effect:convertToShape.rectangle");
        let mut open = Open::new("rect", egui::vec2(1200.0, 900.0));
        // A selected path, so the effect can run: a refused pick is not recorded.
        open.app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        open.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
        open.frame(vec![]);
        let before = open.listed();
        assert_eq!(before.first().map(String::as_str), Some(tool));
        let at = before.iter().position(|id| id == effect).unwrap();
        open.press(egui::Key::ArrowDown, at);
        open.press(egui::Key::Enter, 1);
        assert_eq!(open.app.ui.recent_commands, [effect]);

        open.app.ui.dialog = None;
        open.app.ui.palette_open = true;
        open.app.ui.palette_query = "rect".into();
        let nodes = open.frame(vec![]);
        let after = open.listed();
        assert_eq!(after.first().map(String::as_str), Some(effect));
        assert_eq!(after.get(1).map(String::as_str), Some(tool), "only the pick moves");
        let label = open.said(effect);
        assert_eq!(field_place(&nodes), format!("1 of {}: {label}", after.len()));
        open.press(egui::Key::Enter, 1);
        assert!(open.app.ui.dialog.is_some(), "Enter ran the effect again: {}", open.app.ui.status);
        assert_eq!(open.app.ui.recent_commands, [effect]);
    }

    /// The kept ranking of a query is ranked again when the recent picks change, and a saved id
    /// with no result now is passed over.
    #[test]
    fn the_kept_ranking_follows_the_recent_picks() {
        let ctx = egui::Context::default();
        let entries = entries(&ctx, Lang::EN);
        let ids = |recent: &[String]| matching(&entries, "rect", recent).into_iter().map(|e| e.id.clone()).collect::<Vec<_>>();
        let plain = ids(&[]);
        assert_eq!(plain.first().map(String::as_str), Some("tool:rectangle"));
        let effect = "effect:convertToShape.rectangle".to_string();
        assert_eq!(ids(std::slice::from_ref(&effect)).first(), Some(&effect));
        assert_eq!(ids(&[]), plain);
        assert_eq!(ids(&["plugin:test.UI.palette_removed".to_string(), effect.clone()]).first(), Some(&effect));
    }

    /// Pick counts for the ids `counts`, picked in that order.
    fn counted(counts: &[(&str, u32)]) -> CommandUses {
        let uses = counts.iter().zip(1..).map(|(&(id, n), last)| (id.to_string(), (n, last))).collect();
        CommandUses { pick: u32::try_from(counts.len()).unwrap(), uses }
    }

    /// A command picked often ranks first among the results that match the query as well as it
    /// does, also when it is no longer a recent pick, and the count is saved with the other
    /// interface settings.
    #[test]
    fn a_command_picked_often_ranks_first_among_results_that_match_as_well() {
        let (tool, effect) = ("tool:rectangle", "effect:convertToShape.rectangle");
        let mut open = Open::new("rect", egui::vec2(1200.0, 900.0));
        // The fixture has art selected, so the effect is not dimmed and its pick is counted.
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        open.app.ui.palette_open = true;
        open.app.ui.palette_query = "rect".into();
        open.frame(vec![]);
        let before = open.listed();
        assert_eq!(before.first().map(String::as_str), Some(tool));
        let at = before.iter().position(|id| id == effect).unwrap();
        open.press(egui::Key::ArrowDown, at);
        open.press(egui::Key::Enter, 1);
        assert_eq!(open.app.ui.command_uses.grade(effect), 1);
        assert_eq!(open.app.ui.command_uses.grade(tool), 0);

        open.app.ui.recent_commands.clear();
        open.app.ui.dialog = None;
        open.app.ui.palette_open = true;
        open.app.ui.palette_query = "rect".into();
        open.frame(vec![]);
        let after = open.listed();
        assert_eq!(after.first().map(String::as_str), Some(effect));
        assert_eq!(after.get(1).map(String::as_str), Some(tool), "only the pick moves");
        let restored: crate::state::UiState = serde_json::from_str(&serde_json::to_string(&open.app.ui).unwrap()).unwrap();
        assert_eq!(restored.command_uses, open.app.ui.command_uses);
    }

    /// The kept ranking of a query is ranked again when a pick count changes.
    #[test]
    fn the_kept_ranking_follows_the_pick_counts() {
        let ctx = egui::Context::default();
        let entries = entries(&ctx, Lang::EN);
        let first = |uses: &CommandUses| super::matching(&entries, "rect", &[], uses).first().map(|e| e.id.clone());
        let effect = "effect:convertToShape.rectangle";
        assert_eq!(first(&CommandUses::default()).as_deref(), Some("tool:rectangle"));
        let mut uses = counted(&[("tool:rectangle", 1)]);
        assert_eq!(first(&uses).as_deref(), Some("tool:rectangle"));
        uses.count(effect);
        assert_eq!(first(&uses).as_deref(), Some("tool:rectangle"), "the same grade: the catalog order");
        uses.count(effect);
        uses.count(effect);
        assert_eq!(first(&uses).as_deref(), Some(effect), "three picks reach grade 2");
        assert_eq!(first(&CommandUses::default()).as_deref(), Some("tool:rectangle"));
    }

    /// A count does not lift a result above one that matches the query better: the exact id
    /// `view.goto1` lists Saved View 1 before Saved View 10, however often that was picked.
    #[test]
    fn a_count_does_not_pass_a_better_match() {
        let ctx = egui::Context::default();
        let entries = entries(&ctx, Lang::EN);
        let uses = counted(&[("view.goto10", 999)]);
        let ids: Vec<&str> = super::matching(&entries, "view.goto1", &[], &uses).into_iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids.first(), Some(&"view.goto1"), "{ids:?}");
        assert!(ids.contains(&"view.goto10"), "{ids:?}");
    }

    /// Counts grow in grades that double: 1 pick gives grade 1, 3 give 2, 7 give 3, and 999 give 9.
    #[test]
    fn pick_counts_rank_in_grades_that_double() {
        let mut uses = CommandUses::default();
        let grades: Vec<u8> = (0..8)
            .map(|_| {
                let g = uses.grade("a");
                uses.count("a");
                g
            })
            .collect();
        assert_eq!(grades, [0, 1, 1, 2, 2, 2, 2, 3]);
        assert_eq!(counted(&[("a", 999)]).grade("a"), 9);
        assert_eq!(counted(&[("a", u32::MAX)]).grade("a"), 9);
        assert!(CommandUses::default().grades(&[]).is_empty(), "no grades without counts");
    }

    /// A new id when 200 are counted removes, of the 100 ids picked longest ago, the id with the
    /// lowest count, of those the one picked longest ago, and keeps the new one.
    #[test]
    fn the_201st_command_removes_the_least_used_and_keeps_itself() {
        let ids: Vec<String> = (0..MAX_COUNTED_COMMANDS).map(|i| format!("c{i}")).collect();
        let counts: Vec<(&str, u32)> = ids.iter().enumerate().map(|(i, id)| (id.as_str(), if i == 5 || i == 9 { 1 } else { 2 })).collect();
        let mut uses = counted(&counts);
        uses.count("new");
        assert_eq!(uses.uses.len(), MAX_COUNTED_COMMANDS);
        assert!(!uses.uses.contains_key("c5") && uses.uses.contains_key("c9"));
        assert_eq!(uses.uses.get("new"), Some(&(1, 201)));
        uses.count("newer");
        assert!(!uses.uses.contains_key("c9"), "c9 was picked before new");
        assert!(uses.uses.contains_key("new") && uses.uses.contains_key("newer"));
        assert_eq!(uses.uses.len(), MAX_COUNTED_COMMANDS);
    }

    /// When all 200 counted ids have 2 picks or more, new ids picked in turn keep their places and
    /// their counts grow; the ids removed are the ones picked longest ago.
    #[test]
    fn new_commands_build_up_counts_when_every_counted_one_has_more_picks() {
        let ids: Vec<String> = (0..MAX_COUNTED_COMMANDS).map(|i| format!("c{i}")).collect();
        let counts: Vec<(&str, u32)> = ids.iter().map(|id| (id.as_str(), 2)).collect();
        let mut uses = counted(&counts);
        for _ in 0..3 {
            for id in ["a", "b", "c"] {
                uses.count(id);
            }
        }
        assert_eq!(uses.uses.len(), MAX_COUNTED_COMMANDS);
        for id in ["a", "b", "c"] {
            assert_eq!(uses.uses.get(id).map(|&(n, _)| n), Some(3), "{id}");
        }
        assert!(!uses.uses.contains_key("c0") && !uses.uses.contains_key("c2") && uses.uses.contains_key("c3"));
        assert!(uses.uses.contains_key("c199"));
    }

    /// When a count reaches 1,000, every count is halved and ids left at 0 are removed.
    #[test]
    fn counts_are_halved_at_one_thousand() {
        let mut uses = counted(&[("a", 999), ("b", 1), ("c", 5)]);
        uses.count("a");
        assert_eq!(uses.uses.get("a"), Some(&(500, 4)));
        assert_eq!(uses.uses.get("c"), Some(&(2, 3)));
        assert!(!uses.uses.contains_key("b"));
    }

    /// At the pick number's limit, the counted ids' pick numbers are renumbered from 1 in their
    /// order, so a pick still takes a number above all of them and a command picked again leaves
    /// the half of the ids picked longest ago.
    #[test]
    fn picks_keep_their_order_at_the_pick_number_limit() {
        let ids: Vec<String> = (0..MAX_COUNTED_COMMANDS).map(|i| format!("c{i:03}")).collect();
        let mut uses = CommandUses { pick: u32::MAX, uses: ids.iter().map(|id| (id.clone(), (2, u32::MAX))).collect() };
        uses.uses.insert("c000".into(), (1, u32::MAX));
        uses.count("c000");
        assert_eq!(uses.uses.get("c000"), Some(&(2, 2)));
        assert_eq!(uses.uses.get("c001"), Some(&(2, 1)));
        uses.count("new");
        assert!(uses.uses.contains_key("c000"), "c000 was picked last: {:?}", uses.uses.get("c000"));
        assert!(!uses.uses.contains_key("c001"));
        assert_eq!(uses.uses.get("new"), Some(&(1, 3)));
        // Equal pick numbers stay equal, and the order of the others is kept.
        let mut uses =
            CommandUses { pick: u32::MAX, uses: [("a", (1, 7)), ("b", (1, u32::MAX)), ("c", (1, 7))].map(|(k, v)| (k.to_string(), v)).into() };
        uses.count("d");
        assert_eq!(
            uses,
            CommandUses { pick: 3, uses: [("a", (1, 1)), ("b", (1, 2)), ("c", (1, 1)), ("d", (1, 3))].map(|(k, v)| (k.to_string(), v)).into() }
        );
    }

    /// Saved counts load what is readable: a count above 999 reads as 999, entries that are not a
    /// pair of counts are dropped, of more than 200 ids the most recent picks are kept, and a value
    /// of the wrong kind, or none (an older settings file), reads as no picks.
    #[test]
    fn saved_pick_counts_load_what_is_readable() {
        let load = |v: serde_json::Value| serde_json::from_value::<crate::state::UiState>(json!({ "command_uses": v })).unwrap().command_uses;
        let uses = load(json!({
            "pick": 3,
            "uses": {"a": [5000, 2], "b": "x", "c": [1], "d": [0, 1], "e": [2, 3, 4], "f": [-1, 2], "g": [4, 1], "h": [1.5, 1], "i": [3, 9]}
        }));
        assert_eq!(uses, CommandUses { pick: 9, uses: [("a", (999, 2)), ("g", (4, 1)), ("i", (3, 9))].map(|(k, v)| (k.to_string(), v)).into() });
        for wrong in [json!("x"), json!(null), json!([1, 2]), json!({"uses": [1]}), json!({"pick": "x", "uses": {}})] {
            assert_eq!(load(wrong.clone()), CommandUses::default(), "{wrong}");
        }
        let older: crate::state::UiState = serde_json::from_value(json!({})).unwrap();
        assert_eq!(older.command_uses, CommandUses::default());
        let many: serde_json::Map<String, serde_json::Value> = (1..=250).map(|i| (format!("c{i}"), json!([1, i]))).collect();
        let uses = load(json!({ "pick": 250, "uses": many }));
        assert_eq!(uses.uses.len(), MAX_COUNTED_COMMANDS);
        assert!(uses.uses.contains_key("c51") && !uses.uses.contains_key("c50"));
        // The next pick is the most recent, also when the saved pick number is behind.
        let mut uses = load(json!({ "pick": 1, "uses": {"a": [1, 9]} }));
        uses.count("b");
        assert_eq!(uses.uses.get("b"), Some(&(1, 10)));
        // Saved pick numbers above the largest the counts hold are compared as saved, and the kept
        // ones are renumbered from 1 in their order.
        let big = u64::from(u32::MAX) + 1;
        let mut many: serde_json::Map<String, serde_json::Value> = (0..MAX_COUNTED_COMMANDS).map(|i| (format!("c{i:03}"), json!([1, big]))).collect();
        many.insert("z".into(), json!([1, big + 1]));
        let uses = load(json!({ "pick": big + 1, "uses": many }));
        assert_eq!(uses.uses.len(), MAX_COUNTED_COMMANDS);
        assert_eq!(uses.uses.get("z"), Some(&(1, 2)), "the newest pick is kept");
        assert_eq!(uses.uses.get("c000"), Some(&(1, 1)));
        assert_eq!(uses.pick, 2);
        let uses = load(json!({ "pick": big, "uses": {"a": [1, 5], "b": [1, 3]} }));
        assert_eq!(uses, CommandUses { pick: 2, uses: [("a", (1, 2)), ("b", (1, 1))].map(|(k, v)| (k.to_string(), v)).into() });
    }

    /// A pick is counted only when Recent Commands records it: a refused pick and a pick of the
    /// palette's own toggle are not counted, and a pick that opens a dialog is.
    #[test]
    fn only_a_pick_recorded_in_recent_commands_is_counted() {
        for (query, id) in [("find and replace", "edit.findReplace"), ("search commands", PALETTE_TOGGLE)] {
            let mut open = Open::new(query, WINDOW);
            let k = open.listed().iter().position(|c| c == id).unwrap_or_else(|| panic!("{query:?} finds {id}"));
            open.frame(vec![]);
            open.press(egui::Key::ArrowDown, k);
            assert_eq!(open.highlighted(), k, "{id}");
            open.press(egui::Key::Enter, 1);
            assert!(open.app.ui.recent_commands.is_empty(), "{id}");
            assert_eq!(open.app.ui.command_uses, CommandUses::default(), "{id}");
        }
        let id = "object.path.cleanUp";
        let mut open = Open::new("clean up", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
        open.app.ui.palette_open = true;
        open.app.ui.palette_query = "clean up".into();
        let k = open.listed().iter().position(|c| c == id).expect("the search finds Clean Up…");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        open.press(egui::Key::Enter, 1);
        assert!(open.app.ui.dialog.is_some());
        assert_eq!(open.app.ui.recent_commands, [id]);
        assert_eq!(open.app.ui.command_uses.grade(id), 1);
    }

    /// A refused pick leaves the recent picks and the pick counts saved before it as they were.
    #[test]
    fn a_refused_pick_leaves_the_earlier_counts_as_they_were() {
        let id = "edit.findReplace";
        let mut open = Open::new("find and replace", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into()];
        for _ in 0..2 {
            open.app.ui.command_uses.count("paint.swap");
        }
        open.app.ui.command_uses.count("object.group");
        let (recent, uses) = (open.app.ui.recent_commands.clone(), open.app.ui.command_uses.clone());
        let k = open.listed().iter().position(|c| c == id).expect("the search finds Find and Replace…");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        open.press(egui::Key::Enter, 1);
        assert!(open.app.ui.dialog.is_none(), "no document: the pick is refused");
        assert_eq!(open.app.ui.recent_commands, recent);
        assert_eq!(open.app.ui.command_uses, uses);
        assert_eq!(open.app.ui.command_uses.grade(id), 0);
    }

    /// #6376: with nothing picked yet, Recent Commands shows a dimmed No recent commands row.
    #[test]
    fn recent_commands_without_picks_show_a_dimmed_row() {
        use egui_kittest::kittest::{NodeT, Queryable};
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let mut h = palette_harness(app);
        assert!(h.query_by_label(tl!("No recent commands")).is_none(), "the list opens on a click");
        h.get_by_label(tl!("Recent Commands")).click();
        h.run_steps(2);
        assert!(h.get_by_label(tl!("No recent commands")).accesskit_node().is_disabled());
        assert!(h.state().ui.palette_open);
    }

    /// The row labels the palette shows for the commands `ids`, in that order.
    fn row_labels(open: &Open, ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| open.said(id)).collect()
    }

    /// An empty search, or one of spaces only, lists the recent picks under a Recent heading, most
    /// recent first, with the first highlighted. A pick whose command is gone is left
    /// out, and no count is shown.
    #[test]
    fn an_empty_search_lists_the_recent_picks_most_recent_first() {
        for query in ["", "   "] {
            let mut open = Open::new("select", WINDOW);
            open.app.ui.recent_commands = ["paint.swap", "plugin:test.UI.palette_gone", "object.group"].map(str::to_string).into();
            open.app.ui.palette_query = query.into();
            let nodes = open.frame(vec![]);
            let shown: Vec<(String, bool)> = rows(&nodes).into_iter().map(|r| (r.0, r.1)).collect();
            let expected = row_labels(&open, &["paint.swap", "object.group"]);
            assert_eq!(shown, [(expected[0].clone(), true), (expected[1].clone(), false)], "{query:?}");
            let labels = labels(&nodes);
            assert!(labels.iter().any(|l| l == tl!("Recent")), "{query:?}: the heading: {labels:?}");
            assert!(!labels.iter().any(|l| l.contains("result")), "{query:?}: no count: {labels:?}");
        }
    }

    /// An empty search with nothing picked yet lists no rows and says what a search finds.
    #[test]
    fn an_empty_search_without_picks_says_what_to_type() {
        let mut open = Open::new("", WINDOW);
        let nodes = open.frame(vec![]);
        assert!(rows(&nodes).is_empty(), "{:?}", rows(&nodes));
        let labels = labels(&nodes);
        assert!(labels.iter().any(|l| l == tl!("Type to search commands, tools, panels and effects")), "{labels:?}");
        let other = [tl!("No commands match"), tl!("Recent"), tl!("No recent commands")];
        assert!(!labels.iter().any(|l| other.contains(&l.as_str())), "{labels:?}");
        assert!(open.app.ui.palette_open);
    }

    /// Enter on an empty search runs the last pick again, and Recent Commands keeps its order.
    #[test]
    fn enter_on_an_empty_search_runs_the_last_pick_again() {
        let mut open = Open::new("", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into()];
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        let before = open.app.session.active().unwrap().history.undo.len();
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert_eq!(open.app.session.active().unwrap().history.undo.len(), before + 1, "{}", open.app.ui.status);
        assert_eq!(open.app.ui.recent_commands, ["paint.swap", "object.group"]);
    }

    /// Text typed into an empty search in the frame that presses Enter runs the first result of the
    /// new query, not the last pick the empty search listed.
    #[test]
    fn enter_with_text_typed_in_its_frame_runs_the_new_querys_first_result() {
        let mut open = Open::new("", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        open.app.ui.recent_commands = vec!["paint.swap".into()];
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        open.frame(vec![]);
        let entries = entries(&open.ctx, Lang::EN);
        let first = super::matching(&entries, "select all", &open.app.ui.recent_commands, &open.app.ui.command_uses)[0].id.clone();
        assert_ne!(first, "paint.swap");
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        open.frame(vec![egui::Event::Text("select all".into()), enter]);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert_eq!(open.app.ui.recent_commands.first(), Some(&first), "{}", open.app.ui.status);
    }

    /// A pick of Clear Recent Commands empties the recent picks without keeping itself, so the
    /// empty search then says what to type, and the saved UI state holds an empty list.
    #[test]
    fn clear_recent_commands_empties_the_recent_picks() {
        const CLEAR: &str = "help.clearRecentCommands";
        let mut open = Open::new("clear recent commands", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into()];
        assert!(menus::enabled(&open.app, CLEAR));
        let k = open.listed().iter().position(|id| id == CLEAR).expect("the command's result");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        assert_eq!(open.highlighted(), k);
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert!(open.app.ui.recent_commands.is_empty(), "{:?}", open.app.ui.recent_commands);
        assert!(!menus::enabled(&open.app, CLEAR), "not enabled with nothing to clear");
        let saved: serde_json::Value = serde_json::to_value(&open.app.ui).unwrap();
        assert_eq!(saved["recent_commands"], serde_json::json!([]));
        open.app.ui.palette_query.clear();
        open.app.ui.palette_open = true;
        let nodes = open.frame(vec![]);
        assert!(rows(&nodes).is_empty(), "{:?}", rows(&nodes));
        let labels = labels(&nodes);
        assert!(labels.iter().any(|l| l == tl!("Type to search commands, tools, panels and effects")), "{labels:?}");
        // Run as a command, it empties the list too.
        open.app.ui.recent_commands = vec!["paint.swap".into()];
        open.app.run(CLEAR, serde_json::json!({})).unwrap();
        assert!(open.app.ui.recent_commands.is_empty());
        // A saved id the list leaves out (a plug-in that is not loaded) still counts, and is cleared.
        open.app.ui.recent_commands = vec!["plugin.notLoaded.filter".into()];
        assert!(menus::enabled(&open.app, CLEAR));
        open.app.run(CLEAR, serde_json::json!({})).unwrap();
        assert!(open.app.ui.recent_commands.is_empty());
    }

    /// Clear Recent Commands also empties the pick counts, and a pick of it from the search is not
    /// counted afterward; with counts left and no recent picks, it is still enabled.
    #[test]
    fn clear_recent_commands_also_clears_the_pick_counts() {
        const CLEAR: &str = "help.clearRecentCommands";
        let mut open = Open::new("clear recent commands", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into()];
        for _ in 0..3 {
            open.app.ui.command_uses.count("paint.swap");
        }
        open.app.ui.command_uses.count("object.group");
        let k = open.listed().iter().position(|id| id == CLEAR).expect("the command's result");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        assert_eq!(open.highlighted(), k);
        open.press(egui::Key::Enter, 1);
        assert!(open.app.ui.recent_commands.is_empty(), "{:?}", open.app.ui.recent_commands);
        assert_eq!(open.app.ui.command_uses, CommandUses::default(), "the pick of Clear is not counted either");
        let saved: crate::state::UiState = serde_json::from_value(serde_json::to_value(&open.app.ui).unwrap()).unwrap();
        assert_eq!(saved.command_uses, CommandUses::default());
        assert!(!menus::enabled(&open.app, CLEAR));
        // Counts without recent picks (Recent Commands emptied some other way) still enable it,
        // and running it as a command empties them.
        open.app.ui.command_uses.count("paint.swap");
        assert!(menus::enabled(&open.app, CLEAR), "enabled while a count is saved");
        open.app.run(CLEAR, serde_json::json!({})).unwrap();
        assert_eq!(open.app.ui.command_uses, CommandUses::default());
        assert_eq!(open.app.ui.command_uses.grade("paint.swap"), 0);
    }

    /// The palette opens again with its first row highlighted, even when it shows the same rows.
    #[test]
    fn the_palette_opens_again_with_its_first_row_highlighted() {
        let mut open = Open::new("", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into(), "view.zoomIn".into()];
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, 2);
        assert_eq!(open.highlighted(), 2);
        open.app.ui.palette_open = false;
        open.frame(vec![]);
        open.app.ui.palette_open = true;
        let nodes = open.frame(vec![]);
        assert_eq!(open.highlighted(), 0);
        assert!(rows(&nodes).first().is_some_and(|r| r.1), "{:?}", rows(&nodes));
        // A list scrolled down shows its first row again, highlighted, when the palette opens with
        // the same search.
        open.app.ui.palette_query = "select".into();
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, VISIBLE_ROWS + 5);
        assert_eq!(open.highlighted(), VISIBLE_ROWS + 5);
        open.app.ui.palette_open = false;
        open.frame(vec![]);
        open.app.ui.palette_open = true;
        let nodes = open.frame(vec![]);
        let first = open.listed().first().cloned().unwrap();
        let shown = rows(&nodes);
        let top = shown.iter().find(|r| r.1).expect("a row is highlighted");
        assert_eq!(top.0, row_labels(&open, &[&first])[0], "{shown:?}");
        assert!(top.2 >= f64::from(open.popup().top()), "the first row is in view: {shown:?}");
    }

    /// A palette closed by a click outside it and opened again in the next frame, as the search
    /// icon does, starts at its first row, so Enter on an empty search runs the last pick.
    #[test]
    fn the_palette_reopened_in_the_next_frame_starts_at_its_first_row() {
        let mut open = Open::new("", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into()];
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, 1);
        assert_eq!(open.highlighted(), 1);
        let at = egui::pos2(WINDOW.x - 4.0, WINDOW.y - 4.0);
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        open.frame(vec![egui::Event::PointerMoved(at), button(true), button(false)]);
        assert!(!open.app.ui.palette_open, "the click outside closes the palette");
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        assert_eq!(open.highlighted(), 0);
        let before = open.app.session.active().unwrap().history.undo.len();
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert_eq!(open.app.session.active().unwrap().history.undo.len(), before + 1, "{}", open.app.ui.status);
        assert_eq!(open.app.ui.recent_commands, ["paint.swap", "object.group"]);
    }

    /// A palette that its command closes and opens again before it is drawn closed starts at its
    /// first row, so Enter on an empty search runs the last pick.
    #[test]
    fn the_palette_toggled_closed_and_open_between_frames_starts_at_its_first_row() {
        let mut open = Open::new("", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into()];
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, 1);
        assert_eq!(open.highlighted(), 1);
        open.app.run("help.commandPalette", json!({})).unwrap();
        assert!(!open.app.ui.palette_open, "the command closes the palette");
        open.app.run("help.commandPalette", json!({})).unwrap();
        assert!(open.app.ui.palette_open, "and opens it again");
        open.frame(vec![]);
        assert_eq!(open.highlighted(), 0);
        let before = open.app.session.active().unwrap().history.undo.len();
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert_eq!(open.app.session.active().unwrap().history.undo.len(), before + 1, "{}", open.app.ui.status);
        assert_eq!(open.app.ui.recent_commands, ["paint.swap", "object.group"]);
    }

    /// A pick of the palette's own toggle is not recorded, and one saved earlier is not listed, so
    /// Enter on an empty search runs the last other pick.
    #[test]
    fn the_palette_toggle_is_not_a_recent_pick() {
        let mut open = Open::new("search commands", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into()];
        let k = open.listed().iter().position(|id| id == PALETTE_TOGGLE).expect("the toggle's result");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        assert_eq!(open.highlighted(), k);
        open.press(egui::Key::Enter, 1);
        assert_eq!(open.app.ui.recent_commands, ["paint.swap"]);
        open.app.ui.recent_commands = vec![PALETTE_TOGGLE.into(), "paint.swap".into()];
        open.app.ui.palette_query.clear();
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        assert_eq!(open.listed(), ["paint.swap"]);
        let nodes = open.frame(vec![]);
        let shown: Vec<(String, bool)> = rows(&nodes).into_iter().map(|r| (r.0, r.1)).collect();
        assert_eq!(shown, [(row_labels(&open, &["paint.swap"])[0].clone(), true)]);
    }

    /// A click on a lower row of the empty search's list runs that pick and moves it to the top of
    /// Recent Commands.
    #[test]
    fn a_click_on_a_recent_pick_runs_it_and_moves_it_first() {
        let mut open = Open::new("", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        open.app.ui.recent_commands = vec!["object.group".into(), "paint.swap".into()];
        open.app.ui.palette_open = true;
        let nodes = open.frame(vec![]);
        let row = rows(&nodes).into_iter().nth(1).expect("the second pick's row");
        assert_eq!(row.0, row_labels(&open, &["paint.swap"])[0]);
        let before = open.app.session.active().unwrap().history.undo.len();
        let at = egui::pos2(open.popup().right() - 24.0, ((row.2 + row.3) / 2.0) as f32);
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        open.frame(vec![egui::Event::PointerMoved(at)]);
        open.frame(vec![button(true)]);
        open.frame(vec![button(false)]);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert_eq!(open.app.session.active().unwrap().history.undo.len(), before + 1, "{}", open.app.ui.status);
        assert_eq!(open.app.ui.recent_commands, ["paint.swap", "object.group"]);
    }

    /// A pick says what it did: it ran, it opened a dialog, or it was refused because its menu item
    /// is dimmed or its panel can't be shown.
    #[test]
    fn a_pick_says_whether_it_ran_opened_a_dialog_or_was_refused() {
        use vectorcraft_testkit::fixtures::Fixture;
        let app = |session| VectorcraftApp::new(session, crate::Services::default());
        assert_eq!(run_pick(&mut app(Fixture::Multi.session()), "object.group"), Picked::Ran);
        assert_eq!(run_pick(&mut app(Fixture::Multi.session()), "tool:knife"), Picked::Ran);
        assert_eq!(run_pick(&mut app(Fixture::Multi.session()), "panel:swatches"), Picked::Ran);
        let mut dialog = app(Fixture::Single.session());
        assert_eq!(run_pick(&mut dialog, "object.path.cleanUp"), Picked::OpenedDialog);
        assert!(dialog.ui.dialog.is_some());
        for id in ["edit.findReplace", "object.group"] {
            assert_eq!(run_pick(&mut app(vectorcraft_engine::Session::new()), id), Picked::Refused, "{id}");
        }
        let mut presenting = app(Fixture::Multi.session());
        presenting.ui.screen_mode = 3;
        assert_eq!(run_pick(&mut presenting, "panel:swatches"), Picked::Refused);
        assert_eq!(run_pick(&mut app(Fixture::Multi.session()), "panel:no-such-panel"), Picked::Refused);
        let mut stuck = app(Fixture::Multi.session());
        stuck.fail_next_tool_choice = Some("no switch".into());
        assert_eq!(run_pick(&mut stuck, "tool:knife"), Picked::Refused);
        assert_eq!(stuck.ui.status, "no switch");
        // With nothing selected, the Effect menu dims every effect, and a plug-in's filter is dimmed.
        let effect = crate::shortcut_editor::effect_items().first().map(|e| format!("effect:{}", e.effect)).expect("an effect");
        let mut empty = app(vectorcraft_engine::Session::new());
        assert_eq!(run_pick(&mut empty, &effect), Picked::Refused, "{effect}");
        assert!(empty.ui.dialog.is_none(), "{effect}");
        assert_eq!(run_pick(&mut app(vectorcraft_engine::Session::new()), "plugin:test.UI.palette_missing"), Picked::Refused);
    }

    /// A pick that is refused while its menu item is dimmed is not recorded in Recent Commands,
    /// from a search or from the recent picks, which keep their order.
    #[test]
    fn a_refused_pick_is_not_recorded() {
        for (query, id) in [("find and replace", "edit.findReplace"), ("group", "object.group")] {
            let mut open = Open::new(query, WINDOW);
            open.app.ui.recent_commands = vec!["paint.swap".into()];
            let k = open.listed().iter().position(|c| c == id).unwrap_or_else(|| panic!("{query:?} finds {id}"));
            open.frame(vec![]);
            open.press(egui::Key::ArrowDown, k);
            assert_eq!(open.highlighted(), k, "{id}");
            open.app.ui.status.clear();
            open.press(egui::Key::Enter, 1);
            assert!(!open.app.ui.palette_open, "{id}: the palette closes");
            assert!(open.app.ui.dialog.is_none(), "{id}");
            assert_eq!(open.app.ui.recent_commands, ["paint.swap"], "{id}");
        }
        let mut open = Open::new("", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into()];
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, 1);
        assert_eq!(open.highlighted(), 1);
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open, "the palette closes");
        assert_eq!(open.app.ui.recent_commands, ["paint.swap", "object.group"]);
        // A tool that can't be chosen now is not recorded either.
        let mut open = Open::new("knife", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into()];
        let k = open.listed().iter().position(|c| c == "tool:knife").expect("the search finds the Knife");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        open.app.fail_next_tool_choice = Some("no switch".into());
        open.press(egui::Key::Enter, 1);
        assert_eq!(open.app.ui.status, "no switch");
        assert_eq!(open.app.ui.recent_commands, ["paint.swap"]);
    }

    /// A pick that opens a dialog is recorded in Recent Commands when the dialog opens, so it stays
    /// there when the dialog is canceled.
    #[test]
    fn a_pick_that_opens_a_dialog_is_recorded_even_when_canceled() {
        let id = "object.path.cleanUp";
        let mut open = Open::new("clean up", WINDOW);
        open.app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
        open.app.ui.palette_open = true;
        open.app.ui.palette_query = "clean up".into();
        open.app.ui.recent_commands = vec!["paint.swap".into()];
        let k = open.listed().iter().position(|c| c == id).expect("the search finds Clean Up…");
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, k);
        open.press(egui::Key::Enter, 1);
        assert!(open.app.ui.dialog.is_some(), "{}", open.app.ui.status);
        assert_eq!(open.app.ui.recent_commands, [id, "paint.swap"]);
        // Cancel closes the dialog without running the command.
        crate::dialogs::cancel(&mut open.app);
        assert!(open.app.ui.dialog.is_none());
        open.frame(vec![]);
        assert_eq!(open.app.ui.recent_commands, [id, "paint.swap"]);
    }

    /// A palette closed by a press outside it and opened again before a closed frame is drawn
    /// starts with its first row highlighted and asks its results again whether they can run.
    #[test]
    fn the_palette_opened_again_at_once_after_closing_starts_afresh() {
        let mut open = Open::new("", WINDOW);
        open.app.ui.recent_commands = vec!["paint.swap".into(), "object.group".into(), "view.zoomIn".into()];
        open.frame(vec![]);
        open.press(egui::Key::ArrowDown, 2);
        assert_eq!(open.highlighted(), 2);
        let at = egui::pos2(2.0, 2.0);
        assert!(!open.popup().contains(at), "{at:?} is outside {:?}", open.popup());
        let press = egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        open.frame(vec![egui::Event::PointerMoved(at), press]);
        assert!(!open.app.ui.palette_open, "the press outside closes the palette");
        let availability = egui::Id::unique("palette-availability");
        assert!(open.ctx.data(|d| d.get_temp::<Availability>(availability)).is_none(), "the answers go when the palette closes");
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        assert_eq!(open.highlighted(), 0, "the first row is highlighted");
    }

    /// #1757: Up and Down move the highlighted result, wrapping at the ends; a new query starts
    /// from the first again.
    #[test]
    fn arrow_keys_move_the_highlighted_result() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.ui.palette_open = true;
        app.ui.palette_query = "zoom".into();
        let ctx = egui::Context::default();
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        let frame = |app: &mut VectorcraftApp, events: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| show(app, ui.ctx())).textures_delta.clear();
            ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap().0
        };
        assert_eq!(frame(&mut app, vec![]), 0);
        assert_eq!(frame(&mut app, vec![key(egui::Key::ArrowDown)]), 1);
        assert_eq!(frame(&mut app, vec![key(egui::Key::ArrowUp)]), 0);
        let last = frame(&mut app, vec![key(egui::Key::ArrowUp)]);
        assert!(last > 1, "Up on the first result wraps to the last: {last}");
        app.ui.palette_query = "zoom in".into();
        assert_eq!(frame(&mut app, vec![]), 0);
        // With more matches than the list holds, Up on the first wraps to the last one it holds, and
        // Down there wraps to the first.
        app.ui.palette_query = "select".into();
        assert!(matching(&entries(&ctx, Lang::EN), "select", &[]).len() > MAX_RESULTS);
        assert_eq!(frame(&mut app, vec![]), 0);
        assert_eq!(frame(&mut app, vec![key(egui::Key::ArrowUp)]), MAX_RESULTS - 1);
        assert_eq!(frame(&mut app, vec![key(egui::Key::ArrowDown)]), 0);
    }

    /// Each command, tool and plug-in is one result: Place… and Print…, which are engine commands
    /// and UI commands of one id, are listed once, under their engine menu path.
    #[test]
    fn each_id_is_one_result() {
        let entries = build(Lang::EN);
        let mut seen = std::collections::HashSet::new();
        let twice: Vec<&str> = entries.iter().filter(|e| !seen.insert(e.id.as_str())).map(|e| e.id.as_str()).collect();
        assert!(twice.is_empty(), "{twice:?}");
        let shown = |id: &str| entries.iter().find(|e| e.id == id).map(|e| e.shown.as_str());
        assert_eq!((shown("file.place"), shown("file.print")), (Some("File › Place…"), Some("Print…")));
    }

    /// No two results for different commands, tools or plug-ins show the same name, in English or
    /// in any language with a complete menu catalog.
    #[test]
    fn different_results_show_different_names() {
        let langs = std::iter::once(Lang::EN).chain(Lang::all().filter(|l| l.complete_menus()));
        let mut shared = vec![];
        for lang in langs {
            let mut by_name: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
            for e in build(lang) {
                let ids = by_name.entry(e.shown).or_default();
                if !ids.contains(&e.id) {
                    ids.push(e.id);
                }
            }
            shared.extend(by_name.into_iter().filter(|(_, ids)| ids.len() > 1).map(|(name, ids)| format!("{}: {name}: {ids:?}", lang.code())));
        }
        assert!(shared.is_empty(), "{} shared names:\n{}", shared.len(), shared.join("\n"));
    }

    /// Results that would share a name show the panel, menu or command they belong to, or their id
    /// when nothing else tells them apart. A query still finds them
    /// by the plain name, the new path in English or in the UI language, and the id.
    #[test]
    fn results_that_share_a_name_show_where_they_belong() {
        let entries = build(Lang::EN);
        let shown = |id: &str| entries.iter().find(|e| e.id == id).map(|e| e.shown.clone()).unwrap_or_else(|| panic!("{id} is a result"));
        for (id, want) in [
            ("file.exportInBackground", "Export › Continue in Background"),
            ("file.openInBackground", "Open › Continue in Background"),
            ("imageTrace.hideRuns", "Image Trace › Continue in Background"),
            ("plugin.hideRuns", "Plug-ins › Continue in Background"),
            ("ui.layersPanelOptions", "Layers › Panel Options…"),
            ("ui.navigatorPanelOptions", "Navigator › Panel Options…"),
            ("ui.charStyleOptions.resetPanel", "Character Style Options › Reset Panel"),
            ("ui.paraStyleOptions.resetPanel", "Paragraph Style Options › Reset Panel"),
            ("window.swatchLibrary.other", "Window › Swatch Libraries › Other Library…"),
            ("window.brushLibrary.other", "Window › Brush Libraries › Other Library…"),
            ("tool:knife", "Tools › Knife"),
            ("path.knife", "Path › Knife"),
            ("addon.pen.close", "Pen › Close Path"),
            ("path.close", "Path › Close Path"),
            ("type.fitHeadline", "Type › Fit Headline (type.fitHeadline)"),
            ("text.fitHeadline", "Type › Fit Headline (text.fitHeadline)"),
            ("shape.ellipse", "Ellipse (shape.ellipse)"),
            ("addon.shapes.ellipse", "Ellipse (addon.shapes.ellipse)"),
            ("perspective.editText", "Object › Perspective › Edit Text"),
            ("ui.action.play", "Play Action (ui.action.play)"),
            ("ui.action.playAt", "Play Action (ui.action.playAt)"),
            ("ui.batchDialog", "Batch…"),
            ("ui.flattenerPreview", "Flattener Preview"),
        ] {
            assert_eq!(shown(id), want, "{id}");
        }
        assert!(shown("perspective.grid.rulers").contains("Perspective Grid"), "{}", shown("perspective.grid.rulers"));
        assert_ne!(shown("perspective.grid.rulers"), shown("view.rulers"));
        assert!(entries.iter().all(|e| e.id != "action.batch" && e.id != "flattener.preview"), "the engine commands are listed as their UI commands");
        // A name nobody else has stays as it was.
        assert_eq!(shown("object.group"), "Object › Group");
        let found = |entries: &[Entry], query: &str| -> Vec<String> {
            entries.iter().filter(|e| query.split_whitespace().all(|w| e.haystack.contains(&w.to_lowercase()))).map(|e| e.id.clone()).collect()
        };
        let id = "file.exportInBackground".to_string();
        for query in ["continue in background", "export continue in background", "file.exportinbackground"] {
            assert!(found(&entries, query).contains(&id), "{query}");
        }
        assert!(!found(&entries, "open continue in background").contains(&id));
        let zh = Lang::from_code("zh-hant").unwrap();
        let zh_entries = build(zh);
        let export = crate::i18n::tr(zh, "Export");
        let background = zh_entries.iter().find(|e| e.id == id).unwrap();
        assert_eq!(background.shown, format!("{export} › {}", crate::i18n::tr(zh, "Continue in Background")));
        assert!(found(&zh_entries, &format!("{export} continue in background")).contains(&id));
    }

    /// Every menu path a result can take when it shares a name, and Window › Show Panel, has a row
    /// in each catalog with complete menus. Each command [`CONTEXTS`] names is a result.
    #[test]
    fn the_paths_of_results_that_share_a_name_translate() {
        let ids: Vec<String> = items().into_iter().map(|(_, id, _)| id).collect();
        for (id, _) in CONTEXTS {
            assert!(ids.iter().any(|i| i == id), "{id} is a result");
        }
        let mut paths: Vec<String> = CONTEXTS.iter().map(|(_, path)| path.to_string()).collect();
        paths.push("Tools".into());
        paths.push(PANEL_PATH.into());
        let mut by_name: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        for d in drafts() {
            by_name.entry(d.name(Lang::EN)).or_default().push(d.id);
        }
        paths.extend(by_name.into_values().filter(|ids| ids.len() > 1).flatten().filter_map(|id| context(&id)));
        let mut missing = vec![];
        for lang in Lang::all().filter(|l| l.complete_menus()) {
            for segment in paths.iter().flat_map(|p| p.split(" › ")) {
                let row = format!("{}: {segment}", lang.code());
                if !crate::i18n::has(lang, segment) && !missing.contains(&row) {
                    missing.push(row);
                }
            }
        }
        assert!(missing.is_empty(), "{}", missing.join("\n"));
    }

    /// A pick of each of two results that shared a name runs its own command: Show Rulers under View
    /// › Rulers toggles the document rulers, and the one under the perspective grid toggles the
    /// grid's rulers.
    #[test]
    fn a_pick_of_a_renamed_result_runs_its_own_command() {
        let entries = build(Lang::EN);
        let picks: Vec<(String, String)> =
            entries.iter().filter(|e| e.haystack.lines().next() == Some("show rulers")).map(|e| (e.id.clone(), e.shown.clone())).collect();
        assert_eq!(picks.len(), 2, "{picks:?}");
        assert!(picks.iter().all(|(_, name)| name != "Show Rulers") && picks[0].1 != picks[1].1, "{picks:?}");
        for (id, name) in &picks {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let label = |app: &VectorcraftApp, id: &str| crate::menus::dynamic_label(app, id, "");
            let before: Vec<String> = picks.iter().map(|(other, _)| label(&app, other)).collect();
            run_pick(&mut app, id);
            for ((other, _), before) in picks.iter().zip(before) {
                assert_eq!(label(&app, other) != before, other == id, "{name}: {other}");
            }
        }
    }

    /// Each filter plug-in has one result, also when two share a name: its menu path translates,
    /// the plug-in's name does not, and a pick runs that filter (a dialog for one with parameters,
    /// nothing with no document open).
    #[test]
    fn each_filter_plugin_is_a_result_of_its_own() {
        use vectorcraft_plugins::wat as tpl;
        let (one, two) = ("test.UI.palette_filter-one", "test.UI.palette_filter-two");
        let (key_one, key_two) = (format!("plugin:{one}"), format!("plugin:{two}"));
        let install = |id: &str, params: &str| {
            let manifest = format!(r#"{{"id":"{id}","name":"Group","version":"1","kind":"filter","params":{params}}}"#);
            vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
        };
        install(one, "{}");
        install(two, r#"{"wiggle":{"type":"bool"}}"#);
        let zh = Lang::from_code("zh-hant").unwrap();
        let built = build(zh);
        let of = |key: &str| built.iter().filter(|e| e.id == key).collect::<Vec<_>>();
        assert_eq!((of(&key_one).len(), of(&key_two).len()), (1, 1));
        assert!(built.iter().all(|e| e.id != "plugin.dialog"), "no result without a filter's id");
        let path = format!("{} › {}", crate::i18n::tr(zh, "Object"), crate::i18n::tr(zh, "Plug-ins"));
        assert_eq!(of(&key_two)[0].shown, format!("{path} › Group…"));
        assert_eq!(of(&key_one)[0].shown, format!("{path} › Group"));
        assert!(of(&key_two)[0].haystack.contains("object › plug-ins › group…"));

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        run_pick(&mut app, &key_two);
        assert!(app.ui.dialog.is_none(), "no document: the dimmed item's pick does nothing");
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        run_pick(&mut app, &key_two);
        let d = app.ui.dialog.take().expect("the filter's dialog opens");
        assert_eq!((d.kind.as_str(), d.str("__plugin")), ("plugin", two.to_string()));
        vectorcraft_plugins::registry::remove(one);
        vectorcraft_plugins::registry::remove(two);
    }

    /// Of the engine commands the palette listed before, it leaves out only the ones
    /// [`crate::shortcut_editor::needs_a_value`] marks: queries with no default key and no item of
    /// the menu bar, and commands that need a value no menu item gives. A pick of each one it leaves
    /// out (with no document, one object selected or several) changes neither the document, its
    /// selection, history and layer state, nor the interface. Every [`PICK_RUNS`] query is still
    /// listed.
    #[test]
    fn the_palette_leaves_out_the_commands_a_pick_cannot_run() {
        use vectorcraft_testkit::fixtures::Fixture;
        let ui_ids: std::collections::HashSet<&str> = menus::UI_COMMANDS.iter().map(|c| c.0).collect();
        // The rule before: every engine command but the aliases, the wrapped commands and the ids of
        // UI commands.
        let before: Vec<&vectorcraft_engine::CommandSpec> = vectorcraft_engine::command_specs()
            .iter()
            .filter(|c| !vectorcraft_engine::cmd::is_alias(c.id) && !WRAPPED.iter().any(|(e, _)| *e == c.id) && !ui_ids.contains(c.id))
            .collect();
        let listed: std::collections::HashSet<String> = items().into_iter().map(|(_, id, _)| id).collect();
        let dropped: Vec<&vectorcraft_engine::CommandSpec> = before.iter().copied().filter(|c| !listed.contains(c.id)).collect();
        let ids: Vec<&str> = dropped.iter().map(|c| c.id).collect();
        for id in ["document.json", "brush.unused", "symbol.unused", "transparency.set"] {
            assert!(ids.contains(&id), "{id} is left out: {ids:?}");
        }
        // What a pick can change: the document with its selection, history, revision and layer
        // state, the Script Console, and the interface's state (an open dialog or panel, the
        // selected tool), the status line aside.
        let look = |app: &VectorcraftApp| {
            let mut ui = serde_json::to_value(&app.ui).unwrap_or_default();
            if let Some(o) = ui.as_object_mut() {
                o.remove("status");
            }
            let doc = app.session.active().map(|st| {
                (st.doc.clone(), format!("{:?}", st.selection), st.history.undo.len(), st.revision, st.active_layer, st.layer_rows.clone())
            });
            (doc, app.session.script_console().lines().len(), ui)
        };
        let acts = |id: &str| {
            [vectorcraft_engine::Session::new(), Fixture::Single.session(), Fixture::Multi.session()].into_iter().any(|session| {
                let mut app = VectorcraftApp::new(session, crate::Services::default());
                let before = look(&app);
                run_pick(&mut app, id);
                look(&app) != before
            })
        };
        // `layer.setProps {}` records an undo step that changes nothing.
        let changed: Vec<&str> = ids.iter().copied().filter(|id| *id != "layer.setProps" && acts(id)).collect();
        assert!(changed.is_empty(), "a pick of these does something: {changed:?}");
        for c in &dropped {
            assert!(c.shortcut.is_none() && crate::shortcut_editor::menu_bar_path(c.id).is_none(), "{}", c.id);
        }
        // The others are still listed, and each of PICK_RUNS, which needs_a_value marks. A pick of
        // each one changes what `look` sees, except the commands that act on the session alone:
        // [`a_pick_of_a_session_command_runs_it`] covers those, or they need a running script, an
        // installed plug-in or saved import matches.
        for c in &before {
            let marked = crate::shortcut_editor::needs_a_value(c.id);
            assert_eq!(listed.contains(c.id), !marked || PICK_RUNS.contains(&c.id), "{}", c.id);
            assert!(marked || !PICK_RUNS.contains(&c.id), "{} needs no entry in PICK_RUNS", c.id);
        }
        let session_only = [
            "script.stop",
            "script.console.clear",
            "plugin.clearCache",
            "prefs.reset",
            "importMatches.reset",
            "edit.undoGroup.begin",
            "edit.undoGroup.end",
            "imageTrace.presets.save",
            "flattener.presets.save",
            "pdf.preset.save",
            "print.presets.save",
            "perspective.presets.save",
            "gradient.presets.save",
            "gradient.presets.delete",
        ];
        for id in PICK_RUNS.iter().filter(|id| !session_only.contains(id)) {
            assert!(acts(id), "a pick of {id} does something");
        }
        for id in PICK_RUNS.iter().chain(&["object.group", "select.key", "view.preciseCursors", "path.close"]) {
            assert!(listed.contains(*id), "{id} stays");
        }
    }

    /// A pick of a query that changes the session runs it: Clear Script Console empties the Script
    /// Console, Reset Preferences resets a changed preference, each Save … Preset command adds a
    /// preset, Delete Gradient Preset deletes the one saved from the selection, and the edits between Begin Undo Group and End Undo Group are one undo step.
    #[test]
    fn a_pick_of_a_session_command_runs_it() {
        let ids: Vec<String> = items().into_iter().map(|(_, id, _)| id).collect();
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
        let pick = |app: &mut VectorcraftApp, id: &str| {
            assert!(ids.iter().any(|i| i == id), "{id} is a result");
            run_pick(app, id);
        };
        app.session.script_frames = true;
        app.run("script.run", json!({"source": "$.writeln('line')", "name": "c.js"})).unwrap();
        for _ in 0..10_000 {
            if !app.session.script_running() {
                break;
            }
            crate::script_runs::poll(&mut app);
        }
        assert!(!app.session.script_running() && !app.session.script_console().lines().is_empty());
        pick(&mut app, "script.console.clear");
        assert!(app.session.script_console().lines().is_empty(), "{}", app.ui.status);

        let default = app.session.prefs.use_precise_cursors;
        app.session.prefs.use_precise_cursors = !default;
        pick(&mut app, "prefs.reset");
        assert_eq!(app.session.prefs.use_precise_cursors, default, "{}", app.ui.status);

        for (save, list) in [
            ("imageTrace.presets.save", "imageTrace.presets"),
            ("flattener.presets.save", "flattener.presets.list"),
            ("pdf.preset.save", "pdf.preset.list"),
            ("print.presets.save", "print.presets.list"),
            ("perspective.presets.save", "perspective.presets.list"),
        ] {
            let count = |app: &mut VectorcraftApp| app.run(list, json!({})).unwrap()["presets"].as_array().map_or(0, Vec::len);
            let before = count(&mut app);
            pick(&mut app, save);
            assert_eq!(count(&mut app), before + 1, "{save}: {}", app.ui.status);
        }
        // Save Gradient Preset saves the selection's gradient, and Delete Gradient Preset deletes
        // the saved preset it matches.
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        app.run("paint.setFill", json!({"gradient": {"kind": "linear", "stops": stops}})).unwrap();
        pick(&mut app, "gradient.presets.save");
        assert_eq!(app.session.prefs.gradient_presets.len(), 1, "{}", app.ui.status);
        pick(&mut app, "gradient.presets.delete");
        assert!(app.session.prefs.gradient_presets.is_empty(), "{}", app.ui.status);

        let undo = |app: &VectorcraftApp| app.session.active().map_or(0, |st| st.history.undo.len());
        let before = undo(&app);
        pick(&mut app, "edit.undoGroup.begin");
        for x in [10, 200] {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        }
        pick(&mut app, "edit.undoGroup.end");
        assert_eq!(undo(&app), before + 1, "{}", app.ui.status);
    }

    /// The UI commands that open an engine command's dialog or ask first are still listed, and the
    /// engine commands they wrap are not.
    #[test]
    fn the_wrapping_ui_commands_stay_listed() {
        let ids: std::collections::HashSet<String> = items().into_iter().map(|(_, id, _)| id).collect();
        for (engine, ui) in WRAPPED {
            assert!(ids.contains(*ui) && !ids.contains(*engine), "{ui} listed, {engine} not");
        }
        assert!(menus::UI_COMMANDS.iter().filter(|c| c.0 != "plugin.dialog" && c.0 != crate::scripts_menu::COMMAND).all(|c| ids.contains(c.0)));
    }

    /// Each panel is a result under Window › Show Panel. A pick shows the panel, and a second pick
    /// leaves it showing rather than hiding it as the Window menu item does.
    #[test]
    fn a_panel_is_a_result_whose_pick_shows_it() {
        let entries = catalog(Lang::EN, false);
        let panels: Vec<&Entry> = entries.iter().filter(|e| e.id.starts_with("panel:")).collect();
        assert_eq!(panels.len(), crate::state::all_panels().count());
        // Many results hold "layers" in their menu path; the panel, named Layers, is shown first, and "color guide" shows the Color Guide panel first too.
        for (query, want) in [("layers", "panel:layers"), ("color guide", "panel:colorGuide"), ("Layers ", "panel:layers")] {
            let all: Vec<&str> =
                entries.iter().filter(|e| query.to_lowercase().split_whitespace().all(|w| e.haystack.contains(w))).map(|e| e.id.as_str()).collect();
            let shown: Vec<&str> = matching(&entries, &query.to_lowercase(), &[]).into_iter().map(|e| e.id.as_str()).collect();
            assert_eq!(shown.first(), Some(&want), "{query}: {shown:?}");
            // English plural and verb forms can find more results than the text alone.
            assert!(all.iter().all(|id| shown.contains(id)), "{query}");
        }
        let layers = entries.iter().find(|e| e.id == "panel:layers").unwrap();
        assert_eq!(layers.shown, "Window › Show Panel › Layers");
        assert_eq!(layers.shortcut, crate::shortcut_editor::panel_shortcut("layers").unwrap_or(""));
        let zh = Lang::from_code("zh-hant").unwrap();
        let shown = build(zh).into_iter().find(|e| e.id == "panel:layers").map(|e| e.shown).unwrap();
        assert_eq!(shown, ["Window", "Show Panel", "Layers"].map(|s| crate::i18n::tr(zh, s)).join(" › "));
        for id in ["swatches", crate::panels::asset_export::ID] {
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            assert!(!crate::panel_docking::showing(&app.ui, id), "{id}");
            run_pick(&mut app, &format!("panel:{id}"));
            assert!(crate::panel_docking::showing(&app.ui, id), "{id}: {}", app.ui.status);
            run_pick(&mut app, &format!("panel:{id}"));
            assert!(crate::panel_docking::showing(&app.ui, id), "{id}: picked twice, still showing");
        }
    }

    /// In Presentation Mode a panel's pick fails as `window.panel` does: the status line says why,
    /// and the panel does not show when the mode ends.
    #[test]
    fn a_panel_pick_in_presentation_mode_changes_nothing() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let (open_panel, dock) = (app.ui.open_panel.clone(), app.ui.dock);
        app.ui.screen_mode = 3;
        run_pick(&mut app, "panel:swatches");
        assert_eq!(app.ui.status, "panels can't be shown in Presentation Mode");
        assert_eq!((app.ui.open_panel.clone(), app.ui.dock), (open_panel, dock));
        app.ui.screen_mode = 0;
        assert!(!crate::panel_docking::showing(&app.ui, "swatches"), "the pick did not place the panel");
    }

    /// Each Effect menu item is a result under its menu path, with its shortcut. With a document
    /// open and an object selected, a pick of Drop Shadow… opens its dialog, as the menu item
    /// does, and changes nothing until the dialog is confirmed.
    #[test]
    fn an_effect_is_a_result_whose_pick_runs_its_menu_item() {
        // Both reads see the same plug-in revision: another test may install or remove a live
        // effect plug-in between them.
        let (entries, effects) = loop {
            let revision = menus::plugin_revision();
            let entries = build(Lang::EN);
            let effects = crate::shortcut_editor::effect_items();
            if menus::plugin_revision() == revision {
                break (entries, effects);
            }
        };
        for e in effects.iter() {
            let key = format!("effect:{}", e.effect);
            assert_eq!(entries.iter().filter(|r| r.id == key).count(), 1, "{key}");
        }
        let found: Vec<&Entry> = entries.iter().filter(|e| "drop shadow".split_whitespace().all(|w| e.haystack.contains(w))).collect();
        let shadow =
            found.iter().find(|e| e.id.starts_with("effect:")).unwrap_or_else(|| panic!("{:?}", found.iter().map(|e| &e.id).collect::<Vec<_>>()));
        let item = effects.iter().find(|e| format!("effect:{}", e.effect) == shadow.id).unwrap();
        assert_eq!(shadow.shown, format!("{} › {}", item.group, item.label));
        let mut picked = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
        let mut clicked = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
        let before = outcome(&picked);
        run_pick(&mut picked, &shadow.id);
        menus::invoke(&mut clicked, item.command, json!({ "effect": item.effect }));
        let after = outcome(&picked);
        assert!(after.0.is_some(), "the dialog opens: {}", after.2);
        assert_eq!(after.1, before.1, "nothing changes yet");
        assert_eq!(after, outcome(&clicked), "the pick runs what the menu item runs");
        // With nothing selected the menu item is dimmed: no dialog opens, and the status line says
        // why.
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.ui.status.clear();
        run_pick(&mut app, &shadow.id);
        assert!(app.ui.dialog.is_none() && !app.ui.status.is_empty(), "{}", app.ui.status);
        // A built-in effect's name translates by its id, as the Effect menu shows it.
        let fr = Lang::from_code("fr").unwrap();
        let add = build(fr).into_iter().find(|e| e.id == "effect:pathfinder.add").map(|e| e.shown).unwrap();
        assert!(add.ends_with(crate::i18n::tr_id(fr, "pathfinder.add", "Add")), "{add}");
        assert_eq!(crate::i18n::tr_id(fr, "pathfinder.add", "Add"), "Réunion");
    }

    /// Two filter plug-ins with one name show their ids after it, the name untranslated, and each
    /// pick runs its own filter.
    #[test]
    fn filter_plugins_with_one_name_show_their_ids() {
        use vectorcraft_plugins::wat as tpl;
        let (one, two) = ("test.UI.palette_same-one", "test.UI.palette_same-two");
        for id in [one, two] {
            let manifest = format!(r#"{{"id":"{id}","name":"Same Name","version":"1","kind":"filter","params":{{"wiggle":{{"type":"bool"}}}}}}"#);
            vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
        }
        let zh = Lang::from_code("zh-hant").unwrap();
        let built = build(zh);
        let path = format!("{} › {}", crate::i18n::tr(zh, "Object"), crate::i18n::tr(zh, "Plug-ins"));
        for id in [one, two] {
            let key = format!("plugin:{id}");
            let shown: Vec<&str> = built.iter().filter(|e| e.id == key).map(|e| e.shown.as_str()).collect();
            assert_eq!(shown, [format!("{path} › Same Name… ({key})")], "{id}");
        }
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        for id in [one, two] {
            run_pick(&mut app, &format!("plugin:{id}"));
            let d = app.ui.dialog.take().expect("the filter's dialog opens");
            assert_eq!(d.str("__plugin"), id.to_string());
        }
        vectorcraft_plugins::registry::remove(one);
        vectorcraft_plugins::registry::remove(two);
    }

    /// The open palette in a window, drawn a frame at a time with the app's theme, in English, with
    /// screen readers on.
    struct Open {
        app: VectorcraftApp,
        ctx: egui::Context,
        size: egui::Vec2,
        time: f64,
        /// Whether the last frame asked for another one at once.
        repaint: bool,
        /// What the last frame drew.
        shapes: Vec<egui::epaint::ClippedShape>,
    }

    impl Open {
        /// The palette open on `query` in a window `size` points big.
        fn new(query: &str, size: egui::Vec2) -> Open {
            crate::i18n::set_current(Lang::EN);
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            app.ui.palette_open = true;
            app.ui.palette_query = query.into();
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            crate::theme::install_fonts(&ctx);
            crate::theme::apply(&ctx, Default::default());
            let mut open = Open { app, ctx, size, time: 0.0, repaint: false, shapes: Vec::new() };
            open.frame(vec![]);
            open
        }

        /// One frame with `events`: its screen-reader tree.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::accesskit::Node> {
            self.time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, self.size);
            let input = egui::RawInput { time: Some(self.time), screen_rect: Some(screen), events, ..Default::default() };
            let app = &mut self.app;
            let mut out = self.ctx.run_ui(input, |ui| show(app, ui.ctx()));
            out.textures_delta.clear();
            self.repaint = out.viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.repaint_delay.is_zero());
            self.shapes = std::mem::take(&mut out.shapes);
            out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
        }

        /// A frame where `k` is pressed `times` times.
        fn press(&mut self, k: egui::Key, times: usize) -> Vec<egui::accesskit::Node> {
            let key = egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            self.frame(vec![key; times])
        }

        /// The color the last frame drew the text `text` in.
        fn color_of(&self, text: &str) -> Option<egui::Color32> {
            fn find(shape: &egui::Shape, text: &str) -> Option<egui::Color32> {
                match shape {
                    egui::Shape::Text(t) if t.galley.text() == text => Some(t.fallback_color),
                    egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
                    _ => None,
                }
            }
            self.shapes.iter().find_map(|s| find(&s.shape, text))
        }

        /// The highlighted result's index.
        fn highlighted(&self) -> usize {
            self.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap().0
        }

        /// The popup's rectangle.
        fn popup(&self) -> egui::Rect {
            self.ctx.memory(|m| m.area_rect(egui::Id::unique("palette"))).unwrap()
        }

        /// What screen readers read for result `id`'s row.
        fn said(&self, id: &str) -> String {
            let entries = entries(&self.ctx, Lang::EN);
            entries.iter().find(|e| e.id == id).map(|e| row_label(e, &home_text(&self.app, e))).unwrap()
        }

        /// The results the palette lists for its query.
        fn listed(&self) -> Vec<String> {
            let entries = entries(&self.ctx, Lang::EN);
            // An empty search lists the recent picks, as `show` does.
            let listed = if self.app.ui.palette_query.trim().is_empty() {
                recent(&entries.list, &self.app.ui.recent_commands)
            } else {
                super::matching(&entries, &self.app.ui.palette_query.to_lowercase(), &self.app.ui.recent_commands, &self.app.ui.command_uses)
            };
            listed.iter().take(MAX_RESULTS).map(|e| e.id.clone()).collect()
        }
    }

    /// The result rows of a screen-reader tree, top to bottom: (label, highlighted, top, bottom).
    fn rows(nodes: &[egui::accesskit::Node]) -> Vec<(String, bool, f64, f64)> {
        let mut rows: Vec<(String, bool, f64, f64)> = nodes
            .iter()
            .filter(|n| n.role() == egui::accesskit::Role::ListItem)
            .map(|n| {
                let b = n.bounds().unwrap();
                (n.label().unwrap_or_default().to_string(), n.is_selected() == Some(true), b.y0, b.y1)
            })
            .collect();
        rows.sort_by(|a, b| a.2.total_cmp(&b.2));
        rows
    }

    /// The text of the labels in a screen-reader tree.
    fn labels(nodes: &[egui::accesskit::Node]) -> Vec<String> {
        nodes.iter().filter(|n| n.role() == egui::accesskit::Role::Label).filter_map(|n| n.label().or(n.value()).map(str::to_string)).collect()
    }

    /// The search field's screen-reader description.
    fn field_description(nodes: &[egui::accesskit::Node]) -> String {
        let field = nodes.iter().find(|n| n.role() == egui::accesskit::Role::TextInput).expect("the search field is in the tree");
        field.description().unwrap_or_default().to_string()
    }

    /// The first line of the search field's description: the highlighted result's place and name,
    /// without the reason a result that can't run now adds under it.
    fn field_place(nodes: &[egui::accesskit::Node]) -> String {
        field_description(nodes).lines().next().unwrap_or_default().to_string()
    }

    const WINDOW: egui::Vec2 = egui::vec2(1280.0, 900.0);

    /// Each result is a 20-point row, rows touch, and 12 show at once; the popup is shorter than
    /// the 14 24-point buttons it showed before.
    #[test]
    fn results_are_compact_rows_that_touch() {
        let mut open = Open::new("select", WINDOW);
        let nodes = open.frame(vec![]);
        let rows = rows(&nodes);
        let inside: Vec<_> = rows.iter().filter(|r| r.3 <= f64::from(open.popup().bottom())).collect();
        assert!(inside.len() >= VISIBLE_ROWS, "{rows:?}");
        for pair in inside.windows(2).take(VISIBLE_ROWS - 1) {
            assert_eq!(pair[0].3 - pair[0].2, f64::from(ROW_HEIGHT), "{pair:?}");
            assert_eq!(pair[0].3, pair[1].2, "rows touch: {pair:?}");
        }
        // 14 results took 458 points before the rows were made compact.
        let height = open.popup().height();
        println!("popup height for {VISIBLE_ROWS} rows: {height}");
        assert!(height <= 330.0, "{height}");
    }

    /// The footer says how many results matched, how many of them the list holds when it holds
    /// fewer, or that none matched.
    #[test]
    fn the_footer_counts_the_results() {
        crate::i18n::set_current(Lang::EN);
        assert_eq!(footer(50, 60), "50 of 60 results. Keep typing to narrow them.");
        assert_eq!(footer(3, 3), "3 results");
        assert_eq!(footer(1, 1), "1 result");
        assert_eq!(footer(0, 0), "No commands match");
        // The count takes each language's form.
        crate::i18n::set_current(Lang::from_code("ru").unwrap());
        let ru = [footer(50, 51), footer(50, 52), footer(2, 2)];
        crate::i18n::set_current(Lang::EN);
        assert_eq!(
            ru,
            [
                "50 из 51 результата. Продолжайте вводить, чтобы сузить их.",
                "50 из 52 результатов. Продолжайте вводить, чтобы сузить их.",
                "2 результата"
            ]
        );
        let mut open = Open::new("select", WINDOW);
        let total = matching(&entries(&open.ctx, Lang::EN), "select", &[]).len();
        assert!(total > MAX_RESULTS, "{total}");
        let nodes = open.frame(vec![]);
        assert!(labels(&nodes).contains(&format!("50 of {total} results. Keep typing to narrow them.")), "{:?}", labels(&nodes));
        // Every result the list holds is reachable: End of the list by keyboard shows the 50th.
        let nodes = open.press(egui::Key::ArrowUp, 1);
        assert_eq!(open.highlighted(), MAX_RESULTS - 1);
        assert!(rows(&nodes).iter().any(|r| r.1), "the 50th row is drawn and highlighted");
    }

    /// With no match, the footer says so and Enter runs nothing and leaves the palette open.
    #[test]
    fn with_no_match_enter_runs_nothing() {
        let mut open = Open::new("qqzzxxnothing", WINDOW);
        let nodes = open.frame(vec![]);
        assert!(rows(&nodes).is_empty());
        assert!(labels(&nodes).contains(&"No commands match".to_string()), "{:?}", labels(&nodes));
        open.app.ui.status.clear();
        open.press(egui::Key::Enter, 1);
        open.frame(vec![]);
        assert!(open.app.ui.palette_open && open.app.ui.status.is_empty(), "{}", open.app.ui.status);
    }

    /// Down 15 times from the first result highlights the 16th, scrolled into view, and Enter runs
    /// it.
    #[test]
    fn the_sixteenth_result_is_reached_by_keyboard_and_runs() {
        let mut open = Open::new("show panel", WINDOW);
        let listed = open.listed();
        assert!(listed.len() > 16, "{listed:?}");
        let nodes = open.press(egui::Key::ArrowDown, 15);
        assert_eq!(open.highlighted(), 15);
        let lit: Vec<_> = rows(&nodes).into_iter().filter(|r| r.1).collect();
        let want = open.said(&listed[15]);
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert_eq!(lit[0].0, want);
        let popup = open.popup();
        assert!(lit[0].2 >= f64::from(popup.top()) && lit[0].3 <= f64::from(popup.bottom()), "scrolled into view: {lit:?} in {popup:?}");
        let panel = listed[15].strip_prefix("panel:").expect("a panel's result").to_string();
        assert!(!crate::panel_docking::showing(&open.app.ui, &panel), "{panel}");
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open);
        assert!(crate::panel_docking::showing(&open.app.ui, &panel), "{panel}: {}", open.app.ui.status);
    }

    /// Page Down and Page Up move the highlight by the rows shown, stopping at the ends; Up and
    /// Down wrap.
    #[test]
    fn page_keys_move_by_a_page_without_wrapping() {
        use egui::Key::{ArrowDown, ArrowUp, PageDown, PageUp};
        let moves = |keys: &[egui::Key]| Moves(keys.to_vec());
        assert_eq!(moves(&[PageDown]).apply(0, 50, 12), 12);
        assert_eq!(moves(&[PageDown]).apply(45, 50, 12), 49);
        assert_eq!(moves(&[PageUp]).apply(5, 50, 12), 0);
        assert_eq!(moves(&[ArrowDown]).apply(49, 50, 12), 0);
        assert_eq!(moves(&[ArrowUp]).apply(0, 50, 12), 49);
        assert_eq!(moves(&[ArrowDown; 3]).apply(48, 50, 12), 1);
        assert_eq!(moves(&[ArrowDown]).apply(0, 0, 12), 0);
        assert_eq!(moves(&[PageDown, ArrowUp]).apply(0, 50, 12), 11, "in the order pressed");
        assert_eq!(moves(&[ArrowUp, PageDown]).apply(0, 50, 12), 49, "in the order pressed");
        let mut open = Open::new("select", WINDOW);
        open.press(egui::Key::PageDown, 1);
        assert_eq!(open.highlighted(), VISIBLE_ROWS);
        open.press(egui::Key::PageDown, 5);
        assert_eq!(open.highlighted(), MAX_RESULTS - 1, "stops at the last");
        open.press(egui::Key::PageUp, 1);
        assert_eq!(open.highlighted(), MAX_RESULTS - 1 - VISIBLE_ROWS);
        open.press(egui::Key::PageUp, 9);
        assert_eq!(open.highlighted(), 0, "stops at the first");
    }

    /// After the list is scrolled with the wheel, the highlight is on a row in view, so Enter runs a
    /// result that can be seen.
    #[test]
    fn a_wheel_scroll_keeps_the_highlight_in_view() {
        assert_eq!(nearest_in_view(0, 100.0, 240.0, 50), 5);
        assert_eq!(nearest_in_view(30, 100.0, 240.0, 50), 16);
        assert_eq!(nearest_in_view(8, 100.0, 240.0, 50), 8);
        assert_eq!(nearest_in_view(3, 0.0, 240.0, 3), 2);
        assert_eq!(nearest_in_view(0, 0.0, 240.0, 0), 0);
        let mut open = Open::new("select", WINDOW);
        let nodes = open.frame(vec![]);
        let first = rows(&nodes).into_iter().next().unwrap();
        let at = egui::pos2(open.popup().center().x, ((first.2 + first.3) / 2.0) as f32);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -200.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        open.frame(vec![egui::Event::PointerMoved(at)]);
        let mut nodes = open.frame(vec![wheel]);
        // The frames that follow, up to the last one the palette or the scroll asks for, which
        // has to show the highlight where Enter runs it.
        for _ in 0..120 {
            if !open.repaint {
                break;
            }
            nodes = open.frame(vec![]);
        }
        assert!(!open.repaint, "the scroll settles");
        let sel = open.highlighted();
        assert!(sel > 0, "the highlight left the row scrolled out of view");
        let popup = open.popup();
        let lit: Vec<_> = rows(&nodes).into_iter().filter(|r| r.1).collect();
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert!(lit[0].2 >= f64::from(popup.top()) && lit[0].3 <= f64::from(popup.bottom()), "{lit:?} in {popup:?}");
        assert_eq!(lit[0].0, open.said(&open.listed()[sel]));
    }

    /// Page Down, Up and Enter pressed in one frame move the highlight in that order, and Enter
    /// runs the result they moved it to.
    #[test]
    fn keys_in_one_frame_move_the_highlight_in_the_order_pressed() {
        let mut open = Open::new("show panel", WINDOW);
        let listed = open.listed();
        assert!(listed.len() > VISIBLE_ROWS, "{listed:?}");
        let panel = listed[VISIBLE_ROWS - 1].strip_prefix("panel:").expect("a panel's result").to_string();
        assert!(!crate::panel_docking::showing(&open.app.ui, &panel), "{panel}");
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        open.frame(vec![key(egui::Key::PageDown), key(egui::Key::ArrowUp), key(egui::Key::Enter)]);
        assert!(!open.app.ui.palette_open);
        assert!(crate::panel_docking::showing(&open.app.ui, &panel), "{panel}: {}", open.app.ui.status);
    }

    /// A scroll in steps of less than half a point a frame still moves the highlight onto a row in
    /// view once the highlighted row leaves it.
    #[test]
    fn a_scroll_in_small_steps_keeps_the_highlight_in_view() {
        let mut open = Open::new("select", WINDOW);
        let nodes = open.frame(vec![]);
        let first = rows(&nodes).into_iter().next().unwrap();
        let at = egui::pos2(open.popup().center().x, ((first.2 + first.3) / 2.0) as f32);
        open.frame(vec![egui::Event::PointerMoved(at)]);
        let wheel = egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0.0, -0.4),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        let mut nodes = Vec::new();
        for _ in 0..200 {
            nodes = open.frame(vec![wheel.clone()]);
        }
        for _ in 0..120 {
            if !open.repaint {
                break;
            }
            nodes = open.frame(vec![]);
        }
        let offset = open.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap().2;
        assert!(offset > ROW_HEIGHT, "the list scrolled past the first row: {offset}");
        assert!(open.highlighted() > 0, "the highlight left the row scrolled out of view");
        let popup = open.popup();
        let lit: Vec<_> = rows(&nodes).into_iter().filter(|r| r.1).collect();
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert!(lit[0].2 >= f64::from(popup.top()) && lit[0].3 <= f64::from(popup.bottom()), "{lit:?} in {popup:?}");
    }

    /// When the window shrinks so the list shows fewer rows, the list scrolls to keep the
    /// highlighted result in view, and Enter runs that result.
    #[test]
    fn a_shorter_window_keeps_the_highlight_in_view() {
        let mut open = Open::new("show panel", WINDOW);
        let listed = open.listed();
        assert!(listed.len() > VISIBLE_ROWS, "{listed:?}");
        open.press(egui::Key::ArrowDown, VISIBLE_ROWS - 1);
        assert_eq!(open.highlighted(), VISIBLE_ROWS - 1);
        open.size = egui::vec2(800.0, 300.0);
        let nodes = open.frame(vec![]);
        assert_eq!(open.highlighted(), VISIBLE_ROWS - 1);
        let popup = open.popup();
        let lit: Vec<_> = rows(&nodes).into_iter().filter(|r| r.1).collect();
        assert_eq!(lit.len(), 1, "{lit:?}");
        assert!(lit[0].2 >= f64::from(popup.top()) && lit[0].3 <= f64::from(popup.bottom()), "{lit:?} in {popup:?}");
        let panel = listed[VISIBLE_ROWS - 1].strip_prefix("panel:").expect("a panel's result").to_string();
        assert!(!crate::panel_docking::showing(&open.app.ui, &panel), "{panel}");
        open.press(egui::Key::Enter, 1);
        assert!(!open.app.ui.palette_open);
        assert!(crate::panel_docking::showing(&open.app.ui, &panel), "{panel}: {}", open.app.ui.status);
    }

    /// The pointer moved onto a row highlights that row alone, and the field's description names
    /// it, on the frame drawn straight after.
    #[test]
    fn the_row_under_the_pointer_is_the_one_highlighted() {
        let mut open = Open::new("select", WINDOW);
        let listed = open.listed();
        let entries = entries(&open.ctx, Lang::EN);
        let nodes = open.frame(vec![]);
        let all = rows(&nodes);
        let middle = |r: &(String, bool, f64, f64)| egui::pos2(open.popup().center().x, ((r.2 + r.3) / 2.0) as f32);
        let (first, fourth) = (middle(&all[0]), middle(&all[3]));
        open.frame(vec![egui::Event::PointerMoved(first)]);
        open.frame(vec![egui::Event::PointerMoved(fourth)]);
        assert_eq!(open.highlighted(), 3);
        assert!(open.repaint, "the frame that moved the highlight asks for another");
        let nodes = open.frame(vec![]);
        let lit: Vec<String> = rows(&nodes).into_iter().filter(|r| r.1).map(|r| r.0).collect();
        let label = open.said(&listed[3]);
        assert_eq!(lit, std::slice::from_ref(&label));
        assert_eq!(field_place(&nodes), format!("4 of {}: {label}", matching(&entries, "select", &[]).len()));
    }

    /// A name too long for its row is cut short with "…" and stays inside the width it is given.
    #[test]
    fn a_long_name_ends_with_an_ellipsis() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let long = "Window › Show Panel › A Panel Name Long Enough Not To Fit In The Row (addon.panels.long)";
            let cut = name_galley(ui.painter(), long, font.clone(), egui::Color32::WHITE, 200.0);
            assert!(cut.elided && cut.size().x <= 200.0, "{:?}", cut.size());
            assert_eq!(cut.rows.last().and_then(|r| r.glyphs.last()).map(|g| g.chr), Some('…'));
            let short = name_galley(ui.painter(), "Edit › Copy", font, egui::Color32::WHITE, 200.0);
            assert!(!short.elided);
        });
        out.textures_delta.clear();
    }

    /// The label and home of English result `id`'s row with the app's current layout.
    fn label_and_home(app: &VectorcraftApp, id: &str) -> (String, String) {
        crate::i18n::set_current(Lang::EN);
        let entries = build(Lang::EN);
        let e = entries.iter().find(|e| e.id == id).unwrap_or_else(|| panic!("{id} is listed"));
        (label_of(e).to_string(), home_text(app, e))
    }

    /// A row shows its label without the menu path, and the menu path of the item of the menu bar
    /// that runs it after it: with that item's label when it differs, and "No menu item" for a
    /// command no item runs.
    #[test]
    fn rows_show_where_each_result_lives() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let row = |id: &str| label_and_home(&app, id);
        assert_eq!(row("effect:stylize.roundCorners"), ("Round Corners…".into(), "Effect › Stylize".into()));
        assert_eq!(row("panel:layers"), ("Layers".into(), "Window".into()));
        assert_eq!(row("object.group"), ("Group".into(), "Object".into()));
        assert_eq!(row("app.quit").1, "File › Exit");
        // Window › Learn runs the Learn command as well as showing the Learn panel.
        assert_eq!(row("window.learn").1, "Window");
        let entries = build(Lang::EN);
        let learn = entries.iter().find(|e| e.id == "window.learn").expect("the Learn command");
        assert_eq!(learn.kind, search::kind("object.group", Some("Object")), "the Learn command ranks as a command of the menu bar");
        let none = entries.iter().find(|e| e.home.is_none() && !e.id.starts_with("tool:")).expect("a command with no menu item");
        assert_eq!((label_of(none), home_text(&app, none).as_str()), (none.shown.as_str(), "No menu item"));
        // A path that keeps two names apart stays, unless the home shows it: the Knife tool's row
        // drops the Tools in front of its name, and an Other Library… row shows its library menu
        // as its home only.
        assert_eq!(row("tool:knife").0, "Knife");
        let other: Vec<(&str, String)> =
            entries.iter().filter(|e| e.shown.ends_with("Other Library…")).map(|e| (label_of(e), home_text(&app, e))).collect();
        assert!(other.len() >= 2 && other.iter().all(|(label, home)| *label == "Other Library…" && home.starts_with("Window › ")), "{other:?}");
        let kept = entries.iter().find(|e| e.label_at == 0 && e.shown.contains(" › ")).expect("a row that keeps its path");
        assert!(kept.home.as_deref().is_none_or(|h| !kept.shown.starts_with(&format!("{h} › "))), "{}", kept.shown);
        for e in entries.iter().filter(|e| e.label_at > 0) {
            assert!(e.home.is_some() || e.id.starts_with("tool:"), "{} has no home but drops its path", e.id);
        }
    }

    /// No two results' rows show the same kind, label and home, in any language, with the menu bar
    /// in the window or the native one.
    #[test]
    fn no_two_rows_read_alike() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let mut alike = vec![];
        for lang in Lang::all() {
            crate::i18n::set_current(lang);
            for native in [false, true] {
                let mut seen = std::collections::HashMap::new();
                for e in &build_in(lang, native) {
                    let row = (kind_of(e).1, label_of(e).to_string(), home_text(&app, e));
                    if let Some(other) = seen.insert(row.clone(), e.id.clone())
                        && other != e.id
                    {
                        alike.push(format!("{} (native {native}): {other} and {}: {row:?}", lang.code(), e.id));
                    }
                }
            }
        }
        crate::i18n::set_current(Lang::EN);
        assert!(alike.is_empty(), "{}", alike.join("\n"));
    }

    /// Every catalog translates the words a row's kind and home are read with.
    #[test]
    fn catalogs_translate_the_kind_and_home_words() {
        let words = ["Tool", "Panel", "Effect", "Filter", "Command", "Toolbar", "{toolbar}, in the {tool} group", "All Tools", "No menu item"];
        let missing: Vec<String> = Lang::all()
            .filter(|l| *l != Lang::EN)
            .flat_map(|l| words.iter().filter(move |w| !crate::i18n::has(l, w)).map(move |w| format!("{}: {w}", l.code())))
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// A tool's row names the toolbar it is on, and the tool its slot's button shows while it is
    /// hidden in that slot's group. A toolbar shown now comes before a hidden Tools panel, which
    /// comes before a hidden toolbar made with New Toolbar.
    #[test]
    fn a_tool_row_names_its_toolbar_slot() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let home = |app: &VectorcraftApp, id: &str| label_and_home(app, id).1;
        assert_eq!(home(&app, "tool:groupSelection"), "Toolbar, in the Direct Selection Tool group");
        assert_eq!(home(&app, "tool:directSelection"), "Toolbar");
        app.select_tool("groupSelection");
        assert_eq!(home(&app, "tool:groupSelection"), "Toolbar");
        assert_eq!(home(&app, "tool:directSelection"), "Toolbar, in the Group Selection Tool group");
        let bar = |name: &str, slot: &[&str], shown: bool| crate::state::CustomToolbar {
            name: name.into(),
            slots: vec![slot.iter().map(|t| t.to_string()).collect()],
            shown,
            pos: [0.0, 0.0],
            dock: None,
            dock_right: false,
            slot_tool: Default::default(),
            double: false,
            floating: vec![],
        };
        // Window › Tools hides the Tools panel while a toolbar made with New Toolbar holds the Pen
        // Tool: the row names that toolbar.
        app.ui.toolbar = false;
        assert_eq!(home(&app, "tool:pen"), "Toolbar");
        app.ui.custom_toolbars.push(bar("Drawing", &["pen", "curvature"], true));
        assert_eq!(home(&app, "tool:pen"), "Drawing");
        assert_eq!(home(&app, "tool:curvature"), "Drawing, in the Pen Tool group");
        app.ui.custom_toolbars[0].name = "{tool}".into();
        assert_eq!(home(&app, "tool:curvature"), "{tool}, in the Pen Tool group");
        app.ui.custom_toolbars[0].name = "Drawing".into();
        // Tab hides the toolbars made with New Toolbar too: the hidden Tools panel comes first.
        app.ui.tab_hid = Some(crate::state::TabHid { dock: true, toolbar: true, control_bar: true });
        assert_eq!(home(&app, "tool:pen"), "Toolbar");
        app.ui.tab_hid = None;
        // The Tools panel shown again comes first.
        app.ui.toolbar = true;
        assert_eq!(home(&app, "tool:pen"), "Toolbar");
        // In Presentation Mode no toolbar shows; the tool still lives on the Tools panel.
        app.ui.screen_mode = 3;
        assert_eq!(home(&app, "tool:pen"), "Toolbar");
        app.ui.screen_mode = 0;
        // A tool that only a hidden toolbar made with New Toolbar holds is named on it, and one no
        // toolbar holds is in All Tools.
        let on_tools: Vec<&str> = crate::toolbar::slots(&app).into_iter().flat_map(|(_, s)| s).collect();
        let loose = vectorcraft_tools::catalog::all_tools().map(|t| t.id).find(|t| !on_tools.contains(t)).expect("a tool not on the Tools panel");
        let id = format!("tool:{loose}");
        assert_eq!(home(&app, &id), "All Tools");
        app.ui.custom_toolbars.push(bar("Cutting", &[loose], false));
        assert_eq!(home(&app, &id), "Cutting");
    }

    /// A row too narrow for its label and home cuts the home first; a label that does not fit
    /// alone is cut before the id it shows after its name, which stays whole.
    #[test]
    fn a_narrow_row_cuts_the_home_first_and_keeps_the_id() {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        let entries = build(Lang::EN);
        let keyed = entries.iter().find(|e| e.shown.ends_with(&format!(" ({})", e.id))).expect("a result that shows its id");
        let copy = entries.iter().find(|e| e.id == "edit.copy").expect("Copy");
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let font = egui::TextStyle::Body.resolve(ui.style());
            let p = ui.painter();
            let lay = |e: &Entry, home: &str, width: f32| row_galleys(p, e, home, font.clone(), egui::Color32::WHITE, egui::Color32::GRAY, width);
            let text = |g: &egui::Galley| g.rows.iter().flat_map(|r| r.glyphs.iter().map(|g| g.chr)).collect::<String>();
            let (label, home, cut) = lay(copy, "Edit", 400.0);
            assert_eq!((text(&label), home.map(|h| text(&h)), cut), ("Copy".into(), Some("Edit".into()), false));
            let long = "A Menu › A Submenu With A Long Name › Another Submenu";
            let (label, home, cut) = lay(copy, long, 160.0);
            let home = home.expect("some of the home fits");
            assert!(cut && home.elided && text(&label) == "Copy", "{}", text(&home));
            assert!(label.size().x + 10.0 + home.size().x <= 160.0 + 0.5);
            // A long German menu path is cut before the label.
            crate::i18n::set_current(Lang::from_code("de").expect("German"));
            let german = build_in(crate::i18n::current(), false);
            let deep = german
                .iter()
                .filter(|e| e.label_at > 0)
                .max_by_key(|e| e.home.as_ref().map_or(0, |h| shown_label(crate::i18n::current(), h).chars().count()));
            let deep = deep.expect("a German row with a home");
            let path = home_text(&app, deep);
            crate::i18n::set_current(Lang::EN);
            let label_width = name_galley(p, label_of(deep), font.clone(), egui::Color32::WHITE, f32::INFINITY).size().x;
            let (label, home, cut) = lay(deep, &path, label_width + 60.0);
            assert!(cut && text(&label) == label_of(deep) && home.is_some_and(|h| h.elided), "{path}");
            let suffix = format!(" ({})", keyed.id);
            let suffix_width = name_galley(p, &suffix, font.clone(), egui::Color32::WHITE, f32::INFINITY).size().x;
            let full = name_galley(p, label_of(keyed), font.clone(), egui::Color32::WHITE, f32::INFINITY).size().x;
            assert!(full - 5.0 > suffix_width + 20.0, "{}", label_of(keyed));
            for width in [full - 5.0, suffix_width + 20.0, suffix_width / 2.0] {
                let (label, home, cut) = lay(keyed, long, width);
                let shown = text(&label);
                assert!(cut && home.is_none(), "{width}");
                assert!(label.size().x <= width + 0.5, "{width}: {shown} is {} wide", label.size().x);
                // An id wider than the row is cut too.
                assert_eq!(shown.ends_with(&format!("…{suffix}")), width > suffix_width, "{width}: {shown}");
            }
        });
        out.textures_delta.clear();
    }

    /// Screen readers read a row as its kind, label, home and shortcut, and the field's
    /// description names them too; the row's height is unchanged.
    #[test]
    fn screen_readers_hear_the_kind_and_home() {
        let mut open = Open::new("round corners", WINDOW);
        let nodes = open.frame(vec![]);
        let said = open.said("effect:stylize.roundCorners");
        assert_eq!(said, "Effect, Round Corners…, Effect › Stylize");
        let row = rows(&nodes).into_iter().find(|r| r.0 == said).expect("its row is drawn");
        assert!((row.3 - row.2 - f64::from(ROW_HEIGHT)).abs() < 0.01, "{row:?}");
        // With no document, Round Corners can't run now, and the reason goes on a second line.
        assert!(field_place(&nodes).ends_with(&said), "{}", field_description(&nodes));
        let open = Open::new("group selection", WINDOW);
        assert_eq!(open.said("tool:groupSelection"), "Tool, Group Selection Tool, Toolbar, in the Direct Selection Tool group");
        let mut open = Open::new("layers", WINDOW);
        let said = open.said("panel:layers");
        assert!(said.starts_with("Panel, Layers, Window"), "{said}");
        let nodes = open.frame(vec![]);
        assert!(rows(&nodes).iter().any(|r| r.0 == said));
    }

    /// The popup stays inside a short window, showing fewer rows when 12 don't fit; a page is the
    /// rows shown.
    #[test]
    fn the_popup_stays_inside_a_short_window() {
        for size in [egui::vec2(1280.0, 600.0), egui::vec2(800.0, 300.0), egui::vec2(800.0, 160.0)] {
            let mut open = Open::new("select", size);
            open.frame(vec![]);
            let popup = open.popup();
            assert!(popup.top() >= 0.0 && popup.bottom() <= size.y, "{size:?}: {popup:?}");
            open.press(egui::Key::PageDown, 1);
            let page = open.highlighted();
            if size.y < 400.0 {
                assert!((1..VISIBLE_ROWS).contains(&page), "a short window shows fewer than 12 rows: {size:?}: {page}");
            } else {
                assert_eq!(page, VISIBLE_ROWS, "{size:?}");
            }
        }
    }

    /// A click anywhere on a row, its empty right end included, runs it.
    #[test]
    fn a_click_anywhere_on_a_row_runs_it() {
        let mut open = Open::new("show panel", WINDOW);
        let listed = open.listed();
        let nodes = open.frame(vec![]);
        let hidden = |id: &String| id.strip_prefix("panel:").is_some_and(|p| !crate::panel_docking::showing(&open.app.ui, p));
        let k = listed.iter().position(hidden).expect("a hidden panel's result");
        assert!(k < VISIBLE_ROWS, "{listed:?}");
        let row = rows(&nodes).into_iter().nth(k).unwrap();
        let popup = open.popup();
        let at = egui::pos2(popup.right() - 24.0, ((row.2 + row.3) / 2.0) as f32);
        let panel = listed[k].strip_prefix("panel:").unwrap_or_default().to_string();
        assert!(!crate::panel_docking::showing(&open.app.ui, &panel), "{panel}");
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        open.frame(vec![egui::Event::PointerMoved(at)]);
        open.frame(vec![button(true)]);
        open.frame(vec![button(false)]);
        assert!(!open.app.ui.palette_open);
        assert!(crate::panel_docking::showing(&open.app.ui, &panel), "{panel}: {}", open.app.ui.status);
    }

    /// The open palette on `query` over the session `session`.
    fn open_on(query: &str, session: vectorcraft_engine::Session) -> Open {
        let mut open = Open::new(query, WINDOW);
        open.app = VectorcraftApp::new(session, crate::Services::default());
        open.app.ui.palette_open = true;
        open.app.ui.palette_query = query.into();
        open.frame(vec![]);
        open
    }

    /// The screen-reader node of the result row named `name`.
    fn row_named<'a>(nodes: &'a [egui::accesskit::Node], name: &str) -> &'a egui::accesskit::Node {
        nodes
            .iter()
            .filter(|n| n.role() == egui::accesskit::Role::ListItem)
            .find(|n| n.label().is_some_and(|l| l == name || l.starts_with(&format!("{name}, "))))
            .unwrap_or_else(|| panic!("a row named {name}: {:?}", rows(nodes)))
    }

    /// Highlights the result `id` with the arrow keys: the frame's screen-reader tree.
    fn highlight(open: &mut Open, id: &str) -> Vec<egui::accesskit::Node> {
        let k = open.listed().iter().position(|e| e == id).unwrap_or_else(|| panic!("{id} is listed"));
        assert!(k < VISIBLE_ROWS, "{:?}", open.listed());
        let nodes = open.press(egui::Key::ArrowDown, k);
        assert_eq!(open.highlighted(), k);
        nodes
    }

    /// With nothing selected, Ungroup stays listed in its place, dimmed, with the engine's reason in
    /// its screen-reader description and in the footer while it is highlighted. Selecting art while
    /// the palette stays open shows it as available on the next frame, in the same order. (A query
    /// other tests' plug-in names do not match, so the list stays the same between frames.)
    #[test]
    fn a_result_that_cannot_run_now_is_dimmed_and_says_why() {
        let mut session = vectorcraft_testkit::fixtures::Fixture::Multi.session();
        session.execute("select.none", &json!({})).unwrap();
        let why = ungroup_reason(&session);
        let mut open = open_on("ungroup", session);
        let entries = entries(&open.ctx, Lang::EN);
        let entry = entries.iter().find(|e| e.id == "object.ungroup").unwrap();
        let name = open.said("object.ungroup");
        let order = open.listed();
        let nodes = highlight(&mut open, "object.ungroup");
        let ungroup = row_named(&nodes, &name);
        assert!(ungroup.is_disabled(), "{ungroup:?}");
        assert_eq!(ungroup.description(), Some(why.as_str()));
        let t = Tokens::get(&open.ctx);
        // The row draws the name without the menu path its home shows.
        assert_eq!(open.color_of(label_of(entry)), Some(t.text_disabled), "the name is drawn dimmed");
        let footer_says = |nodes: &[egui::accesskit::Node]| labels(nodes).contains(&why);
        assert!(footer_says(&nodes), "{:?}", labels(&nodes));
        // The arrow keys leave screen readers on the search field, so its description says why too.
        let place = format!("{} of {}: {name}", open.highlighted() + 1, order.len());
        assert_eq!(field_description(&nodes), format!("{place}\n{why}"));
        let dimmed_rows: Vec<String> = rows(&nodes).into_iter().map(|r| r.0).collect();

        open.app.run("select.all", json!({})).unwrap();
        let nodes = open.frame(vec![]);
        let ungroup = row_named(&nodes, &name);
        assert!(!ungroup.is_disabled() && ungroup.description().is_none(), "{ungroup:?}");
        assert_eq!(open.color_of(label_of(entry)), Some(t.text));
        assert!(!footer_says(&nodes), "{:?}", labels(&nodes));
        assert_eq!(field_description(&nodes), place);
        // Whether a result can run never moves it.
        assert_eq!(open.listed(), order);
        assert_eq!(rows(&nodes).into_iter().map(|r| r.0).collect::<Vec<_>>(), dimmed_rows);
    }

    /// Why Object › Ungroup can't run in `session`, as the palette shows it: the engine's reason,
    /// with the result's name and no command id.
    fn ungroup_reason(session: &vectorcraft_engine::Session) -> String {
        let reason = (vectorcraft_engine::find_command("object.ungroup").unwrap().enabled)(session).unwrap_err();
        let why = crate::i18n::msg(&format!("command `Object › Ungroup` is not available right now: {reason}"));
        assert!(!why.contains("object.ungroup"), "{why}");
        why
    }

    /// The pointer resting on a result that can't run now shows why in a tooltip, as well as in
    /// the footer of the result it highlights.
    #[test]
    fn hovering_a_result_that_cannot_run_now_says_why() {
        let mut session = vectorcraft_testkit::fixtures::Fixture::Multi.session();
        session.execute("select.none", &json!({})).unwrap();
        let why = ungroup_reason(&session);
        let mut open = open_on("ungroup", session);
        let name = open.said("object.ungroup");
        let nodes = open.frame(vec![]);
        let b = row_named(&nodes, &name).bounds().unwrap();
        let at = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
        open.frame(vec![egui::Event::PointerMoved(at)]);
        let says = |nodes: &[egui::accesskit::Node]| labels(nodes).iter().filter(|l| **l == why).count();
        let shown = (0..30).map(|_| says(&open.frame(vec![]))).max().unwrap_or(0);
        assert_eq!(shown, 2, "the footer and the tooltip");
    }

    /// Under an empty search, a highlighted recent pick that can't run now is dimmed and the line
    /// under the picks says why.
    #[test]
    fn a_recent_pick_that_cannot_run_now_says_why() {
        let mut session = vectorcraft_testkit::fixtures::Fixture::Multi.session();
        session.execute("select.none", &json!({})).unwrap();
        let why = ungroup_reason(&session);
        let mut open = open_on("", session);
        open.app.ui.recent_commands = vec!["object.ungroup".to_string(), "object.group".to_string()];
        let nodes = open.frame(vec![]);
        assert_eq!(open.highlighted(), 0);
        let rows: Vec<_> = nodes.iter().filter(|n| n.role() == egui::accesskit::Role::ListItem).collect();
        assert_eq!(rows.len(), 2, "{rows:?}");
        assert!(rows.iter().all(|n| n.is_disabled()), "{rows:?}");
        assert!(labels(&nodes).contains(&why), "{:?}", labels(&nodes));
        let popup = open.popup();
        assert!(popup.bottom() <= WINDOW.y, "{popup:?}");
    }

    /// The Recent Commands list beside the search field draws a pick that can't run now dimmed,
    /// disabled for screen readers with the reason as its description, and says why on hover.
    #[test]
    fn a_recent_command_that_cannot_run_now_is_dimmed_in_the_list() {
        use egui::accesskit::Role;
        use egui_kittest::kittest::{NodeT, Queryable};
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        app.run("select.none", json!({})).unwrap();
        let why = ungroup_reason(&app.session);
        app.ui.recent_commands = vec!["object.ungroup".into(), "select.all".into()];
        let mut h = palette_harness(app);
        h.ctx.all_styles_mut(|s| s.interaction.tooltip_delay = 0.0);
        h.get_by_label(tl!("Recent Commands")).click();
        h.run_steps(2);
        let shown = |id: &str| entries(&h.ctx, Lang::EN).iter().find(|e| e.id == id).map(|e| e.shown.clone()).unwrap();
        let (ungroup, all) = (shown("object.ungroup"), shown("select.all"));
        // A row's label also holds its shortcut.
        let row = |name: &str| {
            let name = name.to_string();
            move |n: &egui_kittest::kittest::AccessKitNode<'_>| n.role() == Role::Button && n.label().is_some_and(|l| l.contains(&name))
        };
        let ungroup_row = h.get_by(row(&ungroup));
        assert!(ungroup_row.accesskit_node().is_disabled());
        assert_eq!(ungroup_row.accesskit_node().description().as_deref(), Some(why.as_str()));
        let all_row = h.get_by(row(&all));
        assert!(!all_row.accesskit_node().is_disabled() && all_row.accesskit_node().description().is_none());
        h.get_by(row(&ungroup)).hover();
        h.run_steps(3);
        assert!(h.query_by_label(&why).is_some(), "the tooltip says why");
    }

    /// The palette drops its answers when it closes, so opening it again, with the keyboard too,
    /// asks its results again.
    #[test]
    fn the_palette_asks_again_when_it_opens_again() {
        let mut session = vectorcraft_testkit::fixtures::Fixture::Multi.session();
        session.execute("select.none", &json!({})).unwrap();
        let mut open = open_on("ungroup", session);
        let id = egui::Id::unique("palette-availability");
        let kept = |open: &Open| open.ctx.data(|d| d.get_temp::<Availability>(id));
        assert!(kept(&open).is_some_and(|a| a.known.contains_key("object.ungroup")));
        open.app.ui.palette_open = false;
        open.frame(vec![]);
        assert!(kept(&open).is_none(), "the answers go when the palette closes");
        open.app.ui.palette_open = true;
        open.frame(vec![]);
        assert!(kept(&open).is_some_and(|a| a.asked > 0 && a.known.contains_key("object.ungroup")), "asked again");
    }

    /// What the results' availability depends on changes the key it is kept under: an open dialog,
    /// the screen mode, a background job, and a save that marks the document saved without a new
    /// revision.
    #[test]
    fn availability_is_asked_again_when_what_it_depends_on_changes() {
        let write_shared: crate::background::SharedWriteFn = std::sync::Arc::new(|_: &str, _: &[u8]| Ok(()));
        let services = crate::Services { write_shared: Some(write_shared), ..Default::default() };
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), services);
        let mut key = availability_key(&app);
        let mut changed = |app: &VectorcraftApp, what: &str| {
            let now = availability_key(app);
            assert_ne!(now, key, "{what}");
            key = now;
        };
        app.ui.dialog = Some(crate::state::Dialog::new("about", json!({})));
        changed(&app, "a dialog opens");
        app.ui.dialog = None;
        changed(&app, "the dialog closes");
        app.ui.screen_mode = 3;
        changed(&app, "Presentation Mode");
        let started = crate::background::run(&mut app, true, "Saving".into(), None, |_| Ok(json!({})), |_, r| r).unwrap();
        assert_eq!(started.get("background"), Some(&json!(true)), "{started}");
        changed(&app, "a background job starts");
        app.run("select.all", json!({})).unwrap();
        app.run("object.group", json!({})).unwrap();
        changed(&app, "an edit");
        let revision = app.session.active().map(|d| d.revision);
        app.session.active_mut().unwrap().mark_saved();
        assert_eq!(app.session.active().map(|d| d.revision), revision);
        changed(&app, "the document is saved");
    }

    /// The drawn results are asked whether they can run once per state: keys pressed in the palette
    /// keep the answers, a command run asks again, and a frame with a click asks again once,
    /// without changing the state the answers are kept for.
    #[test]
    fn availability_is_read_once_per_state() {
        let mut session = vectorcraft_testkit::fixtures::Fixture::Multi.session();
        session.execute("select.none", &json!({})).unwrap();
        let mut open = open_on("ungroup", session);
        let id = egui::Id::unique("palette-availability");
        let read = |open: &Open| open.ctx.data(|d| d.get_temp::<Availability>(id)).expect("the availability is kept");
        let first = read(&open);
        assert!(first.known.get("object.ungroup").is_some_and(Option::is_some), "{:?}", first.known);
        // The key also takes in state shared by every test (the language, the loaded fonts, the
        // installed plug-ins), which other tests change at the same time, so the presses are tried
        // again until one try keeps the key.
        let kept = (0..20).any(|_| {
            open.frame(vec![]);
            let before = read(&open).key;
            open.press(egui::Key::ArrowDown, 1);
            open.press(egui::Key::X, 1);
            read(&open).key == before
        });
        assert!(kept, "key presses keep the answers");
        let first = read(&open);
        open.app.run("select.all", json!({})).unwrap();
        open.frame(vec![]);
        let after = read(&open);
        assert_ne!(after.key, first.key);
        assert_eq!(after.known.get("object.ungroup"), Some(&None), "asked again: {:?}", after.known);
        let release = egui::Event::PointerButton {
            pos: egui::pos2(1.0, 1.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        };
        // The same retries as for the key presses.
        let once = (0..20).any(|_| {
            open.frame(vec![]);
            let before = read(&open);
            open.frame(vec![release.clone()]);
            let clicked = read(&open);
            open.frame(vec![]);
            let next = read(&open);
            let asked_again = clicked.key == before.key && clicked.asked > before.asked;
            asked_again && (next.key, next.asked) == (clicked.key, clicked.asked)
        });
        assert!(once, "a click asks again on its own frame only");
    }

    /// Paste is dimmed while the clipboard is empty and nothing can be pasted from the system
    /// clipboard, and not while the system clipboard can be pasted.
    #[test]
    fn paste_is_dimmed_only_while_nothing_can_be_pasted() {
        let mut open = open_on("paste", vectorcraft_testkit::fixtures::Fixture::Multi.session());
        open.app.system_paste = false;
        let name = open.said("edit.paste");
        let nodes = open.frame(vec![]);
        let paste = row_named(&nodes, &name);
        assert!(paste.is_disabled(), "{paste:?}");
        assert_eq!(paste.description(), Some("command `Edit › Paste` is not available right now: clipboard is empty"));
        open.app.system_paste = true;
        let nodes = open.frame(vec![]);
        assert!(!row_named(&nodes, &name).is_disabled());
    }

    /// With no document open, Paste is dimmed with the engine's reason, though the system
    /// clipboard still reads as pasteable until its next check.
    #[test]
    fn paste_is_dimmed_with_no_document_open() {
        let mut open = open_on("paste", vectorcraft_engine::Session::new());
        open.app.system_paste = true;
        let name = open.said("edit.paste");
        let nodes = open.frame(vec![]);
        let paste = row_named(&nodes, &name);
        assert!(paste.is_disabled(), "{paste:?}");
        let reason = (vectorcraft_engine::find_command("edit.paste").unwrap().enabled)(&open.app.session).unwrap_err();
        let why = format!("command `Edit › Paste` is not available right now: {reason}");
        assert_eq!(paste.description(), Some(why.as_str()));
        assert!(!menus::enabled(&open.app, "edit.paste"));
    }

    /// A tool can always be chosen; a panel can't be shown in Presentation Mode, and says so.
    #[test]
    fn tools_are_never_dimmed_and_panels_are_in_presentation_mode() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let tools: Vec<String> = build(Lang::EN).into_iter().map(|e| e.id).filter(|id| id.starts_with("tool:")).collect();
        assert!(!tools.is_empty());
        assert!(tools.iter().all(|id| unavailable(&app, id, id).is_none()), "{tools:?}");
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        assert_eq!(unavailable(&app, "panel:layers", "Window › Layers"), None);
        app.ui.screen_mode = 3;
        assert_eq!(unavailable(&app, "panel:layers", "Window › Layers").as_deref(), Some("panels can't be shown in Presentation Mode"));
    }

    /// A UI command without an engine reason says it is not available, and a filter plug-in or an
    /// effect asks about the command its pick runs. The message names the result as the palette
    /// shows it, never by the command's id.
    #[test]
    fn a_result_without_an_engine_reason_says_it_is_not_available() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        assert!(vectorcraft_engine::find_command("ui.newBrush").is_none());
        let brush = unavailable(&app, "ui.newBrush", "New Brush…");
        assert_eq!(brush.as_deref(), Some("command `New Brush…` is not available right now"));
        let filter = unavailable(&app, "plugin:any.filter", "Object › Plug-ins › Any Filter…");
        assert_eq!(filter.as_deref(), Some("command `Object › Plug-ins › Any Filter…` is not available right now"));
        let effect = crate::shortcut_editor::effect_items().first().map(|e| format!("effect:{}", e.effect)).expect("an Effect menu item");
        let why = unavailable(&app, &effect, "Effect › Any").unwrap_or_else(|| panic!("{effect}"));
        assert!(why.starts_with("command `Effect › Any` is not available right now"), "{why}");
        let app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Multi.session(), crate::Services::default());
        assert_eq!(unavailable(&app, "ui.newBrush", "New Brush…"), None);
        assert_eq!(unavailable(&app, "plugin:any.filter", "Any Filter…"), None);
        assert_eq!(unavailable(&app, &effect, "Effect › Any"), None, "{effect}");
    }

    /// Every result's reason names it as the palette shows it: no command id appears in it.
    #[test]
    fn no_reason_shows_a_command_id() {
        let app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let entries = build(Lang::EN);
        let reasons: Vec<(String, String)> =
            entries.iter().filter_map(|e| unavailable(&app, &e.id, &e.shown).map(|why| (e.id.clone(), why))).collect();
        assert!(reasons.len() > 100, "with no document most results can't run: {}", reasons.len());
        for (id, why) in &reasons {
            let command = crate::keymap::command_of(id).id;
            assert!(!why.contains(&format!("`{command}`")), "{id}: {why}");
        }
    }

    /// Screen readers stay on the field: its description names the highlighted result and its
    /// place, and follows the highlight; the rows are list items, the highlighted one selected, and
    /// the footer's count is a label.
    #[test]
    fn screen_readers_hear_the_highlighted_result_and_the_count() {
        let mut open = Open::new("show panel", WINDOW);
        let listed = open.listed();
        let entries = entries(&open.ctx, Lang::EN);
        let said: Vec<String> = listed.iter().map(|id| open.said(id)).collect();
        let label = |i: usize| said[i].clone();
        let nodes = open.frame(vec![]);
        // More results match than the list holds: the place counts every match, as the footer does.
        let total = matching(&entries, "show panel", &[]).len();
        assert!(total > MAX_RESULTS && listed.len() == MAX_RESULTS, "{total}");
        assert_eq!(field_place(&nodes), format!("1 of {total}: {}", label(0)));
        let lit: Vec<String> = rows(&nodes).into_iter().filter(|r| r.1).map(|r| r.0).collect();
        assert_eq!(lit, [label(0)]);
        assert!(labels(&nodes).contains(&format!("50 of {total} results. Keep typing to narrow them.")), "{:?}", labels(&nodes));
        let nodes = open.press(egui::Key::ArrowDown, 1);
        assert_eq!(field_place(&nodes), format!("2 of {total}: {}", label(1)));
        // Fewer results match than the list holds.
        let mut open = Open::new("eyedropper", WINDOW);
        let n = open.listed().len();
        assert!((1..MAX_RESULTS).contains(&n), "{n}");
        let first = open.said(&open.listed()[0]);
        let nodes = open.frame(vec![]);
        assert_eq!(field_place(&nodes), format!("1 of {n}: {first}"));
        assert!(labels(&nodes).contains(&footer(n, n)), "{:?}", labels(&nodes));
    }

    /// When a modal window that is not a dialog, such as the About window, closes the palette, the
    /// widget under the window's backdrop does not get the keyboard focus back.
    #[test]
    fn a_modal_window_that_closes_the_palette_keeps_the_focus_from_the_widget_under_it() {
        let ctx = egui::Context::default();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let mut text = String::new();
        let field = egui::Id::unique("other field");
        let mut frame = |app: &mut VectorcraftApp, focus: bool| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                if focus {
                    r.request_focus();
                }
                show(app, ui.ctx());
            })
            .textures_delta
            .clear();
        };
        frame(&mut app, true);
        frame(&mut app, false);
        app.ui.palette_open = true;
        frame(&mut app, false);
        frame(&mut app, false);
        // As `panel_docking::close_overlays_for_modal` does when the About window opens.
        app.ui.about = true;
        app.ui.palette_open = false;
        frame(&mut app, false);
        frame(&mut app, false);
        assert_ne!(ctx.memory(|m| m.focused()), Some(field), "the field under the About window does not take the focus");
    }

    /// The widget that had the keyboard focus when the palette opened has it again once the
    /// palette closes.
    #[test]
    fn closing_gives_the_focus_back() {
        let ctx = egui::Context::default();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let mut text = String::new();
        let field = egui::Id::unique("other field");
        let mut frame = |app: &mut VectorcraftApp, focus: bool| {
            ctx.run_ui(egui::RawInput::default(), |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                if focus {
                    r.request_focus();
                }
                show(app, ui.ctx());
            })
            .textures_delta
            .clear();
        };
        frame(&mut app, true);
        frame(&mut app, false);
        assert_eq!(ctx.memory(|m| m.focused()), Some(field));
        app.ui.palette_open = true;
        frame(&mut app, false);
        frame(&mut app, false);
        assert_ne!(ctx.memory(|m| m.focused()), Some(field), "the search field takes the focus");
        app.ui.palette_open = false;
        frame(&mut app, false);
        frame(&mut app, false);
        assert_eq!(ctx.memory(|m| m.focused()), Some(field));
        // A pick that opens a dialog leaves the focus to the dialog.
        let mut app = VectorcraftApp::new(vectorcraft_testkit::fixtures::Fixture::Single.session(), crate::Services::default());
        frame(&mut app, true);
        app.ui.palette_open = true;
        frame(&mut app, false);
        frame(&mut app, false);
        app.ui.palette_open = false;
        run_pick(&mut app, "object.path.cleanUp");
        assert!(app.ui.dialog.is_some(), "Clean Up… opens its dialog: {}", app.ui.status);
        frame(&mut app, false);
        frame(&mut app, false);
        assert_ne!(ctx.memory(|m| m.focused()), Some(field), "the dialog keeps the focus");
        // A click on a field in a window above the backdrop closes the palette and leaves the
        // focus in that field.
        let ctx = egui::Context::default();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let (mut text, mut above_text) = (String::new(), String::new());
        let above = egui::Id::unique("field above");
        let corner = egui::pos2(40.0, 400.0);
        let mut time = 0.0;
        let mut frame = |app: &mut VectorcraftApp, focus: bool, events: Vec<egui::Event>| {
            time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW);
            let input = egui::RawInput { time: Some(time), screen_rect: Some(screen), events, ..Default::default() };
            ctx.run_ui(input, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                if focus {
                    r.request_focus();
                }
                // Drawn before the palette, so the palette's own focus requests come after the field's.
                egui::Area::new(egui::Id::unique("window above")).order(egui::Order::Foreground).fixed_pos(corner).show(ui.ctx(), |ui| {
                    ui.add(egui::TextEdit::singleline(&mut above_text).id(above));
                });
                show(app, ui.ctx());
            })
            .textures_delta
            .clear();
        };
        frame(&mut app, true, vec![]);
        frame(&mut app, false, vec![]);
        app.ui.palette_open = true;
        frame(&mut app, false, vec![]);
        frame(&mut app, false, vec![]);
        let at = corner + egui::vec2(20.0, 8.0);
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, false, vec![egui::Event::PointerMoved(at)]);
        frame(&mut app, false, vec![button(true)]);
        frame(&mut app, false, vec![button(false)]);
        assert!(!app.ui.palette_open, "the click on the window above closes the palette");
        frame(&mut app, false, vec![]);
        frame(&mut app, false, vec![]);
        assert_eq!(ctx.memory(|m| m.focused()), Some(above), "the clicked field keeps the focus");
    }

    /// A pick with the keyboard from Recent Commands that opens no dialog gives the focus back to
    /// the widget that had it when the palette opened, though the picked row had the keyboard.
    #[test]
    fn a_pick_from_recent_commands_gives_the_focus_back() {
        let ctx = egui::Context::default();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        app.ui.recent_commands = vec!["paint.swap".into()];
        let mut text = String::new();
        let field = egui::Id::unique("other field");
        let mut frame = |app: &mut VectorcraftApp, focus: bool, events: Vec<egui::Event>| {
            ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut text).id(field));
                if focus {
                    r.request_focus();
                }
                show(app, ui.ctx());
            })
            .textures_delta
            .clear();
        };
        frame(&mut app, true, vec![]);
        frame(&mut app, false, vec![]);
        app.ui.palette_open = true;
        frame(&mut app, false, vec![]);
        egui::Popup::open_id(&ctx, recent_list_id());
        frame(&mut app, false, vec![]);
        frame(&mut app, false, vec![]);
        assert!(recent_list_open(&ctx), "the list is open");
        assert_ne!(ctx.memory(|m| m.focused()), Some(field), "the row has the keyboard");
        let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        frame(&mut app, false, vec![enter]);
        assert!(!app.ui.palette_open, "the pick closes the palette");
        assert!(app.ui.dialog.is_none(), "{}", app.ui.status);
        frame(&mut app, false, vec![]);
        frame(&mut app, false, vec![]);
        assert_eq!(ctx.memory(|m| m.focused()), Some(field));
    }

    /// The whole window, with a document whose rectangle is selected, drawn a frame at a time in
    /// English with screen readers on.
    struct Window {
        app: VectorcraftApp,
        ctx: egui::Context,
        time: f64,
        /// Draw a window with an "Above" button at this place, in the Foreground order after the
        /// app, as a progress window is drawn, and count its clicks.
        above: Option<egui::Pos2>,
        above_clicks: usize,
        /// Draw a modal window over the backdrop before the app's windows, as a progress window
        /// of a plug-in, trace or open run is drawn.
        progress: bool,
    }

    impl Window {
        fn new() -> Window {
            crate::i18n::set_current(Lang::EN);
            let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
            assert!(!app.session.doc().unwrap().selection.is_empty());
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let mut window = Window { app, ctx, time: 0.0, above: None, above_clicks: 0, progress: false };
            window.frame(vec![]);
            window.frame(vec![]);
            window
        }

        /// One frame of the whole window with `events`: its screen-reader tree.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::accesskit::Node> {
            self.time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW);
            let input = egui::RawInput { time: Some(self.time), screen_rect: Some(screen), events, ..Default::default() };
            let app = &mut self.app;
            let (above, clicks, progress) = (self.above, &mut self.above_clicks, self.progress);
            let mut out = self.ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
                if progress {
                    let id = egui::Id::unique("test-progress");
                    crate::dialogs::modal::show_at(ui.ctx(), "Progress", id, egui::Vec2::ZERO, 12, None, |ui| ui.label("Working"));
                }
                app.ui(ui);
                if let Some(at) = above {
                    let area = egui::Area::new(egui::Id::unique("test-above")).order(egui::Order::Foreground).fixed_pos(at);
                    area.show(ui.ctx(), |ui| {
                        if ui.button("Above").clicked() {
                            *clicks += 1;
                        }
                    });
                }
            });
            out.textures_delta.clear();
            out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
        }

        /// A primary click at `at`, a frame for the move, the press and the release: the release
        /// frame's screen-reader tree.
        fn click(&mut self, at: egui::Pos2) -> Vec<egui::accesskit::Node> {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)])
        }

        /// A press of `button` at `from`, a move to `to` and the release there, a frame each.
        fn drag(&mut self, button: egui::PointerButton, from: egui::Pos2, to: egui::Pos2) {
            let event = |pos, pressed| egui::Event::PointerButton { pos, button, pressed, modifiers: egui::Modifiers::NONE };
            self.frame(vec![egui::Event::PointerMoved(from)]);
            self.frame(vec![event(from, true)]);
            self.frame(vec![egui::Event::PointerMoved(to)]);
            self.frame(vec![event(to, false)]);
        }

        fn selection(&self) -> String {
            format!("{:?}", self.app.session.doc().unwrap().selection)
        }

        fn popup(&self) -> egui::Rect {
            self.ctx.memory(|m| m.area_rect(egui::Id::unique("palette"))).unwrap()
        }
    }

    /// The middle of the button whose screen-reader name is `name`, alone or followed by its
    /// shortcut in parentheses.
    fn button_at(nodes: &[egui::accesskit::Node], name: &str) -> egui::Pos2 {
        let names: Vec<_> = nodes.iter().filter(|n| n.role() == egui::accesskit::Role::Button).filter_map(|n| n.label()).collect();
        let named = |l: &str| l == name || l.strip_prefix(name).is_some_and(|rest| rest.starts_with(' ') && rest.trim_start().starts_with('('));
        let node = nodes
            .iter()
            .find(|n| n.role() == egui::accesskit::Role::Button && n.label().is_some_and(named))
            .unwrap_or_else(|| panic!("a button named {name}: {names:?}"));
        let b = node.bounds().unwrap();
        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    }

    /// A click on the canvas outside the popup closes it and leaves the selection as it was.
    #[test]
    fn a_click_on_the_canvas_closes_the_palette() {
        let mut window = Window::new();
        let before = window.selection();
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        let canvas = window.app.canvas_rect.expect("the canvas is drawn");
        let at = canvas.left_bottom() + egui::vec2(30.0, -30.0);
        assert!(!window.popup().contains(at), "{at:?} is outside {:?}", window.popup());
        window.click(at);
        assert!(!window.app.ui.palette_open, "the click outside closes the palette");
        window.frame(vec![]);
        assert_eq!(window.selection(), before, "the click does not reach the canvas");
    }

    /// #7906: a pick in the macOS menu bar that opens a dialog closes the palette, and the search
    /// field does not keep the keyboard from the dialog.
    #[test]
    fn a_dialog_opened_from_the_system_menu_closes_the_palette() {
        let mut window = Window::new();
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        window.frame(vec![]);
        let field = window.ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::unique("palette-field"))).expect("the search field is drawn");
        assert_eq!(window.ctx.memory(|m| m.focused()), Some(field), "the search field has the keyboard");
        menus::invoke_from_system_menu(&mut window.app, &window.ctx, "object.move", json!({}));
        assert!(window.app.ui.dialog.is_some(), "Move… opens its dialog: {}", window.app.ui.status);
        window.frame(vec![]);
        assert!(!window.app.ui.palette_open, "the dialog closes the palette");
        window.frame(vec![]);
        assert!(window.app.ui.dialog.is_some(), "the dialog stays open");
        assert_ne!(window.ctx.memory(|m| m.focused()), Some(field), "the search field does not keep the keyboard");
    }

    /// #7906: with the palette open, the About window that opens without a press closes the palette.
    #[test]
    fn the_about_window_closes_the_palette() {
        let mut window = Window::new();
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        menus::invoke_from_system_menu(&mut window.app, &window.ctx, "help.about", json!({}));
        assert!(window.app.ui.about, "About opens");
        window.frame(vec![]);
        assert!(!window.app.ui.palette_open, "the About window closes the palette");
    }

    /// #7906: a window drawn over the modal backdrop, as a plug-in, trace or open run's progress
    /// window is, closes the palette in the pass it first draws.
    #[test]
    fn a_progress_window_closes_the_palette() {
        let mut window = Window::new();
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        assert!(window.app.ui.palette_open, "the palette is open");
        window.progress = true;
        window.frame(vec![]);
        assert!(!window.app.ui.palette_open, "the progress window closes the palette");
    }

    /// #7906: the palette opened from the macOS menu bar while a dialog is open closes again, and its
    /// search field does not take the keyboard from the dialog.
    #[test]
    fn the_palette_opened_over_a_dialog_closes() {
        let mut window = Window::new();
        menus::invoke(&mut window.app, "object.move", json!({}));
        assert!(window.app.ui.dialog.is_some(), "Move… opens its dialog: {}", window.app.ui.status);
        window.frame(vec![]);
        window.frame(vec![]);
        let before = window.ctx.memory(|m| m.focused());
        menus::invoke_from_system_menu(&mut window.app, &window.ctx, "help.commandPalette", json!({}));
        assert!(window.app.ui.palette_open, "the command opens the palette");
        window.frame(vec![]);
        window.frame(vec![]);
        assert!(!window.app.ui.palette_open, "the dialog keeps the palette closed");
        assert!(window.app.ui.dialog.is_some(), "the dialog stays open");
        let field = window.ctx.data(|d| d.get_temp::<egui::Id>(egui::Id::unique("palette-field")));
        let now = window.ctx.memory(|m| m.focused());
        assert!(field.is_none() || now != field, "the search field does not take the keyboard");
        assert_eq!(now, before, "the dialog keeps its focus");
    }

    /// #7906 as reported: with the palette open, a click on the Effect menu in the window's menu bar
    /// closes the palette, and a dialog opened afterwards is not under it.
    #[test]
    fn a_click_on_the_menu_bar_closes_the_palette_before_a_dialog_opens() {
        let mut window = Window::new();
        window.app.ui.palette_open = true;
        let nodes = window.frame(vec![]);
        window.click(button_at(&nodes, "Effect"));
        assert!(!window.app.ui.palette_open, "the click on the menu bar closes the palette");
        menus::invoke(&mut window.app, "object.move", json!({}));
        assert!(window.app.ui.dialog.is_some(), "Move… opens its dialog: {}", window.app.ui.status);
        window.frame(vec![]);
        window.frame(vec![]);
        assert!(!window.app.ui.palette_open, "the palette stays closed over the dialog");
    }

    /// Opens `overlay` ("flyout", "drawer" or "menu") in a new window, then opens Move… without a
    /// press, as the macOS menu bar or an automation command does: whether the overlay is still open.
    fn overlay_open_after_a_dialog_opens(overlay: &str) -> bool {
        let mut window = Window::new();
        match overlay {
            "flyout" => {
                window.app.ui.flyout = Some(0);
                let tools = vec!["rectangle".to_string(), "ellipse".to_string()];
                window.ctx.data_mut(|d| d.insert_temp(egui::Id::unique("flyout-tools"), tools));
            }
            "drawer" => window.app.ui.tools_drawer = true,
            _ => {
                let nodes = window.frame(vec![]);
                window.click(button_at(&nodes, "Object"));
            }
        }
        window.frame(vec![]);
        let open = |w: &Window| w.app.ui.flyout.is_some() || w.app.ui.tools_drawer || egui::Popup::is_any_open(&w.ctx);
        assert!(open(&window), "the {overlay} is open");
        menus::invoke(&mut window.app, "object.move", json!({}));
        assert!(window.app.ui.dialog.is_some(), "Move… opens its dialog: {}", window.app.ui.status);
        window.frame(vec![]);
        open(&window)
    }

    /// #7906: a dialog that opens without a press closes a tool group's flyout.
    #[test]
    fn a_dialog_opened_without_a_press_closes_a_flyout() {
        assert!(!overlay_open_after_a_dialog_opens("flyout"), "the dialog closes the flyout");
    }

    /// #7906: a dialog that opens without a press closes the All Tools drawer.
    #[test]
    fn a_dialog_opened_without_a_press_closes_the_tools_drawer() {
        assert!(!overlay_open_after_a_dialog_opens("drawer"), "the dialog closes the drawer");
    }

    /// #7906: a dialog that opens without a press closes an open menu.
    #[test]
    fn a_dialog_opened_without_a_press_closes_a_menu() {
        assert!(!overlay_open_after_a_dialog_opens("menu"), "the dialog closes the menu");
    }

    /// The palette opened in the pass after a dialog closes stays open, and the next dialog to open,
    /// one pass later, closes it.
    #[test]
    fn the_palette_opened_as_a_dialog_closes_stays_open_until_the_next_dialog() {
        let mut window = Window::new();
        menus::invoke(&mut window.app, "object.move", json!({}));
        assert!(window.app.ui.dialog.is_some(), "Move… opens its dialog: {}", window.app.ui.status);
        window.frame(vec![]);
        window.frame(vec![]);
        crate::dialogs::cancel(&mut window.app);
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        assert!(window.app.ui.palette_open, "the palette stays open");
        menus::invoke(&mut window.app, "object.move", json!({}));
        assert!(window.app.ui.dialog.is_some(), "Move… opens its dialog again: {}", window.app.ui.status);
        window.frame(vec![]);
        assert!(!window.app.ui.palette_open, "the next dialog closes the palette");
    }

    /// A click on another control outside the popup closes the popup and does not press the
    /// control.
    #[test]
    fn a_click_on_another_control_closes_the_palette_without_pressing_it() {
        let mut window = Window::new();
        let tool = window.app.session.tool_id();
        window.app.ui.palette_open = true;
        let nodes = window.frame(vec![]);
        let at = button_at(&nodes, "Pen Tool");
        assert!(!window.popup().contains(at), "{at:?} is outside {:?}", window.popup());
        window.click(at);
        assert!(!window.app.ui.palette_open, "the click outside closes the palette");
        window.frame(vec![]);
        assert_eq!(window.app.session.tool_id(), tool, "the click does not pick the tool");
        // With the palette closed, the same click picks it.
        window.click(at);
        assert_eq!(window.app.session.tool_id(), "pen");
    }

    /// A right-click outside the popup closes it, and so does a drag that starts outside it.
    #[test]
    fn a_right_click_or_a_drag_outside_closes_the_palette() {
        let mut window = Window::new();
        let before = window.selection();
        let canvas = window.app.canvas_rect.expect("the canvas is drawn");
        let at = canvas.left_bottom() + egui::vec2(30.0, -30.0);
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        assert!(!window.popup().contains(at), "{at:?} is outside {:?}", window.popup());
        window.drag(egui::PointerButton::Secondary, at, at);
        assert!(!window.app.ui.palette_open, "the right-click outside closes the palette");
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        window.drag(egui::PointerButton::Primary, at, at + egui::vec2(80.0, -60.0));
        assert!(!window.app.ui.palette_open, "the drag outside closes the palette");
        window.frame(vec![]);
        assert_eq!(window.selection(), before, "the drag does not reach the canvas");
    }

    /// A click on a window drawn above the backdrop, as a progress window is, closes the palette
    /// and presses that window's button, beside the popup and where the window lies over it.
    #[test]
    fn a_click_on_a_window_above_closes_the_palette_and_reaches_it() {
        for over in [false, true] {
            let mut window = Window::new();
            window.app.ui.palette_open = true;
            window.frame(vec![]);
            let popup = window.popup();
            window.above = Some(if over { popup.center() } else { egui::pos2(40.0, 400.0) });
            window.frame(vec![]);
            let nodes = window.frame(vec![]);
            let at = button_at(&nodes, "Above");
            assert_eq!(popup.contains(at), over, "{at:?} and the popup {popup:?}");
            window.click(at);
            assert!(!window.app.ui.palette_open, "over the popup: {over}; the click on the window closes the palette");
            assert_eq!(window.above_clicks, 1, "over the popup: {over}; the window gets the click");
        }
    }

    /// The click on the app bar's search field that opens the palette leaves it open; the next
    /// click on the field closes it.
    #[test]
    fn the_click_that_opens_the_palette_does_not_close_it() {
        let mut window = Window::new();
        let nodes = window.frame(vec![]);
        let at = button_at(&nodes, "Search commands");
        window.click(at);
        assert!(window.app.ui.palette_open, "the click opens the palette");
        window.frame(vec![]);
        window.frame(vec![]);
        assert!(window.app.ui.palette_open, "and it stays open");
        window.click(at);
        assert!(!window.app.ui.palette_open, "a second click closes it");
    }

    /// Clicks inside the popup keep it open: one in the search field, and one on a result, which
    /// runs that result.
    #[test]
    fn clicks_inside_the_palette_keep_it_or_run_a_result() {
        let mut window = Window::new();
        window.app.ui.palette_open = true;
        window.app.ui.palette_query = "show panel".into();
        window.frame(vec![]);
        let nodes = window.frame(vec![]);
        let popup = window.popup();
        let middle = |b: egui::accesskit::Rect| egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
        let field = nodes
            .iter()
            .filter(|n| n.role() == egui::accesskit::Role::TextInput)
            .filter_map(|n| n.bounds().map(middle))
            .find(|p| popup.contains(*p))
            .expect("the search field");
        window.click(field);
        window.frame(vec![]);
        assert!(window.app.ui.palette_open, "a click in the field keeps the palette open");
        assert_eq!(window.app.ui.palette_query, "show panel");
        let nodes = window.frame(vec![]);
        let entries = entries(&window.ctx, Lang::EN);
        let listed: Vec<_> = matching(&entries, "show panel", &[]).into_iter().take(MAX_RESULTS).collect();
        let hidden = |e: &&&Entry| e.id.strip_prefix("panel:").is_some_and(|p| !crate::panel_docking::showing(&window.app.ui, p));
        let e = listed.iter().find(hidden).expect("a hidden panel's result");
        let label = row_label(e, &home_text(&window.app, e));
        let row = rows(&nodes).into_iter().find(|r| r.0 == label).expect("its row is drawn");
        let panel = e.id.strip_prefix("panel:").unwrap_or_default().to_string();
        window.click(egui::pos2(window.popup().center().x, ((row.2 + row.3) / 2.0) as f32));
        assert!(!window.app.ui.palette_open);
        assert!(crate::panel_docking::showing(&window.app.ui, &panel), "{panel}: {}", window.app.ui.status);
    }

    /// The entries and their index are built when the palette first opens, not before.
    #[test]
    fn the_search_index_is_built_when_the_palette_opens() {
        type Cached = ((&'static str, u64, u64), bool, Arc<Catalog>);
        let built = |ctx: &egui::Context| ctx.data(|d| d.get_temp::<Cached>(egui::Id::unique("palette-entries"))).is_some();
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        let frame = |app: &mut VectorcraftApp| {
            ctx.run_ui(egui::RawInput::default(), |ui| show(app, ui.ctx())).textures_delta.clear();
        };
        frame(&mut app);
        assert!(!built(&ctx), "nothing is built while the palette is closed");
        app.ui.palette_open = true;
        frame(&mut app);
        assert!(built(&ctx));
    }

    /// The highlight stays on its result when the results change under the same query: when a
    /// plug-in whose name ranks above it is installed, a shortcut changes, or the language
    /// changes.
    #[test]
    fn the_highlight_stays_on_its_result_when_the_results_change() {
        use vectorcraft_plugins::wat as tpl;
        let mut open = Open::new("group", WINDOW);
        // Deep enough in the list that the highlighted result's name does not start with the query,
        // also while other tests' "Group" filter plug-ins are installed.
        open.press(egui::Key::ArrowDown, 6);
        let held = |open: &Open| open.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap();
        let id = held(&open).4;
        assert_eq!(open.listed().get(6), Some(&id));
        // A name no other test gives a plug-in, so the other tests' results keep their names.
        let filter = "test.UI.palette_highlight-group";
        let manifest = format!(r#"{{"id":"{filter}","name":"Group Highlight Probe","version":"1","kind":"filter","params":{{}}}}"#);
        vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
        open.frame(vec![]);
        let listed = open.listed();
        let plugin = listed.iter().position(|r| *r == format!("plugin:{filter}"));
        vectorcraft_plugins::registry::remove(filter);
        let was = listed.iter().position(|r| *r == id);
        assert!(plugin.is_some() && plugin < was, "the plug-in ranks above the highlight: {listed:?}");
        let (sel, .., now) = held(&open);
        assert_eq!((listed.get(sel), &now), (Some(&id), &id), "after the plug-in was installed");
        crate::shortcut_editor::GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        open.frame(vec![]);
        assert_eq!(held(&open).4, id, "after a shortcut changed");
        crate::i18n::set_current(Lang::from_code("de").unwrap());
        open.frame(vec![]);
        let (sel, .., now) = held(&open);
        let de = entries(&open.ctx, Lang::from_code("de").unwrap());
        let de_listed: Vec<String> = matching(&de, "group", &[]).iter().map(|e| e.id.clone()).collect();
        crate::i18n::set_current(Lang::EN);
        assert_eq!((de_listed.get(sel), &now), (Some(&id), &id), "after the language changed");
    }

    /// A highlighted last shown result that a new result pushes past the limit stays shown and
    /// highlighted, in the last row, so Enter still runs it; screen readers hear its place among
    /// every match.
    #[test]
    fn the_highlighted_last_result_stays_shown_when_a_result_ranks_above_it() {
        use vectorcraft_plugins::wat as tpl;
        // A query no other test searches for, so the plug-in below leaves their results alone.
        let mut open = Open::new("path", WINDOW);
        assert!(matching(&entries(&open.ctx, Lang::EN), "path", &[]).len() > MAX_RESULTS);
        open.press(egui::Key::ArrowUp, 1);
        let held = |open: &Open| open.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap();
        let (sel, .., id) = held(&open);
        assert_eq!(sel, MAX_RESULTS - 1);
        let filter = "test.UI.palette_last-highlight-path";
        let manifest = format!(r#"{{"id":"{filter}","name":"Path Last Highlight Probe","version":"1","kind":"filter","params":{{}}}}"#);
        vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
        open.frame(vec![]);
        let entries = entries(&open.ctx, Lang::EN);
        let ranked: Vec<&Entry> = matching(&entries, "path", &[]);
        let nodes = open.frame(vec![]);
        let (sel, .., now) = held(&open);
        vectorcraft_plugins::registry::remove(filter);
        let plugin = ranked.iter().position(|e| e.id == format!("plugin:{filter}"));
        assert!(plugin.is_some_and(|p| p < MAX_RESULTS), "the plug-in is shown: {plugin:?}");
        assert_eq!(ranked.iter().position(|e| e.id == id), Some(MAX_RESULTS), "the plug-in pushed the highlight past the limit");
        assert_eq!((sel, &now), (MAX_RESULTS - 1, &id), "the highlight stays on its result, in the last row");
        let label = row_label(ranked[MAX_RESULTS], &home_text(&open.app, ranked[MAX_RESULTS]));
        assert!(rows(&nodes).iter().any(|r| r.1 && r.0 == label), "its row is drawn and highlighted");
        assert_eq!(field_place(&nodes), format!("{} of {}: {label}", MAX_RESULTS + 1, ranked.len()));
    }

    impl Window {
        /// Opens the palette on `query` and draws until the toolbars and the menu bar have noted
        /// their controls: the last frame's screen-reader tree.
        fn search(&mut self, query: &str) -> Vec<egui::accesskit::Node> {
            self.app.ui.palette_open = true;
            self.app.ui.palette_query = query.into();
            self.frame(vec![]);
            self.frame(vec![])
        }

        /// The highlighted result's id.
        fn highlighted(&self) -> String {
            self.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap().4
        }

        /// The callout the last frame drew: the outlined control, the box and its text.
        fn callout(&self) -> Callout {
            self.ctx.data(|d| d.get_temp::<Callout>(callout_id())).flatten()
        }

        /// The text of the labels in the popup in the screen-reader tree `nodes`: the footer's.
        fn popup_labels(&self, nodes: &[egui::accesskit::Node]) -> Vec<String> {
            let popup = self.popup();
            nodes
                .iter()
                .filter(|n| n.role() == egui::accesskit::Role::Label)
                .filter(|n| n.bounds().is_some_and(|b| popup.contains(egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32))))
                .filter_map(|n| n.label().or(n.value()).map(str::to_string))
                .collect()
        }
    }

    /// The bounds of the control whose screen-reader name is `name`, alone or followed by its
    /// shortcut in parentheses.
    fn bounds_of(nodes: &[egui::accesskit::Node], name: &str) -> egui::Rect {
        let at = button_at(nodes, name);
        let node = nodes
            .iter()
            .filter(|n| n.role() == egui::accesskit::Role::Button)
            .find(|n| n.bounds().is_some_and(|b| ((b.x0 + b.x1) / 2.0) as f32 == at.x && ((b.y0 + b.y1) / 2.0) as f32 == at.y))
            .unwrap();
        let b = node.bounds().unwrap();
        egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))
    }

    /// Highlighting a result with an item of the in-window menu bar outlines that menu's title, as
    /// drawn in the same frame, and the box under it shows the item's whole path.
    #[test]
    fn the_highlighted_result_outlines_its_menu_title() {
        let mut window = Window::new();
        let nodes = window.search("round corners");
        assert_eq!(window.highlighted(), "effect:stylize.roundCorners");
        let (outlined, callout, text) = window.callout().expect("a callout");
        assert_eq!(text, "Effect › Stylize › Round Corners…");
        assert_eq!(outlined, bounds_of(&nodes, "Effect"));
        let callout = callout.expect("a box");
        assert!(callout.top() >= outlined.bottom(), "the box is under the title: {callout:?} {outlined:?}");
        // A panel's home is the Window menu.
        window.search("layers");
        assert_eq!(window.highlighted(), "panel:layers");
        let (outlined, _, text) = window.callout().expect("a callout");
        assert_eq!(text, "Window › Layers");
        assert_eq!(outlined, bounds_of(&window.frame(vec![]), "Window"));
        // Closing the palette ends the callout.
        window.app.ui.palette_open = false;
        window.frame(vec![]);
        assert_eq!(window.callout(), None);
    }

    struct NativeMenus;
    impl crate::native_menu::Backend for NativeMenus {
        fn sync(&mut self, _: &crate::native_menu::MenuBar) {}
        fn drain(&mut self) -> Vec<crate::native_menu::Event> {
            Vec::new()
        }
    }

    /// With the system's menu bar, nothing is drawn over it: the footer names the menu path.
    #[test]
    fn with_a_native_menu_bar_the_footer_names_the_path() {
        let mut window = Window::new();
        window.app.services.native_menu = Some(crate::native_menu::NativeMenu::new(Box::new(NativeMenus)));
        let nodes = window.search("round corners");
        assert_eq!(window.highlighted(), "effect:stylize.roundCorners");
        assert_eq!(window.callout(), None);
        assert!(labels(&nodes).contains(&"In the menu bar: Effect › Stylize › Round Corners…".to_string()), "{:?}", labels(&nodes));
        // The application menu's items have no label of their own: the path ends at the menu.
        let nodes = window.search("help.about");
        assert_eq!(window.highlighted(), "help.about");
        let menu = format!("In the menu bar: {}", crate::native_menu::APP_NAME);
        assert!(labels(&nodes).contains(&menu), "{:?}", labels(&nodes));
    }

    /// A built-in effect's item in the path translates by its effect id, as the Effect menu shows
    /// it, another item by its command id, and a plug-in's name stays as written.
    #[test]
    fn the_path_names_an_effect_item_as_the_effect_menu_does() {
        let mut window = Window::new();
        window.app.services.native_menu = Some(crate::native_menu::NativeMenu::new(Box::new(NativeMenus)));
        let fr = Lang::from_code("fr").unwrap();
        crate::i18n::set_current(fr);
        let list = entries(&window.ctx, fr);
        let add = list.iter().find(|e| e.id == "effect:pathfinder.add").expect("the Add effect");
        let shown = match pointer(&window.app, &window.ctx, add) {
            Pointer::Footer(text) => text,
            Pointer::At(_, text, _) => text,
        };
        crate::i18n::set_current(Lang::EN);
        assert!(shown.ends_with(" › Réunion"), "{shown}");
        // A live-effect plug-in's name stays as written, as the Effect menu shows it.
        let plugin = Entry {
            id: format!("effect:{}", vectorcraft_plugins::effect::effect_id("test.ui.add")),
            shortcut: String::new(),
            shown: "Add".into(),
            haystack: String::new(),
            home: Some("Effect › Plug-ins".into()),
            menu_item: "Add",
            kind: 0,
            label_at: 0,
            names_item: false,
        };
        assert_ne!(crate::i18n::tr(fr, "Add"), "Add");
        assert_eq!(item_label(fr, &plugin), "Add");
        // So does a filter plug-in's name, as Object › Plug-ins shows it.
        let filter = Entry { id: "plugin:test.ui.add".into(), home: Some("Object › Plug-ins".into()), ..plugin };
        assert_eq!(item_label(fr, &filter), "Add");
        // Another item translates by the id of the command it runs, as the menu bar draws it.
        let es = Lang::from_code("es").unwrap();
        let close = entries(&window.ctx, es).iter().find(|e| e.id == "file.close").map(|e| (e.menu_item, item_label(es, e)));
        assert_ne!(crate::i18n::tr(es, "Close"), "Cerrar archivo");
        assert_eq!(close, Some(("Close", "Cerrar archivo")));
    }

    /// A tool hidden in a group outlines its group's button and says how to open the group; once
    /// chosen from the group, the button shows it and the box names the tool alone. The box is on
    /// the canvas side of the toolbar.
    #[test]
    fn a_tool_result_outlines_its_toolbar_slot() {
        let mut window = Window::new();
        let nodes = window.search("group selection tool");
        assert_eq!(window.highlighted(), "tool:groupSelection");
        let (outlined, callout, text) = window.callout().expect("a callout");
        assert_eq!(text, "In the Direct Selection Tool group: press and hold to open it");
        assert_eq!(outlined, bounds_of(&nodes, "Direct Selection Tool"));
        // Screen readers hear how to open the group with the search field's description.
        let said: Vec<_> = nodes.iter().filter(|n| n.role() == egui::accesskit::Role::TextInput).filter_map(|n| n.description()).collect();
        assert!(said.iter().any(|d| d.ends_with(", In the Direct Selection Tool group: press and hold to open it")), "{said:?}");
        let callout = callout.expect("a box");
        assert!(callout.left() > outlined.right(), "right of a toolbar docked left: {callout:?} {outlined:?}");
        window.app.select_tool("groupSelection");
        let nodes = window.search("group selection tool");
        let (outlined, _, text) = window.callout().unwrap_or_else(|| panic!("a callout: {:?} {:?}", labels(&nodes), window.popup()));
        assert_eq!(text, "Group Selection Tool");
        assert_eq!(outlined, bounds_of(&nodes, "Group Selection Tool"));
        // Docked right, the box goes on its left. The Zoom Tool's slot is below the popup.
        window.app.ui.toolbar_right = true;
        window.frame(vec![]);
        let nodes = window.search("zoom tool");
        assert_eq!(window.highlighted(), "tool:zoom");
        let (outlined, callout, text) = window.callout().unwrap_or_else(|| panic!("a callout: {:?} {:?}", labels(&nodes), window.popup()));
        assert_eq!(text, "Zoom Tool");
        assert!(!window.popup().intersects(outlined), "{outlined:?} {:?}", window.popup());
        let callout = callout.expect("a box");
        assert!(callout.right() < outlined.left(), "left of a toolbar docked right: {callout:?} {outlined:?}");
    }

    /// A shown toolbar made with New Toolbar called Drawing that floats at `pos` and holds the Pen
    /// Tool and the Curvature Tool in one group.
    fn drawing_toolbar(pos: [f32; 2]) -> crate::state::CustomToolbar {
        crate::state::CustomToolbar {
            name: "Drawing".into(),
            slots: vec![vec!["pen".into(), "curvature".into()]],
            shown: true,
            pos,
            dock: None,
            dock_right: false,
            slot_tool: Default::default(),
            double: false,
            floating: vec![],
        }
    }

    /// With the Tools panel hidden, a shown toolbar made with New Toolbar that holds the tool gets
    /// the outline; a group torn off it into a strip has the tool's own button outlined.
    #[test]
    fn a_tool_on_a_toolbar_made_with_new_toolbar_is_outlined_there() {
        let mut window = Window::new();
        window.app.ui.toolbar = false;
        window.app.ui.custom_toolbars.push(drawing_toolbar([100.0, 200.0]));
        let nodes = window.search("pen tool");
        assert_eq!(window.highlighted(), "tool:pen");
        let (outlined, callout, text) = window.callout().expect("a callout");
        assert_eq!(text, "Pen Tool");
        assert!(callout.is_some());
        assert_eq!(outlined, bounds_of(&nodes, "Pen Tool"));
        assert!(outlined.left() >= 100.0, "on the toolbar made with New Toolbar: {outlined:?}");
        let bar = window.app.ui.custom_toolbars[0].name.clone();
        assert_eq!(tool_home(&window.app, "curvature"), format!("{bar}, in the Pen Tool group"));
        // The group torn off into a strip.
        window.app.ui.custom_toolbars[0]
            .floating
            .push(crate::state::FloatingFlyout { tools: vec!["pen".into(), "curvature".into()], pos: [600.0, 500.0] });
        window.frame(vec![]);
        window.frame(vec![]);
        let (outlined, _, text) = window.callout().expect("a callout");
        assert_eq!(text, "Pen Tool");
        assert!(outlined.left() >= 600.0 && outlined.top() >= 500.0, "on the strip: {outlined:?}");
        // The row's home names no group: the strip shows each tool on its own button.
        assert_eq!(tool_home(&window.app, "curvature"), bar);
    }

    /// A group torn off the Tools panel into a strip has the tool's own button on the strip
    /// outlined, with the tool's name.
    #[test]
    fn a_strip_torn_off_the_tools_panel_is_pointed_at() {
        let mut window = Window::new();
        let slot = crate::toolbar::slots(&window.app).into_iter().map(|(_, s)| s).find(|s| s.contains(&"groupSelection")).expect("the group");
        let tools = slot.iter().map(|t| t.to_string()).collect();
        window.app.ui.floating_flyouts.push(crate::state::FloatingFlyout { tools, pos: [600.0, 500.0] });
        window.frame(vec![]);
        window.search("group selection tool");
        assert_eq!(window.highlighted(), "tool:groupSelection");
        let (outlined, callout, text) = window.callout().expect("a callout");
        assert_eq!(text, "Group Selection Tool");
        assert!(callout.is_some());
        assert!(outlined.left() >= 600.0 && outlined.top() >= 500.0, "on the strip: {outlined:?}");
        // The row's home names no group: the strip shows each tool on its own button.
        assert_eq!(tool_home(&window.app, "groupSelection"), "Toolbar");
    }

    /// Two viewports that note controls in turn keep one pass each in the list, whichever noted
    /// first, also after one of them stops noting controls.
    #[test]
    fn the_noted_controls_hold_one_pass_per_viewport() {
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(10.0, 10.0));
        let spot = |viewport: egui::ViewportId, pass: u64| Spot {
            at: SpotAt::Menu("File".into()),
            viewport,
            pass,
            layer: egui::LayerId::background(),
            clip: rect,
            rect,
        };
        let (a, b) = (egui::ViewportId::ROOT, egui::ViewportId::from_hash_of("another window"));
        let mut list = Vec::new();
        for pass in 0..20 {
            for _ in 0..3 {
                add_spot(&mut list, spot(a, pass));
            }
            for _ in 0..2 {
                add_spot(&mut list, spot(b, pass));
            }
            assert_eq!(list.len(), 5, "pass {pass}");
            assert!(list.iter().all(|s| s.pass == pass), "pass {pass}");
        }
        // The other window stops noting controls.
        for pass in 20..40 {
            for _ in 0..3 {
                add_spot(&mut list, spot(a, pass));
            }
            assert_eq!(list.len(), 5, "pass {pass}");
        }
    }

    /// Beside a button, the box's text wraps to at most three rows to fit between the button and
    /// the popup; with no popup beside it, it is one row.
    #[test]
    fn a_box_beside_a_button_wraps_to_fit_beside_the_popup() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
        let mut checked = false;
        let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
        let mut out = ctx.run_ui(input, |ui| {
            let ctx = ui.ctx();
            let rect = egui::Rect::from_min_size(egui::pos2(20.0, 100.0), egui::vec2(28.0, 28.0));
            let place = Spot { at: SpotAt::Slot(None, vec![]), viewport: ctx.viewport_id(), pass: 0, layer: ui.layer_id(), clip: screen, rect };
            let popup = egui::Rect::from_min_size(egui::pos2(rect.right() + 150.0, 50.0), egui::vec2(400.0, 300.0));
            let text = "In the Direct Selection Tool group: press and hold to open it";
            let (boxed, galley) = callout_box(ctx, &place, text, Some(popup));
            assert!((2..=BOX_ROWS).contains(&galley.rows.len()), "{} rows", galley.rows.len());
            assert!(boxed.left() > rect.right() && boxed.right() <= popup.left(), "{boxed:?} {popup:?}");
            let (_, one) = callout_box(ctx, &place, text, None);
            assert_eq!(one.rows.len(), 1);
            checked = true;
        });
        out.textures_delta.clear();
        assert!(checked);
    }

    /// The box never covers the button it points at: in a narrow window it wraps to fit between
    /// the button and the window's edge, and where it cannot, only the outline is drawn and the
    /// footer shows the text.
    #[test]
    fn the_box_keeps_off_its_button() {
        let text = "In the Direct Selection Tool group: press and hold to open it";
        let check = |width: f32, left: f32| {
            let ctx = egui::Context::default();
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(width, 300.0));
            let mut how = None;
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx();
                let rect = egui::Rect::from_min_size(egui::pos2(left, 100.0), egui::vec2(28.0, 28.0));
                let place = Spot { at: SpotAt::Slot(None, vec![]), viewport: ctx.viewport_id(), pass: 0, layer: ui.layer_id(), clip: screen, rect };
                let aimed = aim(ctx, None, &place, text);
                if aimed == Aim::Box {
                    let (boxed, _) = callout_box(ctx, &place, text, None);
                    assert!(!boxed.intersects(rect) && screen.contains_rect(boxed), "{boxed:?} {rect:?}");
                }
                how = Some(aimed);
            });
            out.textures_delta.clear();
            how
        };
        assert_eq!(check(400.0, 180.0), Some(Aim::Box));
        assert_eq!(check(400.0, 192.0), Some(Aim::Box));
        assert_eq!(check(180.0, 60.0), Some(Aim::Outline));
    }

    /// The callout never covers the palette: a button under the popup is not pointed at, and a
    /// button whose box would cover the popup is outlined with its text in the footer.
    #[test]
    fn the_callout_stays_off_the_palette() {
        let mut window = Window::new();
        window.app.ui.toolbar = false;
        window.app.ui.custom_toolbars.push(drawing_toolbar([600.0, 120.0]));
        let nodes = window.search("pen tool");
        assert_eq!(window.highlighted(), "tool:pen");
        assert!(window.popup().contains(egui::pos2(620.0, 140.0)), "the toolbar is under the popup: {:?}", window.popup());
        assert_eq!(window.callout(), None);
        // The toolbar is shown, under the popup: the footer names it and does not say it is hidden.
        let footer = window.popup_labels(&nodes);
        assert!(footer.contains(&"Drawing".to_string()), "{footer:?}");
        assert!(!footer.iter().any(|l| l.contains("not shown now")), "{footer:?}");
        // Left of the popup, with the box's place to its right on the popup.
        let popup = window.popup();
        window.app.ui.custom_toolbars[0].pos = [popup.left() - 60.0, popup.top() + 30.0];
        window.frame(vec![]);
        let nodes = window.frame(vec![]);
        let (outlined, callout, text) = window.callout().expect("a callout");
        assert!(!popup.intersects(outlined), "{outlined:?} {popup:?}");
        assert_eq!((callout, text.as_str()), (None, "Pen Tool"));
        assert!(labels(&nodes).contains(&"Pen Tool".to_string()), "{:?}", labels(&nodes));
    }

    /// In the pass where more results make the popup taller and it now covers the button, the
    /// callout is checked against the popup's new size, and the next pass's footer names the home.
    #[test]
    fn the_callout_keeps_off_a_popup_that_grew_in_the_same_pass() {
        let mut window = Window::new();
        window.app.ui.toolbar = false;
        window.app.ui.custom_toolbars.push(drawing_toolbar([100.0, 120.0]));
        window.search("pen tool");
        let small = window.popup();
        window.app.ui.custom_toolbars[0].pos = [small.center().x, small.bottom() + 2.0];
        window.frame(vec![]);
        window.frame(vec![]);
        assert_eq!(window.highlighted(), "tool:pen");
        let (outlined, ..) = window.callout().expect("a callout below the popup");
        window.app.ui.palette_query = "pen".into();
        window.frame(vec![]);
        assert_eq!(window.highlighted(), "tool:pen");
        assert!(window.popup().intersects(outlined), "the popup grew over the button: {:?} {outlined:?}", window.popup());
        assert_eq!(window.callout(), None);
        let nodes = window.frame(vec![]);
        let footer = window.popup_labels(&nodes);
        assert!(footer.contains(&"Drawing".to_string()), "{footer:?}");
        assert!(!footer.iter().any(|l| l.contains("not shown now")), "{footer:?}");
    }

    /// Under the recent picks of an empty search, the footer still says where the highlighted
    /// pick's home is when no callout can show it, without a count of the results.
    #[test]
    fn under_the_recent_picks_the_footer_names_a_home_out_of_view() {
        let mut window = Window::new();
        window.app.ui.recent_commands = vec!["tool:pen".into()];
        window.app.ui.screen_mode = 3;
        let nodes = window.search("");
        assert_eq!(window.highlighted(), "tool:pen");
        assert_eq!(window.callout(), None);
        let footer = window.popup_labels(&nodes);
        assert!(footer.contains(&"Toolbar, not shown now".to_string()), "{footer:?}");
        assert!(!footer.iter().any(|l| l.ends_with(" result") || l.ends_with(" results")), "no count: {footer:?}");
    }

    /// No callout where no control in view shows the home: in Presentation Mode, with the Tools
    /// panel hidden, and for a command with no menu item. The footer says where it lives.
    #[test]
    fn a_home_out_of_view_is_named_in_the_footer() {
        let mut window = Window::new();
        window.app.ui.screen_mode = 3;
        let nodes = window.search("pen tool");
        assert_eq!(window.highlighted(), "tool:pen");
        assert_eq!(window.callout(), None);
        assert!(labels(&nodes).contains(&"Toolbar, not shown now".to_string()), "{:?}", labels(&nodes));
        window.app.ui.screen_mode = 0;
        window.app.ui.toolbar = false;
        let nodes = window.search("pen tool");
        assert_eq!(window.callout(), None);
        assert!(labels(&nodes).contains(&"Toolbar, not shown now".to_string()), "{:?}", labels(&nodes));
        // Toolbars hidden with Tab.
        window.app.ui.toolbar = true;
        window.app.run("window.dock", serde_json::json!({})).expect("Tab");
        assert!(window.app.ui.tab_hid.is_some());
        let nodes = window.search("pen tool");
        assert_eq!(window.highlighted(), "tool:pen");
        assert_eq!(window.callout(), None);
        assert!(labels(&nodes).contains(&"Toolbar, not shown now".to_string()), "{:?}", labels(&nodes));
        window.app.run("window.dock", serde_json::json!({})).expect("Tab");
        assert!(window.app.ui.tab_hid.is_none() && window.app.ui.toolbar);
        let entries = entries(&window.ctx, Lang::EN);
        let none = entries.iter().find(|e| e.home.is_none() && !e.id.starts_with("tool:") && e.shortcut.is_empty()).expect("no menu item");
        let nodes = window.search(&none.id);
        assert_eq!(window.highlighted(), none.id);
        assert_eq!(window.callout(), None);
        assert!(labels(&nodes).contains(&"No menu item".to_string()), "{:?}", labels(&nodes));
        // With a shortcut, the footer gives it.
        let keyed = entries.iter().find(|e| e.home.is_none() && !e.id.starts_with("tool:") && !e.shortcut.is_empty()).expect("a shortcut");
        let nodes = window.search(&keyed.id);
        assert_eq!(window.highlighted(), keyed.id);
        assert_eq!(window.callout(), None);
        let line = format!("No menu item, {}", menus::shown_shortcut(&keyed.shortcut));
        assert!(labels(&nodes).contains(&line), "{line} in {:?}", labels(&nodes));
    }

    /// In the pass that opens the palette, the menu bar and the toolbars have noted no control yet:
    /// the footer does not call a home that is in view "not shown now", and the next pass points
    /// at it.
    #[test]
    fn the_pass_that_opens_the_palette_names_no_home_as_hidden() {
        let mut window = Window::new();
        // Opened once and closed, the popup has its size, so the pass that opens it again draws
        // it. One pass a frame, so the frame shows that pass.
        window.search("pen tool");
        window.app.ui.palette_open = false;
        window.frame(vec![]);
        window.ctx.options_mut(|o| o.max_passes = std::num::NonZeroUsize::MIN);
        window.app.ui.palette_open = true;
        let nodes = window.frame(vec![]);
        assert_eq!(window.highlighted(), "tool:pen");
        let footer = window.popup_labels(&nodes);
        assert!(!footer.is_empty(), "the popup is drawn: {:?}", labels(&nodes));
        assert!(!footer.iter().any(|l| l.contains("not shown now")), "{footer:?}");
        window.frame(vec![]);
        assert!(window.callout().is_some());
    }

    /// Moving the pointer onto a row moves the callout to that result's home in the same frame.
    #[test]
    fn the_callout_follows_the_pointer() {
        let mut window = Window::new();
        let nodes = window.search("select");
        let entries = entries(&window.ctx, Lang::EN);
        let x = window.popup().center().x;
        let target = rows(&nodes)
            .into_iter()
            .filter(|r| !r.1)
            .find_map(|r| {
                let e = entries.iter().find(|e| row_label(e, &home_text(&window.app, e)) == r.0)?;
                let path = e.home.as_ref().filter(|_| !e.id.starts_with("tool:") && !e.menu_item.is_empty())?;
                Some((e.id.clone(), format!("{path} › {}", e.menu_item), (r.2 + r.3) / 2.0))
            })
            .expect("a row with a menu item");
        let (id, text, y) = target;
        window.frame(vec![egui::Event::PointerMoved(egui::pos2(x, y as f32 - 3.0))]);
        window.frame(vec![egui::Event::PointerMoved(egui::pos2(x, y as f32))]);
        assert_eq!(window.highlighted(), id);
        assert_eq!(window.callout().map(|c| c.2), Some(text));
    }

    /// Down moves the callout to the next result's home in the same frame, and moving the
    /// highlight changes no toolbar, flyout, drawer or tool.
    #[test]
    fn the_callout_follows_the_highlight_and_reveals_nothing() {
        let mut window = Window::new();
        window.search("select");
        let before = (window.app.ui.flyout, window.app.ui.tools_drawer, window.app.ui.toolbar, window.app.session.tool_id());
        let entries = entries(&window.ctx, Lang::EN);
        for _ in 0..6 {
            let key =
                egui::Event::Key { key: egui::Key::ArrowDown, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            window.frame(vec![key]);
            let id = window.highlighted();
            let e = entries.iter().find(|e| e.id == id).unwrap();
            let callout = window.callout();
            if let Some(path) = e.home.as_ref().filter(|_| !id.starts_with("tool:")) {
                assert_eq!(callout.map(|c| c.2), Some(format!("{path} › {}", e.menu_item)), "{id}");
            }
            assert!(window.app.ui.learn_locate.is_none());
        }
        let after = (window.app.ui.flyout, window.app.ui.tools_drawer, window.app.ui.toolbar, window.app.session.tool_id());
        assert_eq!(before, after);
    }

    /// Every catalog translates the callout's and the footer's words.
    #[test]
    fn catalogs_translate_the_callout_words() {
        let words = ["In the menu bar: {path}", "In the {group} group: press and hold to open it", "{home}, not shown now"];
        let missing: Vec<String> = Lang::all()
            .filter(|l| *l != Lang::EN)
            .flat_map(|l| words.iter().filter(move |w| !crate::i18n::has(l, w)).map(move |w| format!("{}: {w}", l.code())))
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    /// A control noted in an earlier pass, in another viewport or clipped by its scroll area is not
    /// pointed at; the box stays inside a narrow window.
    #[test]
    fn only_a_control_in_view_in_this_pass_is_pointed_at() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(300.0, 200.0));
        let mut checked = false;
        for _ in 0..2 {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                let ctx = ui.ctx();
                let pass = ctx.cumulative_pass_nr_for(ctx.viewport_id());
                let rect = egui::Rect::from_min_size(egui::pos2(270.0, 10.0), egui::vec2(28.0, 28.0));
                let at = |name: &str| SpotAt::Menu(name.into());
                let spot = |name: &str, viewport, pass, clip| Spot { at: at(name), viewport, pass, layer: ui.layer_id(), clip, rect };
                let list = vec![
                    spot("old", ctx.viewport_id(), pass.wrapping_sub(1), screen),
                    spot("other", egui::ViewportId::from_hash_of("detached"), pass, screen),
                    spot("clipped", ctx.viewport_id(), pass, egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(280.0, 200.0))),
                    spot("shown", ctx.viewport_id(), pass, screen),
                ];
                ctx.data_mut(|d| d.insert_temp(spots_id(), list));
                for name in ["old", "other", "clipped"] {
                    assert!(super::spot(ctx, |a| *a == at(name)).is_none(), "{name}");
                }
                let shown = super::spot(ctx, |a| *a == at("shown")).expect("shown");
                let text = "Effect › Stylize › Round Corners… and a long tail of words";
                let r = callout(ctx, &Spot { at: SpotAt::Slot(None, vec![]), ..shown.clone() }, Some(text), None).unwrap();
                assert!(screen.contains_rect(r), "{r:?}");
                let r = callout(ctx, &shown, Some(text), None).unwrap();
                assert!(screen.contains_rect(r), "{r:?}");
                checked = true;
            });
            out.textures_delta.clear();
        }
        assert!(checked);
    }

    /// A control under a window drawn above its layer is not pointed at; with the window moved off
    /// it, it is.
    #[test]
    fn a_control_under_a_window_is_not_pointed_at() {
        let ctx = egui::Context::default();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 300.0));
        let rect = egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::vec2(28.0, 28.0));
        let at = || SpotAt::Menu("bar".into());
        for (cover_at, pointed) in [(egui::pos2(10.0, 10.0), false), (egui::pos2(200.0, 150.0), true)] {
            let mut seen = None;
            // The second pass sees both areas laid out.
            for _ in 0..2 {
                let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
                let mut out = ctx.run_ui(input, |ui| {
                    let ctx = ui.ctx().clone();
                    ctx.data_mut(|d| d.insert_temp(spots_wanted_id(), true));
                    egui::Area::new(egui::Id::unique("test-toolbar")).order(egui::Order::Middle).fixed_pos(egui::Pos2::ZERO).show(&ctx, |ui| {
                        ui.allocate_rect(rect, egui::Sense::hover());
                        note_spot(ui, rect, at);
                    });
                    egui::Area::new(egui::Id::unique("test-cover")).order(egui::Order::Middle).fixed_pos(cover_at).show(&ctx, |ui| {
                        ui.allocate_space(egui::vec2(80.0, 80.0));
                    });
                    seen = Some(super::spot(&ctx, |a| *a == at()).is_some());
                });
                out.textures_delta.clear();
            }
            assert_eq!(seen, Some(pointed), "{cover_at:?}");
        }
    }
}
