//! The command palette: fuzzy search over the commands a pick runs, the tools, the panels and the
//! Effect menu's items.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
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
}

/// The palette's entries in `lang`. They are built when the language, the installed plug-ins or
/// the keyboard shortcuts (shown beside each entry) change, not on every frame while a query is
/// typed.
/// What the entries depend on: the language, the installed plug-ins and the shortcuts.
fn cache_key(lang: crate::i18n::Lang) -> (&'static str, u64, u64) {
    (lang.code(), menus::plugin_revision(), crate::shortcut_editor::GENERATION.load(std::sync::atomic::Ordering::Relaxed))
}

fn entries(ctx: &egui::Context, lang: crate::i18n::Lang) -> Arc<Vec<Entry>> {
    type Cached = ((&'static str, u64, u64), Arc<Vec<Entry>>);
    let key = cache_key(lang);
    let id = egui::Id::unique("palette-entries");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<Cached>(id))
        && k == key
    {
        return v;
    }
    let built: Arc<Vec<Entry>> = Arc::new(build(lang));
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (key, built.clone())));
    built
}

/// The palette's entries in `lang`: [`items`], then one per filter plug-in's Object › Plug-ins
/// item (`plugin:<id>`), whose name is not translated, each with a name no result for another id
/// shows, in English and in `lang` ([`distinct_names`]). A query still matches the name before
/// it was made distinct.
fn build(lang: crate::i18n::Lang) -> Vec<Entry> {
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
            Entry { id: d.id, shortcut: d.shortcut, shown, haystack }
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
        // a floating panel away.
        if let Err(e) = crate::panel_docking::reveal_panel(app, panel) {
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

/// The results the palette shows for the lowercased query `q`: the first 14 entries that hold every
/// word of it, the ones whose name without its menu path is the query (the Layers panel for
/// "layers") first, each group in catalog order.
fn matching<'a>(entries: &'a [Entry], q: &str) -> Vec<&'a Entry> {
    let words: Vec<&str> = q.split_whitespace().collect();
    let whole = words.join(" ");
    // The plain, English and shown names are the haystack's first three lines.
    let named = |e: &Entry| e.haystack.lines().take(3).any(|l| l.rsplit(" › ").next().is_some_and(|n| n.trim_end_matches('…') == whole));
    let mut found: Vec<&Entry> = entries.iter().filter(|e| words.iter().all(|w| e.haystack.contains(w))).collect();
    if !whole.is_empty() {
        found.sort_by_key(|e| !named(e));
    }
    found.truncate(14);
    found
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !app.ui.palette_open {
        return;
    }
    crate::shortcuts::hold_back_keys(app, ctx);
    let t = Tokens::get(ctx);
    let q = app.ui.palette_query.to_lowercase();
    let entries = entries(ctx, crate::i18n::current());
    let matches = matching(&entries, &q);
    let mut run: Option<String> = None;
    // The highlighted result (#1757): Up and Down move it (wrapping), the pointer over a result
    // takes it, and a new query starts again from the first.
    let sel_id = egui::Id::unique("palette-selected");
    let (mut sel, last_q) = ctx.data(|d| d.get_temp::<(usize, String)>(sel_id)).unwrap_or_default();
    if last_q != q {
        sel = 0;
    }
    let n = matches.len();
    if n > 0 {
        let (down, up) =
            ctx.input_mut(|i| (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp)));
        if down {
            sel = (sel + 1) % n;
        }
        if up {
            sel = (sel + n - 1) % n;
        }
        sel = sel.min(n - 1);
    }
    egui::Area::new(egui::Id::unique("palette")).order(egui::Order::Foreground).anchor(egui::Align2::CENTER_TOP, [0.0, 90.0]).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(t.panel).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(520.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut app.ui.palette_query)
                    .align(egui::Align2::LEFT_CENTER)
                    .hint_text(tl!("Search commands, tools, panels and effects…"))
                    .desired_width(500.0),
            );
            r.request_focus();
            ui.add_space(6.0);
            for (i, e) in matches.iter().enumerate() {
                let resp = ui.add(
                    egui::Button::new(e.shown.as_str())
                        .shortcut_text(menus::shown_shortcut(&e.shortcut))
                        .min_size(egui::vec2(500.0, 24.0))
                        .selected(i == sel),
                );
                if resp.hovered() && ui.input(|i| i.pointer.delta() != egui::Vec2::ZERO) {
                    sel = i;
                }
                if resp.clicked() {
                    run = Some(e.id.clone());
                }
            }
            if ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(e) = matches.get(sel)
            {
                run = Some(e.id.clone());
            }
        });
    });
    ctx.data_mut(|d| d.insert_temp(sel_id, (sel, q)));
    if let Some(id) = run {
        app.ui.palette_open = false;
        run_pick(app, &id);
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
            ctx.data(|d| d.get_temp::<(usize, String)>(egui::Id::unique("palette-selected"))).unwrap().0
        };
        assert_eq!(frame(&mut app, vec![]), 0);
        assert_eq!(frame(&mut app, vec![key(egui::Key::ArrowDown)]), 1);
        assert_eq!(frame(&mut app, vec![key(egui::Key::ArrowUp)]), 0);
        let last = frame(&mut app, vec![key(egui::Key::ArrowUp)]);
        assert!(last > 1, "Up on the first result wraps to the last: {last}");
        app.ui.palette_query = "zoom in".into();
        assert_eq!(frame(&mut app, vec![]), 0);
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
        let entries = build(Lang::EN);
        let panels: Vec<&Entry> = entries.iter().filter(|e| e.id.starts_with("panel:")).collect();
        assert_eq!(panels.len(), crate::state::all_panels().count());
        // More than 14 results hold "layers" in their menu path; the panel, named Layers, is shown
        // first, and "color guide" shows the Color Guide panel first too.
        for (query, want) in [("layers", "panel:layers"), ("color guide", "panel:colorGuide"), ("Layers ", "panel:layers")] {
            let all = entries.iter().filter(|e| query.to_lowercase().split_whitespace().all(|w| e.haystack.contains(w))).count();
            let shown: Vec<&str> = matching(&entries, &query.to_lowercase()).into_iter().map(|e| e.id.as_str()).collect();
            assert_eq!(shown.first(), Some(&want), "{query}: {shown:?}");
            assert_eq!(shown.len(), all.min(14), "{query}");
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
}
