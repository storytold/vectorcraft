//! The command palette: fuzzy search over the commands a pick runs, the tools, the panels and the
//! Effect menu's items.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
use crate::{VectorcraftApp, menus};

mod search;

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
/// gradient, Define Perspective Grid makes the document's grid, and Select Slice deselects the
/// slices.
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
}

/// The palette's entries, the index a query is looked up in, and the last query's ranked results.
pub(crate) struct Catalog {
    list: Vec<Entry>,
    index: search::Index,
    ranked: std::sync::Mutex<Option<(String, Arc<[u32]>)>>,
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
        list.iter().map(|e| e.home.as_deref().map(|h| format!("{} › {}", shown_label(lang, h), crate::i18n::tr(lang, e.menu_item)))).collect();
    let index = search::Index::build(&list, &shown_homes);
    Catalog { list, index, ranked: std::sync::Mutex::new(None) }
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
            Entry { id: d.id, shortcut: d.shortcut, shown, haystack, home, menu_item, kind }
        })
        .collect()
}

/// Runs the palette result `id`: selects a tool, or invokes a command with the params its entry
/// stands for. A command whose menu bar item gives params runs what a click on that item runs
/// ([`crate::shortcut_editor::menu_target_now`]), so Clean Up… and Find and Replace… open their
/// dialogs. While that item is dimmed, no dialog opens and the status line says why the command
/// can't run. A panel's result shows the panel, and an effect's result runs what its Effect menu
/// item runs ([`crate::keymap::command_of`]).
pub(crate) fn run_pick(app: &mut VectorcraftApp, id: &str) {
    if let Some(tool) = id.strip_prefix("tool:") {
        app.select_tool(tool);
    } else if let Some(panel) = id.strip_prefix("panel:") {
        // A panel that can be seen stays as it is, where `window.panel` would put an open flyout or
        // a floating panel away. In Presentation Mode the pick fails and changes nothing, as
        // `window.panel` does.
        if app.ui.screen_mode >= 3 {
            app.status("panels can't be shown in Presentation Mode");
        } else if let Err(e) = crate::panel_docking::reveal_panel(app, panel) {
            app.status(e);
        }
    } else if id.starts_with("plugin:") || id.starts_with("effect:") {
        let command = crate::keymap::command_of(id);
        // While the Effect menu dims the item (nothing selected), no dialog opens and the status
        // line says why.
        if id.starts_with("effect:") && !menus::enabled(app, &command.id) {
            if let Err(why) = crate::shortcut_editor::dimmed(app, &command.id) {
                app.status(why);
            }
        } else {
            menus::invoke(app, &command.id, command.params);
        }
    } else {
        match crate::shortcut_editor::menu_target_now(app, id) {
            Some(Ok((id, p))) => menus::invoke(app, &id, p),
            Some(Err(why)) => app.status(why),
            None => menus::invoke(app, id, json!({})),
        }
    }
}

/// The entries the query `q` matches, best first ([`search::rank`]). The ranking of the last query
/// is kept, so a frame that draws the same query does not rank again. [`show`] lists the first
/// [`MAX_RESULTS`] and counts the rest.
fn matching<'a>(catalog: &'a Catalog, q: &str) -> Vec<&'a Entry> {
    let q = search::normalize(q);
    let kept = catalog.ranked.lock().ok().and_then(|last| last.as_ref().filter(|(k, _)| *k == q).map(|(_, r)| r.clone()));
    let ranked = kept.unwrap_or_else(|| {
        let ranked: Arc<[u32]> = search::rank(&catalog.list, &catalog.index, &q).into();
        // A poisoned lock only loses the kept ranking: the next frame ranks again.
        if let Ok(mut last) = catalog.ranked.lock() {
            *last = Some((q, ranked.clone()));
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

/// What a result row shows and screen readers read: its name, and its shortcut after a comma.
fn row_label(e: &Entry) -> String {
    let shortcut = menus::shown_shortcut(&e.shortcut);
    if shortcut.is_empty() { e.shown.clone() } else { format!("{}, {shortcut}", e.shown) }
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

/// How many times each key that moves the highlighted result was pressed this frame.
#[derive(Default)]
struct Moves {
    down: usize,
    up: usize,
    page_down: usize,
    page_up: usize,
}

impl Moves {
    fn any(&self) -> bool {
        self.down + self.up + self.page_down + self.page_up > 0
    }

    /// The highlighted result of `n` after these moves from `sel`, a page being `page` rows: Up and
    /// Down wrap at the ends, Page Up and Page Down stop at them.
    fn apply(&self, sel: usize, n: usize, page: usize) -> usize {
        let Some(last) = n.checked_sub(1) else { return 0 };
        let sel = sel.min(last);
        let sel = (sel + self.down % n) % n;
        let sel = (sel + n - self.up % n) % n;
        let sel = sel.saturating_add(page.saturating_mul(self.page_down)).min(last);
        sel.saturating_sub(page.saturating_mul(self.page_up))
    }
}

/// `name` laid out on one line at most `width` points wide; a longer name ends with "…".
fn name_galley(painter: &egui::Painter, name: &str, font: egui::FontId, color: egui::Color32, width: f32) -> Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), font, color);
    job.wrap = egui::text::TextWrapping { max_width: width.max(0.0), max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
    painter.layout_job(job)
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

/// How many picks the Recent Commands list beside the search field holds.
pub(crate) const MAX_RECENT_COMMANDS: usize = 5;

/// How many ids the saved Recent Commands list holds at most, unavailable ones included.
const MAX_SAVED_RECENT_COMMANDS: usize = 4 * MAX_RECENT_COMMANDS;

/// Puts `id` first in the Recent Commands ids `list`, removing its earlier place, and keeps the
/// newest [`MAX_RECENT_COMMANDS`] ids that are `known`. An id that is not `known` now (a plug-in
/// that is disabled or not loaded yet) keeps its place among them, so it shows again when its
/// command comes back. The list never holds more than [`MAX_SAVED_RECENT_COMMANDS`] ids.
fn remember(list: &mut Vec<String>, id: &str, known: impl Fn(&str) -> bool) {
    list.retain(|c| c != id);
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

/// The entries the Recent Commands list shows for the ids `ids`, most recent first. An id with no
/// entry (a removed plug-in) is skipped.
fn recent<'a>(entries: &'a [Entry], ids: &[String]) -> Vec<&'a Entry> {
    ids.iter().take(MAX_SAVED_RECENT_COMMANDS).filter_map(|id| entries.iter().find(|e| e.id == *id)).take(MAX_RECENT_COMMANDS).collect()
}

/// The id of the Recent Commands list's popup.
fn recent_list_id() -> egui::Id {
    egui::Id::unique("palette-recent-commands")
}

/// Is the palette's Recent Commands list open?
pub(crate) fn recent_list_open(ctx: &egui::Context) -> bool {
    egui::Popup::is_id_open(ctx, recent_list_id())
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // The widget that had the keyboard focus when the palette opened takes it back when the palette
    // closes, unless a pick opened a dialog, which focuses its own first field, or the press that
    // closed it focused another widget, such as a field in a window above the backdrop.
    let focus_id = egui::Id::unique("palette-focus-before");
    let field_id = egui::Id::unique("palette-field");
    if !app.ui.palette_open {
        let field = ctx.data_mut(|d| {
            let field = d.get_temp::<egui::Id>(field_id);
            d.remove::<egui::Id>(field_id);
            field
        });
        let now = ctx.memory(|m| m.focused());
        if let Some(Some(before)) = ctx.data_mut(|d| d.remove_temp::<Option<egui::Id>>(focus_id))
            && app.ui.dialog.is_none()
            && (now.is_none() || now == field)
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
    let mut matches = matching(&entries, &q);
    let total = matches.len();
    // The highlighted result (#1757): Up and Down move it (wrapping), Page Up and Page Down move it
    // by a page, the pointer over a result takes it, and a new query starts again from the first.
    let sel_id = egui::Id::unique("palette-selected");
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
    let recent = recent(&entries.list, &app.ui.recent_commands);
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
            let mut key = |k| i.count_and_consume_key(egui::Modifiers::NONE, k);
            moves = Moves {
                down: key(egui::Key::ArrowDown),
                up: key(egui::Key::ArrowUp),
                page_down: key(egui::Key::PageDown),
                page_up: key(egui::Key::PageUp),
            };
        });
    }
    let screen = ctx.content_rect();
    // The highlighted result the rows and the field's description are drawn with.
    let mut drawn = sel;
    // The backdrop covers the canvas, the toolbars, the docked panels and the app bar under the
    // popup, in the order of a modal dialog's backdrop (`dialogs::modal::backdrop`), so the press
    // that closes the palette does not press a control under it. Windows in the Foreground order
    // stay above it. Not focusable (`Sense::click()` is): Tab stays in the popup.
    egui::Area::new(egui::Id::unique("palette-backdrop")).order(egui::Order::Middle).fixed_pos(screen.min).show(ctx, |ui| {
        ui.allocate_rect(screen, egui::Sense::CLICK);
    });
    // The area keeps itself inside the window (egui's `Area::constrain` is on), so in a window too
    // short for the field, one row and the footer below TOP, the popup moves up.
    egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, TOP]).show(ctx, |ui| {
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
                    egui::Popup::menu(&button).id(recent_list_id()).show(|ui| {
                        if recent.is_empty() {
                            ui.add_enabled(false, egui::Button::new(tl!("No recent commands")));
                        }
                        for (i, e) in recent.iter().enumerate() {
                            let row = ui.add(egui::Button::new(e.shown.as_str()).shortcut_text(menus::shown_shortcut(&e.shortcut)));
                            if i == rsel {
                                row.request_focus();
                            }
                            if row.clicked() {
                                run = Some(e.id.clone());
                            }
                        }
                    });
                    r
                })
                .inner;
            ui.ctx().data_mut(|d| d.insert_temp(field_id, r.id));
            ui.add_space(6.0);
            // The list shows as many rows as fit above the footer in the window, up to
            // VISIBLE_ROWS; a page is that many rows.
            let spacing = ui.spacing().item_spacing.y;
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
            // its place among every match, as the footer counts them.
            if let Some(e) = matches.get(sel) {
                let rank = if sel + 1 == matches.len() { kept.unwrap_or(sel) } else { sel };
                let pos = (rank + 1).to_string();
                let of = total.to_string();
                let said = crate::i18n::fmt(tl!("{pos} of {total}: {name}"), &[("pos", &pos), ("total", &of), ("name", &row_label(e))]);
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
                            let font = egui::TextStyle::Body.resolve(ui.style());
                            let shortcut = menus::shown_shortcut(&e.shortcut);
                            let p = ui.painter();
                            let shortcut_at = rect.right_center() - egui::vec2(8.0, 0.0);
                            let used = p.text(shortcut_at, egui::Align2::RIGHT_CENTER, &shortcut, font.clone(), t.text_dim);
                            let name_end = if shortcut.is_empty() { rect.right() - 8.0 } else { used.left() - 12.0 };
                            let name_at = rect.left() + 8.0;
                            let name = name_galley(p, &e.shown, font, t.text, name_end - name_at);
                            p.galley(egui::pos2(name_at, rect.center().y - name.size().y / 2.0), name, t.text);
                            // `WidgetInfo::selected` reaches screen readers as a toggled state, so the
                            // row sets the selected state on its node itself.
                            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ListItem, true, row_label(e)));
                            ui.ctx().accesskit_node_builder(resp.id, |node| node.set_selected(on));
                            if resp.clicked() {
                                run = Some(e.id.clone());
                            }
                        }
                    });
                    let before = offset;
                    offset = out.state.clamped_offset().y;
                    // After a scroll with the wheel or the scroll bar, the highlight moves to the
                    // nearest row in view, so Enter runs a result that can be seen.
                    if !new_query && !moves.any() && (offset - before).abs() > 0.5 {
                        sel = nearest_in_view(sel, offset, view, n);
                    }
                });
            }
            ui.add_space(6.0);
            ui.label(egui::RichText::new(footer(n, total)).color(t.text_dim));
            if !list_open
                && !history_focused
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(e) = matches.get(sel)
            {
                run = Some(e.id.clone());
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
    // The pointer or a scroll moved the highlight after the rows were drawn: the next frame draws
    // it where it now is.
    if sel != drawn {
        ctx.request_repaint();
    }
    let sel_key = matches.get(sel).map(|e| e.id.clone()).unwrap_or_default();
    ctx.data_mut(|d| {
        d.insert_temp::<Highlight>(sel_id, (sel, q, offset, view, sel_key));
        d.insert_temp(rsel_id, rsel);
    });
    if let Some(id) = run {
        app.ui.palette_open = false;
        remember(&mut app.ui.recent_commands, &id, |c| entries.list.iter().any(|e| e.id == c));
        run_pick(app, &id);
    } else if outside {
        app.ui.palette_open = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

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
        let matches = matching(&entries, query);
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
            ("edit.findReplace", "command"),
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
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
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

    /// #6376: Recent Commands keeps the last five picks.
    #[test]
    fn recent_commands_keep_the_last_five_picks() {
        let mut list = vec![];
        let ids = ["a", "b", "c", "d", "e", "f", "g"];
        for id in ids {
            remember(&mut list, id, |_| true);
        }
        assert_eq!(list, ["g", "f", "e", "d", "c"]);
        let entries = build(Lang::EN);
        let many: Vec<String> = entries.iter().take(MAX_RECENT_COMMANDS + 2).map(|e| e.id.clone()).collect();
        assert_eq!(recent(&entries, &many).len(), MAX_RECENT_COMMANDS);
    }

    /// #6376: a command that does not exist now, such as a plug-in's filter while the plug-in is
    /// not loaded, is left out of Recent Commands. A pick keeps it in the saved list, so it shows
    /// again when the command comes back; five other commands picked after it push it out.
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
        let four: Vec<String> = entries.iter().filter(|e| e.id != "paint.swap" && e.id != "object.group").take(4).map(|e| e.id.clone()).collect();
        for id in &four {
            remember(&mut list, id, known);
        }
        let expected: Vec<String> =
            four.iter().rev().cloned().chain(["paint.swap".to_string(), "plugin:test.UI.palette_removed".to_string()]).collect();
        assert_eq!(list, expected, "object.group, the sixth known id, and the ids after it go");
        // Unavailable ids are capped too.
        let mut list: Vec<String> = (0..50).map(|i| format!("plugin:test.UI.gone{i}")).collect();
        remember(&mut list, "paint.swap", known);
        assert_eq!(list.len(), MAX_SAVED_RECENT_COMMANDS);
        assert_eq!(list.first().map(String::as_str), Some("paint.swap"));
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
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
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
        assert!(matching(&entries(&ctx, Lang::EN), "select").len() > MAX_RESULTS);
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
            let all = entries.iter().filter(|e| query.to_lowercase().split_whitespace().all(|w| e.haystack.contains(w))).count();
            let shown: Vec<&str> = matching(&entries, &query.to_lowercase()).into_iter().map(|e| e.id.as_str()).collect();
            assert_eq!(shown.first(), Some(&want), "{query}: {shown:?}");
            assert_eq!(shown.len(), all, "{query}");
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
        let entries = build(Lang::EN);
        let effects = crate::shortcut_editor::effect_items();
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
            let mut open = Open { app, ctx, size, time: 0.0, repaint: false };
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
            out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
        }

        /// A frame where `k` is pressed `times` times.
        fn press(&mut self, k: egui::Key, times: usize) -> Vec<egui::accesskit::Node> {
            let key = egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
            self.frame(vec![key; times])
        }

        /// The highlighted result's index.
        fn highlighted(&self) -> usize {
            self.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap().0
        }

        /// The popup's rectangle.
        fn popup(&self) -> egui::Rect {
            self.ctx.memory(|m| m.area_rect(egui::Id::unique("palette"))).unwrap()
        }

        /// The results the palette lists for its query.
        fn listed(&self) -> Vec<String> {
            let entries = entries(&self.ctx, Lang::EN);
            matching(&entries, &self.app.ui.palette_query.to_lowercase()).iter().take(MAX_RESULTS).map(|e| e.id.clone()).collect()
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
        let total = matching(&entries(&open.ctx, Lang::EN), "select").len();
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
        let entries = entries(&open.ctx, Lang::EN);
        let want = entries.iter().find(|e| e.id == listed[15]).map(row_label).unwrap();
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
        let moves = |down, up, page_down, page_up| Moves { down, up, page_down, page_up };
        assert_eq!(moves(0, 0, 1, 0).apply(0, 50, 12), 12);
        assert_eq!(moves(0, 0, 1, 0).apply(45, 50, 12), 49);
        assert_eq!(moves(0, 0, 0, 1).apply(5, 50, 12), 0);
        assert_eq!(moves(1, 0, 0, 0).apply(49, 50, 12), 0);
        assert_eq!(moves(0, 1, 0, 0).apply(0, 50, 12), 49);
        assert_eq!(moves(3, 0, 0, 0).apply(48, 50, 12), 1);
        assert_eq!(moves(1, 0, 0, 0).apply(0, 0, 12), 0);
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
        assert_eq!(lit[0].0, row_label(entries(&open.ctx, Lang::EN).iter().find(|e| e.id == open.listed()[sel]).unwrap()));
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
        let label = entries.iter().find(|e| e.id == listed[3]).map(row_label).unwrap();
        assert_eq!(lit, std::slice::from_ref(&label));
        assert_eq!(field_description(&nodes), format!("4 of {}: {label}", matching(&entries, "select").len()));
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

    /// Screen readers stay on the field: its description names the highlighted result and its
    /// place, and follows the highlight; the rows are list items, the highlighted one selected, and
    /// the footer's count is a label.
    #[test]
    fn screen_readers_hear_the_highlighted_result_and_the_count() {
        let mut open = Open::new("show panel", WINDOW);
        let listed = open.listed();
        let entries = entries(&open.ctx, Lang::EN);
        let label = |i: usize| entries.iter().find(|e| e.id == listed[i]).map(row_label).unwrap();
        let nodes = open.frame(vec![]);
        // More results match than the list holds: the place counts every match, as the footer does.
        let total = matching(&entries, "show panel").len();
        assert!(total > MAX_RESULTS && listed.len() == MAX_RESULTS, "{total}");
        assert_eq!(field_description(&nodes), format!("1 of {total}: {}", label(0)));
        let lit: Vec<String> = rows(&nodes).into_iter().filter(|r| r.1).map(|r| r.0).collect();
        assert_eq!(lit, [label(0)]);
        assert!(labels(&nodes).contains(&format!("50 of {total} results. Keep typing to narrow them.")), "{:?}", labels(&nodes));
        let nodes = open.press(egui::Key::ArrowDown, 1);
        assert_eq!(field_description(&nodes), format!("2 of {total}: {}", label(1)));
        // Fewer results match than the list holds.
        let mut open = Open::new("copy", WINDOW);
        let n = open.listed().len();
        assert!((1..MAX_RESULTS).contains(&n), "{n}");
        let first = entries.iter().find(|e| e.id == open.listed()[0]).map(row_label).unwrap();
        let nodes = open.frame(vec![]);
        assert_eq!(field_description(&nodes), format!("1 of {n}: {first}"));
        assert!(labels(&nodes).contains(&footer(n, n)), "{:?}", labels(&nodes));
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
            let mut window = Window { app, ctx, time: 0.0, above: None, above_clicks: 0 };
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
            let (above, clicks) = (self.above, &mut self.above_clicks);
            let mut out = self.ctx.run_ui(input, |ui| {
                app.logic(ui.ctx());
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

    /// The click on the app bar's search button that opens the palette leaves it open; the next
    /// click on the button closes it.
    #[test]
    fn the_click_that_opens_the_palette_does_not_close_it() {
        let mut window = Window::new();
        let nodes = window.frame(vec![]);
        let at = button_at(&nodes, "Search commands, tools, panels and effects");
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
        let listed: Vec<_> = matching(&entries, "show panel").into_iter().take(MAX_RESULTS).collect();
        let hidden = |e: &&&Entry| e.id.strip_prefix("panel:").is_some_and(|p| !crate::panel_docking::showing(&window.app.ui, p));
        let e = listed.iter().find(hidden).expect("a hidden panel's result");
        let label = row_label(e);
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
        let de_listed: Vec<String> = matching(&de, "group").iter().map(|e| e.id.clone()).collect();
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
        assert!(matching(&entries(&open.ctx, Lang::EN), "path").len() > MAX_RESULTS);
        open.press(egui::Key::ArrowUp, 1);
        let held = |open: &Open| open.ctx.data(|d| d.get_temp::<Highlight>(egui::Id::unique("palette-selected"))).unwrap();
        let (sel, .., id) = held(&open);
        assert_eq!(sel, MAX_RESULTS - 1);
        let filter = "test.UI.palette_last-highlight-path";
        let manifest = format!(r#"{{"id":"{filter}","name":"Path Last Highlight Probe","version":"1","kind":"filter","params":{{}}}}"#);
        vectorcraft_plugins::registry::install_bytes(&wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap()).unwrap();
        open.frame(vec![]);
        let entries = entries(&open.ctx, Lang::EN);
        let ranked: Vec<&Entry> = matching(&entries, "path");
        let nodes = open.frame(vec![]);
        let (sel, .., now) = held(&open);
        vectorcraft_plugins::registry::remove(filter);
        let plugin = ranked.iter().position(|e| e.id == format!("plugin:{filter}"));
        assert!(plugin.is_some_and(|p| p < MAX_RESULTS), "the plug-in is shown: {plugin:?}");
        assert_eq!(ranked.iter().position(|e| e.id == id), Some(MAX_RESULTS), "the plug-in pushed the highlight past the limit");
        assert_eq!((sel, &now), (MAX_RESULTS - 1, &id), "the highlight stays on its result, in the last row");
        let label = row_label(ranked[MAX_RESULTS]);
        assert!(rows(&nodes).iter().any(|r| r.1 && r.0 == label), "its row is drawn and highlighted");
        assert_eq!(field_description(&nodes), format!("{} of {}: {label}", MAX_RESULTS + 1, ranked.len()));
    }
}
