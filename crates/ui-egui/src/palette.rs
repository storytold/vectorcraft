//! The command palette: fuzzy search over every command and tool.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
use crate::{VectorcraftApp, menus};

/// Engine commands the palette leaves out, each with the UI command of the same name that runs it
/// the way the panels and menus do: (engine id, UI command id). The Layers panel's Delete
/// Selection asks before deleting a layer that holds art, and the other UI commands open the
/// command's dialog or file chooser. Scripts and the control channel still run the engine ids.
const WRAPPED: &[(&str, &str)] = &[
    ("layer.delete", "ui.layerDelete"),
    ("brush.new", "ui.newBrush"),
    ("brush.options", "ui.brushOptions"),
    ("symbol.options", "ui.symbolOptions"),
    ("svgFilter.import", "ui.importSvgFilter"),
    ("text.convertListToText", "type.convertListToText"),
];

/// Everything the palette searches: (label, command id or `tool:<id>`, shortcut).
pub fn items() -> Vec<(String, String, String)> {
    let mut items = vec![];
    // Aliases kept for older scripts run a command that is listed already, and a wrapped engine
    // command is listed as its UI command.
    let plugin_commands = vectorcraft_engine::cmd::plugin::command_specs();
    let listed = |id: &str| !vectorcraft_engine::cmd::is_alias(id) && !WRAPPED.iter().any(|(engine, _)| *engine == id);
    let labeled = |menu: &[&str], label: &str| {
        let path = menu.join(" › ");
        if path.is_empty() { label.to_string() } else { format!("{path} › {label}") }
    };
    for c in vectorcraft_engine::command_specs().iter().filter(|c| listed(c.id)).chain(plugin_commands) {
        items.push((labeled(c.menu, c.label), c.id.to_string(), menus::shortcut_of(c.id).unwrap_or("").to_string()));
    }
    // `plugin.dialog` runs nothing without an id: [`build`] lists each filter instead. A script of
    // the Scripts folder runs from File › Scripts, by its path. A UI command that wraps an engine
    // command takes that command's menu path, so a search for the path still finds it.
    for c in menus::UI_COMMANDS.iter().filter(|c| c.0 != "plugin.dialog" && c.0 != crate::scripts_menu::COMMAND) {
        let engine = WRAPPED.iter().find(|(_, ui)| *ui == c.0).and_then(|(engine, _)| vectorcraft_engine::find_command(engine));
        let label = engine.map_or_else(|| c.1.to_string(), |e| labeled(e.menu, c.1));
        items.push((label, c.0.to_string(), menus::shortcut_of(c.0).unwrap_or("").to_string()));
    }
    for tool in vectorcraft_tools::catalog::all_tools() {
        items.push((tool.label.to_string(), format!("tool:{}", tool.id), crate::shortcut_editor::tool_shortcut(tool.id).unwrap_or("").to_string()));
    }
    items
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
    /// The English label (what agents document), the shown label and the id (and the id of the
    /// engine command a UI command wraps), lowercased, one per line: a query word (which has no
    /// line break) matches within one of them.
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
/// item (`plugin:<id>`), whose name is not translated.
fn build(lang: crate::i18n::Lang) -> Vec<Entry> {
    let entry = |label: String, shown: String, id: String, shortcut: String| {
        let mut haystack = format!("{}\n{}\n{}", label.to_lowercase(), shown.to_lowercase(), id.to_lowercase());
        // A UI command that wraps an engine command is also found by the engine command's id.
        if let Some((engine, _)) = WRAPPED.iter().find(|(_, ui)| *ui == id) {
            haystack.push('\n');
            haystack.push_str(&engine.to_lowercase());
        }
        Entry { id, shortcut, shown, haystack }
    };
    let mut out: Vec<Entry> = items()
        .into_iter()
        .map(|(label, id, shortcut)| {
            let shown = shown_label(lang, &label);
            entry(label, shown, id, shortcut)
        })
        .collect();
    out.extend(crate::shortcut_editor::filter_items().iter().map(|f| {
        let shortcut = crate::shortcut_editor::filter_shortcut(&f.id).unwrap_or("").to_string();
        let label = format!("{} › {}", f.group, f.label);
        let shown = format!("{} › {}", shown_label(lang, &f.group), f.label);
        entry(label, shown, format!("plugin:{}", f.id), shortcut)
    }));
    out
}

/// Runs the palette result `id`: selects a tool, or invokes a command with the params its entry
/// stands for. A command whose menu bar item gives params runs what a click on that item runs
/// ([`crate::shortcut_editor::menu_target_now`]), so Clean Up… and Find and Replace… open their
/// dialogs. While that item is dimmed, no dialog opens and the status line says why the command
/// can't run.
pub(crate) fn run_pick(app: &mut VectorcraftApp, id: &str) {
    if let Some(tool) = id.strip_prefix("tool:") {
        app.select_tool(tool);
    } else if id.starts_with("plugin:") {
        let command = crate::keymap::command_of(id);
        menus::invoke(app, &command.id, command.params);
    } else {
        match crate::shortcut_editor::menu_target_now(app, id) {
            Some(Ok((id, p))) => menus::invoke(app, &id, p),
            Some(Err(why)) => app.status(why),
            None => menus::invoke(app, id, json!({})),
        }
    }
}

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
    if !app.ui.palette_open {
        return;
    }
    crate::shortcuts::hold_back_keys(app, ctx);
    let t = Tokens::get(ctx);
    let q = app.ui.palette_query.to_lowercase();
    let entries = entries(ctx, crate::i18n::current());
    let matches: Vec<&Entry> = entries.iter().filter(|e| q.split_whitespace().all(|w| e.haystack.contains(w))).take(14).collect();
    let recent = recent(&entries, &app.ui.recent_commands);
    // While the Recent Commands list is open, the arrow keys and Enter go to its rows, and the
    // search field leaves them the keyboard.
    let list_open = recent_list_open(ctx);
    let mut run: Option<String> = None;
    let mut history_focused = false;
    // The highlighted result (#1757): Up and Down move it (wrapping), the pointer over a result
    // takes it, and a new query starts again from the first.
    let sel_id = egui::Id::unique("palette-selected");
    let (mut sel, last_q) = ctx.data(|d| d.get_temp::<(usize, String)>(sel_id)).unwrap_or_default();
    if last_q != q {
        sel = 0;
    }
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
    if n > 0 && !list_open {
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
            ui.horizontal(|ui| {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut app.ui.palette_query)
                        .align(egui::Align2::LEFT_CENTER)
                        .hint_text(tl!("Search commands and tools…"))
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
            });
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
            if !list_open
                && !history_focused
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
                && let Some(e) = matches.get(sel)
            {
                run = Some(e.id.clone());
            }
        });
    });
    ctx.data_mut(|d| {
        d.insert_temp(sel_id, (sel, q));
        d.insert_temp(rsel_id, rsel);
    });
    if let Some(id) = run {
        app.ui.palette_open = false;
        remember(&mut app.ui.recent_commands, &id, |c| entries.iter().any(|e| e.id == c));
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
        let matches: Vec<&Entry> = entries.iter().filter(|e| query.split_whitespace().all(|word| e.haystack.contains(word))).take(14).collect();
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
}
