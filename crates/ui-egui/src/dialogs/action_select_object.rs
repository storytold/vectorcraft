//! Actions panel menu › Select Object… (`ui.action.insertSelectObject`): Set Selection, which adds a
//! step that selects the objects whose Attributes note matches the note typed.
//!
//! Its fields: `note`, `caseSensitive`, `wholeWord`, and `set`, `action` and `step`, the place of the
//! new step (no `set` and `action`: the recording in progress). OK is off while the note is empty.
//! An invalid place is an error, and the dialog stays open.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldOk, StackDialog, dialog_id};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::{ActionDialog, Form, Target, label, merged, open_or_say};
use crate::VectorcraftApp;
use crate::panels::actions::INSERT_SELECT_OBJECT;

/// The key the dialog answers for.
const KEY: &str = "actionSelectObject";

/// What Set Selection edits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectObjectFields {
    /// The words the objects' Attributes notes are searched for.
    pub note: String,
    /// Whether the search tells upper and lower case apart.
    pub case_sensitive: bool,
    /// Whether a match must be a whole word.
    pub whole_word: bool,
}

/// Why OK is off: nothing to search for.
fn check(fields: &SelectObjectFields) -> Option<Label> {
    fields.note.trim().is_empty().then(|| label("Enter the note to select by"))
}

/// The Set Selection dialog on the dialog host.
pub struct SelectObjectDialog {
    id: DialogId,
    /// Where the step goes.
    pub target: Target,
    form: Form<SelectObjectFields>,
}

/// Opens Set Selection for action `target` as (set, action), after `step`, else for the recording
/// in progress.
pub fn open(app: &mut VectorcraftApp, target: Option<(usize, usize)>, step: Option<usize>) {
    let dialog = SelectObjectDialog { id: dialog_id(KEY), target: Target::new(target, step), form: Form::new(SelectObjectFields::default(), check) };
    open_or_say(app, dialog);
}

impl ActionDialog for SelectObjectDialog {
    type Answer = (Target, SelectObjectFields);

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.form.take().map(|fields| (self.target, fields))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, (target, fields): Self::Answer) -> Result<(), String> {
        let mut params = json!({"note": fields.note, "caseSensitive": fields.case_sensitive, "wholeWord": fields.whole_word});
        target.add_to(&mut params);
        app.run(INSERT_SELECT_OBJECT, params).map(|_| ())
    }
}

impl StackDialog for SelectObjectDialog {
    type Result = SelectObjectFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SelectObjectFields>> {
        self.form.show(ui, label("Set Selection"), None, |ui, fields| {
            FieldGroup::new(label("Set Selection")).show(ui, |ui, _| {
                Field::new(label("Note:")).show(ui, |ui, _| {
                    TextField::new(label("Note:"), Bind::bound(&mut fields.note)).show(ui);
                });
                Checkbox::new(Bind::bound(&mut fields.case_sensitive)).label(label("Case Sensitive")).show(ui);
                Checkbox::new(Bind::bound(&mut fields.whole_word)).label(label("Whole Word")).show(ui);
            });
        })
    }

    fn confirm(&mut self, _: &Context) -> Option<SelectObjectFields> {
        self.form.confirm()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.form.held_state()
    }

    fn json_fields(&self) -> Option<Value> {
        Some(merged(self.form.json_fields()?, self.target.json()))
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        match self.target.set_field(field, &value) {
            Some(done) => done,
            None => self.form.set_field(ctx, field, value),
        }
    }
}
