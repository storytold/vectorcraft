//! The command palette: the commands a pick runs, the tools, the panels and the Effect menu's
//! items as the framework palette's entries, with their search index.

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::IconId;
use craft::keymap::CommandRef;
use craft::menu::{MenuBar, Node};
use craft::ui::contract::Bind;
use craft::ui::menu::{HelpEntry, SearchIndex, SearchWords, Synonyms};
use craft::ui::patterns::panels::{CommandPalette, PanelWords};
use craft::ui::primitives::IconSpec;
use craft::ui::runtime::env::Env;

use crate::i18n::Lang;
use crate::{VectorcraftApp, menus};

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
    /// Has [`distinct_names`] given the result its [`context`]?
    in_context: bool,
    /// Does the result show its id after its name?
    keyed: bool,
}

impl Draft {
    fn new(path: &str, label: &str, id: String) -> Draft {
        Draft { path: path.to_string(), label: label.to_string(), translates: true, id, in_context: false, keyed: false }
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
        out.push(Draft::new(&c.menu.join(" › "), c.label, c.id.to_string()));
    }
    // `plugin.dialog` runs nothing without an id: [`named_drafts`] lists each filter instead. A
    // script of the Scripts folder runs from File › Scripts, by its path. The palette's own toggle
    // is no result ([`PALETTE_TOGGLE`]). A UI command that wraps an engine command, or has an
    // engine command's id, takes that command's menu path, so a search for the path still finds it.
    let skipped = ["plugin.dialog", crate::scripts_menu::COMMAND, PALETTE_TOGGLE];
    for c in menus::UI_COMMANDS.iter().filter(|c| !skipped.contains(&c.0)) {
        let engine = WRAPPED.iter().find(|(_, ui)| *ui == c.0).map_or(c.0, |(engine, _)| *engine);
        let engine = vectorcraft_engine::find_command(engine);
        let path = engine.map(|e| e.menu.join(" › ")).unwrap_or_default();
        out.push(Draft::new(&path, c.1, c.0.to_string()));
    }
    for tool in vectorcraft_tools::catalog::all_tools() {
        out.push(Draft::new("", tool.label, format!("tool:{}", tool.id)));
    }
    // A panel's result shows the panel, as `window.panel` does without `toggle`, under Window ›
    // Show Panel.
    for (id, label) in crate::state::all_panels() {
        out.push(Draft::new(PANEL_PATH, label, format!("panel:{id}")));
    }
    // An Effect menu item runs what its item runs; a live-effect plug-in's name does not translate.
    for e in crate::shortcut_editor::effect_items().iter() {
        out.push(Draft {
            translates: vectorcraft_plugins::effect::plugin_id(&e.effect).is_none(),
            ..Draft::new(&e.group, &e.label, format!("effect:{}", e.effect))
        });
    }
    out
}

/// Everything the palette searches: (label, command id or `tool:<id>`). Labels are the English
/// names before [`distinct_names`] makes them distinct; a result's shortcut is its command's in the
/// window's keymap.
pub fn items() -> Vec<(String, String)> {
    drafts().into_iter().map(|d| (d.name(crate::i18n::Lang::EN), d.id)).collect()
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

/// The label of the item of the menu bar that runs result `id`, English `menu_item`, in `lang`,
/// as the menu bar draws it ([`menus::display_label`]). A built-in effect's item translates by
/// its effect id first, as the Effect menu's does. Any other item translates by the id of the
/// command it runs first (File › Close is Cerrar archivo in Spanish), and a name the menu shows as
/// written ([`menus::shows_a_name`]: a filter or live-effect plug-in's) does not translate.
fn item_label(lang: crate::i18n::Lang, id: &str, menu_item: &str) -> String {
    if let Some(effect) = id.strip_prefix("effect:") {
        return match vectorcraft_plugins::effect::plugin_id(effect) {
            Some(_) => menu_item.to_string(),
            None => crate::i18n::tr_id(lang, effect, menu_item).to_string(),
        };
    }
    // The command the menu item runs: `plugin.dialog` for a filter, `window.panel` for a panel and
    // the engine command a UI command wraps.
    let id = match id {
        id if id.starts_with("plugin:") => "plugin.dialog",
        id if id.starts_with("panel:") => "window.panel",
        id => WRAPPED.iter().find(|(_, ui)| *ui == id).map_or(id, |(engine, _)| engine),
    };
    if menus::shows_a_name(id, menu_item) { menu_item.to_string() } else { crate::i18n::tr_id(lang, id, menu_item).to_string() }
}

/// The results with names no result for another id shows, in English and in `lang`: [`drafts`],
/// then one per filter plug-in's Object › Plug-ins item (`plugin:<id>`), whose name is not
/// translated ([`distinct_names`]).
fn named_drafts(lang: Lang) -> Vec<Draft> {
    let mut drafts = drafts();
    drafts.extend(
        crate::shortcut_editor::filter_items()
            .iter()
            .map(|f| Draft { translates: false, ..Draft::new(&f.group, &f.label, format!("plugin:{}", f.id)) }),
    );
    distinct_names(&mut drafts, &[Lang::EN, lang]);
    drafts
}

/// The params text of each draft's command, which its description is read from
/// ([`description`]): an engine or plug-in command's, a UI command's (the one it wraps keeps its
/// own) and an effect's. A tool, a panel and a filter plug-in have none.
fn params_texts(list: &[Draft]) -> Vec<&'static str> {
    let engine = vectorcraft_engine::command_specs().iter().chain(vectorcraft_engine::cmd::plugin::command_specs());
    let mut by_id: std::collections::HashMap<&str, &'static str> = engine.map(|c| (c.id, c.params)).collect();
    by_id.extend(menus::UI_COMMANDS.iter().map(|c| (c.0, c.3)));
    list.iter()
        .map(|d| match d.id.strip_prefix("effect:") {
            Some(effect) => vectorcraft_effects::effect_info(effect).map_or("", |info| info.params),
            None => by_id.get(d.id.as_str()).copied().unwrap_or(""),
        })
        .collect()
}

/// The most characters of a command's description the index reads (VectorCraft's own limit).
const DESCRIPTION_CHARS: usize = 300;

/// A command's description, read from its params text: the text without each `{…}` group that is
/// not inside a `{`, `[` or `(` group (brackets inside double quotes or backticks are text, as in
/// the engine's params object), without each such
/// `[…]` or `(…)` group just after a `→`, and without the `→` just before a removed group, with
/// each run of whitespace made one space and cut to its first [`DESCRIPTION_CHARS`] characters at
/// the end of a word. A group that is never closed stays in the text, and so does a result written
/// as words after a `→`. Params text that is only a group has an empty description.
fn description(params: &str) -> String {
    let mut out = String::new();
    // The end of the text copied to `out`, and whether a removed group is open.
    let (mut copied, mut open) = (0, false);
    for (i, ch, depth) in crate::shortcut_editor::outside_quotes_and_backticks(params) {
        let before = params.get(copied..i).unwrap_or_default();
        match ch {
            '{' | '[' | '(' if depth == 0 && (ch == '{' || before.trim_end().ends_with('→')) => {
                out.push_str(before);
                (copied, open) = (i, true);
            }
            '}' | ']' | ')' if depth == 1 && open => {
                let kept = out.trim_end().strip_suffix('→').unwrap_or(&out).len();
                out.truncate(kept);
                out.push(' ');
                (copied, open) = (i + ch.len_utf8(), false);
            }
            _ => {}
        }
    }
    out.push_str(params.get(copied..).unwrap_or_default());
    let text = out.split_whitespace().collect::<Vec<_>>().join(" ");
    // A cut inside a word drops the rest of that word, so no part of a word is read as a word.
    let Some((head, rest)) = text.char_indices().nth(DESCRIPTION_CHARS).and_then(|(end, _)| text.split_at_checked(end)) else { return text };
    let head = if rest.starts_with(' ') { head } else { head.rsplit_once(' ').map_or("", |(kept, _)| kept) };
    head.trim_end().to_string()
}

/// The first place in the menu bar of an item a result runs ([`menu_homes`]).
struct Home {
    /// The menus that hold the item, `›`-separated, as the bar shows them.
    path: String,
    /// The item's label as the tree writes it (English), which may differ from the result's (File ›
    /// Exit runs Quit VectorCraft).
    item: String,
}

/// The first place in `bar` of each item a result runs, by result id: the command id (a wrapped
/// engine command's under its UI command), `panel:<id>` for a Window menu panel (Learn too),
/// `effect:<id>` for an Effect menu item and `plugin:<id>` for an Object › Plug-ins filter.
fn menu_homes(bar: &MenuBar) -> HashMap<String, Home> {
    fn key(command: &CommandRef) -> String {
        let field = |k: &str| command.params.get(k).and_then(serde_json::Value::as_str);
        match (command.id.as_str(), field("panel"), field("effect"), field("id")) {
            ("window.panel", Some(panel), ..) => format!("panel:{panel}"),
            ("window.learn", ..) => format!("panel:{}", crate::panels::learn::ID),
            ("effect.dialog" | "effect.apply", _, Some(effect), _) => format!("effect:{effect}"),
            ("plugin.dialog", _, _, Some(filter)) => format!("plugin:{filter}"),
            (id, ..) => WRAPPED.iter().find(|(engine, _)| *engine == id).map_or(id, |(_, ui)| ui).to_string(),
        }
    }
    fn walk(nodes: &[Node], path: &mut Vec<String>, out: &mut HashMap<String, Home>) {
        for node in nodes {
            match node {
                Node::Item(item) => {
                    let home = || Home { path: path.join(" › "), item: item.label.clone() };
                    out.entry(key(&item.command)).or_insert_with(home);
                    // Window › Learn runs `window.learn`, which is a result of its own too.
                    if item.command.id == "window.learn" {
                        out.entry(item.command.id.clone()).or_insert_with(home);
                    }
                }
                Node::Submenu { label, children, .. } => {
                    path.push(label.clone());
                    walk(children, path, out);
                    path.pop();
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for menu in &bar.menus {
        walk(&menu.children, &mut vec![menu.title.clone()], &mut out);
    }
    out
}

/// The menu bar the user sees, whose places the homes name: the native application menu bar, with
/// About, Settings, Language, Appearance and Quit in its application menu, when the app installed
/// one, else the in-window bar.
fn home_bar(app: &VectorcraftApp) -> std::borrow::Cow<'_, MenuBar> {
    if app.services.native_menu.is_some() {
        // PLAN: assumed menus-w7 `pub(crate) fn options() -> craft::menu::LayoutOptions<'static>` in a `pub(crate) mod native` of `menus`
        std::borrow::Cow::Owned(craft::menu::mac_layout(&app.menus.bar, &crate::menus::native::options()).bar)
    } else {
        std::borrow::Cow::Borrowed(&app.menus.laid)
    }
}

/// The kind of result `id` is: the icon its row starts with, and the word for the kind in the UI
/// language. A tool shows its own icon.
fn kind_of(id: &str) -> (&'static str, &'static str) {
    match id.split_once(':') {
        Some(("tool", tool)) => (vectorcraft_tools::tool_info(tool).map_or("square-dashed", |t| crate::icons::tool_icon(t.icon)), tl!("Tool")),
        Some(("panel", _)) => ("layout-grid", tl!("Panel")),
        Some(("effect", _)) => ("dc-fx", tl!("Effect")),
        Some(("plugin", _)) => ("dc-filter", tl!("Filter")),
        _ => ("menu", tl!("Command")),
    }
}

/// Where result `d` lives, as its row shows it: a tool's place on the toolbars ([`tool_home`]),
/// the menu path of the item of the menu bar that runs it (`home`, with the item's label when it
/// differs from the result's), or "No menu item".
fn home_text(app: &VectorcraftApp, d: &Draft, home: Option<&Home>) -> String {
    let lang = crate::i18n::current();
    if let Some(tool) = d.id.strip_prefix("tool:") {
        return tool_home(app, tool);
    }
    match home {
        Some(h) if !h.item.is_empty() && h.item != d.label => format!("{} › {}", h.path, item_label(lang, &d.id, &h.item)),
        Some(h) => h.path.clone(),
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
    // Tab hides every toolbar with the panels, as the main window's shell left it.
    let tab_hidden = app.ui_ctx.as_ref().is_some_and(|ctx| craft::ui::shell::chrome_state(ctx, egui::ViewportId::ROOT).hidden);
    let toolbars_shown = app.ui.screen_mode < 3 && !tab_hidden;
    let main = crate::toolbar::slots(app).into_iter().map(|(_, s)| s).find(|s| s.contains(&tool));
    // A group torn off into a strip shows each of its tools on its own button.
    let main = main.map(|slot| {
        let torn_off = slot.len() > 1 && crate::toolbar::is_floating(app, None, slot[0]);
        place(tl!("Toolbar"), if torn_off { None } else { Some(crate::toolbar::shown_tool(app, &slot)) })
    });
    let active = app.session.tool_id();
    let custom = |shown: bool| {
        app.ui.custom_toolbars.iter().filter(|b| crate::custom_toolbars::is_on(&app.ui, b) == shown).find_map(|bar| {
            let slot = bar.slots.iter().find(|s| s.iter().any(|t| t == tool))?;
            let tools: Vec<&'static str> = slot.iter().filter_map(|id| vectorcraft_tools::tool_info(id).map(|t| t.id)).collect();
            if tools.len() > 1 && tools.first().is_some_and(|first| crate::toolbar::is_floating(app, Some(&bar.name), first)) {
                return Some(place(&bar.name, None));
            }
            Some(place(&bar.name, crate::custom_toolbars::shown_tool(bar, active, &tools)))
        })
    };
    if toolbars_shown {
        if crate::toolbar::shown(&app.ui)
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

/// The home of each tool result, in the order [`drafts`] lists the tools.
fn tool_homes(app: &VectorcraftApp) -> Vec<String> {
    vectorcraft_tools::catalog::all_tools().map(|t| tool_home(app, t.id)).collect()
}

/// Whether `e` is a tool's result.
fn is_tool(e: &HelpEntry) -> bool {
    e.ids().first().is_some_and(|id| id.starts_with("tool:"))
}

/// The palette's entries in `lang`, one per [`named_drafts`] result. A pick runs the entry's
/// command, the one its result id stands for ([`crate::keymap::command_of`]); the runner runs a
/// command with no params as its menu item does. An entry is found by its label, its English name,
/// its menu path, its id (and the id of the engine command a UI command wraps) and its
/// description, and shows its home ([`home_text`]) in place of its path. The homes name places in
/// the menu bar as it is shown now.
fn entries(app: &VectorcraftApp, lang: Lang) -> Vec<HelpEntry> {
    let homes = menu_homes(&home_bar(app));
    let drafts = named_drafts(lang);
    let params = params_texts(&drafts);
    drafts
        .iter()
        .zip(params)
        .filter(|(d, _)| d.id != menus::TODO)
        .map(|(d, params)| {
            let home = homes.get(&d.id);
            let shown = d.name(lang);
            let path = shown_label(lang, &d.path);
            // A row with a home shows its label without the menu path in front of it. A path that
            // makes the name distinct ([`distinct_names`]) stays, unless the row's home shows that
            // same path or it is the Tools in front of a tool's name, which the tool's toolbar shows.
            let tool = d.id.starts_with("tool:");
            let drop = if d.in_context { home.is_some_and(|h| h.path == path) || (tool && d.path == "Tools") } else { home.is_some() || tool };
            let label = match shown.strip_prefix(&format!("{path} › ")) {
                Some(label) if drop && !d.path.is_empty() => label.to_string(),
                _ => shown.clone(),
            };
            let segments: Vec<String> = if d.path.is_empty() { Vec::new() } else { path.split(" › ").map(str::to_string).collect() };
            let wrapped = WRAPPED.iter().find(|(_, ui)| *ui == d.id).map(|(engine, _)| (*engine).to_string());
            let entry = HelpEntry::new(crate::keymap::command_of(&d.id), label)
                .with_english(d.name(Lang::EN))
                .with_path(segments)
                .with_home(home_text(app, d, home))
                .with_icon(IconSpec::svg(IconId::new(kind_of(&d.id).0)))
                .with_ids(std::iter::once(d.id.clone()).chain(wrapped));
            let description = description(params);
            if description.is_empty() { entry } else { entry.with_words(description) }
        })
        .collect()
}

/// What the palette's entries are built from besides the tool homes: the catalog's generation
/// (the language, the plug-ins, the Scripts folder, the shortcuts and the fonts) and whether the
/// homes follow the native menu bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BuildKey {
    catalog: u64,
    native: bool,
}

/// The palette's entries and their index, one pair in one order: the index names entries by
/// their place. `tool_homes` are the homes the tool entries show.
#[derive(Clone)]
pub(crate) struct Built {
    entries: Vec<HelpEntry>,
    // PLAN: an `Arc`, not the `SearchIndex` the decision names: `SearchIndex` is not `Clone`, and re-homing a tool entry goes through `Arc::make_mut` on `Built`
    index: Arc<SearchIndex>,
    tool_homes: Vec<String>,
}

/// The entries in the UI language and their index, which `ui`'s words fold and expand.
fn build(app: &VectorcraftApp, ui: &egui::Ui) -> Built {
    let entries = entries(app, crate::i18n::current());
    let env = Env::of(ui);
    let view = env.view();
    let index = SearchIndex::new(&view, &entries, &Synonyms::of(&view.words::<SearchWords>()));
    Built { entries, index: Arc::new(index), tool_homes: tool_homes(app) }
}

/// The built entries for the current [`BuildKey`]: built again when the key moves (the index is
/// as costly as production's, so not per pass or per open), with the tool entries re-homed in
/// place when a toolbar changes. A home is shown and never searched, so the index stays.
fn kept_build(app: &mut VectorcraftApp, ui: &egui::Ui) -> Arc<Built> {
    let key = BuildKey { catalog: app.menus.catalog_generation(), native: app.services.native_menu.is_some() };
    // PLAN: assumed hub-w1 `CanvasCache::palette: Option<(BuildKey, Arc<Built>)>` (decision request)
    let mut built = match app.canvas.palette.take() {
        Some((kept, built)) if kept == key => built,
        _ => Arc::new(build(app, ui)),
    };
    let homes = tool_homes(app);
    if built.tool_homes != homes {
        let b = Arc::make_mut(&mut built);
        for (entry, home) in b.entries.iter_mut().filter(|e| is_tool(e)).zip(&homes) {
            *entry = entry.clone().with_home(home.clone());
        }
        b.tool_homes = homes;
    }
    app.canvas.palette = Some((key, built.clone()));
    built
}

/// How many command ids [`CommandUses`] counts at most.
const MAX_COUNTED_COMMANDS: usize = 200;

/// The count at which [`CommandUses::count`] halves every count.
const HALVING_COUNT: u32 = 1_000;

/// How many times each command was picked in the palette: commands picked often rank first among
/// the results that match a query as well (the palette's pick counts). It counts at most
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

    /// How many times `id` was picked, as counted: 0 for an id never picked or no longer counted.
    pub(crate) fn count_of(&self, id: &str) -> u32 {
        self.uses.get(id).map_or(0, |&(n, _)| n)
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

/// The command that opens and closes the palette. It is no result: picking it would close the
/// palette and open it again.
const PALETTE_TOGGLE: &str = "help.commandPalette";

/// Opens the palette when it is closed, and closes it when it is open.
pub(crate) fn toggle(app: &mut VectorcraftApp) {
    app.ui.palette_open = !app.ui.palette_open;
}

/// Records a palette pick whose run did not fail, which the runner reports: its count ranks it
/// among the results that match a query as well. The palette has already put it first in the
/// recent picks.
// PLAN: counted by the command's key, not its id as the sheet says: panels, effects and filters share an id (`window.panel`, `effect.dialog`, `plugin.dialog`) and differ only in params
pub(crate) fn record_pick(app: &mut VectorcraftApp, command: &CommandRef) {
    app.ui.command_uses.count(&command.key_text());
}

/// Why a palette pick of `command` runs nothing now, as the status line says it, or `None` while it
/// can run: a panel can't be shown in Presentation Mode, and any other result's menu item is dimmed
/// (an effect's or a filter's while nothing is selected), with the engine's reason when it gives one.
/// A tool can always be chosen; a choice that fails says so itself.
pub(crate) fn refusal(app: &VectorcraftApp, command: &CommandRef) -> Option<String> {
    let id = command.id.as_str();
    if id.starts_with("tool:") {
        return None;
    }
    if id == "window.panel" {
        return (app.ui.screen_mode >= 3).then(|| "panels can't be shown in Presentation Mode".to_string());
    }
    let why = match crate::menus::why_off(app, id)? {
        Some(why) => Some(why),
        // Dimmed before the engine was asked (no Appearance subject, an input method composing).
        None => vectorcraft_engine::find_command(id).and_then(|c| (c.enabled)(&app.session).err()),
    };
    Some(match why {
        Some(why) => vectorcraft_engine::EngineError::Disabled(id.to_string(), why).to_string(),
        None => format!("command `{id}` is not available right now"),
    })
}

/// Shows the palette while it is open. The framework palette ranks, caps the results, keeps the
/// recent picks and closes on a press outside it; a pick reaches the app as a command fired from
/// the palette, which the runner runs and records ([`record_pick`]). Nothing is built while the
/// palette is closed.
pub fn show(app: &mut VectorcraftApp, ui: &mut egui::Ui) {
    if !app.ui.palette_open {
        return;
    }
    let built = kept_build(app, ui);
    let label = Env::of(ui).words::<PanelWords>().palette_search.clone();
    let crate::state::UiState { palette_open, recent_commands, command_uses, .. } = &mut app.ui;
    let uses: &CommandUses = command_uses;
    let count = |command: &CommandRef| uses.count_of(&command.key_text());
    CommandPalette::new(label, &built.entries, &built.index, Bind::bound(palette_open)).recents(Bind::bound(recent_commands)).uses(&count).show(ui);
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft::ui::menu::rank;
    use craft::ui::runtime::commands::{self, FirePhase, FireSource, Fired, ResolvedCommand};
    use craft::ui::runtime::frame::{ChromeToggle, request_chrome_toggle};
    use craft::ui::runtime::overlay;
    use serde_json::json;
    use vectorcraft_engine::Session;
    use vectorcraft_testkit::fixtures::Fixture;

    use crate::i18n::LangExt as _;

    /// An app over `session` in a window of its own with its menu model built, in English.
    fn app_on(session: Session) -> VectorcraftApp {
        crate::i18n::set_current(Lang::EN);
        let mut app = crate::testing::windowed(VectorcraftApp::new(session, crate::Services::default()));
        let ctx = app.ui_ctx.clone().unwrap();
        crate::menus::model::refresh(&mut app, &ctx);
        app
    }

    /// Fires `command` from `source` and runs it as the window runs a fired command.
    fn fire(app: &mut VectorcraftApp, ctx: &egui::Context, command: CommandRef, source: FireSource) {
        let fired = Fired::new(command, source, FirePhase::Press, ctx.input(|i| i.time));
        crate::shortcuts::run_fired(app, ctx, &fired);
    }

    /// A pick of result `id`: the command its entry runs ([`crate::keymap::command_of`]), fired
    /// from the palette.
    fn pick(app: &mut VectorcraftApp, id: &str) {
        let ctx = app.ui_ctx.clone().unwrap();
        fire(app, &ctx, crate::keymap::command_of(id), FireSource::Palette);
    }

    /// Cancels, or confirms, every open dialog; the window's pump runs what the answer does.
    fn close_dialogs(app: &mut VectorcraftApp, confirm: bool) {
        let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) else { return };
        for id in dialogs.with(|host| host.open_dialogs()).into_iter().map(|d| d.id) {
            if confirm {
                crate::tests::picks::answer_dialog(app, id, &[], true);
            } else {
                dialogs.with(|host| host.cancel_by_id(&ctx, id));
                crate::dialogs::host::pump(app, &ctx);
            }
        }
    }

    /// Whether a dialog is open, the active document's undo depth and the status line.
    type Outcome = (bool, Option<usize>, String);

    /// The [`Outcome`] `app` shows.
    fn outcome(app: &VectorcraftApp) -> Outcome {
        (app.any_dialog_open(), app.session.active().map(|st| st.history.undo.len()), app.ui.status.clone())
    }

    /// The result id of entry `e`.
    fn id_of(e: &HelpEntry) -> &str {
        e.ids().first().map_or("", String::as_str)
    }

    /// The entry of result `id`.
    fn entry<'a>(entries: &'a [HelpEntry], id: &str) -> &'a HelpEntry {
        entries.iter().find(|e| id_of(e) == id).unwrap_or_else(|| panic!("{id} is a result"))
    }

    /// The shown name of each result in `lang`: (id, name).
    fn names(lang: Lang) -> Vec<(String, String)> {
        named_drafts(lang)
            .into_iter()
            .map(|d| {
                let name = d.name(lang);
                (d.id, name)
            })
            .collect()
    }

    /// The name result `id` shows in `names`.
    fn name_of(names: &[(String, String)], id: &str) -> String {
        names.iter().find(|(i, _)| i == id).map(|(_, n)| n.clone()).unwrap_or_else(|| panic!("{id} is a result"))
    }

    /// The entries and index [`show`] builds for `app` in the current language.
    fn built_for(app: &VectorcraftApp) -> Built {
        let ctx = crate::testing::ui_context();
        let mut built = None;
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| built = Some(build(app, ui)));
        out.textures_delta.clear();
        built.expect("the pass ran")
    }

    /// The ids of the results `query` lists, best first, with no recent picks or pick counts.
    fn ranked(built: &Built, query: &str) -> Vec<String> {
        rank(&built.index, query, &[], &|_| 0).into_iter().filter_map(|i| built.entries.get(i)).map(|e| id_of(e).to_string()).collect()
    }

    /// The lowercased texts a word of a query is text in: the English name, the label, the menu
    /// path and the ids, one per line.
    fn haystack(e: &HelpEntry) -> String {
        let english = e.english().map_or("", |l| l.as_str());
        let path = e.path().iter().map(|p| p.as_str()).collect::<Vec<_>>().join(" › ");
        [english, e.label().as_str(), path.as_str()]
            .into_iter()
            .chain(e.ids().iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase()
    }

    /// The ids of the results whose texts hold every word of `query` ([`haystack`]).
    fn substring_rule(entries: &[HelpEntry], query: &str) -> Vec<String> {
        let q = query.to_lowercase();
        entries.iter().filter(|e| q.split_whitespace().all(|w| haystack(e).contains(w))).map(|e| id_of(e).to_string()).collect()
    }

    /// Searching for the preference's name, Use Precise Cursors, finds the command that toggles it,
    /// and choosing the first result changes the preference.
    #[test]
    fn use_precise_cursors_palette_result_toggles_the_preference() {
        let mut app = app_on(Session::new());
        let built = built_for(&app);
        let found = ranked(&built, "use precise cursors");
        assert!(found.iter().any(|id| id == "view.preciseCursors"), "preference toggle appears for its name");
        assert!(built.entries.iter().filter(|e| id_of(e) == "view.capsLock").all(|e| e.label().as_str() != "Use Precise Cursors"));
        let before = app.session.prefs.use_precise_cursors;
        pick(&mut app, found.first().expect("query has a result"));
        assert_ne!(app.session.prefs.use_precise_cursors, before, "first result toggles the preference");
    }

    /// The palette lists `select.key` once, as Cancel Key Object (#541): a pick runs it with no id,
    /// which cancels the key object. `engine.commands` gives agents the same label.
    #[test]
    fn the_palette_lists_cancel_key_object_once() {
        let items = items();
        let ids: Vec<&str> = items.iter().filter(|(label, ..)| label == "Cancel Key Object").map(|(_, id)| id.as_str()).collect();
        assert_eq!(ids, ["select.key"]);
        let mut app = app_on(vectorcraft_engine::Session::new());
        let commands = crate::control::all_commands(&app);
        let key = commands.as_array().unwrap().iter().find(|c| c["id"] == "select.key").unwrap();
        assert_eq!(key["label"], "Cancel Key Object");
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let a = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
        let b = app.run("shape.rectangle", json!({"x": 80, "y": 50, "width": 30, "height": 30})).unwrap()["id"].clone();
        app.run("select.set", json!({"ids": [a, b]})).unwrap();
        app.run("select.key", json!({"id": b})).unwrap();
        assert!(app.session.active().unwrap().selection.key.is_some());
        pick(&mut app, ids[0]);
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
            assert!(items.iter().all(|(_, id)| id != engine), "{engine} is left out");
            let listed: Vec<&str> = items.iter().filter(|(_, id)| id == ui).map(|(label, ..)| label.as_str()).collect();
            let path: String = spec.menu.iter().map(|m| format!("{m} › ")).collect();
            assert_eq!(listed, [format!("{path}{}", wrapper.1)], "{ui} is listed once, with {engine}'s menu path");
        }
        // A search for Delete Selection, or for its menu path, finds the Layers panel's, and not the
        // engine command that deletes without asking; the path translates.
        let built = built_for(&app_on(Session::new()));
        for (engine, ui) in WRAPPED {
            assert!(entry(&built.entries, ui).ids().iter().any(|id| id == engine), "a search for {engine} finds {ui}");
        }
        for query in ["delete selection", "layers delete", "layer.delete"] {
            let found = ranked(&built, query);
            assert!(found.iter().any(|id| id == "ui.layerDelete") && !found.iter().any(|id| id == "layer.delete"), "{query}: {found:?}");
        }
        let zh = Lang::from_code("zh-hant").unwrap();
        let want = ["Window", "Layers", "Delete Selection"].map(|s| crate::i18n::tr(zh, s)).join(" › ");
        assert_eq!(name_of(&names(zh), "ui.layerDelete"), want);
    }

    /// Every result a search for Delete Selection gives asks before deleting a layer that holds art:
    /// Cancel keeps it, and OK deletes it in one undo step. A script's `layer.delete` still deletes
    /// at once.
    #[test]
    fn a_pick_of_delete_selection_asks_before_deleting_a_layer_with_art() {
        let entries = entries(&app_on(Session::new()), Lang::EN);
        let picks: Vec<String> = substring_rule(&entries, "delete selection");
        assert!(!picks.is_empty(), "the search finds Delete Selection");
        let node = |v: &serde_json::Value| vectorcraft_doc::NodeId(v.as_u64().unwrap());
        let exists = |app: &VectorcraftApp, v: &serde_json::Value| app.session.active().unwrap().doc.node(node(v)).is_some();
        let undo = |app: &VectorcraftApp| app.session.active().unwrap().history.undo.len();
        for id in &picks {
            let mut app = app_on(Session::new());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let layer = app.run("layer.new", json!({})).unwrap()["id"].clone();
            let art = app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].clone();
            app.run("select.none", json!({})).unwrap();
            let before = undo(&app);
            pick(&mut app, id);
            assert!(app.any_dialog_open(), "{id} asks first: {}", app.ui.status);
            assert!(exists(&app, &layer) && exists(&app, &art) && undo(&app) == before, "{id}: nothing is deleted yet");
            close_dialogs(&mut app, false);
            assert!(exists(&app, &layer) && exists(&app, &art) && undo(&app) == before, "{id}: Cancel keeps the layer");
            pick(&mut app, id);
            close_dialogs(&mut app, true);
            assert!(!exists(&app, &layer) && !exists(&app, &art), "{id}: OK deletes the layer and its art");
            assert_eq!(undo(&app), before + 1, "{id}: one undo step");
            app.run("edit.undo", json!({})).unwrap();
            app.run("layer.delete", json!({ "ids": [layer] })).unwrap();
            assert!(!app.any_dialog_open() && !exists(&app, &layer), "a script deletes at once");
        }
    }

    /// The Asset Export commands the palette lists.
    const ASSET_EXPORT: [&str; 6] =
        ["assets.export", "assets.remove", "assets.rename", "assets.settings.set", "assets.export3D", "assets.set3DFormat"];

    /// #4864: a pick of an Asset Export command, which the palette runs with no params, opens the
    /// Asset Export panel instead of failing on the missing params or exporting nowhere.
    #[test]
    fn a_pick_of_an_asset_export_command_opens_its_panel() {
        let showing = |app: &VectorcraftApp| crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID);
        for id in ASSET_EXPORT {
            assert!(items().iter().any(|(_, item)| item == id), "{id} is in the palette");
            let mut app = app_on(Session::new());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            app.ui.status.clear();
            assert!(!showing(&app), "{id}");
            pick(&mut app, id);
            assert!(showing(&app), "{id}: {}", app.ui.status);
            assert!(!app.ui.status.contains("invalid parameters"), "{id}: {}", app.ui.status);
            // A second pick leaves the open panel open, docked or floating.
            pick(&mut app, id);
            assert!(showing(&app), "{id}: picked twice");
            app.run("window.panel.float", json!({"panel": crate::panels::asset_export::ID})).unwrap();
            assert!(showing(&app), "{id}: floating");
            pick(&mut app, id);
            assert!(showing(&app), "{id}: floating, picked again");
        }
    }

    /// With no document open, the Asset Export commands are dimmed, and a pick of one that reaches
    /// the runner shows no panel and says why it can't run.
    #[test]
    fn a_dimmed_asset_export_command_does_not_open_its_panel() {
        for id in ASSET_EXPORT {
            let mut app = app_on(Session::new());
            let state = app.menus.states.get(&CommandRef::new(id)).cloned();
            assert!(state.is_some_and(|s| !s.is_enabled()), "{id} is dimmed with no document open");
            app.ui.status.clear();
            pick(&mut app, id);
            assert!(!crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: {}", app.ui.status);
            assert!(!app.ui.status.is_empty(), "{id}: the status line says why");
        }
    }

    /// A pick of a "…" command whose menu bar item gives params opens that item's dialog, Smart
    /// Punctuation opens its dialog and Smooth opens its slider; nothing changes until the dialog
    /// is confirmed.
    #[test]
    fn a_pick_opens_the_dialog_its_menu_item_opens() {
        type Opened = fn(&VectorcraftApp) -> bool;
        let any: Opened = |app| app.any_dialog_open();
        let expected: [(&str, Opened); 5] = [
            ("object.path.cleanUp", any),
            ("object.mesh.create", any),
            ("type.smartPunctuation", |app| {
                app.dialogs().is_some_and(|d| {
                    d.with(|h| h.open_dialogs().iter().any(|d| d.key.as_deref() == Some(crate::dialogs::smart_punctuation::COMMAND)))
                })
            }),
            ("edit.findReplace", |app| app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<craft::text::ui::find::FindReplaceDialog>()))),
            ("object.path.smooth", crate::dialogs::smooth_slider::showing),
        ];
        for (id, opened) in expected {
            assert!(items().iter().any(|(_, item)| item == id), "{id} is in the palette");
            let mut app = app_on(Fixture::Single.session());
            app.ui.status.clear();
            let before = app.session.active().unwrap().history.undo.len();
            pick(&mut app, id);
            assert!(opened(&app), "{id} opens its dialog: {}", app.ui.status);
            assert_eq!(app.session.active().unwrap().history.undo.len(), before, "{id} changes nothing yet");
            assert!(app.ui.status.is_empty(), "{id}: {}", app.ui.status);
        }
    }

    /// Every palette command whose menu bar items all give it the same params does what a click on
    /// such an item does: with no document open, in an empty document and with one or several
    /// objects selected, the pick and the click leave the same dialog, undo steps and status. While
    /// the item is dimmed, the pick opens no dialog, changes nothing and says why.
    #[test]
    fn every_pick_runs_what_its_menu_item_runs() {
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<&'a craft::menu::Item>) {
            for node in nodes {
                match node {
                    Node::Submenu { children, .. } => walk(children, out),
                    // Items that list an installed plug-in or show a name that is user data come and
                    // go as tests beside this one install plug-ins or add toolbars, and no shortcut or
                    // pick reads them.
                    Node::Item(it) if !menus::lists_plugin(&it.command.id, &it.command.params) && !menus::shows_a_name(&it.command.id, &it.label) => {
                        out.push(it);
                    }
                    _ => {}
                }
            }
        }
        let state = |n: usize| match n.checked_sub(1) {
            None => Session::new(),
            Some(k) => Fixture::ALL[k].session(),
        };
        let model = app_on(state(0));
        let mut rows = vec![];
        for menu in &model.menus.bar.menus {
            walk(&menu.children, &mut rows);
        }
        let mut checked: Vec<(String, CommandRef)> = vec![];
        for (_, id) in items() {
            let of_id: Vec<&CommandRef> = rows.iter().map(|it| &it.command).filter(|c| c.id == id).collect();
            let Some(first) = of_id.first() else { continue };
            if first.params.as_object().is_none_or(serde_json::Map::is_empty) || of_id.iter().any(|c| c.params != first.params) {
                continue;
            }
            checked.push((id, (*first).clone()));
        }
        assert!(checked.iter().any(|(id, _)| id == "edit.findReplace"), "{:?}", checked.iter().map(|c| &c.0).collect::<Vec<_>>());
        let (mut compared, mut dimmed) = (0, 0);
        for (id, item) in &checked {
            for n in 0..=Fixture::ALL.len() {
                let mut picked = app_on(state(n));
                let mut clicked = app_on(state(n));
                picked.ui.status.clear();
                clicked.ui.status.clear();
                // A row the menu leaves out in this state has no click to compare with.
                let Some(row) = clicked.menus.states.get(item).cloned() else { continue };
                let before = outcome(&picked);
                pick(&mut picked, id);
                if row.is_enabled() {
                    let window = clicked.ui_ctx.clone().unwrap();
                    fire(&mut clicked, &window, item.clone(), FireSource::Menu);
                    assert_eq!(outcome(&picked), outcome(&clicked), "{id} [state {n}]");
                    compared += 1;
                } else {
                    let after = outcome(&picked);
                    assert_eq!((after.0, after.1), (false, before.1), "{id} [state {n}] is dimmed");
                    assert!(!after.2.is_empty(), "{id} [state {n}]: the status line says why");
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
            ("addon.image.removeBackground", Fixture::Multi.session()),
            ("object.createObjectMosaic", Fixture::Multi.session()),
            ("edit.findReplace", Session::new()),
        ];
        for (id, session) in cases {
            let mut app = app_on(session);
            assert!(!menus::enabled(&app, id), "{id}");
            app.ui.status.clear();
            let before = app.session.active().map(|st| st.history.undo.len());
            pick(&mut app, id);
            assert!(!app.any_dialog_open(), "{id}");
            assert_eq!(app.session.active().map(|st| st.history.undo.len()), before, "{id}");
            assert!(app.ui.status.contains("not available"), "{id}: {}", app.ui.status);
        }
    }

    /// A pick of a command that opens no dialog runs it at once.
    #[test]
    fn a_pick_of_a_command_without_a_dialog_runs_at_once() {
        for id in ["object.group", "paint.swap"] {
            assert!(items().iter().any(|(_, item)| item == id), "{id} is in the palette");
            let mut app = app_on(Fixture::Multi.session());
            let before = app.session.active().unwrap().history.undo.len();
            pick(&mut app, id);
            assert!(!app.any_dialog_open(), "{id}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), before + 1, "{id}: {}", app.ui.status);
        }
    }

    /// Pick counts for the ids `counts`, picked in that order.
    fn counted(counts: &[(&str, u32)]) -> CommandUses {
        let uses = counts.iter().zip(1..).map(|(&(id, n), last)| (id.to_string(), (n, last))).collect();
        CommandUses { pick: u32::try_from(counts.len()).unwrap(), uses }
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

    /// A pick is counted only when it is recorded in the recent picks: a pick that opens a dialog
    /// is, and a refused pick leaves the earlier recent picks and counts as they were.
    #[test]
    fn only_a_pick_recorded_in_recent_commands_is_counted() {
        let mut window = Window::on(Fixture::Single.session());
        let clean_up = CommandRef::new("object.path.cleanUp");
        window.pick_typed("object.path.cleanUp", 0);
        assert!(window.app.any_dialog_open(), "{}", window.app.ui.status);
        assert_eq!(window.app.ui.recent_commands, [clean_up.clone()]);
        assert_eq!(window.app.ui.command_uses.count_of(&clean_up.key_text()), 1);
        // With no document, Find and Replace is refused.
        let mut window = Window::on(Session::new());
        window.app.ui.recent_commands = vec![CommandRef::new("paint.swap"), CommandRef::new("object.group")];
        for id in ["paint.swap", "paint.swap", "object.group"] {
            window.app.ui.command_uses.count(id);
        }
        let (recent, uses) = (window.app.ui.recent_commands.clone(), window.app.ui.command_uses.clone());
        window.pick_typed("edit.findReplace", 0);
        assert!(!window.app.any_dialog_open(), "no document: the pick is refused");
        assert_eq!(window.app.ui.recent_commands, recent);
        assert_eq!(window.app.ui.command_uses, uses);
    }

    /// A pick says what it did: it ran, it opened a dialog, or it was refused because its menu item
    /// is dimmed, its panel can't be shown or its tool can't be chosen.
    #[test]
    fn a_pick_says_whether_it_ran_opened_a_dialog_or_was_refused() {
        let picked = |session: Session, id: &str| {
            let mut app = app_on(session);
            app.ui.status.clear();
            let before = outcome(&app);
            pick(&mut app, id);
            (before, outcome(&app), app)
        };
        // Ran: one undo step and no dialog.
        let (before, after, _) = picked(Fixture::Multi.session(), "object.group");
        assert_eq!((after.0, after.1), (false, before.1.map(|n| n + 1)), "{}", after.2);
        let (_, _, app) = picked(Fixture::Multi.session(), "tool:knife");
        assert_eq!(app.session.tool_id(), "knife");
        let (_, _, app) = picked(Fixture::Multi.session(), "panel:swatches");
        assert!(crate::panel_docking::showing(&app.ui, "swatches"), "{}", app.ui.status);
        // Opened a dialog, with nothing changed yet.
        let (before, after, _) = picked(Fixture::Single.session(), "object.path.cleanUp");
        assert_eq!((after.0, after.1), (true, before.1));
        // Refused: nothing opens or changes, and the status line says why.
        for id in ["edit.findReplace", "object.group"] {
            let (before, after, _) = picked(Session::new(), id);
            assert!(!after.0 && after.1 == before.1 && !after.2.is_empty(), "{id}: {after:?}");
        }
        let mut presenting = app_on(Fixture::Multi.session());
        presenting.ui.screen_mode = 3;
        pick(&mut presenting, "panel:swatches");
        assert!(!presenting.ui.status.is_empty() && !crate::panel_docking::showing(&presenting.ui, "swatches"));
        let mut stuck = app_on(Fixture::Multi.session());
        stuck.fail_next_tool_choice = Some("no switch".into());
        pick(&mut stuck, "tool:knife");
        assert_eq!(stuck.ui.status, "no switch");
        // With nothing selected, the Effect menu dims every effect.
        let effect = crate::shortcut_editor::effect_items().first().map(|e| format!("effect:{}", e.effect)).expect("an effect");
        let (_, after, _) = picked(Session::new(), &effect);
        assert!(!after.0, "{effect}");
    }

    /// A pick refused while its result is dimmed is not recorded in the recent picks, which keep
    /// their order, nor counted; nor is a pick whose run fails (a tool that can't be chosen now).
    #[test]
    fn a_refused_pick_is_not_recorded() {
        let earlier = vec![CommandRef::new("paint.swap")];
        let mut window = Window::on(Fixture::Multi.session());
        window.app.run("select.none", json!({})).unwrap();
        window.app.ui.recent_commands = earlier.clone();
        window.frame(vec![]);
        assert!(!window.resolved("object.ungroup").enabled, "Ungroup is dimmed with nothing selected");
        window.pick_typed("object.ungroup", 0);
        assert_eq!(window.app.ui.recent_commands, earlier, "a dimmed result does not run");
        assert!(window.app.ui.command_uses.is_empty());
        window.app.fail_next_tool_choice = Some("no switch".into());
        window.pick_typed("tool:knife", 0);
        assert_eq!(window.app.ui.status, "no switch");
        assert!(window.app.ui.command_uses.is_empty(), "a failed run is not counted");
    }

    /// A pick that opens a dialog is recorded in the recent picks when the dialog opens, so it
    /// stays there when the dialog is canceled.
    #[test]
    fn a_pick_that_opens_a_dialog_is_recorded_even_when_canceled() {
        let clean_up = CommandRef::new("object.path.cleanUp");
        let mut window = Window::on(Fixture::Single.session());
        window.app.ui.recent_commands = vec![CommandRef::new("paint.swap")];
        window.pick_typed("object.path.cleanUp", 0);
        assert!(window.app.any_dialog_open(), "{}", window.app.ui.status);
        let recorded = vec![clean_up.clone(), CommandRef::new("paint.swap")];
        assert_eq!(window.app.ui.recent_commands, recorded);
        // Cancel closes the dialog without running the command.
        close_dialogs(&mut window.app, false);
        assert!(!window.app.any_dialog_open());
        window.frame(vec![]);
        assert_eq!(window.app.ui.recent_commands, recorded);
        assert_eq!(window.app.ui.command_uses.count_of(&clean_up.key_text()), 1, "counted once, at the pick");
    }

    /// Each command, tool and plug-in is one result: Place… and Print…, which are engine commands
    /// and UI commands of one id, are listed once, under their engine menu path.
    #[test]
    fn each_id_is_one_result() {
        let entries = entries(&app_on(Session::new()), Lang::EN);
        let mut seen = std::collections::HashSet::new();
        let twice: Vec<&str> = entries.iter().map(id_of).filter(|id| !seen.insert(*id)).collect();
        assert!(twice.is_empty(), "{twice:?}");
        let names = names(Lang::EN);
        assert_eq!((name_of(&names, "file.place"), name_of(&names, "file.print")), ("File › Place…".to_string(), "Print…".to_string()));
    }

    /// No two results for different commands, tools or plug-ins show the same name, in English or
    /// in any language with a complete menu catalog.
    #[test]
    fn different_results_show_different_names() {
        let langs = std::iter::once(Lang::EN).chain(Lang::all().filter(|l| l.complete_menus()));
        let mut shared = vec![];
        for lang in langs {
            let mut by_name: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
            for (id, name) in names(lang) {
                let ids = by_name.entry(name).or_default();
                if !ids.contains(&id) {
                    ids.push(id);
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
        let names = names(Lang::EN);
        let shown = |id: &str| name_of(&names, id);
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
        assert!(names.iter().all(|(id, _)| id != "action.batch" && id != "flattener.preview"), "the engine commands are listed as their UI commands");
        // A name nobody else has stays as it was.
        assert_eq!(shown("object.group"), "Object › Group");
        let entries = entries(&app_on(Session::new()), Lang::EN);
        let id = "file.exportInBackground".to_string();
        for query in ["continue in background", "export continue in background", "file.exportinbackground"] {
            assert!(substring_rule(&entries, query).contains(&id), "{query}");
        }
        assert!(!substring_rule(&entries, "open continue in background").contains(&id));
        let zh = Lang::from_code("zh-hant").unwrap();
        let export = crate::i18n::tr(zh, "Export");
        assert_eq!(name_of(&self::names(zh), &id), format!("{export} › {}", crate::i18n::tr(zh, "Continue in Background")));
        let zh_entries = super::entries(&app_on(Session::new()), zh);
        assert!(substring_rule(&zh_entries, &format!("{export} continue in background")).contains(&id));
    }

    /// Every menu path a result can take when it shares a name, and Window › Show Panel, has a row
    /// in each catalog with complete menus. Each command [`CONTEXTS`] names is a result.
    #[test]
    fn the_paths_of_results_that_share_a_name_translate() {
        let ids: Vec<String> = items().into_iter().map(|(_, id)| id).collect();
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
    /// grid's rulers. Each entry runs the command its result id stands for.
    #[test]
    fn a_pick_of_a_renamed_result_runs_its_own_command() {
        let app = app_on(Session::new());
        let entries = entries(&app, Lang::EN);
        for e in &entries {
            assert_eq!(e.command(), &crate::keymap::command_of(id_of(e)), "{}", id_of(e));
        }
        let named = names(Lang::EN);
        let picks: Vec<(String, String)> =
            named_drafts(Lang::EN).into_iter().filter(|d| d.label == "Show Rulers").map(|d| (d.id.clone(), name_of(&named, &d.id))).collect();
        assert_eq!(picks.len(), 2, "{picks:?}");
        assert!(picks.iter().all(|(_, name)| name != "Show Rulers") && picks[0].1 != picks[1].1, "{picks:?}");
        for (id, name) in &picks {
            let mut app = app_on(Session::new());
            app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
            let label = |app: &VectorcraftApp, id: &str| crate::menus::dynamic_label(app, id, "");
            let before: Vec<String> = picks.iter().map(|(other, _)| label(&app, other)).collect();
            pick(&mut app, id);
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
        let built = names(zh);
        let of = |key: &str| built.iter().filter(|(id, _)| id == key).map(|(_, name)| name.clone()).collect::<Vec<_>>();
        assert_eq!((of(&key_one).len(), of(&key_two).len()), (1, 1));
        assert!(built.iter().all(|(id, _)| id != "plugin.dialog"), "no result without a filter's id");
        let path = format!("{} › {}", crate::i18n::tr(zh, "Object"), crate::i18n::tr(zh, "Plug-ins"));
        assert_eq!(of(&key_two)[0], format!("{path} › Group…"));
        assert_eq!(of(&key_one)[0], format!("{path} › Group"));
        assert_eq!(name_of(&names(Lang::EN), &key_two), "Object › Plug-ins › Group…", "found by its English name");

        let mut app = app_on(Session::new());
        pick(&mut app, &key_two);
        assert!(!app.any_dialog_open(), "no document: the dimmed item's pick does nothing");
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        pick(&mut app, &key_two);
        assert!(app.any_dialog_open(), "the filter's dialog opens: {}", app.ui.status);
        close_dialogs(&mut app, false);
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
        let ui_ids: std::collections::HashSet<&str> = menus::UI_COMMANDS.iter().map(|c| c.0).collect();
        // The rule before: every engine command but the aliases, the wrapped commands and the ids of
        // UI commands.
        let before: Vec<&vectorcraft_engine::CommandSpec> = vectorcraft_engine::command_specs()
            .iter()
            .filter(|c| !vectorcraft_engine::cmd::is_alias(c.id) && !WRAPPED.iter().any(|(e, _)| *e == c.id) && !ui_ids.contains(c.id))
            .collect();
        let listed: std::collections::HashSet<String> = items().into_iter().map(|(_, id)| id).collect();
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
            (doc, app.session.script_console().lines().len(), app.any_dialog_open(), ui)
        };
        let acts = |id: &str| {
            [Session::new(), Fixture::Single.session(), Fixture::Multi.session()].into_iter().any(|session| {
                let mut app = app_on(session);
                let before = look(&app);
                pick(&mut app, id);
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
    /// preset, Delete Gradient Preset deletes the one saved from the selection, and the edits
    /// between Begin Undo Group and End Undo Group are one undo step.
    #[test]
    fn a_pick_of_a_session_command_runs_it() {
        let ids: Vec<String> = items().into_iter().map(|(_, id)| id).collect();
        let mut app = app_on(Fixture::Single.session());
        let pick_listed = |app: &mut VectorcraftApp, id: &str| {
            assert!(ids.iter().any(|i| i == id), "{id} is a result");
            pick(app, id);
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
        pick_listed(&mut app, "script.console.clear");
        assert!(app.session.script_console().lines().is_empty(), "{}", app.ui.status);

        let default = app.session.prefs.use_precise_cursors;
        app.session.prefs.use_precise_cursors = !default;
        pick_listed(&mut app, "prefs.reset");
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
            pick_listed(&mut app, save);
            assert_eq!(count(&mut app), before + 1, "{save}: {}", app.ui.status);
        }
        // Save Gradient Preset saves the selection's gradient, and Delete Gradient Preset deletes
        // the saved preset it matches.
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        app.run("paint.setFill", json!({"gradient": {"kind": "linear", "stops": stops}})).unwrap();
        pick_listed(&mut app, "gradient.presets.save");
        assert_eq!(app.session.prefs.gradient_presets.len(), 1, "{}", app.ui.status);
        pick_listed(&mut app, "gradient.presets.delete");
        assert!(app.session.prefs.gradient_presets.is_empty(), "{}", app.ui.status);

        let undo = |app: &VectorcraftApp| app.session.active().map_or(0, |st| st.history.undo.len());
        let before = undo(&app);
        pick_listed(&mut app, "edit.undoGroup.begin");
        for x in [10, 200] {
            app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
        }
        pick_listed(&mut app, "edit.undoGroup.end");
        assert_eq!(undo(&app), before + 1, "{}", app.ui.status);
    }

    /// The UI commands that open an engine command's dialog or ask first are still listed, and the
    /// engine commands they wrap are not.
    #[test]
    fn the_wrapping_ui_commands_stay_listed() {
        let ids: std::collections::HashSet<String> = items().into_iter().map(|(_, id)| id).collect();
        for (engine, ui) in WRAPPED {
            assert!(ids.contains(*ui) && !ids.contains(*engine), "{ui} listed, {engine} not");
        }
        let skipped = ["plugin.dialog", crate::scripts_menu::COMMAND, PALETTE_TOGGLE];
        assert!(menus::UI_COMMANDS.iter().filter(|c| !skipped.contains(&c.0)).all(|c| ids.contains(c.0)));
    }

    /// Each panel is a result under Window › Show Panel. A pick shows the panel, and a second pick
    /// leaves it showing rather than hiding it as the Window menu item does.
    #[test]
    fn a_panel_is_a_result_whose_pick_shows_it() {
        let built = built_for(&app_on(Session::new()));
        let panels = built.entries.iter().filter(|e| id_of(e).starts_with("panel:")).count();
        assert_eq!(panels, crate::state::all_panels().count());
        // Many results hold "layers" in their menu path; the panel, named Layers, is shown first,
        // and "color guide" shows the Color Guide panel first too.
        for (query, want) in [("layers", "panel:layers"), ("color guide", "panel:colorGuide"), ("Layers ", "panel:layers")] {
            let shown = ranked(&built, query);
            assert_eq!(shown.first().map(String::as_str), Some(want), "{query}: {shown:?}");
            // English plural and verb forms can find more results than the text alone.
            assert!(substring_rule(&built.entries, query).iter().all(|id| shown.contains(id)), "{query}");
        }
        assert_eq!(name_of(&names(Lang::EN), "panel:layers"), "Window › Show Panel › Layers");
        let zh = Lang::from_code("zh-hant").unwrap();
        assert_eq!(name_of(&names(zh), "panel:layers"), ["Window", "Show Panel", "Layers"].map(|s| crate::i18n::tr(zh, s)).join(" › "));
        for id in ["swatches", crate::panels::asset_export::ID] {
            let mut app = app_on(Session::new());
            assert!(!crate::panel_docking::showing(&app.ui, id), "{id}");
            pick(&mut app, &format!("panel:{id}"));
            assert!(crate::panel_docking::showing(&app.ui, id), "{id}: {}", app.ui.status);
            pick(&mut app, &format!("panel:{id}"));
            assert!(crate::panel_docking::showing(&app.ui, id), "{id}: picked twice, still showing");
        }
    }

    /// In Presentation Mode a panel's pick fails as `window.panel` does: the status line says why,
    /// and the panel does not show when the mode ends.
    #[test]
    fn a_panel_pick_in_presentation_mode_changes_nothing() {
        let mut app = app_on(Session::new());
        let (open_panel, dock) = (app.ui.open_panel.clone(), app.ui.dock_workspace.clone());
        app.ui.screen_mode = 3;
        pick(&mut app, "panel:swatches");
        assert!(app.ui.status.contains("Presentation Mode"), "{}", app.ui.status);
        assert_eq!((app.ui.open_panel.clone(), app.ui.dock_workspace.clone()), (open_panel, dock));
        app.ui.screen_mode = 0;
        assert!(!crate::panel_docking::showing(&app.ui, "swatches"), "the pick did not place the panel");
    }

    /// Each Effect menu item is a result under its menu path. With a document open and an object
    /// selected, a pick of Drop Shadow… opens its dialog, as the menu item does, and changes nothing
    /// until the dialog is confirmed.
    #[test]
    fn an_effect_is_a_result_whose_pick_runs_its_menu_item() {
        // Both reads see the same plug-in revision: another test may install or remove a live
        // effect plug-in between them.
        let (built, effects) = loop {
            let revision = menus::plugin_revision();
            let built = built_for(&app_on(Session::new()));
            let effects = crate::shortcut_editor::effect_items();
            if menus::plugin_revision() == revision {
                break (built, effects);
            }
        };
        for e in effects.iter() {
            let key = format!("effect:{}", e.effect);
            assert_eq!(built.entries.iter().filter(|r| id_of(r) == key).count(), 1, "{key}");
        }
        let found = ranked(&built, "drop shadow");
        let shadow = found.iter().find(|id| id.starts_with("effect:")).unwrap_or_else(|| panic!("{found:?}"));
        let item = effects.iter().find(|e| format!("effect:{}", e.effect) == *shadow).unwrap();
        assert_eq!(name_of(&names(Lang::EN), shadow), format!("{} › {}", item.group, item.label));
        let mut picked = app_on(Fixture::Single.session());
        let mut clicked = app_on(Fixture::Single.session());
        let before = outcome(&picked);
        pick(&mut picked, shadow);
        let click = CommandRef::with_params(item.command, json!({ "effect": item.effect }));
        let window = clicked.ui_ctx.clone().unwrap();
        fire(&mut clicked, &window, click, FireSource::Menu);
        let after = outcome(&picked);
        assert!(after.0, "the dialog opens: {}", after.2);
        assert_eq!(after.1, before.1, "nothing changes yet");
        assert_eq!(after, outcome(&clicked), "the pick runs what the menu item runs");
        // With nothing selected the menu item is dimmed: no dialog opens, and the status line says
        // why.
        let mut app = app_on(Session::new());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        app.ui.status.clear();
        pick(&mut app, shadow);
        assert!(!app.any_dialog_open() && !app.ui.status.is_empty(), "{}", app.ui.status);
        // A built-in effect's name translates by its id, as the Effect menu shows it.
        let fr = Lang::from_code("fr").unwrap();
        let add = name_of(&names(fr), "effect:pathfinder.add");
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
        let built = names(zh);
        let path = format!("{} › {}", crate::i18n::tr(zh, "Object"), crate::i18n::tr(zh, "Plug-ins"));
        for id in [one, two] {
            let key = format!("plugin:{id}");
            let shown: Vec<&str> = built.iter().filter(|(i, _)| *i == key).map(|(_, name)| name.as_str()).collect();
            assert_eq!(shown, [format!("{path} › Same Name… ({key})")], "{id}");
        }
        let mut app = app_on(Session::new());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        for id in [one, two] {
            pick(&mut app, &format!("plugin:{id}"));
            assert!(app.any_dialog_open(), "the filter's dialog opens: {}", app.ui.status);
            close_dialogs(&mut app, false);
        }
        vectorcraft_plugins::registry::remove(one);
        vectorcraft_plugins::registry::remove(two);
    }

    /// The home English result `id` shows with the app's current layout.
    fn home_of(app: &VectorcraftApp, id: &str) -> String {
        crate::i18n::set_current(Lang::EN);
        let entries = entries(app, Lang::EN);
        entry(&entries, id).home().map(|h| h.as_str().to_string()).unwrap_or_default()
    }

    /// eval-menus must-fix 18: a row shows where its result lives. A menu command's home is the
    /// menu that holds its item and its label leaves that path out; a home names the item when its
    /// label differs; a tool's home is its toolbar; a command no item runs says so and keeps its
    /// whole name.
    #[test]
    fn rows_show_where_each_result_lives() {
        let app = app_on(Session::new());
        let entries = entries(&app, Lang::EN);
        let home = |id: &str| entry(&entries, id).home().map(|h| h.as_str().to_string()).unwrap_or_default();
        let label = |id: &str| entry(&entries, id).label().as_str().to_string();
        let menu = |title: &str| crate::i18n::tr(Lang::EN, title).to_string();
        assert!(home("object.group").starts_with(&menu("Object")), "{}", home("object.group"));
        assert!(!label("object.group").contains(" › "), "{}", label("object.group"));
        assert!(home("panel:layers").starts_with(&menu("Window")), "{}", home("panel:layers"));
        assert!(home("effect:stylize.roundCorners").starts_with(&menu("Effect")), "{}", home("effect:stylize.roundCorners"));
        // Quit VectorCraft's item is File › Exit: the home names the item after the menu.
        let quit = home("app.quit");
        assert!(quit.starts_with(&menu("File")) && quit.split(" › ").count() > 1, "{quit}");
        // Window › Learn runs the Learn command as well as showing the Learn panel.
        assert_eq!(home("window.learn"), home(&format!("panel:{}", crate::panels::learn::ID)));
        for tool in ["knife", "pen"] {
            assert_eq!(home(&format!("tool:{tool}")), tool_home(&app, tool), "{tool}");
        }
        assert!(!label("tool:knife").contains(" › "), "{}", label("tool:knife"));
        let none =
            entries.iter().find(|e| !is_tool(e) && e.home().is_some_and(|h| h.as_str() == tl!("No menu item"))).expect("a command with no menu item");
        assert_eq!(none.label().as_str(), name_of(&names(Lang::EN), id_of(none)), "a row with no home keeps its whole name");
    }

    /// eval-menus must-fix 18: no two results' rows show the same kind, label and home, in any
    /// language.
    #[test]
    fn no_two_rows_read_alike() {
        let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
        let mut alike = vec![];
        for lang in Lang::all() {
            crate::i18n::set_current(lang);
            crate::menus::model::refresh(&mut app, &crate::testing::ui_context());
            let mut seen = std::collections::HashMap::new();
            for e in &entries(&app, lang) {
                let row = (kind_of(id_of(e)).0, e.label().as_str().to_string(), e.home().map(|h| h.as_str().to_string()));
                if let Some(other) = seen.insert(row.clone(), id_of(e).to_string())
                    && other != id_of(e)
                {
                    alike.push(format!("{}: {other} and {}: {row:?}", lang.code(), id_of(e)));
                }
            }
        }
        crate::i18n::set_current(Lang::EN);
        assert!(alike.is_empty(), "{}", alike.join("\n"));
    }

    /// Every catalog translates the words a row's home is read with.
    #[test]
    fn catalogs_translate_the_kind_and_home_words() {
        let words = ["Toolbar", "{toolbar}, in the {tool} group", "All Tools", "No menu item"];
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
        // A window, so Tab hides the chrome in the shell's pass.
        let mut window = Window::on(Session::new());
        let app = &mut window.app;
        let home = |app: &VectorcraftApp, id: &str| home_of(app, id);
        assert_eq!(home(&app, "tool:groupSelection"), "Toolbar, in the Direct Selection Tool group");
        assert_eq!(home(&app, "tool:directSelection"), "Toolbar");
        app.select_tool("groupSelection");
        assert_eq!(home(&app, "tool:groupSelection"), "Toolbar");
        assert_eq!(home(&app, "tool:directSelection"), "Toolbar, in the Group Selection Tool group");
        // A toolbar made with New Toolbar holding the tools of `slot`; a hidden one has its panel closed.
        let bar = |app: &mut VectorcraftApp, name: &str, slot: &[&str], shown: bool| {
            crate::custom_toolbars::create(app, name).unwrap();
            let toolbar = crate::custom_toolbars::find_mut(&mut app.ui, name).unwrap();
            toolbar.slots = vec![slot.iter().map(|t| t.to_string()).collect()];
            let panel = crate::custom_toolbars::panel_id(toolbar.id);
            if !shown {
                crate::panel_docking::hide_group(app, &panel).unwrap();
            }
        };
        // Window › Tools hides the Tools panel while a toolbar made with New Toolbar holds the Pen
        // Tool: the row names that toolbar.
        crate::toolbar::set_shown(app, false);
        assert_eq!(home(&app, "tool:pen"), "Toolbar");
        bar(app, "Drawing", &["pen", "curvature"], true);
        assert_eq!(home(&app, "tool:pen"), "Drawing");
        assert_eq!(home(&app, "tool:curvature"), "Drawing, in the Pen Tool group");
        app.ui.custom_toolbars[0].name = "{tool}".into();
        assert_eq!(home(&app, "tool:curvature"), "{tool}, in the Pen Tool group");
        app.ui.custom_toolbars[0].name = "Drawing".into();
        // Tab hides the toolbars made with New Toolbar too: the hidden Tools panel comes first.
        request_chrome_toggle(&window.ctx, egui::ViewportId::ROOT, ChromeToggle::Panels);
        window.frame(vec![]);
        assert_eq!(home(&window.app, "tool:pen"), "Toolbar");
        request_chrome_toggle(&window.ctx, egui::ViewportId::ROOT, ChromeToggle::ShowPanels);
        window.frame(vec![]);
        let app = &mut window.app;
        // The Tools panel shown again comes first.
        crate::toolbar::set_shown(app, true);
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
        bar(app, "Cutting", &[loose], false);
        assert_eq!(home(&app, &id), "Cutting");
    }

    /// Why Object › Ungroup can't run in `session`: the engine's reason.
    fn ungroup_reason(session: &Session) -> String {
        (vectorcraft_engine::find_command("object.ungroup").unwrap().enabled)(session).unwrap_err().to_string()
    }

    /// A Multi document with nothing selected, open in a window, and why Ungroup can't run in it.
    fn nothing_selected() -> (Window, String) {
        let mut session = Fixture::Multi.session();
        session.execute("select.none", &json!({})).unwrap();
        let why = ungroup_reason(&session);
        (Window::on(session), why)
    }

    /// The hint result `id`'s row shows while it can't run, or `None` while it can.
    fn hint_of(resolved: &ResolvedCommand) -> Option<String> {
        (!resolved.enabled).then(|| resolved.hint.as_deref().unwrap_or_default().to_string())
    }

    /// With nothing selected, Ungroup is listed dimmed with the engine's reason as its hint;
    /// selecting art shows it as available on the next frame.
    #[test]
    fn a_result_that_cannot_run_now_is_dimmed_and_says_why() {
        let (mut window, why) = nothing_selected();
        let hint = hint_of(&window.resolved("object.ungroup")).expect("Ungroup is dimmed");
        assert!(hint.contains(&why), "{hint}");
        window.app.run("select.all", json!({})).unwrap();
        window.frame(vec![]);
        assert_eq!(hint_of(&window.resolved("object.ungroup")), None, "Ungroup can run");
    }

    /// The footer of a highlighted or hovered result that can't run shows its row's hint: the
    /// engine's reason, naming the result as the palette shows it and never by its id.
    #[test]
    fn hovering_a_result_that_cannot_run_now_says_why() {
        let (window, why) = nothing_selected();
        let hint = hint_of(&window.resolved("object.ungroup")).expect("Ungroup is dimmed");
        assert!(hint.contains(&why) && !hint.contains("`object.ungroup`"), "{hint}");
    }

    /// Under an empty search, a recent pick that can't run now is listed dimmed with its hint, as
    /// the result row of the same command is.
    #[test]
    fn a_recent_pick_that_cannot_run_now_says_why() {
        let (mut window, why) = nothing_selected();
        window.app.ui.recent_commands = vec![CommandRef::new("object.ungroup"), CommandRef::new("object.group")];
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        let built = window.built();
        for command in &window.app.ui.recent_commands {
            let listed = built.entries.iter().find(|e| e.command() == command).expect("a recent pick has its entry");
            let hint = hint_of(&commands::resolve(&window.ctx, listed.command().clone()));
            assert!(hint.is_some(), "{} is dimmed", command.id);
        }
        assert!(hint_of(&window.resolved("object.ungroup")).is_some_and(|h| h.contains(&why)));
    }

    /// The recent picks list a command that can't run now dimmed, and one that can as it is.
    #[test]
    fn a_recent_command_that_cannot_run_now_is_dimmed_in_the_list() {
        let (mut window, _) = nothing_selected();
        window.app.ui.recent_commands = vec![CommandRef::new("object.ungroup"), CommandRef::new("select.all")];
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        assert!(!window.resolved("object.ungroup").enabled);
        let all = window.resolved("select.all");
        assert!(all.enabled && hint_of(&all).is_none());
    }

    /// Paste is dimmed while the clipboard is empty and nothing can be pasted from the system
    /// clipboard, and not while the system clipboard can be pasted.
    #[test]
    fn paste_is_dimmed_only_while_nothing_can_be_pasted() {
        let mut window = Window::on(Fixture::Multi.session());
        window.app.system_paste = false;
        window.frame(vec![]);
        let hint = hint_of(&window.resolved("edit.paste")).expect("Paste is dimmed");
        assert!(hint.contains("clipboard is empty"), "{hint}");
        window.app.system_paste = true;
        window.frame(vec![]);
        assert!(window.resolved("edit.paste").enabled);
    }

    /// With no document open, Paste is dimmed with the engine's reason, though the system
    /// clipboard still reads as pasteable until its next check.
    #[test]
    fn paste_is_dimmed_with_no_document_open() {
        let mut window = Window::on(Session::new());
        window.app.system_paste = true;
        window.frame(vec![]);
        let reason = (vectorcraft_engine::find_command("edit.paste").unwrap().enabled)(&window.app.session).unwrap_err().to_string();
        let hint = hint_of(&window.resolved("edit.paste")).expect("Paste is dimmed");
        assert!(hint.contains(&reason), "{hint}");
        assert!(!menus::enabled(&window.app, "edit.paste"));
    }

    /// A tool can always be chosen; a panel can't be shown in Presentation Mode, and says so.
    #[test]
    fn tools_are_never_dimmed_and_panels_are_in_presentation_mode() {
        let mut window = Window::new();
        let tools: Vec<String> = items().into_iter().map(|(_, id)| id).filter(|id| id.starts_with("tool:")).collect();
        assert!(!tools.is_empty());
        assert!(tools.iter().all(|id| window.resolved(id).enabled), "{tools:?}");
        assert!(window.resolved("panel:layers").enabled);
        window.app.ui.screen_mode = 3;
        // A command run moves the menus' state key, so the states are read again.
        window.app.run("select.none", json!({})).unwrap();
        window.frame(vec![]);
        let hint = hint_of(&window.resolved("panel:layers")).expect("panels are dimmed in Presentation Mode");
        assert!(hint.contains("Presentation Mode"), "{hint}");
    }

    /// A UI command without an engine reason says it is not available, and an effect asks about
    /// the command its pick runs.
    #[test]
    fn a_result_without_an_engine_reason_says_it_is_not_available() {
        assert!(vectorcraft_engine::find_command("ui.newBrush").is_none());
        let effect = crate::shortcut_editor::effect_items().first().map(|e| format!("effect:{}", e.effect)).expect("an Effect menu item");
        let window = Window::on(Session::new());
        for id in ["ui.newBrush", effect.as_str()] {
            let hint = hint_of(&window.resolved(id)).unwrap_or_else(|| panic!("{id} is dimmed"));
            assert!(hint.contains("not available"), "{id}: {hint}");
        }
        let window = Window::on(Fixture::Multi.session());
        for id in ["ui.newBrush", effect.as_str()] {
            assert!(window.resolved(id).enabled, "{id}");
        }
    }

    /// Every result's reason names it as the palette shows it: no command id appears in it.
    #[test]
    fn no_reason_shows_a_command_id() {
        let window = Window::on(Session::new());
        let reasons: Vec<(String, String)> = items().into_iter().filter_map(|(_, id)| hint_of(&window.resolved(&id)).map(|why| (id, why))).collect();
        assert!(reasons.len() > 100, "with no document most results can't run: {}", reasons.len());
        for (id, why) in &reasons {
            let command = crate::keymap::command_of(id).id;
            assert!(!why.contains(&format!("`{command}`")), "{id}: {why}");
        }
    }

    /// The sixteenth result of a query, past the rows in view, is reached with the keyboard; Enter
    /// runs it through the runner, which records it once in the recent picks and the counts.
    #[test]
    fn the_sixteenth_result_is_reached_by_keyboard_and_runs() {
        let mut window = Window::new();
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        let built = window.built();
        let query = "select";
        let order = rank(&built.index, query, &[], &|_| 0);
        assert!(order.len() > 16, "{query} lists more than 16 results");
        let sixteenth = built.entries[order[15]].command().clone();
        window.pick_typed(query, 15);
        assert_eq!(window.app.ui.recent_commands, [sixteenth.clone()], "{}", window.app.ui.status);
        assert_eq!(window.app.ui.command_uses.count_of(&sixteenth.key_text()), 1);
    }

    /// The queries the palette's tests use and a list of things people type.
    const QUERIES: &[&str] = &[
        "select",
        "zoom",
        "zoom in",
        "show panel",
        "group",
        "layers",
        "color guide",
        "drop shadow",
        "use precise cursors",
        "continue in background",
        "export continue in background",
        "show rulers",
        "delete selection",
        "cancel key object",
        "fit headline",
        "batch",
        "flattener preview",
        "panel options",
        "close path",
        "knife",
        "play action",
        "reset panel",
        "other library",
        "ellipse",
        "blur",
        "save",
        "export",
        "rect",
        "type",
        "ea",
        "bring front",
        "ruler",
        "deselect",
        "view.goto1",
        "zzzz",
        "pen",
        "outlines",
        "join paths",
        "rounded corners",
        "scaling",
        "rotated",
        "eyedroper",
        "rectange",
        "elipse",
        "gruop",
        "remove bg",
        "outline text",
        "flip",
        "combine",
        "vectorize",
        "colour",
        "transparent background",
        "png",
        "object.group",
        "file.exportinbackground",
        "window",
        "s",
        "e",
        "…",
        "› ",
    ];

    /// Queries from the list of things people type, on the English results: some list their command
    /// first, others within the first few results.
    #[test]
    fn common_queries_list_the_command_near_the_top() {
        let c = built_for(&app_on(Session::new()));
        let first = |q: &str| ranked(&c, q).first().cloned().unwrap_or_default();
        let place = |q: &str, id: &str| ranked(&c, q).iter().position(|r| r == id);
        let within = |q: &str, id: &str, n: usize| place(q, id).is_some_and(|p| p < n);
        assert_eq!(first("ea"), "file.exportAs", "the initials of Export As…");
        assert_eq!(first("save"), "file.save");
        assert!(within("export", "file.exportAs", 3));
        assert_eq!(first("rect"), "tool:rectangle");
        let (tool, effect) = (place("rect", "tool:rectangle"), place("rect", "effect:convertToShape.rectangle"));
        assert!(effect.is_some() && tool < effect, "{tool:?} {effect:?}");
        assert!(within("type", "tool:type", 2));
        assert!(within("layers", "panel:layers", 3));
        assert_eq!(first("drop shadow"), "effect:stylize.dropShadow");
        assert_eq!(first("view.goto1"), "view.goto1", "the whole id, above Saved View 10");
        assert_eq!(first("bring front"), "object.arrange.bringToFront");
        assert!(within("ruler", "view.rulers", 4));
        assert!(place("ruler", "perspective.grid.rulers").is_some());
        assert_eq!(first("deselect"), "select.none");
        let mut blurs: Vec<String> = ranked(&c, "blur").into_iter().take(3).collect();
        blurs.sort();
        assert_eq!(blurs, ["effect:blur.gaussian", "effect:blur.radial", "effect:blur.smart"]);
        assert!(ranked(&c, "zzzz").is_empty());
        // English names are searched in every language.
        crate::i18n::set_current(Lang::from_code("zh-hant").unwrap());
        let zh = built_for(&app_on_current(Session::new()));
        crate::i18n::set_current(Lang::EN);
        for q in ["save", "rect"] {
            assert_eq!(ranked(&zh, q).first(), ranked(&c, q).first(), "{q}");
        }
    }

    /// An app over `session` with its menu model built in the current language.
    fn app_on_current(session: Session) -> VectorcraftApp {
        let mut app = VectorcraftApp::new(session, crate::Services::default());
        crate::menus::model::refresh(&mut app, &crate::testing::ui_context());
        app
    }

    /// Each result's English name, shown label (in English, German and Chinese, Traditional) and id
    /// find it, first unless another result has that name; its id always lists it first.
    #[test]
    fn every_result_is_found_first_by_its_name_and_its_id() {
        for code in ["en", "de", "zh-hant"] {
            crate::i18n::set_current(Lang::from_code(code).unwrap());
            let c = built_for(&app_on_current(Session::new()));
            let plain = |n: &str| n.trim_end_matches('…').to_lowercase();
            for e in &c.entries {
                let english = e.english().map_or("", |l| l.as_str());
                for query in [english, e.label().as_str()] {
                    let results = ranked(&c, query);
                    assert!(results.iter().any(|r| r == id_of(e)), "{code}: {query} finds {}", id_of(e));
                    let named = c
                        .entries
                        .iter()
                        .filter(|o| plain(o.label().as_str()) == plain(query) || o.english().is_some_and(|n| plain(n.as_str()) == plain(query)))
                        .count();
                    if named <= 1 {
                        assert_eq!(results.first().map(String::as_str), Some(id_of(e)), "{code}: {query}");
                    }
                }
                assert_eq!(ranked(&c, id_of(e)).first().map(String::as_str), Some(id_of(e)), "{code}: {}", id_of(e));
            }
        }
        crate::i18n::set_current(Lang::EN);
    }

    /// For each query, every result the palette found by its words before is still found, and
    /// when it found no more than the palette lists, each of them is among the results it lists.
    #[test]
    fn every_result_found_before_is_still_found_and_shown() {
        use craft::ui::patterns::panels::RESULTS;
        assert_eq!(QUERIES.len(), 60);
        let c = built_for(&app_on(Session::new()));
        for q in QUERIES {
            let before = substring_rule(&c.entries, q);
            let now = ranked(&c, q);
            for id in &before {
                let at = now.iter().position(|r| r == id);
                assert!(at.is_some(), "{q}: {id} is still found");
                if before.len() <= RESULTS {
                    assert!(at < Some(RESULTS), "{q}: {id} is shown at {at:?}");
                }
            }
        }
    }

    /// The window's size in the frames of [`Window`].
    const WINDOW: egui::Vec2 = egui::vec2(1280.0, 900.0);

    /// The whole window, drawn a frame at a time in English with screen readers on.
    struct Window {
        app: VectorcraftApp,
        ctx: egui::Context,
        time: f64,
    }

    /// A press of `key` with no modifiers.
    fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }
    }

    impl Window {
        /// A document whose rectangle is selected.
        fn new() -> Window {
            let mut window = Window::on(vectorcraft_engine::Session::new());
            window.app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
            window.app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
            assert!(!window.app.session.doc().unwrap().selection.is_empty());
            window.frame(vec![]);
            window
        }

        /// The app over `session`, after two frames.
        fn on(session: Session) -> Window {
            crate::i18n::set_current(Lang::EN);
            let app = VectorcraftApp::new(session, crate::Services::default());
            let ctx = crate::testing::ui_context();
            ctx.enable_accesskit();
            let mut window = Window { app, ctx, time: 0.0 };
            window.frame(vec![]);
            window.frame(vec![]);
            window
        }

        /// One frame of the whole window with `events`: its screen-reader tree.
        fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::accesskit::Node> {
            self.time += 0.1;
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, WINDOW);
            let input = egui::RawInput { time: Some(self.time), screen_rect: Some(screen), events, ..Default::default() };
            let out = crate::testing::app_frame(&mut self.app, &self.ctx, input);
            out.platform_output.accesskit_update.map(|u| u.nodes.into_iter().map(|(_, n)| n).collect()).unwrap_or_default()
        }

        /// A primary click at `at`, a frame for the move, the press and the release.
        fn click(&mut self, at: egui::Pos2) {
            let button =
                |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
            self.frame(vec![egui::Event::PointerMoved(at)]);
            self.frame(vec![button(true)]);
            self.frame(vec![button(false)]);
        }

        /// Opens the palette afresh (closed for a frame, so no earlier query stays), types
        /// `query`, presses Down `down` times and Enter, a frame each, and draws two more frames,
        /// in which the app runs what was picked.
        fn pick_typed(&mut self, query: &str, down: usize) {
            self.app.ui.palette_open = false;
            self.frame(vec![]);
            self.app.ui.palette_open = true;
            self.frame(vec![]);
            self.frame(vec![egui::Event::Text(query.into())]);
            for _ in 0..down {
                self.frame(vec![key(egui::Key::ArrowDown)]);
            }
            self.frame(vec![key(egui::Key::Enter)]);
            self.frame(vec![]);
            self.frame(vec![]);
        }

        /// What the open palette was last built with.
        fn built(&self) -> Arc<Built> {
            self.app.canvas.palette.as_ref().map(|(_, built)| built.clone()).expect("the open palette is built")
        }

        /// Result `id`'s command as the palette's row shows it after the last frame.
        fn resolved(&self, id: &str) -> Arc<ResolvedCommand> {
            commands::resolve(&self.ctx, crate::keymap::command_of(id))
        }
    }

    /// The middle of the node whose screen-reader name is `name`.
    fn node_at(nodes: &[egui::accesskit::Node], name: &str) -> egui::Pos2 {
        let node = nodes.iter().find(|n| n.label().is_some_and(|l| l == name)).unwrap_or_else(|| panic!("a node named {name}"));
        let b = node.bounds().unwrap();
        egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32)
    }

    /// #7906: a modal window that opens without a press (a dialog from the macOS menu bar or an
    /// automation command, the About window) closes the open palette in that pass; the palette
    /// opened over a dialog stays closed; and one opened in the pass after the dialog closes stays
    /// open until the next dialog opens.
    #[test]
    fn a_dialog_opened_without_a_press_closes_the_palette() {
        for opens in ["object.move", "help.about"] {
            let mut window = Window::new();
            window.app.ui.palette_open = true;
            window.frame(vec![]);
            fire(&mut window.app, &window.ctx, CommandRef::new(opens), FireSource::Menu);
            assert!(window.app.any_dialog_open(), "{opens} opens its window: {}", window.app.ui.status);
            window.frame(vec![]);
            assert!(!window.app.ui.palette_open, "{opens}: the window closes the palette");
            toggle(&mut window.app);
            window.frame(vec![]);
            window.frame(vec![]);
            assert!(!window.app.ui.palette_open, "{opens}: the window keeps the palette closed");
            assert!(window.app.any_dialog_open(), "{opens}: the window stays open");
            close_dialogs(&mut window.app, false);
            window.app.ui.palette_open = true;
            window.frame(vec![]);
            assert!(window.app.ui.palette_open, "{opens}: the palette opened after the window closed stays open");
            fire(&mut window.app, &window.ctx, CommandRef::new("object.move"), FireSource::Menu);
            window.frame(vec![]);
            assert!(!window.app.ui.palette_open, "{opens}: the next dialog closes the palette");
        }
    }

    /// Opens `overlay` ("drawer" or "menu") in a new window, then opens Move… without a press, as
    /// the macOS menu bar or an automation command does: whether the overlay is still open.
    fn overlay_open_after_a_dialog_opens(overlay: &str) -> bool {
        let mut window = Window::new();
        let popup = match overlay {
            "drawer" => {
                window.app.ui.tools_drawer = true;
                None
            }
            _ => {
                let nodes = window.frame(vec![]);
                window.click(node_at(&nodes, "Object"));
                overlay::top(&window.ctx)
            }
        };
        window.frame(vec![]);
        let open = |w: &Window| w.app.ui.tools_drawer || popup.is_some_and(|id| overlay::is_open(&w.ctx, id));
        assert!(open(&window), "the {overlay} is open");
        fire(&mut window.app, &window.ctx, CommandRef::new("object.move"), FireSource::Menu);
        assert!(window.app.any_dialog_open(), "Move… opens its dialog: {}", window.app.ui.status);
        window.frame(vec![]);
        open(&window)
    }

    /// #7906: a dialog that opens without a press closes the All Tools drawer and an open menu.
    #[test]
    fn a_dialog_opened_without_a_press_closes_a_flyout_the_drawer_and_a_menu() {
        // PLAN: no flyout row: its opener is a chrome-w10 toolbar control this set cannot name; `close_overlays_for_modal` closes it with the same `close_popups` call as the menu
        for overlay in ["drawer", "menu"] {
            assert!(!overlay_open_after_a_dialog_opens(overlay), "the dialog closes the {overlay}");
        }
    }

    /// Count trial: the entries and their index are built when the palette first opens, kept while
    /// it is typed in, picked from, closed, opened again and while a state the catalog does not
    /// read changes, and built again when the catalog's generation moves (the language here).
    #[test]
    fn the_entries_are_built_again_only_when_the_catalog_or_the_language_changes() {
        let mut window = Window::new();
        assert!(window.app.canvas.palette.is_none(), "nothing is built while the palette is closed");
        let built = |w: &Window| w.app.canvas.palette.as_ref().map(|(key, b)| (*key, b.index.clone())).expect("the open palette is built");
        let mut seen = vec![];
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        seen.push(built(&window));
        window.frame(vec![egui::Event::Text("group".into())]);
        seen.push(built(&window));
        window.frame(vec![key(egui::Key::Enter)]);
        window.frame(vec![]);
        seen.push(built(&window));
        window.app.run("select.none", json!({})).unwrap();
        window.app.ui.palette_open = true;
        window.frame(vec![]);
        seen.push(built(&window));
        let english = seen.last().map(|(key, _)| *key);
        crate::i18n::set_current(Lang::from_code("de").unwrap());
        window.frame(vec![]);
        window.frame(vec![]);
        seen.push(built(&window));
        crate::i18n::set_current(Lang::EN);
        assert_ne!(seen.last().map(|(key, _)| *key), english, "the language moves the catalog's generation");
        // Another test may install a plug-in meanwhile, which moves the key too: the rule is a
        // build exactly when the key moves.
        for pair in seen.windows(2) {
            let [(k0, i0), (k1, i1)] = pair else { continue };
            assert_eq!(!Arc::ptr_eq(i0, i1), k0 != k1, "{k0:?} → {k1:?}");
        }
        let builds = seen.windows(2).filter(|p| !Arc::ptr_eq(&p[0].1, &p[1].1)).count();
        assert!(builds >= 1 && builds < seen.len() - 1, "{builds} builds over {} opens and edits", seen.len());
    }
}
