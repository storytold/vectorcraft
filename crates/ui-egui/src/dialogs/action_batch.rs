//! Actions panel menu › Batch…: play the selected action on every file in a
//! folder, or on every data set of the open document (`action.batch`).
//!
//! Fields: `set` and `action` (the action's place), `actionName`, `source` (`folder` or
//! `datasets`), `datasetFileNaming`, `folder`, `includeSubdirs`, `destination` (`none`,
//! `saveAndClose` or `folder`), `destinationFolder`, `overrideOpen`, `overrideSave`,
//! `overrideExport` and `stopOnError`. OK plays the action; a source or destination folder left
//! empty, or Data Sets without a document, is an error and the dialog stays open.

use std::hash::Hash;
use std::sync::Arc;

use craft::automation::actions::Action;
use craft::host::PickRequest;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldOk, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::patterns::form_dialogs::PathRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::{ActionDialog, Form, label, merged, set_index};
use crate::VectorcraftApp;
use crate::file_asks::{self, Owner};
use crate::panels::actions::{has_stop, is_ui_step, model};
// PLAN: assumed `playback::included_commands(&Action) -> Vec<Value>` (menus-w15) is reachable from here (`pub(crate) mod playback` or a re-export).
use crate::panels::actions::playback::included_commands;

/// The key the dialog answers for.
const KEY: &str = "actionBatch";

/// What Batch plays on.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BatchSource {
    /// Every file of a folder.
    #[default]
    #[serde(rename = "folder")]
    Folder,
    /// Every data set of the open document.
    #[serde(rename = "datasets")]
    DataSets,
}

/// What Batch does with each file or data set when the action is done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BatchDestination {
    /// Leaves it open.
    None,
    /// Saves it and closes it.
    SaveAndClose,
    /// Saves a copy in a folder.
    #[default]
    Folder,
}

/// How the files of a data set batch into a folder are named.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DatasetNaming {
    /// The document's name and a number.
    #[default]
    FileNumber,
    /// The document's name and the data set's.
    FileDataSetName,
    /// The data set's name.
    DataSetName,
}

/// What Batch edits.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BatchFields {
    /// What the action plays on.
    pub source: BatchSource,
    /// The names of the files a data set batch saves into a folder.
    pub dataset_file_naming: DatasetNaming,
    /// The source folder.
    pub folder: String,
    /// Whether the source folder's subfolders play too.
    pub include_subdirs: bool,
    /// What happens to each file or data set afterwards.
    pub destination: BatchDestination,
    /// The destination folder.
    pub destination_folder: String,
    /// Whether the action's Open steps open the batch's file instead of their own.
    pub override_open: bool,
    /// Whether the action's Save steps save into the destination folder.
    pub override_save: bool,
    /// Whether the action's Export steps export into the destination folder.
    pub override_export: bool,
    /// Whether an error ends the batch.
    pub stop_on_error: bool,
}

impl Default for BatchFields {
    fn default() -> Self {
        BatchFields {
            source: BatchSource::default(),
            dataset_file_naming: DatasetNaming::default(),
            folder: String::new(),
            include_subdirs: false,
            destination: BatchDestination::default(),
            destination_folder: String::new(),
            override_open: true,
            override_save: true,
            override_export: true,
            stop_on_error: true,
        }
    }
}

/// Batch asks for no check of its own: its refusals come with OK, from the command.
fn check(_: &BatchFields) -> Option<Label> {
    None
}

/// A folder row of the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Folder {
    /// The source folder.
    Source,
    /// The destination folder.
    Destination,
}

/// The Batch dialog on the dialog host.
pub struct BatchDialog {
    id: DialogId,
    /// Identifies this opening even when another Batch dialog reuses `id`.
    folder_owner: Arc<()>,
    /// The index of the action's set.
    pub set: usize,
    /// The index of the action in its set.
    pub action: usize,
    action_name: String,
    form: Form<BatchFields>,
    /// Whether the batch would save every data set over the same file; the pump reads the action's
    /// steps and gives the answer.
    pub(crate) warn: bool,
    /// The Browse presses the pump has not filed yet.
    asked: Vec<Folder>,
}

impl BatchDialog {
    /// The values the controls edit.
    pub(crate) fn fields(&self) -> &BatchFields {
        &self.form.values
    }

    /// The Browse presses since the last call and the opening that made them.
    pub(crate) fn take_asked(&mut self) -> (Arc<()>, Vec<Folder>) {
        (self.folder_owner.clone(), std::mem::take(&mut self.asked))
    }

    /// Writes the folder the picker answered into its row.
    fn set_folder(&mut self, which: Folder, path: String) {
        match which {
            Folder::Source => self.form.values.folder = path,
            Folder::Destination => self.form.values.destination_folder = path,
        }
    }
}

/// Open the Batch dialog for the selected action (`set`, `action` index the sets and the set's actions).
pub fn open(app: &mut VectorcraftApp, set: usize, action: usize) -> Result<Value, String> {
    let a = model::flat(&app.ui.actions, set, action).and_then(|flat| app.ui.actions.action(flat)).ok_or("no such action")?;
    if a.steps.is_empty() {
        return Err("the action has no steps".into());
    }
    if a.enabled_steps().next().is_none() {
        return Err("the action has no included steps".into());
    }
    refuse_steps(&included_commands(a))?;
    let dialog = BatchDialog {
        id: dialog_id(KEY),
        folder_owner: Arc::new(()),
        set,
        action,
        action_name: a.name.clone(),
        form: Form::new(BatchFields::default(), check),
        warn: false,
        asked: Vec::new(),
    };
    super::actions::open(app, dialog)?;
    Ok(Value::Null)
}

/// Err when the included steps `commands` hold a step that Batch does not play: a stop, or a View
/// menu step, which acts on a document window ([`is_ui_step`]).
fn refuse_steps(commands: &[Value]) -> Result<(), String> {
    if has_stop(commands) {
        return Err("Batch cannot play an action with a stop".into());
    }
    if commands.iter().any(is_ui_step) {
        return Err("Batch cannot play an action with a View menu step".into());
    }
    Ok(())
}

/// Whether the batch the dialog would run saves every data set over the same file
/// ([`vectorcraft_engine::cmd::fileio::batch_overwrites_one_file`]).
pub(crate) fn overwrites_one_file(app: &VectorcraftApp, set: usize, action: usize, fields: &BatchFields) -> bool {
    let steps = model::flat(&app.ui.actions, set, action).and_then(|flat| app.ui.actions.action(flat)).map_or(&[][..], |a| a.steps.as_slice());
    let commands = steps.iter().map(|step| json!({"command": step.command, "params": step.params})).collect();
    vectorcraft_engine::cmd::fileio::batch_overwrites_one_file(&app.session, &params(fields, "", commands))
}

/// Files the request for `which`; the answer goes to the row of the Batch dialog that asked.
// PLAN: PathRow shows Browse whether or not the host offers pickers; `file_asks::ask` says so when it cannot ask.
pub(crate) fn ask_folder(app: &mut VectorcraftApp, owner: Arc<()>, which: Folder) {
    let title = match which {
        Folder::Source => crate::i18n::t("Source Folder"),
        Folder::Destination => crate::i18n::t("Destination Folder"),
    };
    file_asks::ask(app, PickRequest::folder("batch").title(title), Owner::Free, move |app, picked| {
        if let Some(path) = picked.paths().first()
            && let Some(dialogs) = app.dialogs()
        {
            dialogs.with(|host| {
                if let Some(dialog) = host.get_mut::<BatchDialog>().filter(|d| Arc::ptr_eq(&d.folder_owner, &owner)) {
                    dialog.set_folder(which, path.to_string_lossy().into_owned());
                }
            });
        }
        Ok(Value::Null)
    });
}

/// The params of `action.batch` for the choices made: the folder, the file naming and the
/// destination folder only where the choices use them.
fn params(fields: &BatchFields, action_name: &str, commands: Vec<Value>) -> Value {
    let mut params = json!({
        "commands": commands,
        "label": action_name,
        "source": fields.source,
        "destination": fields.destination,
        "overrideOpen": fields.override_open,
        "overrideSave": fields.override_save,
        "overrideExport": fields.override_export,
        "stopOnError": fields.stop_on_error,
        "includeSubdirs": fields.include_subdirs,
    });
    if fields.source == BatchSource::Folder {
        params["folder"] = json!(fields.folder);
    }
    if names_data_set_files(fields) {
        params["datasetFileNaming"] = json!(fields.dataset_file_naming);
    }
    if fields.destination == BatchDestination::Folder {
        params["destinationFolder"] = json!(fields.destination_folder);
    }
    params
}

/// Whether the File Name choice names the batch's output: a Data Sets source with a
/// destination folder. With None or Save and Close the dialog sends no folder, so no file
/// takes the data set's name.
fn names_data_set_files(fields: &BatchFields) -> bool {
    fields.source == BatchSource::DataSets && fields.destination == BatchDestination::Folder
}

impl ActionDialog for BatchDialog {
    type Answer = (usize, usize, BatchFields);

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.form.take().map(|fields| (self.set, self.action, fields))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, (set, action, fields): Self::Answer) -> Result<(), String> {
        let a: Action = model::flat(&app.ui.actions, set, action).and_then(|flat| app.ui.actions.action(flat)).ok_or("no such action")?.clone();
        let commands = included_commands(&a);
        if commands.is_empty() {
            return Err("the action has no included steps".into());
        }
        refuse_steps(&commands)?;
        if fields.source == BatchSource::Folder && fields.folder.trim().is_empty() {
            return Err("choose a source folder".into());
        }
        if fields.destination == BatchDestination::Folder && fields.destination_folder.trim().is_empty() {
            return Err("choose a destination folder".into());
        }
        if fields.source == BatchSource::DataSets && app.session.active().is_none() {
            return Err("open a document with data sets".into());
        }
        let result = app.replay("action.batch", params(&fields, &a.name, commands))?;
        let count = result["count"].as_u64().unwrap_or(0);
        let failed = result["failed"].as_u64().unwrap_or(0);
        if failed > 0 {
            app.status(format!("Batch: {count} processed, {failed} failed"));
        } else {
            app.status(format!("Batch: {count} processed"));
        }
        Ok(())
    }
}

/// A row that picks one of `entries` for `value`.
fn select_row<T: Copy + Eq + Hash + std::fmt::Debug + Send + Sync + 'static>(ui: &mut Ui, name: &str, value: &mut T, entries: &[Entry<T>]) {
    Field::new(label(name)).show(ui, |ui, _| {
        if let Some(picked) = Select::new(label(name), Bind::given(*value), entries).show(ui).value().copied() {
            *value = picked;
        }
    });
}

/// A row with a path field and its Browse button; `asked` takes the press.
fn folder_row(ui: &mut Ui, name: &str, value: &mut String, which: Folder, asked: &mut Vec<Folder>) {
    Field::new(label(name)).show(ui, |ui, _| {
        if PathRow::new(label(name), Bind::bound(value)).show(ui).browse {
            asked.push(which);
        }
    });
}

impl StackDialog for BatchDialog {
    type Result = BatchFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<BatchFields>> {
        let sources = [Entry::new(BatchSource::Folder, label("Folder")), Entry::new(BatchSource::DataSets, label("Data Sets"))];
        let destinations = [
            Entry::new(BatchDestination::None, label("None")),
            Entry::new(BatchDestination::SaveAndClose, label("Save and Close")),
            Entry::new(BatchDestination::Folder, label("Folder")),
        ];
        let namings = [
            Entry::new(DatasetNaming::FileNumber, label("File + Number")),
            Entry::new(DatasetNaming::FileDataSetName, label("File + Data Set Name")),
            Entry::new(DatasetNaming::DataSetName, label("Data Set Name")),
        ];
        let (name, warn) = (&self.action_name, self.warn);
        let mut asked = Vec::new();
        let shown = self.form.show(ui, label("Batch"), None, |ui, fields| {
            FieldGroup::new(label("Batch")).show(ui, |ui, _| {
                Text::new(Label::from(name.clone())).show(ui);
                select_row(ui, "Play:", &mut fields.source, &sources);
                if fields.source == BatchSource::Folder {
                    folder_row(ui, "Source Folder:", &mut fields.folder, Folder::Source, &mut asked);
                    Checkbox::new(Bind::bound(&mut fields.include_subdirs)).label(label("Include All Subdirectories")).show(ui);
                } else {
                    Text::new(label("Uses every data set of the open document.")).shade(Shade::Muted).show(ui);
                }
                select_row(ui, "Destination:", &mut fields.destination, &destinations);
                if fields.destination == BatchDestination::Folder {
                    folder_row(ui, "Destination Folder:", &mut fields.destination_folder, Folder::Destination, &mut asked);
                }
                if names_data_set_files(fields) {
                    select_row(ui, "File Name:", &mut fields.dataset_file_naming, &namings);
                }
            });
            FieldGroup::new(label("Overrides")).show(ui, |ui, _| {
                Checkbox::new(Bind::bound(&mut fields.override_open)).label(label("Override Action \"Open\" Commands")).show(ui);
                Checkbox::new(Bind::bound(&mut fields.override_save)).label(label("Override Action \"Save\" Commands")).show(ui);
                Checkbox::new(Bind::bound(&mut fields.override_export)).label(label("Override Action \"Export\" Commands")).show(ui);
                Checkbox::new(Bind::bound(&mut fields.stop_on_error)).label(label("Stop on Errors")).show(ui);
            });
            if warn {
                Text::new(label("Each data set saves over the same file, which keeps only the last data set.")).shade(Shade::Muted).show(ui);
            }
        });
        self.asked.extend(asked);
        shown
    }

    fn confirm(&mut self, _: &Context) -> Option<BatchFields> {
        self.form.confirm()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.form.held_state()
    }

    fn json_fields(&self) -> Option<Value> {
        Some(merged(self.form.json_fields()?, json!({"set": self.set, "action": self.action, "actionName": self.action_name})))
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        let rejected = || FieldRefusal::Rejects(&value).message(field);
        match field {
            "set" | "action" => {
                let mut index = None;
                set_index(&mut index, field, &value)?;
                let index = index.ok_or_else(rejected)?;
                if field == "set" {
                    self.set = index
                } else {
                    self.action = index
                }
                Ok(())
            }
            "actionName" => {
                self.action_name = value.as_str().ok_or_else(rejected)?.to_owned();
                Ok(())
            }
            _ => self.form.set_field(ctx, field, value),
        }
    }
}

#[cfg(test)]
mod tests {
    use craft::automation::actions::{Action, ActionStep};
    use vectorcraft_engine::Session;

    use super::*;
    use crate::dialogs::actions::pump;

    /// An app with one set holding `steps` as the action "Draw", and a window to show dialogs in.
    fn app_with(steps: Vec<ActionStep>) -> VectorcraftApp {
        let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
        app.ui.actions = Default::default();
        app.ui.actions.push(Action { set: Some("Test".into()), ..Action::new("Draw", steps) }).unwrap();
        app
    }

    /// Sets the open dialog's fields as `ui.dialog.set` does.
    fn set_fields(app: &VectorcraftApp, fields: &[(&str, Value)]) {
        let ctx = app.ui_ctx.clone().unwrap();
        app.dialogs().unwrap().with(|host| {
            for (field, value) in fields {
                host.get_mut::<BatchDialog>().unwrap().set_json_field(&ctx, field, value.clone()).unwrap();
            }
        });
    }

    #[test]
    fn batch_folder_answers_belong_to_the_dialog_that_asked() {
        let step = ActionStep::new("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
        let mut app = app_with(vec![step]);
        let picker = crate::tests::picks::scripted(&mut app);
        let ctx = app.ui_ctx.clone().unwrap();
        let dialogs = app.dialogs().unwrap();
        let request_folders = |app: &mut VectorcraftApp| {
            dialogs.with(|host| host.get_mut::<BatchDialog>().unwrap().asked.extend([Folder::Source, Folder::Destination]));
            pump(app, &ctx);
        };
        let folders = || {
            dialogs.with(|host| {
                let fields = host.get::<BatchDialog>().unwrap().fields();
                (fields.folder.clone(), fields.destination_folder.clone())
            })
        };
        let answer = |app: &mut VectorcraftApp, source: &str, destination: &str| {
            picker.answer(&[destination]);
            picker.answer(&[source]);
            file_asks::poll(app);
        };

        open(&mut app, 0, 0).unwrap();
        request_folders(&mut app);
        assert_eq!(picker.requests().len(), 2);
        dialogs.with(|host| host.cancel::<BatchDialog>(&ctx));
        open(&mut app, 0, 0).unwrap();
        set_fields(&app, &[("folder", json!("new-source")), ("destinationFolder", json!("new-destination"))]);
        answer(&mut app, "old-source", "old-destination");
        assert_eq!(folders(), ("new-source".into(), "new-destination".into()));

        request_folders(&mut app);
        assert_eq!(picker.requests().len(), 4);
        answer(&mut app, "picked-source", "picked-destination");
        assert_eq!(folders(), ("picked-source".into(), "picked-destination".into()));
    }

    /// Batch plays an action recorded with a key its command no longer reads, as Play does.
    #[test]
    fn an_action_with_a_key_no_longer_read_still_plays_in_a_batch() {
        let old = json!({"x": 10, "y": 10, "width": 20, "height": 20, "cornerStyle": "round"});
        let mut app = app_with(vec![ActionStep::new("shape.rectangle", old.clone())]);
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("dataset.new", json!({"name": "a"})).unwrap();
        assert!(app.run("shape.rectangle", old).is_err());
        open(&mut app, 0, 0).unwrap();
        set_fields(&app, &[("source", json!("datasets")), ("destination", json!("none"))]);
        let ctx = app.ui_ctx.clone().unwrap();
        // OK files the choices, the pump answers them, and the next OK closes the dialog with them.
        assert!(!app.dialogs().unwrap().with(|host| host.confirm::<BatchDialog>(&ctx)));
        pump(&mut app, &ctx);
        assert!(app.dialogs().unwrap().with(|host| host.confirm::<BatchDialog>(&ctx)));
        assert!(!app.dialogs().unwrap().with(|host| host.is_open::<BatchDialog>()));
        assert_eq!(app.session.active().unwrap().doc.layers[0].children().unwrap().len(), 1);
        assert!(!app.session.replaying());
    }

    #[test]
    fn data_set_naming_params_follow_dialog_choice() {
        let mut fields = BatchFields::default();
        assert_eq!(fields.dataset_file_naming, DatasetNaming::FileNumber);
        fields.source = BatchSource::DataSets;
        for (choice, name) in [
            (DatasetNaming::FileNumber, "fileNumber"),
            (DatasetNaming::FileDataSetName, "fileDataSetName"),
            (DatasetNaming::DataSetName, "dataSetName"),
        ] {
            fields.dataset_file_naming = choice;
            assert_eq!(params(&fields, "Test", vec![])["datasetFileNaming"], name);
        }
        fields.dataset_file_naming = DatasetNaming::DataSetName;
        for destination in [BatchDestination::None, BatchDestination::SaveAndClose] {
            fields.destination = destination;
            let p = params(&fields, "Test", vec![]);
            assert!(p.get("datasetFileNaming").is_none(), "{destination:?}: {p}");
            assert!(p.get("destinationFolder").is_none(), "{destination:?}: {p}");
        }
        fields.destination = BatchDestination::Folder;
        fields.source = BatchSource::Folder;
        assert!(params(&fields, "Test", vec![]).get("datasetFileNaming").is_none());
    }

    #[test]
    fn data_sets_warn_about_a_recorded_save_left_in_place() {
        let save = ActionStep::new("document.save", json!({"path": "/x/a.vectorcraft"}));
        let app = app_with(vec![save]);
        let warns = |source, destination, override_save| {
            let fields = BatchFields { source, destination, override_save, destination_folder: "/out".into(), ..BatchFields::default() };
            overwrites_one_file(&app, 0, 0, &fields)
        };
        let (folder, data_sets) = (BatchSource::Folder, BatchSource::DataSets);
        assert!(!warns(folder, BatchDestination::None, true), "a folder source opens a file per pass");
        assert!(warns(data_sets, BatchDestination::None, true));
        assert!(warns(data_sets, BatchDestination::SaveAndClose, true));
        assert!(!warns(data_sets, BatchDestination::Folder, true), "Override Save points the save into the folder");
        assert!(warns(data_sets, BatchDestination::Folder, false));
    }
}
