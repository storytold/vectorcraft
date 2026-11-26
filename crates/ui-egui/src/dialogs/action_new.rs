//! Actions panel › Create New Action (and ≡ › New Action…): the name, set, function key and color
//! of a new action. Record adds the action and starts recording into it (`ui.action.new`).
//!
//! Fields: `set` (the index of the set the action goes into), `name`, `key` (0 for none, 1 to 15
//! for F1 to F15), `cmd` (Ctrl, Command on macOS), `shift`, `color` (the file name of an
//! [`ActionColor`](craft::automation::actions::ActionColor), `""` for none) and `automaticAlignTo`,
//! which chooses the live selection's reference for Align at playback while keeping recorded step
//! targets. An invalid set or function key is an error: Record then adds nothing and the dialog
//! stays open.

use craft::automation::actions::ActionList;
use craft::ui::contract::{Bind, Entry, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldOk, StackDialog, dialog_id};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::pickers::Select;
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::{ActionDialog, ActionOptions, Form, automatic_align_options, key_and_color, key_and_color_params, key_reason, label};
use crate::VectorcraftApp;
use crate::panels::actions::{NEW_ACTION, model};

/// The key the dialog answers for.
const KEY: &str = "actionNew";

/// What New Action edits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewActionFields {
    /// The index of the set the action goes into.
    pub set: usize,
    /// The action's name.
    pub name: String,
    /// The function key, color and Align choice.
    #[serde(flatten)]
    pub options: ActionOptions,
}

/// Why OK is off: no name, or a function key the system keeps.
fn check(fields: &NewActionFields) -> Option<Label> {
    if fields.name.trim().is_empty() {
        return Some(label("Enter a name for the action"));
    }
    key_reason(&fields.options)
}

/// The New Action dialog on the dialog host.
pub struct NewActionDialog {
    id: DialogId,
    form: Form<NewActionFields>,
    /// The names of the sets the Action Set row offers, as the app lists them.
    sets: Vec<String>,
}

impl NewActionDialog {
    fn new(fields: NewActionFields, sets: Vec<String>) -> Self {
        NewActionDialog { id: dialog_id(KEY), form: Form::new(fields, check), sets }
    }

    /// Gives the Action Set row the app's set names.
    pub(crate) fn set_sets(&mut self, sets: &[String]) {
        if self.sets != sets {
            self.sets = sets.to_vec();
        }
    }

    /// The index of the set the action goes into, for a set edit that moves the sets to follow.
    pub(crate) fn set_mut(&mut self) -> &mut usize {
        &mut self.form.values.set
    }
}

/// Whether New Action can open: there is a set to put the action in and nothing is recording.
pub fn can_open(app: &VectorcraftApp) -> bool {
    !app.ui.actions.sets().is_empty() && app.ui.recording.is_none()
}

/// "Action N" with the first number from the set's action count plus one that no action of the
/// list uses: action names are unique in the whole list.
fn free_name(list: &ActionList, set: &str) -> String {
    let taken = |n: usize| list.actions().iter().any(|action| action.name == format!("Action {n}"));
    let n = (model::members(list, set).len() + 1..).find(|&n| !taken(n)).unwrap_or(1);
    format!("Action {n}")
}

/// Opens New Action for the current set, else the first set, named by [`free_name`], with no
/// function key and no color.
pub fn open(app: &mut VectorcraftApp) -> Result<(), String> {
    if app.ui.recording.is_some() {
        return Err("an action is already recording".into());
    }
    let sets: Vec<String> = app.ui.actions.sets().into_iter().map(str::to_owned).collect();
    // PLAN: assumed `app.ui.action_set` is the current set's name (menus-w13 `deserialize_current_set`).
    let set = app.ui.action_set.as_deref().and_then(|current| sets.iter().position(|s| s == current)).unwrap_or(0);
    let name = sets.get(set).map(|set| free_name(&app.ui.actions, set)).ok_or("no such action set")?;
    let fields = NewActionFields { set, name, options: ActionOptions::default() };
    super::actions::open(app, NewActionDialog::new(fields, sets))
}

impl ActionDialog for NewActionDialog {
    type Answer = NewActionFields;

    fn take_ok(&mut self) -> Option<NewActionFields> {
        self.form.take()
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, fields: NewActionFields) -> Result<(), String> {
        let mut params = key_and_color_params(&fields.options);
        params["set"] = json!(fields.set);
        params["name"] = json!(fields.name.trim());
        app.run(NEW_ACTION, params).map(|_| ())
    }
}

impl StackDialog for NewActionDialog {
    type Result = NewActionFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<NewActionFields>> {
        let sets: Vec<Entry<usize>> = self.sets.iter().enumerate().map(|(i, set)| Entry::new(i, Label::from(set.clone()))).collect();
        self.form.show(ui, label("New Action"), Some(label("Record")), |ui, fields| {
            FieldGroup::new(label("New Action")).show(ui, |ui, _| {
                Field::new(label("Name:")).show(ui, |ui, _| {
                    TextField::new(label("Name:"), Bind::bound(&mut fields.name)).show(ui);
                });
                Field::new(label("Action Set:")).show(ui, |ui, _| {
                    if let Some(set) = Select::new(label("Action Set:"), Bind::given(fields.set), &sets).show(ui).value().copied() {
                        fields.set = set;
                    }
                });
                key_and_color(ui, &mut fields.options);
                automatic_align_options(ui, &mut fields.options);
            });
        })
    }

    fn confirm(&mut self, _: &Context) -> Option<NewActionFields> {
        self.form.confirm()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.form.held_state()
    }

    fn json_fields(&self) -> Option<Value> {
        self.form.json_fields()
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        self.form.set_field(ctx, field, value)
    }
}
