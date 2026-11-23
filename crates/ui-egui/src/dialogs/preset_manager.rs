//! The preset managers (PDF Presets, Print Presets, Transparency Flattener Presets) on one dialog body: a
//! list of presets with New, Edit, Delete, Import and Export under it, and the selected preset's details
//! beside it. The body only draws and queues what the user did as [`ManagerEvent`]s; [`pump`] carries them
//! out through each manager's [`Source`], so a rule (a name check, the preset above a deleted one) lives in
//! the manager's own module.
//!
//! The type shares its name with `ui_patterns::presets::PresetManager`, a kind select over a browser,
//! which this dialog does not use: it has a detail column and buttons under the list.

use std::borrow::Cow;
use std::sync::Arc;

use craft::behavior::{DisplayOrder, ItemSource, Selection};
use craft::host::PickRequest;
use craft::theme::variant::Tone;
use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Props as _, SelectMode, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, FieldRefusal, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::notices::Notice;
use craft::ui::patterns::form_dialogs::{DialogFields, Param, ParamRows, set_param_field};
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::overlay::CloseReason;
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use serde_json::Value;

use super::{flattener_presets, pdf_presets, print_presets};
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};

/// The most presets the list shows before it scrolls (about ten rows).
const LIST_ROWS: u16 = 10;

/// One preset of the list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ManagerRow {
    /// The preset's name, which the commands take.
    pub name: String,
    /// What the list shows: the name, in brackets for a built-in preset.
    pub label: String,
    /// A built-in preset is read only.
    pub built_in: bool,
}

impl ManagerRow {
    /// The row of the preset `name`.
    pub(crate) fn new(name: impl Into<String>, built_in: bool) -> Self {
        let name = name.into();
        let label = if built_in { format!("[{}]", tl!(&name)) } else { name.clone() };
        ManagerRow { name, label, built_in }
    }
}

/// Makes detail elements with the words of the drawing `Ui`'s `Env`.
pub(crate) type LazyDetails = Arc<dyn Fn(&Env) -> Vec<Detail> + Send + Sync>;

/// One element of the detail column.
pub(crate) enum Detail {
    /// The preset's name, strong.
    Title(String),
    /// A line of text.
    Text(String),
    /// A label and its value.
    Line(String, String),
    /// A dim caption.
    Note(String),
    /// A warning.
    Warning(String),
    /// The parameter rows of the preset's options over their values; the flag makes them read only.
    Rows(Vec<Param>, DialogFields, bool),
    /// Elements whose text needs the framework's words, which only the `Env` of the drawing `Ui` gives.
    Lazy(LazyDetails),
}

/// A button under the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ManagerAction {
    /// A new preset starting from the selected one.
    New,
    /// Edit the selected preset (not a built-in one).
    Edit,
    /// Delete the selected preset (not a built-in one).
    Delete,
    /// Add the presets of a file.
    Import,
    /// Save the selected preset to a file.
    Export,
}

impl ManagerAction {
    /// The button's label and tooltip. `editor`: New and Edit open the preset editor; otherwise New makes a saved
    /// copy at once.
    fn words(self, editor: bool) -> (String, Option<String>) {
        let (label, tip) = match self {
            ManagerAction::New if editor => (tl!("New…"), Some(tl!("A new preset starting from the selected one"))),
            ManagerAction::New => (tl!("New"), Some(tl!("A saved copy of the selected preset"))),
            ManagerAction::Edit => (tl!("Edit…"), None),
            ManagerAction::Delete => (tl!("Delete"), None),
            ManagerAction::Import => (tl!("Import…"), None),
            ManagerAction::Export => (tl!("Export…"), Some(tl!("Save the selected preset to a file"))),
        };
        (label.to_string(), tip.map(|tip| tip.to_string()))
    }

    /// Whether the action changes the selected preset, so a built-in one refuses it.
    fn changes(self) -> bool {
        matches!(self, ManagerAction::Edit | ManagerAction::Delete)
    }
}

/// What the user did in the manager; [`pump`] hands each to the manager's `run`.
#[derive(Clone, Debug)]
pub(crate) enum ManagerEvent {
    /// A preset was picked in the list.
    Select(String),
    /// A button was pressed.
    Action(ManagerAction),
    /// The Name field committed this name.
    Rename(String),
    /// A row of the options changed; these are the values after it.
    Edited(DialogFields),
}

/// What distinguishes one manager from another: its key, words and buttons, and how it reads and changes its
/// presets. Each manager's module holds one.
pub(crate) struct Source {
    /// The dialog's key (`dialog_id` of it is the dialog's id).
    pub key: &'static str,
    /// The dialog's title.
    pub title: fn() -> String,
    /// The buttons under the list, in order.
    pub actions: &'static [ManagerAction],
    /// New and Edit open the preset editor, whose OK comes back to the manager, so the manager closes for it.
    pub editor: bool,
    /// The selected saved preset's name is a field in the detail column.
    pub renames: bool,
    /// The presets, built-in ones first.
    pub rows: fn(&VectorcraftApp) -> Vec<ManagerRow>,
    /// The stored value of the preset `name`, to see that the details no longer show what is stored.
    pub stored: fn(&VectorcraftApp, &str) -> Value,
    /// The detail column of the preset `name`.
    pub detail: fn(&VectorcraftApp, &str) -> Vec<Detail>,
    /// Carries out an event on the selected preset; the name to select afterwards, `None` to keep the selection.
    pub run: fn(&mut VectorcraftApp, &ManagerEvent, &str) -> Result<Option<String>, String>,
}

/// The row whose name is `name`, case and surrounding space ignored.
fn find<'a>(rows: &'a [ManagerRow], name: &str) -> Option<&'a ManagerRow> {
    rows.iter().find(|row| row.name.eq_ignore_ascii_case(name.trim()))
}

/// The index to select after deleting a saved preset that was the `saved`-th (`None`: the first) of the saved
/// ones: the one above it, the last built-in one when it was the first saved preset.
pub(crate) fn above(saved: Option<usize>, built_ins: usize) -> usize {
    (saved.unwrap_or(0) + built_ins).saturating_sub(1)
}

/// Ask for a preset file and run `command` on it, then select the first preset it imported in the open manager
/// `key`. Returns at once: the answer arrives later.
pub(crate) fn import(
    app: &mut VectorcraftApp,
    key: &'static str,
    command: &'static str,
    filter: &str,
    exts: &[&str],
) -> Result<Option<String>, String> {
    // PLAN: assumed `file_asks::open_params(&Picked) -> Option<Map<String, Value>>` (fl.A23) also names a picked file
    // of the web, which the import commands may not read yet.
    let request = PickRequest::open(key).filter(filter, exts);
    file_asks::ask(app, request, Owner::Free, move |app, picked| {
        let params = file_asks::open_params(&picked).ok_or("cancelled")?;
        let imported = app.run(command, Value::Object(params))?;
        if let Some(first) = imported["imported"].get(0).and_then(Value::as_str) {
            with_manager(app, dialog_id(key), |manager| manager.selected = first.to_owned());
        }
        Ok(imported)
    });
    Ok(None)
}

/// Opens the manager of `source` on the preset `selected` (the first one when none has that name).
pub(crate) fn open(app: &mut VectorcraftApp, source: &'static Source, selected: &str) {
    let rows = (source.rows)(app);
    let selected = find(&rows, selected).or(rows.first()).map(|row| row.name.clone()).unwrap_or_default();
    let stored = (source.stored)(app, &selected);
    let detail = (source.detail)(app, &selected);
    let manager = PresetManager::new(source, rows, selected, stored, detail);
    if let Some(dialogs) = app.dialogs() {
        dialogs.with(|h| h.open(manager));
    }
}

/// Runs `f` on the open manager `id`; `None` when it is not open.
fn with_manager<R>(app: &VectorcraftApp, id: DialogId, f: impl FnOnce(&mut PresetManager) -> R) -> Option<R> {
    app.dialogs()?.with(|h| h.get_mut_by_id::<PresetManager>(id).map(f))
}

/// A preset manager dialog: draws the rows and the detail it holds and queues the user's events.
pub(crate) struct PresetManager {
    id: DialogId,
    source: &'static Source,
    title: Label,
    rows: Vec<ManagerRow>,
    selected: String,
    detail: Vec<Detail>,
    /// What the details were built from: the preset they are for and its stored value.
    built_for: String,
    stored: Value,
    /// The text of the Name field, for a saved preset of a manager that renames.
    name: Option<String>,
    events: Vec<ManagerEvent>,
}

impl PresetManager {
    fn new(source: &'static Source, rows: Vec<ManagerRow>, selected: String, stored: Value, detail: Vec<Detail>) -> Self {
        let mut manager = PresetManager {
            id: dialog_id(source.key),
            source,
            title: Label::from((source.title)()),
            rows: Vec::new(),
            selected: String::new(),
            detail: Vec::new(),
            built_for: String::new(),
            stored: Value::Null,
            name: None,
            events: Vec::new(),
        };
        manager.update(rows, selected, stored, Some(detail));
        manager
    }

    /// Queues `event` as if the user had caused it; the next [`pump`] carries it out.
    pub(crate) fn queue(&mut self, event: ManagerEvent) {
        self.events.push(event);
    }

    /// The events queued since the last call.
    fn take_events(&mut self) -> Vec<ManagerEvent> {
        std::mem::take(&mut self.events)
    }

    /// Whether the details differ from what the store holds now.
    fn is_stale(&self, rows: &[ManagerRow], selected: &str, stored: &Value) -> bool {
        self.rows != rows || self.built_for != selected || self.stored != *stored
    }

    /// Selects `selected`; with `detail`, takes `rows`, `stored` and the details as the new view.
    fn update(&mut self, rows: Vec<ManagerRow>, selected: String, stored: Value, detail: Option<Vec<Detail>>) {
        self.selected = selected;
        let Some(detail) = detail else { return };
        let saved = find(&rows, &self.selected).is_some_and(|row| !row.built_in);
        self.name = (self.source.renames && saved).then(|| self.selected.clone());
        self.built_for = self.selected.clone();
        self.rows = rows;
        self.stored = stored;
        self.detail = detail;
    }

    /// Whether the selected preset is a built-in one.
    fn built_in(&self) -> bool {
        find(&self.rows, &self.selected).is_none_or(|row| row.built_in)
    }
}

impl StackDialog for PresetManager {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(self.source.key)
    }

    // The three managers are one type; each key is its own dialog.
    fn instance(&self) -> Option<DialogId> {
        Some(self.id)
    }

    fn show(&mut self, ui: &mut egui::Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let title = self.title.clone();
        let PresetManager { source, rows, selected, detail, name, events, .. } = self;
        let built_in = find(rows, selected).is_none_or(|row| row.built_in);
        let shown = Dialog::new(title)
            .buttons([FooterButton::new(tl!("Close").to_string(), DialogRole::Accept)])
            .collection_body()
            .show(ui, |ui, _| draw(ui, source, rows, selected, built_in, (detail.as_mut_slice(), name)));
        // The body can run twice in a pass (measure, then paint); `inner` is the last run's value.
        events.extend(shown.inner.unwrap_or_default());
        match shown.output.closed? {
            CloseReason::Enter | CloseReason::Activated => Some(DialogResult::Ok(())),
            _ => Some(DialogResult::Cancel),
        }
    }

    fn confirm(&mut self, _: &egui::Context) -> Option<()> {
        Some(())
    }

    fn json_fields(&self) -> Option<Value> {
        let mut fields = serde_json::Map::new();
        fields.insert("selected".into(), Value::from(self.selected.as_str()));
        fields.insert("name".into(), Value::from(self.name.as_deref().unwrap_or(&self.selected)));
        for item in &self.detail {
            if let Detail::Rows(_, values, _) = item
                && let Ok(Value::Object(values)) = serde_json::to_value(values)
            {
                fields.extend(values);
            }
        }
        Some(Value::Object(fields))
    }

    fn set_json_field(&mut self, _: &egui::Context, field: &str, value: Value) -> Result<(), String> {
        if field == "selected" {
            let row = value.as_str().and_then(|name| find(&self.rows, name)).ok_or_else(|| FieldRefusal::Rejects(&value).message(field))?;
            self.selected = row.name.clone();
            return Ok(());
        }
        if field == "name" {
            let (Some(name), Some(text)) = (self.name.as_mut(), value.as_str()) else {
                return Err(FieldRefusal::Inactive.message(field));
            };
            *name = text.to_owned();
            let rename = ManagerEvent::Rename(text.to_owned());
            self.queue(rename);
            return Ok(());
        }
        let rows = self.detail.iter_mut().find_map(|item| match item {
            Detail::Rows(params, values, read_only) if params.iter().any(|param| param.key == field) => Some((params, values, *read_only)),
            _ => None,
        });
        let Some((params, values, read_only)) = rows else { return Err(FieldRefusal::Unknown.message(field)) };
        if read_only {
            return Err(FieldRefusal::Inactive.message(field));
        }
        set_param_field(params, values, field, value)?;
        let edited = ManagerEvent::Edited(values.clone());
        self.queue(edited);
        Ok(())
    }
}

/// The rows of the list as an item source: a row's key is its position.
struct Names<'a>(&'a [ManagerRow]);

impl DisplayOrder<usize> for Names<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|row| Cow::Borrowed(row.label.as_str()))
    }
}

impl ItemSource<usize> for Names<'_> {}

/// One pass of the body: the list and its buttons, and beside them the details. Returns what the user did.
fn draw(
    ui: &mut egui::Ui,
    source: &Source,
    rows: &[ManagerRow],
    selected: &str,
    built_in: bool,
    (detail, name): (&mut [Detail], &mut Option<String>),
) -> Vec<ManagerEvent> {
    let mut events = Vec::new();
    Row::new().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            Text::new(tl!("Presets:").to_string()).shade(Shade::Muted).show(ui);
            list(ui, rows, selected, &mut events);
            Row::new().show(ui, |ui, _| {
                for &action in source.actions {
                    let (label, tip) = action.words(source.editor);
                    let mut button = Button::new(label).enabled(!(built_in && action.changes()));
                    if let Some(tip) = tip {
                        button = button.tooltip(tip);
                    }
                    if button.show(ui).clicked() {
                        events.push(ManagerEvent::Action(action));
                    }
                }
            });
        });
        Stack::new().show(ui, |ui, _| {
            if let Some(typed) = name.as_mut() {
                Field::new(tl!("Name").to_string()).show(ui, |ui, _| {
                    // Enter commits in the field; it does not close the dialog.
                    let edit = TextField::new(tl!("Name").to_string(), Bind::bound(typed)).submits().show(ui);
                    if let Some(committed) = edit.events.committed() {
                        events.push(ManagerEvent::Rename(committed.clone()));
                    }
                });
            }
            show_detail(ui, detail, &mut events);
        });
    });
    events
}

/// The single-selection list of the presets; a pick queues `Select`.
fn list(ui: &mut egui::Ui, rows: &[ManagerRow], selected: &str, events: &mut Vec<ManagerEvent>) {
    let current = rows.iter().position(|row| row.name == selected);
    let mut selection = current.map_or_else(Selection::default, Selection::single);
    let _response = List::new(tl!("Presets").to_string())
        .id_salt("presets")
        .select(SelectMode::Single)
        .selection(Bind::bound(&mut selection))
        .max_rows(LIST_ROWS)
        .show_items(ui, &Names(rows), &mut |at, _| ListRow::new(rows.get(*at).map(|row| row.label.clone()).unwrap_or_default()));
    let picked = selection.items.first().copied();
    if picked != current
        && let Some(row) = picked.and_then(|at| rows.get(at))
    {
        events.push(ManagerEvent::Select(row.name.clone()));
    }
}

/// Draws the elements of the detail column.
fn show_detail(ui: &mut egui::Ui, detail: &mut [Detail], events: &mut Vec<ManagerEvent>) {
    for item in detail {
        match item {
            Detail::Title(text) => {
                Text::new(text.clone()).role(TypeRole::BodyStrong).show(ui);
            }
            Detail::Text(text) => {
                Text::new(text.clone()).show(ui);
            }
            Detail::Line(label, value) => {
                Field::new(label.clone()).show(ui, |ui, _| {
                    Text::new(value.clone()).show(ui);
                });
            }
            Detail::Note(text) => {
                Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
            }
            Detail::Warning(text) => {
                Notice::new(Tone::Warn, text.clone()).show(ui);
            }
            Detail::Rows(params, values, read_only) => {
                let shown = ParamRows::new(tl!("Options").to_string(), params.as_slice(), Bind::bound(&mut *values)).read_only(*read_only).show(ui);
                if shown.changed {
                    events.push(ManagerEvent::Edited(values.clone()));
                }
            }
            Detail::Lazy(build) => {
                let mut items = build(&Env::of(ui));
                show_detail(ui, &mut items, events);
            }
        }
    }
}

/// Carries out the events the open managers queued, then shows each manager what the store holds now: the rows and
/// the selected preset's details are rebuilt when they differ from the store, so a value being typed is never
/// overwritten and a change made elsewhere (an import, the editor's OK, a command) shows at once. A pass drains the
/// queue once, so a second pass of the same frame finds none.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &egui::Context) {
    // PLAN: assumed `print_presets::SOURCE` (export-w20): a `static Source` like `pdf_presets::SOURCE`.
    for source in [&pdf_presets::SOURCE, &print_presets::SOURCE, &flattener_presets::SOURCE] {
        pump_source(app, ctx, source);
    }
}

fn pump_source(app: &mut VectorcraftApp, ctx: &egui::Context, source: &'static Source) {
    let id = dialog_id(source.key);
    let Some((events, mut selected)) = with_manager(app, id, |m| (m.take_events(), m.selected.clone())) else {
        return;
    };
    // A refused event leaves the details as typed; rebuilding puts the stored values back.
    let mut refused = false;
    for event in events {
        if let ManagerEvent::Select(name) = event {
            selected = name;
            continue;
        }
        match (source.run)(app, &event, &selected) {
            Ok(next) => {
                selected = next.unwrap_or(selected);
                if source.editor
                    && matches!(event, ManagerEvent::Action(ManagerAction::New | ManagerAction::Edit))
                    && let Some(dialogs) = app.dialogs()
                {
                    dialogs.with(|h| h.cancel_by_id(ctx, id));
                }
            }
            Err(e) => {
                app.status(e);
                refused = true;
            }
        }
    }
    let rows = (source.rows)(app);
    let selected = find(&rows, &selected).or(rows.first()).map(|row| row.name.clone()).unwrap_or_default();
    let stored = (source.stored)(app, &selected);
    let stale = refused || with_manager(app, id, |m| m.is_stale(&rows, &selected, &stored)).unwrap_or(false);
    let detail = stale.then(|| (source.detail)(app, &selected));
    with_manager(app, id, |manager| manager.update(rows, selected, stored, detail));
}
