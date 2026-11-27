//! The dialog behind Select → Edit Selection… and View → Edit Views…: the document's saved names in a list,
//! renamed in place or deleted with Delete. Nothing changes until OK, which sends every rename and delete as
//! one `edits` list to the kind's command (one undo step); Cancel leaves them as they were. A name the
//! command refuses (two saved items would share it) keeps the dialog open with the reason.
//!
//! Fields: `count` and, for each saved item `i`, `orig<i>` (its name when the dialog opened), `name<i>` (as
//! edited) and `deleted<i>`; `selected` (the row picked).

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource, Selection};
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldAnswer, HeldOk, StackDialog, dialog_id};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{List, ListRow};
use craft::ui::notices::{Notice, NoticeLayout};
use craft::ui::patterns::form_dialogs::{FormDialog, ResetFooter, ResetPick};
use craft::ui::runtime::env::Label;
use craft::ui::style::SurfaceKind;
use serde_json::{Value, json};

use crate::VectorcraftApp;

// PLAN: ui-egui's Cargo.toml has no `craft` dependency (it names `craft-ui` and other crates); the `craft::` paths need it (owner: hub-w1/hub-w2, whoever edits the manifest).

/// What tells the two kinds of saved names apart: the dialog kind, its words and the command that applies the
/// edits.
#[derive(Clone, Copy, Debug)]
pub(crate) struct SavedNames {
    /// The dialog kind; its key on the host, so two kinds are two dialogs.
    pub kind: &'static str,
    /// The dialog's title.
    pub title: &'static str,
    /// The label of the list.
    pub list: &'static str,
    /// The command that applies `{edits}`.
    pub command: &'static str,
    /// The most names the dialog lists.
    pub max: usize,
}

/// The part of every saved-names dialog's id the kind is added to.
const KEY: &str = "savedNames";

/// The rows of the list the user sees at once; more scroll.
const SHOWN_ROWS: u16 = 9;

const NAME: ColumnKey = ColumnKey::new("name");

/// The id of the dialog of `kind` on the host.
pub(crate) fn id_of(kind: &str) -> DialogId {
    dialog_id((KEY, kind))
}

/// One saved name as the dialog edits it.
#[derive(Clone, Debug, PartialEq)]
struct NameRow {
    /// The name when the dialog opened: what the command finds the item by.
    orig: String,
    name: String,
    deleted: bool,
}

/// What the dialog edits: every name, deleted ones included, in the order they were saved.
#[derive(Clone, Debug, Default, PartialEq)]
struct NamesDraft {
    rows: Vec<NameRow>,
}

impl NamesDraft {
    /// The rows still in the list (not deleted), by their index in `rows`.
    fn visible(&self) -> Vec<usize> {
        self.rows.iter().enumerate().filter(|(_, row)| !row.deleted).map(|(at, _)| at).collect()
    }

    /// The first picked row that is still in the list.
    fn selected(&self, selection: &Selection<usize>) -> Option<usize> {
        selection.items.iter().copied().find(|at| self.rows.get(*at).is_some_and(|row| !row.deleted))
    }

    /// The edits OK applies: each deleted item, and each one whose name was changed.
    fn edits(&self) -> Vec<Value> {
        self.rows
            .iter()
            .filter_map(|row| {
                let name = row.name.trim();
                if row.deleted {
                    Some(json!({ "name": row.orig, "delete": true }))
                } else if !name.is_empty() && name != row.orig {
                    Some(json!({ "name": row.orig, "newName": name }))
                } else {
                    None
                }
            })
            .collect()
    }

    /// The index in `rows` a field named `prefix` and a number refers to, when the number is written as the
    /// dialog writes it and names a row.
    fn row_of(&self, field: &str, prefix: &str) -> Option<usize> {
        let digits = field.strip_prefix(prefix)?;
        let at: usize = digits.parse().ok()?;
        (at.to_string() == digits && at < self.rows.len()).then_some(at)
    }
}

/// The rows still in the list, keyed by their index in the draft.
struct Shown<'a> {
    rows: &'a [NameRow],
    at: &'a [usize],
}

impl DisplayOrder<usize> for Shown<'_> {
    fn len(&self) -> usize {
        self.at.len()
    }

    fn key(&self, at: usize) -> usize {
        self.at.get(at).copied().unwrap_or_default()
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        self.at.iter().position(|row| row == key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        let row = self.rows.get(*self.at.get(at)?)?;
        Some(Cow::Borrowed(row.name.as_str()))
    }
}

impl ItemSource<usize> for Shown<'_> {}

/// The saved-names dialog of one kind.
pub(crate) struct SavedNamesDialog {
    id: DialogId,
    document: u64,
    reverts: u64,
    names: SavedNames,
    draft: NamesDraft,
    /// The picked row, by its index in the draft; one at most.
    selection: Selection<usize>,
    /// The edits OK filed, until the command answered them.
    held: HeldAnswer<Vec<Value>>,
}

impl SavedNamesDialog {
    /// The dialog over `list`, with the first name picked.
    fn new(document: u64, reverts: u64, names: SavedNames, list: Vec<String>) -> Self {
        let selection = if list.is_empty() { Selection::default() } else { Selection::single(0) };
        let rows = list.into_iter().map(|name| NameRow { orig: name.clone(), name, deleted: false }).collect();
        SavedNamesDialog { id: id_of(names.kind), document, reverts, names, draft: NamesDraft { rows }, selection, held: HeldAnswer::new() }
    }

    /// OK as its button gives it: with nothing edited the dialog closes at once (`Some`); with edits they wait for
    /// the command's answer (`None`), and once it accepted them the dialog closes.
    fn press_ok(&mut self) -> Option<()> {
        if self.held.is_accepted() {
            return self.held.finish().map(|_| ());
        }
        if self.held.is_waiting() {
            return None;
        }
        let edits = self.draft.edits();
        if edits.is_empty() {
            return Some(());
        }
        self.held.file(edits);
        None
    }

    /// Picks the first row still in the list, or none when every row is deleted.
    fn reselect(&mut self) {
        self.selection = self.draft.visible().first().map_or_else(Selection::default, |at| Selection::single(*at));
    }

    /// Shows the list and Delete over `draft`.
    fn rows(ui: &mut egui::Ui, names: SavedNames, draft: &mut NamesDraft, selection: &mut Selection<usize>) {
        let visible = draft.visible();
        let selected = draft.selected(selection);
        let mut events = Vec::new();
        let mut delete = false;
        Row::new().show(ui, |ui, _| {
            Field::new(names.list).show(ui, |ui, _| {
                let order = Shown { rows: &draft.rows, at: &visible };
                let columns = vec![ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0))];
                let out = List::<usize>::new(names.list)
                    .surface(SurfaceKind::Sunken)
                    .header(false)
                    .select(SelectMode::Single)
                    .rename(true)
                    .rename_column(NAME)
                    .columns(columns)
                    .selection(Bind::bound(&mut *selection))
                    .max_rows(SHOWN_ROWS)
                    .show_items(ui, &order, &mut |key, icx| {
                        let name = draft.rows.get(*key).map(|row| row.name.clone()).unwrap_or_default();
                        ListRow::new(name).cells(icx.cells()).state(icx.row.clone())
                    });
                events = out.events;
            });
            delete = Button::new(tl!("Delete")).enabled(selected.is_some()).show(ui).clicked();
        });
        for event in &events {
            if let CollectionEvent::Rename(edit) = event
                && let Some(rename) = edit.value()
                && !rename.name.trim().is_empty()
                && let Some(row) = draft.rows.get_mut(rename.key)
            {
                rename.name.trim().clone_into(&mut row.name);
            }
        }
        if let (true, Some(at)) = (delete, selected)
            && let Some(row) = draft.rows.get_mut(at)
        {
            row.deleted = true;
            // The row below takes its place (the one above, for the last).
            let next = visible.iter().copied().find(|row| *row > at).or_else(|| visible.iter().copied().rfind(|row| *row < at));
            *selection = next.map_or_else(Selection::default, Selection::single);
        }
    }
}

impl StackDialog for SavedNamesDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(self.names.kind)
    }

    fn instance(&self) -> Option<DialogId> {
        Some(self.id)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.held.is_accepted() {
            return self.press_ok().map(DialogResult::Ok);
        }
        // The refusal shows until the edits change.
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.draft.edits()) {
            self.held.clear_refusal();
        }
        let footer = ResetFooter::new(ui, None, true);
        let waiting = self.held.is_waiting();
        let reason = self.held.refusal().map(|(reason, _)| reason.clone());
        let SavedNamesDialog { names, draft, selection, .. } = &mut *self;
        let names = *names;
        let shown = FormDialog::new(names.title, Bind::bound(draft)).accept_stays_open(true).ok_enabled(!waiting).show(ui, |ui, _, draft| {
            Self::rows(ui, names, draft, selection);
            if let Some(reason) = &reason {
                Notice::new(Tone::Bad, reason.clone()).layout(NoticeLayout::Inline).show(ui);
            }
            true
        });
        // `picked` is this pass's press: a second pass of the frame has none, so OK files once.
        if matches!(footer.pick(shown.picked), Some(ResetPick::Accept)) && self.press_ok().is_some() {
            return Some(DialogResult::Ok(()));
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }

    /// Answers OK as its button does: none while the command has not answered the edits.
    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        self.press_ok()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// `count`, `orig<i>`, `name<i>` and `deleted<i>` of each saved name, and `selected`, the row picked (the first
    /// one left when none is).
    fn json_fields(&self) -> Option<Value> {
        let mut fields = json!({ "count": self.draft.rows.len() });
        for (at, row) in self.draft.rows.iter().enumerate() {
            fields[format!("orig{at}")] = json!(row.orig);
            fields[format!("name{at}")] = json!(row.name);
            fields[format!("deleted{at}")] = json!(row.deleted);
        }
        let selected = self.draft.selected(&self.selection).or_else(|| self.draft.visible().first().copied());
        fields["selected"] = json!(selected.unwrap_or_default());
        Some(fields)
    }

    /// Sets `name<i>` (text), `deleted<i>` (a switch) or `selected` (a row) as the list would; `orig<i>` and `count`
    /// are read only.
    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        let rejects = || FieldRefusal::Rejects(&value).message(field);
        if field == "count" || self.draft.row_of(field, "orig").is_some() {
            return Err(FieldRefusal::ReadOnly.message(field));
        }
        if field == "selected" {
            let at = value.as_u64().and_then(|at| usize::try_from(at).ok()).filter(|at| *at < self.draft.rows.len()).ok_or_else(rejects)?;
            self.selection = Selection::single(at);
            if self.draft.selected(&self.selection).is_none() {
                self.reselect();
            }
            return Ok(());
        }
        if let Some(at) = self.draft.row_of(field, "name") {
            let name = value.as_str().ok_or_else(rejects)?;
            if let Some(row) = self.draft.rows.get_mut(at) {
                name.clone_into(&mut row.name);
            }
            return Ok(());
        }
        if let Some(at) = self.draft.row_of(field, "deleted") {
            let deleted = value.as_bool().ok_or_else(rejects)?;
            if let Some(row) = self.draft.rows.get_mut(at) {
                row.deleted = deleted;
            }
            if self.draft.selected(&self.selection).is_none() {
                self.reselect();
            }
            return Ok(());
        }
        Err(FieldRefusal::Unknown.message(field))
    }
}

/// Opens the dialog of `names` on `list`, the saved names in their order (the first `names.max` of them).
pub(crate) fn open(app: &mut VectorcraftApp, names: SavedNames, mut list: Vec<String>) -> Result<(), String> {
    list.truncate(names.max);
    let active = app.session.active().ok_or("no document open")?;
    let (document, reverts) = (active.uid, active.reverts);
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(SavedNamesDialog::new(document, reverts, names, list)));
    Ok(())
}

/// Runs the edits the open saved-names dialogs filed, and answers each dialog with the command's outcome: it
/// closes when the command took the edits, and stays open with the reason when it refused them.
pub(crate) fn pump(app: &mut VectorcraftApp, _ctx: &egui::Context) {
    let Some(dialogs) = app.dialogs() else { return };
    let filed: Vec<_> = dialogs.with(|host| {
        host.open_dialogs()
            .into_iter()
            .filter_map(|open| {
                let dialog = host.get_mut_by_id::<SavedNamesDialog>(open.id)?;
                let edits = dialog.held.take()?;
                Some((open.id, dialog.document, dialog.reverts, dialog.names.command, edits))
            })
            .collect()
    });
    for (id, document, reverts, command, edits) in filed {
        let outcome = match app.session.active().filter(|active| active.uid == document) {
            Some(active) if active.reverts != reverts => Err(tl!("The document was reverted. Reopen the dialog to edit saved names.").to_owned()),
            Some(_) => app.run(command, json!({ "edits": edits })),
            None => Err(tl!("Saved names can only be edited in the document this dialog was opened for.").to_owned()),
        };
        if let Err(reason) = &outcome {
            app.status(reason.clone());
        }
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut_by_id::<SavedNamesDialog>(id) {
                dialog.held.answer(outcome.map(|_| ()).map_err(Label::from));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct SavedFile(std::path::PathBuf);

    impl Drop for SavedFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    #[test]
    fn saved_names_drafts_do_not_edit_a_reverted_drawing() {
        for is_view in [false, true] {
            let path = SavedFile(std::env::temp_dir().join(format!("vc-saved-names-revert-{}-{is_view}.vectorcraft", std::process::id())));
            let services = crate::Services {
                read: Some(Box::new(|path: &str| std::fs::read(path).map_err(|e| e.to_string()))),
                write: Some(Box::new(|path: &str, bytes: &[u8]| std::fs::write(path, bytes).map_err(|e| e.to_string()))),
                ..Default::default()
            };
            let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), services));
            let add_shape = |app: &mut VectorcraftApp, x| {
                app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 20, "height": 20})).unwrap();
            };
            let names = |app: &mut VectorcraftApp| {
                if is_view {
                    let views = &app.session.active().unwrap().doc.views;
                    json!(views.iter().map(|view| &view.name).collect::<Vec<_>>())
                } else {
                    app.run("select.savedList", json!({})).unwrap()
                }
            };
            let open = |app: &mut VectorcraftApp| {
                if is_view {
                    crate::dialogs::edit_views::open(app).unwrap();
                    crate::dialogs::edit_views::KIND
                } else {
                    crate::dialogs::edit_selection::open(app).unwrap();
                    crate::dialogs::edit_selection::KIND
                }
            };
            app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
            add_shape(&mut app, 10);
            for name in ["Shared", "Spare"] {
                app.run(if is_view { "view.saved.new" } else { "select.save" }, json!({"name": name})).unwrap();
            }
            app.run("file.saveAs", json!({"path": path.0})).unwrap();
            assert!(path.0.is_file());
            add_shape(&mut app, 40);
            let old = app.session.active().unwrap();
            let (document, reverts) = (old.uid, old.reverts);
            let kind = open(&mut app);
            let id = id_of(kind);
            let ctx = app.ui_ctx.clone().unwrap();
            let dialogs = app.dialogs().unwrap();
            dialogs.with(|host| {
                host.set_json_field(&ctx, id, "name0", json!("Stale")).unwrap();
                host.set_json_field(&ctx, id, "deleted1", json!(true)).unwrap();
                assert!(!host.confirm_by_id(&ctx, id));
                assert_eq!(host.held_state(id), Some(HeldOk::Waiting));
            });

            app.run("file.revert", json!({"confirmed": true})).unwrap();
            let active = app.session.active().unwrap();
            assert_eq!((active.uid, active.reverts), (document, reverts + 1), "Revert preserves the tab's UID");
            assert_eq!(names(&mut app), json!(["Shared", "Spare"]));
            let undo = app.session.active().unwrap().history.undo.len();
            pump(&mut app, &ctx);

            assert_eq!(names(&mut app), json!(["Shared", "Spare"]), "{kind} cannot apply its old draft to the reloaded drawing");
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo);
            assert!(matches!(dialogs.with(|host| host.held_state(id)), Some(HeldOk::Refused(_))));
            assert!(dialogs.with(|host| host.cancel_by_id(&ctx, id)));
            assert_eq!(open(&mut app), kind);
            dialogs.with(|host| {
                host.set_json_field(&ctx, id, "name0", json!("Revised")).unwrap();
                host.confirm_by_id(&ctx, id);
            });
            pump(&mut app, &ctx);
            assert!(matches!(dialogs.with(|host| host.held_state(id)), Some(HeldOk::Accepted)));
            assert_eq!(names(&mut app), json!(["Revised", "Spare"]));
            assert_eq!(app.session.active().unwrap().history.undo.len(), undo + 1);
        }
    }

    #[test]
    fn saved_names_edits_stay_with_their_document() {
        for is_view in [false, true] {
            let mut app = crate::testing::windowed(VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default()));
            for size in [100, 200] {
                app.run("file.new", json!({"width": size, "height": size})).unwrap();
                app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
                for name in ["Shared", "Spare"] {
                    app.run(if is_view { "view.saved.new" } else { "select.save" }, json!({"name": name})).unwrap();
                }
            }
            let names = |app: &mut VectorcraftApp| {
                if is_view {
                    let views = &app.session.active().unwrap().doc.views;
                    json!(views.iter().map(|view| &view.name).collect::<Vec<_>>())
                } else {
                    app.run("select.savedList", json!({})).unwrap()
                }
            };
            let activate = |app: &mut VectorcraftApp, index| app.run("document.activate", json!({"index": index})).unwrap();
            activate(&mut app, 0);
            let kind = if is_view {
                crate::dialogs::edit_views::open(&mut app).unwrap();
                crate::dialogs::edit_views::KIND
            } else {
                crate::dialogs::edit_selection::open(&mut app).unwrap();
                crate::dialogs::edit_selection::KIND
            };
            let id = id_of(kind);
            let ctx = app.ui_ctx.clone().unwrap();
            let host = app.dialogs().unwrap();
            host.with(|h| {
                h.set_json_field(&ctx, id, "name0", json!("Revised")).unwrap();
                h.set_json_field(&ctx, id, "deleted1", json!(true)).unwrap();
                h.confirm_by_id(&ctx, id);
            });

            activate(&mut app, 1);
            let b_undo = app.session.active().unwrap().history.undo.len();
            pump(&mut app, &ctx);
            assert_eq!(names(&mut app), json!(["Shared", "Spare"]), "the destination's saved names stay unchanged for {kind}");
            assert_eq!(app.session.active().unwrap().history.undo.len(), b_undo);
            assert!(matches!(host.with(|h| h.held_state(id)), Some(HeldOk::Refused(_))));

            activate(&mut app, 0);
            let a_undo = app.session.active().unwrap().history.undo.len();
            host.with(|h| h.confirm_by_id(&ctx, id));
            pump(&mut app, &ctx);
            assert!(matches!(host.with(|h| h.held_state(id)), Some(HeldOk::Accepted)));
            assert_eq!(names(&mut app), json!(["Revised"]));
            assert_eq!(app.session.active().unwrap().history.undo.len(), a_undo + 1);
        }
    }
}
