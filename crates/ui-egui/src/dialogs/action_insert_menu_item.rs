//! Actions panel menu › Insert Menu Item… (`ui.action.insertMenuItem`): the menu items that can be
//! steps of an action, to pick one from the list or find it by name.
//!
//! Its fields: `find` (the words the list is narrowed to), `command` and `params` (the item picked;
//! `command` is empty until one is), `note` (`none` or `several` when Find picked nothing), and
//! `set`, `action` and `step`, the place of the new step (no `set` and `action`: the recording in
//! progress). Picking an item runs nothing. OK inserts it; an invalid place is an error, and the
//! dialog stays open.

use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _, Sizing, clicked};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::layout_components::Row as Line;
use craft::ui::list::{List, ListRow};
use craft::ui::primitives::Revision;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::{ActionDialog, Form, Target, label, merged, open_or_say};
use crate::i18n::{Lang, LangExt};
use crate::panels::actions::INSERT_MENU_ITEM;
use crate::{VectorcraftApp, menus};

/// The key the dialog answers for.
const KEY: &str = "actionInsertMenuItem";

/// The most rows the list shows at once; Find narrows it.
const MAX_LISTED: usize = 200;

/// Why Find picked nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum FindNote {
    /// Nothing to say.
    #[default]
    #[serde(rename = "")]
    Clear,
    /// No menu item matches.
    #[serde(rename = "none")]
    NoMatch,
    /// Several match: the user picks one from the list.
    #[serde(rename = "several")]
    Several,
}

/// What Insert Menu Item edits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InsertMenuItemFields {
    /// The words the list is narrowed to.
    pub find: String,
    /// The command of the item picked; empty until one is.
    pub command: String,
    /// The params of the item picked.
    pub params: Value,
    /// Why the last Find picked nothing.
    pub note: FindNote,
}

impl Default for InsertMenuItemFields {
    fn default() -> Self {
        InsertMenuItemFields { find: String::new(), command: String::new(), params: json!({}), note: FindNote::Clear }
    }
}

/// Why OK is off: no item picked.
fn check(fields: &InsertMenuItemFields) -> Option<Label> {
    fields.command.is_empty().then(|| label("Pick a menu item"))
}

/// A row of the list: an item of [`menus::insertable_items`], with what it shows and what Find
/// matches worked out once.
pub struct Row {
    /// The item's command.
    pub command: String,
    /// The item's params.
    pub params: Value,
    /// The menu path and label in the UI language (`Object › Group`).
    pub shown: String,
    /// The English path and label, the shown one and the command id, lowercased, one per line.
    haystack: String,
}

/// The rows in `lang`, for menus that name fonts in English when `english_names` is set and list
/// the `listed` names.
pub(crate) fn build(lang: Lang, english_names: bool, listed: &menus::Listed) -> Vec<Row> {
    menus::insertable_items(english_names, listed)
        .into_iter()
        .map(|item| {
            let english = item.path.iter().chain(std::iter::once(&item.label)).map(String::as_str).collect::<Vec<_>>().join(" › ");
            // A saved proof setup keeps its file's name.
            let own_name = item.command == crate::dialogs::proof_setup::LOAD;
            let path = item.path.iter().map(|s| crate::i18n::tr(lang, s));
            let name = if own_name { item.label.as_str() } else { crate::i18n::tr(lang, &item.label) };
            let shown = path.chain(std::iter::once(name)).collect::<Vec<_>>().join(" › ");
            let haystack = format!("{}\n{}\n{}", english.to_lowercase(), shown.to_lowercase(), item.command.to_lowercase());
            Row { command: item.command, params: item.params, shown, haystack }
        })
        .collect()
}

/// What the rows were built from: the language, the installed plug-ins, the font name setting, the
/// loaded fonts, whether the installed fonts are cataloged, the saved proof setups and the names the
/// menus list ([`menus::Listed`]).
pub(crate) type RowsKey = (&'static str, u64, bool, u64, bool, u64, u64);

/// The rows that every word of `find` matches, in menu order.
fn matches<'a>(rows: &'a [Row], find: &str) -> impl Iterator<Item = &'a Row> {
    let words: Vec<String> = find.to_lowercase().split_whitespace().map(str::to_string).collect();
    rows.iter().filter(move |r| words.iter().all(|w| r.haystack.contains(w.as_str())))
}

/// Picks `row`.
fn pick(fields: &mut InsertMenuItemFields, row: &Row) {
    fields.command.clone_from(&row.command);
    fields.params.clone_from(&row.params);
    fields.note = FindNote::Clear;
}

/// Find: picks the one row that `find` matches; with none or several, picks nothing and notes it.
fn find(rows: &[Row], fields: &mut InsertMenuItemFields) {
    let found: Vec<&Row> = matches(rows, &fields.find).take(2).collect();
    match found[..] {
        [row] => pick(fields, row),
        [] => fields.note = FindNote::NoMatch,
        _ => fields.note = FindNote::Several,
    }
}

/// Whether `row` is the item picked.
fn picked(fields: &InsertMenuItemFields, row: &Row) -> bool {
    fields.command == row.command && fields.params == row.params
}

/// The texts of the list's rows as a collection source.
struct Listed<'a>(&'a [String]);

impl DisplayOrder<usize> for Listed<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|text| Cow::Borrowed(text.as_str()))
    }
}

impl ItemSource<usize> for Listed<'_> {}

/// The Insert Menu Item dialog on the dialog host.
pub struct InsertMenuItemDialog {
    id: DialogId,
    /// Where the step goes.
    pub target: Target,
    form: Form<InsertMenuItemFields>,
    rows: Vec<Row>,
    /// What `rows` were built from; the pump builds them again when it changes.
    key: Option<RowsKey>,
}

impl InsertMenuItemDialog {
    /// What the rows would be built from now.
    pub(crate) fn rows_key(app: &VectorcraftApp) -> RowsKey {
        let db = vectorcraft_text::FontDb::global();
        // Read before building, as the Font menu does: fonts that load meanwhile build the rows again.
        let (cataloged, generation) = (db.installed_fonts_ready(), db.generation());
        (
            crate::i18n::current().code(),
            menus::plugin_revision(),
            app.session.prefs.font_names_in_english,
            generation,
            cataloged,
            crate::dialogs::proof_setup::saved_revision(),
            {
                let mut h = DefaultHasher::new();
                menus::Listed::of(app).hash(&mut h);
                h.finish()
            },
        )
    }

    /// What the rows were built from.
    pub(crate) fn built_from(&self) -> Option<RowsKey> {
        self.key
    }

    /// Replaces the rows with `rows`, built from `key`.
    pub(crate) fn set_rows(&mut self, key: RowsKey, rows: Vec<Row>) {
        self.key = Some(key);
        self.rows = rows;
    }

    /// The items the list offers before Find narrows them.
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
}

/// Opens Insert Menu Item for action `target` as (set, action), after `step`, else for the recording
/// in progress.
pub fn open(app: &mut VectorcraftApp, target: Option<(usize, usize)>, step: Option<usize>) {
    let key = InsertMenuItemDialog::rows_key(app);
    let rows = build(crate::i18n::current(), app.session.prefs.font_names_in_english, &menus::Listed::of(app));
    let dialog = InsertMenuItemDialog {
        id: dialog_id(KEY),
        target: Target::new(target, step),
        form: Form::new(InsertMenuItemFields::default(), check),
        rows,
        key: Some(key),
    };
    open_or_say(app, dialog);
}

impl ActionDialog for InsertMenuItemDialog {
    type Answer = (Target, InsertMenuItemFields);

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.form.take().map(|fields| (self.target, fields))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, (target, fields): Self::Answer) -> Result<(), String> {
        let mut params = json!({"command": fields.command, "params": fields.params});
        target.add_to(&mut params);
        app.run(INSERT_MENU_ITEM, params).map(|_| ())
    }
}

impl StackDialog for InsertMenuItemDialog {
    type Result = InsertMenuItemFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<InsertMenuItemFields>> {
        let rows = &self.rows;
        let rows_key = self.key;
        self.form.show(ui, label("Insert Menu Item"), None, |ui, fields| {
            FieldGroup::new(label("Insert Menu Item")).show(ui, |ui, _| {
                let before = fields.find.clone();
                let mut find_now = false;
                Field::new(label("Find:")).show(ui, |ui, _| {
                    Line::new().show(ui, |ui, _| {
                        TextField::search(label("Find"), Bind::bound(&mut fields.find)).show(ui);
                        find_now = clicked(&Button::new(label("Find")).id_salt("find").show(ui));
                    });
                });
                if fields.find != before {
                    fields.note = FindNote::Clear;
                }
                if find_now {
                    find(rows, fields);
                }
                let note = match fields.note {
                    FindNote::Clear => None,
                    FindNote::NoMatch => Some("No menu item matches."),
                    FindNote::Several => Some("Several menu items match. Choose one from the list."),
                };
                if let Some(note) = note {
                    Text::new(label(note)).shade(Shade::Muted).show(ui);
                }
                let listed: Vec<&Row> = matches(rows, &fields.find).take(MAX_LISTED).collect();
                let texts: Vec<String> = listed.iter().map(|row| row.shown.clone()).collect();
                let selected = listed.iter().position(|row| picked(fields, row));
                let mut selection = selected.map_or_else(Selection::default, Selection::single);
                List::<usize>::new(label("Menu Items"))
                    .select(SelectMode::Single)
                    .rows_stamp(Revision::of(&(rows_key, &fields.find, selected)).get())
                    .selection(Bind::bound(&mut selection))
                    .sizing(Sizing::Content)
                    .max_rows(8)
                    .show_items(ui, &Listed(&texts), &mut |key, _| ListRow::new(texts.get(*key).cloned().unwrap_or_default()));
                let chosen = selection.items.first().copied();
                if chosen != selected
                    && let Some(row) = chosen.and_then(|at| listed.get(at))
                {
                    pick(fields, row);
                }
                let current = Label::from(rows.iter().find(|row| picked(fields, row)).map_or_else(
                    || if fields.command.is_empty() { crate::i18n::t("None").to_owned() } else { fields.command.clone() },
                    |row| row.shown.clone(),
                ));
                Field::new(label("Menu Item:")).show(ui, |ui, _| {
                    Text::new(current.clone()).show(ui);
                });
                Text::new(label("Items that open a dialog, and File, Window and Help menu items, are not listed.")).shade(Shade::Muted).show(ui);
            });
        })
    }

    fn confirm(&mut self, _: &Context) -> Option<InsertMenuItemFields> {
        self.form.confirm()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.form.held_state()
    }

    /// The fields and the place; the params of an item the request may not read are left out.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.form.json_fields()?;
        if let Some(Value::Object(params)) = fields.get_mut("params") {
            crate::control::leave_out_unreadable_step(&self.form.values.command, params);
        }
        Some(merged(fields, self.target.json()))
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        match self.target.set_field(field, &value) {
            Some(done) => done,
            None => self.form.set_field(ctx, field, value),
        }
    }
}

#[cfg(test)]
mod tests {
    use craft::automation::actions::Action;
    use craft::menu::Node;
    use egui::{Key, Modifiers};

    use super::*;
    use crate::dialogs::actions::pump;
    use crate::tests::panel_harness::Panel;

    fn fields(find: &str, command: &str) -> InsertMenuItemFields {
        InsertMenuItemFields { find: find.into(), command: command.into(), ..InsertMenuItemFields::default() }
    }

    #[test]
    fn find_picks_the_one_match_and_notes_none_or_several() {
        let rows = build(Lang::EN, true, &menus::Listed::default());
        let mut f = fields("object group", "");
        find(&rows, &mut f);
        assert_eq!((f.note, f.command.as_str()), (FindNote::Several, ""), "Group and Ungroup both match");
        f.find = "OBJECT.UNGROUP".into();
        find(&rows, &mut f);
        assert_eq!((f.command.as_str(), f.params.clone(), f.note), ("object.ungroup", json!({}), FindNote::Clear));
        f.find = "no such menu item here".into();
        find(&rows, &mut f);
        assert_eq!((f.note, f.command.as_str()), (FindNote::NoMatch, "object.ungroup"), "a failed Find keeps the pick");
    }

    #[test]
    fn arrow_navigation_inserts_the_command_after_the_selected_filtered_result() {
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.ui.actions = Default::default();
        app.ui.actions.push(Action { set: Some("Test".into()), ..Action::new("View", Vec::new()) }).unwrap();
        let available = build(Lang::EN, true, &menus::Listed::default());
        let fit_params = available.iter().find(|row| row.command == "view.fitArtboard").unwrap().params.clone();
        let rows = [
            ("view.zoomIn", "First command", "first command"),
            ("view.zoomOut", "Second command", "remaining commands"),
            ("view.fitArtboard", "Third command", "remaining commands"),
            ("view.outline", "Fourth command", "remaining commands"),
        ]
        .into_iter()
        .map(|(command, shown, haystack)| Row {
            command: command.into(),
            params: available.iter().find(|row| row.command == command).unwrap().params.clone(),
            shown: shown.into(),
            haystack: haystack.into(),
        })
        .collect();
        let mut panel = Panel::new(app, |app, ui| app.dialogs().unwrap().with(|host| host.run(ui)));
        let dialog = InsertMenuItemDialog {
            id: dialog_id(KEY),
            target: Target::new(Some((0, 0)), None),
            form: Form::new(InsertMenuItemFields::default(), check),
            rows,
            key: None,
        };
        open_or_say(&mut panel.app(), dialog);
        panel.frame();
        panel.frame();

        let dialogs = panel.app().dialogs().unwrap();
        let command = || dialogs.with(|host| host.get::<InsertMenuItemDialog>().unwrap().form.values.command.clone());
        panel.click("First command");
        assert_eq!(command(), "view.zoomIn");
        dialogs.with(|host| {
            host.set_json_field(panel.ctx(), dialog_id(KEY), "find", json!("remaining")).unwrap();
            host.set_json_field(panel.ctx(), dialog_id(KEY), "command", json!("view.fitArtboard")).unwrap();
            host.set_json_field(panel.ctx(), dialog_id(KEY), "params", fit_params).unwrap();
        });
        panel.frame();
        assert!(!panel.shows("First command"));
        panel.key(Key::ArrowDown, Modifiers::NONE);
        assert_eq!(command(), "view.outline");
        panel.click(tl!("OK"));
        let mut app = panel.app();
        pump(&mut app, panel.ctx());
        let steps = &app.ui.actions.action(0).unwrap().steps;
        assert_eq!(steps.len(), 1);
        assert_eq!(steps[0].command, "view.outline");
    }

    /// The list holds the menu items that run an engine command, and the View menu items that the
    /// app runs itself, without asking for values first, outside the application, File, Window and
    /// Help menus, and leaves out Undo, Redo, the Pastes, Image Trace's Make and Make and Expand,
    /// Pattern's Make and Edit Pattern and the saved-selection, saved-view and recent-font items.
    #[test]
    fn the_list_holds_only_menu_items_that_can_be_steps() {
        let items = menus::insertable_items(true, &menus::Listed::default());
        assert!(items.len() > 100, "only {} items", items.len());
        for item in &items {
            let in_app = menus::plays_in_app(&item.command);
            assert!(vectorcraft_engine::find_command(&item.command).is_some() != in_app, "{item:?}");
            assert!(!in_app || item.path[0] == "View", "{item:?}");
            assert!(!item.label.ends_with('…'), "{item:?}");
            assert!(!["VectorCraft", "File", "Window", "Help"].contains(&item.path[0].as_str()), "{item:?}");
            assert!(item.params.is_object(), "{item:?}");
        }
        let has = |command: &str| items.iter().any(|i| i.command == command);
        for command in [
            "object.group",
            "object.ungroup",
            "object.arrange.bringToFront",
            "object.align",
            "view.outline",
            "view.zoomIn",
            "view.fitArtboard",
            "view.rotate",
            "view.rulers",
            "view.snapToGrid",
            "view.guides.lock",
        ] {
            assert!(has(command), "{command}");
        }
        let aligns = items.iter().filter(|i| i.command == "object.align").count();
        assert_eq!(aligns, 7, "each Object › Align item, by its params");
        for command in [
            "edit.undo",
            "edit.redo",
            "edit.paste",
            "edit.pasteInFront",
            "object.move",
            "imageTrace.make",
            "imageTrace.makeAndExpand",
            "object.pattern.make",
            "object.pattern.edit",
            "file.save",
            "window.panel",
            "view.goto1",
            "view.saved.new",
            "select.recall1",
        ] {
            assert!(!has(command), "{command}");
        }
    }

    /// Every View menu item that the app runs itself is listed, apart from the saved-view slots, the
    /// Perspective Grid presets (`ui.` commands, whose items follow the presets in the preferences)
    /// and the items that ask for values first.
    #[test]
    fn every_view_item_the_app_runs_is_listed() {
        let items = menus::insertable_items(true, &menus::Listed::default());
        let view =
            menus::menu_tree_named(true, &menus::Listed::default()).into_iter().find(|(title, _)| *title == "View").map(|(_, nodes)| nodes).unwrap();
        fn walk<'a>(nodes: &'a [Node], out: &mut Vec<(&'a str, &'a str)>) {
            for node in nodes {
                match node {
                    Node::Item(item) => out.push((item.label.as_str(), item.command.id.as_str())),
                    Node::Submenu { children, .. } => walk(children, out),
                    _ => {}
                }
            }
        }
        let mut all = vec![];
        walk(&view, &mut all);
        for (label, id) in all {
            let ui = vectorcraft_engine::find_command(id).is_none() && menus::UI_COMMANDS.iter().any(|c| c.0 == id);
            if ui && !label.ends_with('…') && !id.starts_with("view.goto") && !id.starts_with("ui.") {
                assert!(items.iter().any(|i| i.command == id), "{label} ({id})");
            }
        }
    }

    /// Rows show their menu path in the UI language, and Find matches the English path, the shown
    /// one and the command id.
    #[test]
    fn rows_show_the_translated_path_and_match_english_shown_text_and_ids() {
        let de = Lang::from_code("de").unwrap();
        let rows = build(de, true, &menus::Listed::default());
        let group = rows.iter().find(|r| r.command == "object.group").unwrap();
        assert_eq!(group.shown, format!("{} › {}", crate::i18n::tr(de, "Object"), crate::i18n::tr(de, "Group")));
        for words in ["object group", group.shown.as_str(), "object.group"] {
            assert!(matches(&rows, words).any(|r| r.command == "object.group"), "{words}");
        }
        let shared = rows.iter().any(|r| rows.iter().any(|o| o.command == r.command && o.params != r.params));
        assert!(shared, "items that share a command are listed apart, by their params");
    }

    /// A proof setup saved while the dialog is open is listed at once, by its file's name in every
    /// UI language: the pump builds the rows again when what they were built from changes.
    #[test]
    fn a_proof_setup_saved_meanwhile_is_listed_by_its_name() {
        use crate::dialogs::proof_setup;
        let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
        let ctx = crate::testing::ui_context();
        app.ui_ctx = Some(ctx.clone());
        let dir = vectorcraft_testkit::TempDir::new("insert-menu-item-proof-setups");
        app.session.attach_proof_setups(dir.join("Proof Setups"));
        proof_setup::refresh_saved(&app);
        proof_setup::wait();
        let de = Lang::from_code("de").unwrap();
        crate::i18n::set_current(de);
        // The installed fonts' scan, which builds the rows again when it ends, is over first.
        vectorcraft_text::FontDb::global().menu_family_list();
        open(&mut app, None, None);
        let listed = |app: &VectorcraftApp| {
            app.dialogs().unwrap().with(|host| {
                let dialog = host.get::<InsertMenuItemDialog>().unwrap();
                dialog.rows().iter().filter(|r| r.command == proof_setup::LOAD).map(|r| r.shown.clone()).collect::<Vec<_>>()
            })
        };
        let before = listed(&app);
        app.run("view.proofSetup.save", json!({"name": "Group", "target": "workingCmyk"})).unwrap();
        proof_setup::refresh_saved(&app);
        proof_setup::wait();
        pump(&mut app, &ctx);
        let after = listed(&app);
        crate::i18n::set_current(Lang::EN);
        assert!(before.is_empty(), "{before:?}");
        let view = crate::i18n::tr(de, "View");
        let menu = crate::i18n::tr(de, "Proof Setup");
        assert_eq!(after, [format!("{view} › {menu} › Group")]);
    }
}
