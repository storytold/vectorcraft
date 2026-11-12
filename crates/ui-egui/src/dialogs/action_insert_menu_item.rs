//! Actions panel menu › Insert Menu Item… (`ui.action.insertMenuItem`): the menu items that can be
//! steps of an action, to pick one from the list or find it by name.
//!
//! Its fields: `find` (the words the list is narrowed to), `command` and `params` (the item picked;
//! `command` is empty until one is), `note` (`none` or `several` when Find picked nothing), and
//! `set`, `action` and `step`, the place of the new step (no `set` and `action`: the recording in
//! progress). Picking an item runs nothing. OK inserts it; an invalid place is an error, and the
//! dialog stays open.

use std::sync::Arc;

use serde_json::{Value, json};

use super::DialogSpec;
use crate::state::Dialog;
use crate::theme::Tokens;
use crate::{VectorcraftApp, menus, widgets};

/// The dialog kind of Insert Menu Item.
pub const KIND: &str = "actionInsertMenuItem";

/// The most rows the list shows at once; Find narrows it.
const MAX_LISTED: usize = 200;

/// The width of the Menu Item label.
const LABEL_W: f32 = 80.0;

pub(super) const SPEC: DialogSpec = DialogSpec {
    heading: |_| tl!("Insert Menu Item").into(),
    body,
    confirm,
    ok_enabled: Some(|_, d| !d.str("command").is_empty()),
    min_width: 460.0,
    ..DialogSpec::FORM
};

/// Opens Insert Menu Item for action `target` as (set, action), after `step`, else for the recording
/// in progress.
pub fn open(app: &mut VectorcraftApp, target: Option<(usize, usize)>, step: Option<usize>) {
    let mut fields = json!({"find": "", "command": "", "params": {}, "note": ""});
    if let Some((set, action)) = target {
        fields["set"] = json!(set);
        fields["action"] = json!(action);
        if let Some(step) = step {
            fields["step"] = json!(step);
        }
    }
    app.ui.dialog = Some(Dialog::new(KIND, fields));
}

/// A row of the list: an item of [`menus::insertable_items`], with what it shows and what Find
/// matches worked out once.
pub(crate) struct Row {
    command: &'static str,
    params: Value,
    /// The menu path and label in the UI language (`Object › Group`).
    shown: String,
    /// The English path and label, the shown one and the command id, lowercased, one per line.
    haystack: String,
}

/// The rows in `lang`, for menus that name fonts in English when `english_names` is set.
fn build(lang: crate::i18n::Lang, english_names: bool) -> Vec<Row> {
    menus::insertable_items(english_names)
        .into_iter()
        .map(|item| {
            let parts = || item.path.iter().chain(std::iter::once(&item.label)).copied();
            let english = parts().collect::<Vec<_>>().join(" › ");
            let shown = parts().map(|s| crate::i18n::tr(lang, s)).collect::<Vec<_>>().join(" › ");
            let haystack = format!("{}\n{}\n{}", english.to_lowercase(), shown.to_lowercase(), item.command.to_lowercase());
            Row { command: item.command, params: item.params, shown, haystack }
        })
        .collect()
}

/// The rows, built again only when the language, the installed plug-ins, the font name setting or
/// the loaded fonts change, not on every frame.
fn rows(app: &VectorcraftApp, ctx: &egui::Context) -> Arc<Vec<Row>> {
    type Key = (&'static str, u64, bool, u64, bool);
    let lang = crate::i18n::current();
    let english_names = app.session.prefs.font_names_in_english;
    let db = vectorcraft_text::FontDb::global();
    // Read before building, as the Font menu does: fonts that load meanwhile build the rows again.
    let (cataloged, generation) = (db.installed_fonts_ready(), db.generation());
    let key: Key = (lang.code(), menus::plugin_revision(), english_names, generation, cataloged);
    let id = egui::Id::unique("action-insert-menu-item-rows");
    if let Some((k, v)) = ctx.data(|d| d.get_temp::<(Key, Arc<Vec<Row>>)>(id))
        && k == key
    {
        return v;
    }
    let built = Arc::new(build(lang, english_names));
    ctx.data_mut(|d| d.insert_temp::<(Key, Arc<Vec<Row>>)>(id, (key, built.clone())));
    built
}

/// The rows that every word of `find` matches, in menu order.
fn matches<'a>(rows: &'a [Row], find: &str) -> impl Iterator<Item = &'a Row> {
    let words: Vec<String> = find.to_lowercase().split_whitespace().map(str::to_string).collect();
    rows.iter().filter(move |r| words.iter().all(|w| r.haystack.contains(w.as_str())))
}

/// Picks `row` in `d`.
fn pick(d: &mut Dialog, row: &Row) {
    d.fields.insert("command".into(), json!(row.command));
    d.fields.insert("params".into(), row.params.clone());
    d.fields.insert("note".into(), json!(""));
}

/// Find: picks the one row that `find` matches; with none or several, picks nothing and notes it.
fn find(rows: &[Row], d: &mut Dialog) {
    let found: Vec<&Row> = matches(rows, &d.str("find")).take(2).collect();
    match found[..] {
        [row] => pick(d, row),
        [] => {
            d.fields.insert("note".into(), json!("none"));
        }
        _ => {
            d.fields.insert("note".into(), json!("several"));
        }
    }
}

/// Whether `row` is the item picked in `d`.
fn picked(d: &Dialog, row: &Row) -> bool {
    d.fields.get("command").and_then(Value::as_str) == Some(row.command) && d.fields.get("params") == Some(&row.params)
}

fn body(app: &mut VectorcraftApp, ui: &mut egui::Ui, d: &mut Dialog) -> bool {
    let rows = rows(app, ui.ctx());
    let dim = Tokens::get(ui.ctx()).text_dim;
    let mut find_now = false;
    ui.horizontal(|ui| {
        if super::form::text_named(ui, d, "find", tl!("Find"), 300.0) {
            d.fields.insert("note".into(), json!(""));
        }
        find_now = widgets::secondary_button(ui, tl!("Find")).clicked();
    });
    if find_now {
        find(&rows, d);
    }
    match d.str("note").as_str() {
        "none" => {
            ui.label(egui::RichText::new(tl!("No menu item matches.")).color(dim));
        }
        "several" => {
            ui.label(egui::RichText::new(tl!("Several menu items match. Choose one from the list.")).color(dim));
        }
        _ => {}
    }
    ui.add_space(4.0);
    let mut chosen: Option<usize> = None;
    let query = d.str("find");
    egui::ScrollArea::vertical().max_height(260.0).auto_shrink([false, true]).show(ui, |ui| {
        for (i, row) in matches(&rows, &query).enumerate().take(MAX_LISTED) {
            if ui.selectable_label(picked(d, row), row.shown.as_str()).clicked() {
                chosen = Some(i);
            }
        }
    });
    if let Some(row) = chosen.and_then(|i| matches(&rows, &query).nth(i)) {
        pick(d, row);
    }
    ui.add_space(4.0);
    let shown = rows.iter().find(|r| picked(d, r)).map_or_else(
        || {
            let command = d.str("command");
            if command.is_empty() { tl!("None").to_string() } else { command }
        },
        |r| r.shown.clone(),
    );
    widgets::label_row(ui, "Menu Item:", LABEL_W, |ui| {
        ui.label(shown);
    });
    ui.label(egui::RichText::new(tl!("Items that open a dialog, and File, Window and Help menu items, are not listed.")).color(dim));
    false
}

fn confirm(app: &mut VectorcraftApp, d: &Dialog) -> Result<Value, String> {
    let mut params = json!({"command": d.str("command"), "params": d.fields.get("params").cloned().unwrap_or_else(|| json!({}))});
    for key in ["set", "action", "step"] {
        if let Some(value) = d.fields.get(key) {
            params[key] = value.clone();
        }
    }
    let r = app.run(crate::panels::actions::INSERT_MENU_ITEM, params)?;
    app.ui.dialog = None;
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::Lang;

    #[test]
    fn find_picks_the_one_match_and_notes_none_or_several() {
        let rows = build(Lang::EN, true);
        let mut d = Dialog::new(KIND, json!({"find": "object group", "command": "", "params": {}, "note": ""}));
        find(&rows, &mut d);
        assert_eq!((d.str("note"), d.str("command")), ("several".to_string(), String::new()), "Group and Ungroup both match");
        d.fields.insert("find".into(), json!("OBJECT.UNGROUP"));
        find(&rows, &mut d);
        assert_eq!((d.str("command"), d.fields["params"].clone(), d.str("note")), ("object.ungroup".to_string(), json!({}), String::new()));
        d.fields.insert("find".into(), json!("no such menu item here"));
        find(&rows, &mut d);
        assert_eq!((d.str("note"), d.str("command")), ("none".to_string(), "object.ungroup".to_string()), "a failed Find keeps the pick");
    }

    /// The list holds the menu items that run an engine command without asking for values first,
    /// outside the application, File, Window and Help menus, and leaves out Undo, Redo, the Pastes,
    /// Object › Align, Image Trace's Make and Make and Expand, Pattern's Make and Edit Pattern and the
    /// saved-selection, saved-view and recent-font items.
    #[test]
    fn the_list_holds_only_menu_items_that_can_be_steps() {
        let items = menus::insertable_items(true);
        assert!(items.len() > 100, "only {} items", items.len());
        for item in &items {
            assert!(vectorcraft_engine::find_command(item.command).is_some(), "{item:?}");
            assert!(!item.label.ends_with('…'), "{item:?}");
            assert!(!["VectorCraft", "File", "Window", "Help"].contains(&item.path[0]), "{item:?}");
            assert!(item.params.is_object(), "{item:?}");
        }
        let has = |command: &str| items.iter().any(|i| i.command == command);
        for command in ["object.group", "object.ungroup", "object.arrange.bringToFront"] {
            assert!(has(command), "{command}");
        }
        for command in [
            "edit.undo",
            "edit.redo",
            "edit.paste",
            "edit.pasteInFront",
            "object.move",
            "object.align",
            "imageTrace.make",
            "imageTrace.makeAndExpand",
            "object.pattern.make",
            "object.pattern.edit",
            "file.save",
            "view.zoomIn",
            "select.recall1",
        ] {
            assert!(!has(command), "{command}");
        }
    }

    /// Rows show their menu path in the UI language, and Find matches the English path, the shown
    /// one and the command id.
    #[test]
    fn rows_show_the_translated_path_and_match_english_shown_text_and_ids() {
        let de = Lang::from_code("de").unwrap();
        let rows = build(de, true);
        let group = rows.iter().find(|r| r.command == "object.group").unwrap();
        assert_eq!(group.shown, format!("{} › {}", crate::i18n::tr(de, "Object"), crate::i18n::tr(de, "Group")));
        for words in ["object group", group.shown.as_str(), "object.group"] {
            assert!(matches(&rows, words).any(|r| r.command == "object.group"), "{words}");
        }
        let shared = rows.iter().any(|r| rows.iter().any(|o| o.command == r.command && o.params != r.params));
        assert!(shared, "items that share a command are listed apart, by their params");
    }
}
