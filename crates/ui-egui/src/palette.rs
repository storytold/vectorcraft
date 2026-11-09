//! The command palette: fuzzy search over every command and tool.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
use crate::{VectorcraftApp, menus};

/// Everything the palette searches: (label, command id or `tool:<id>`, shortcut).
pub fn items() -> Vec<(String, String, String)> {
    let mut items = vec![];
    // Aliases kept for older scripts run a command that is listed already.
    let plugin_commands = vectorcraft_engine::cmd::plugin::command_specs();
    for c in vectorcraft_engine::command_specs().iter().filter(|c| !vectorcraft_engine::cmd::is_alias(c.id)).chain(plugin_commands) {
        let path = c.menu.join(" › ");
        let label = if path.is_empty() { c.label.to_string() } else { format!("{path} › {}", c.label) };
        items.push((label, c.id.to_string(), menus::shortcut_of(c.id).unwrap_or("").to_string()));
    }
    // `plugin.dialog` runs nothing without an id: [`build`] lists each filter instead.
    for c in menus::UI_COMMANDS.iter().filter(|c| c.0 != "plugin.dialog") {
        items.push((c.1.to_string(), c.0.to_string(), menus::shortcut_of(c.0).unwrap_or("").to_string()));
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
    /// The English label (what agents document), the shown label and the id, lowercased, one per
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
/// item (`plugin:<id>`), whose name is not translated.
fn build(lang: crate::i18n::Lang) -> Vec<Entry> {
    let entry = |label: String, shown: String, id: String, shortcut: String| {
        let haystack = format!("{}\n{}\n{}", label.to_lowercase(), shown.to_lowercase(), id.to_lowercase());
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
/// stands for.
pub(crate) fn run_pick(app: &mut VectorcraftApp, id: &str) {
    if let Some(tool) = id.strip_prefix("tool:") {
        app.select_tool(tool);
    } else if id.starts_with("plugin:") {
        let command = crate::keymap::command_of(id);
        menus::invoke(app, &command.id, command.params);
    } else {
        menus::invoke(app, id, json!({}));
    }
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
                    .hint_text(tl!("Search commands and tools…"))
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
        let matches: Vec<&Entry> = entries.iter().filter(|e| query.split_whitespace().all(|word| e.haystack.contains(word))).take(14).collect();
        assert!(matches.iter().any(|e| e.id == "view.preciseCursors"), "preference toggle appears for its name");
        assert!(entries.iter().filter(|e| e.id == "view.capsLock").all(|e| e.shown != "Use Precise Cursors"));

        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), crate::Services::default());
        let before = app.session.prefs.use_precise_cursors;
        let first = matches.first().expect("query has a result");
        menus::invoke(&mut app, &first.id, json!({}));
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
        // Run a pick the way `show` runs it.
        menus::invoke(&mut app, ids[0], json!({}));
        let st = app.session.active().unwrap();
        assert_eq!((st.selection.len(), st.selection.key), (2, None), "both objects stay selected, without a key");
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
            // Run a pick the way `show` runs it.
            menus::invoke(&mut app, id, json!({}));
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: {}", app.ui.status);
            assert!(!app.ui.status.contains("invalid parameters"), "{id}: {}", app.ui.status);
            // A second pick leaves the open panel open, docked or floating.
            menus::invoke(&mut app, id, json!({}));
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: picked twice");
            app.run("window.panel.float", json!({"panel": crate::panels::asset_export::ID})).unwrap();
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: floating");
            menus::invoke(&mut app, id, json!({}));
            assert!(crate::panel_docking::showing(&app.ui, crate::panels::asset_export::ID), "{id}: floating, picked again");
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
