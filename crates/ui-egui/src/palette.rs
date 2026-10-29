//! The command palette: fuzzy search over every command and tool.

use std::sync::Arc;

use serde_json::json;

use crate::theme::Tokens;
use crate::{VectorcraftApp, menus};

/// Everything the palette searches: (label, command id or `tool:<id>`, shortcut).
pub fn items() -> Vec<(String, String, String)> {
    let mut items = vec![];
    // Aliases kept for older scripts run a command that is listed already.
    for c in vectorcraft_engine::command_specs().iter().filter(|c| !vectorcraft_engine::cmd::is_alias(c.id)) {
        let path = c.menu.join(" › ");
        let label = if path.is_empty() { c.label.to_string() } else { format!("{path} › {}", c.label) };
        items.push((label, c.id.to_string(), menus::shortcut_of(c.id).unwrap_or("").to_string()));
    }
    for c in menus::UI_COMMANDS {
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
struct Entry {
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
    let built: Arc<Vec<Entry>> = Arc::new(
        items()
            .into_iter()
            .map(|(label, id, shortcut)| {
                let shown = shown_label(lang, &label);
                let haystack = format!("{}\n{}\n{}", label.to_lowercase(), shown.to_lowercase(), id.to_lowercase());
                Entry { id, shortcut, shown, haystack }
            })
            .collect(),
    );
    ctx.data_mut(|d| d.insert_temp::<Cached>(id, (key, built.clone())));
    built
}

pub fn show(app: &mut VectorcraftApp, ctx: &egui::Context) {
    if !app.ui.palette_open {
        return;
    }
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
                        .shortcut_text(menus::pretty_shortcut(&e.shortcut))
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
        if let Some(tool) = id.strip_prefix("tool:") {
            app.select_tool(tool);
        } else {
            menus::invoke(app, &id, json!({}));
        }
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
}
