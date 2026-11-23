//! Actions panel menu › Insert Stop… (`ui.action.insertStop`) and the message box a playing action
//! shows at a stop.
//!
//! Record Stop's fields: `message`, `allowContinue`, and `set`, `action` and `step`, the place of the
//! new stop (no `set` and `action`: the recording in progress). An invalid place is an error, and
//! the dialog stays open.
//!
//! The message box is a standard prompt titled with the action's name. Stop or Cancel closes it, and
//! the playback keeps waiting for Play Current Selection or Stop Playing/Recording. With
//! `allowContinue`, Continue plays on.

use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, HeldOk, PromptChoice, StackDialog, StandardPrompt, dialog_id};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::{ActionDialog, Form, Target, label, merged, open_or_say};
use crate::VectorcraftApp;
use crate::panels::actions::INSERT_STOP;

/// The key of a stop's message box.
pub const MESSAGE_KIND: &str = "actionStopMessage";

/// The key Record Stop answers for.
const KEY: &str = "actionStopRecord";

/// A stop's message box on the dialog host. The wrapper keeps its answer apart from the app's other
/// prompts, which take the bare [`StandardPrompt`]'s.
pub(crate) struct StopMessage(pub(crate) StandardPrompt);

crate::dialogs::confirm::wrapped_dialog!(StopMessage, StandardPrompt, MESSAGE_KIND);

/// What Record Stop edits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordStopFields {
    /// The text playback shows at the stop.
    pub message: String,
    /// Whether the message box offers Continue.
    pub allow_continue: bool,
}

/// OK is always on: a stop may have no message.
fn check(_: &RecordStopFields) -> Option<Label> {
    None
}

/// The Record Stop dialog on the dialog host.
pub struct RecordStopDialog {
    id: DialogId,
    /// Where the stop goes.
    pub target: Target,
    form: Form<RecordStopFields>,
}

/// Opens Record Stop for action `target` as (set, action), after `step`, else for the recording in
/// progress.
pub fn open_record(app: &mut VectorcraftApp, target: Option<(usize, usize)>, step: Option<usize>) {
    let dialog = RecordStopDialog { id: dialog_id(KEY), target: Target::new(target, step), form: Form::new(RecordStopFields::default(), check) };
    open_or_say(app, dialog);
}

impl ActionDialog for RecordStopDialog {
    type Answer = (Target, RecordStopFields);

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.form.take().map(|fields| (self.target, fields))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, (target, fields): Self::Answer) -> Result<(), String> {
        let mut params = json!({"message": fields.message, "allowContinue": fields.allow_continue});
        target.add_to(&mut params);
        app.run(INSERT_STOP, params).map(|_| ())
    }
}

impl StackDialog for RecordStopDialog {
    type Result = RecordStopFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<RecordStopFields>> {
        self.form.show(ui, label("Record Stop"), None, |ui, fields| {
            FieldGroup::new(label("Record Stop")).show(ui, |ui, _| {
                Field::new(label("Message:")).show(ui, |ui, _| {
                    TextField::new(label("Message:"), Bind::bound(&mut fields.message)).show(ui);
                });
                Checkbox::new(Bind::bound(&mut fields.allow_continue)).label(label("Allow Continue")).show(ui);
            });
        })
    }

    fn confirm(&mut self, _: &Context) -> Option<RecordStopFields> {
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

/// Shows the message box of a stop in the action named `action`.
pub(crate) fn open_message(app: &mut VectorcraftApp, action: &str, message: &str, allow_continue: bool) {
    let mut prompt = StandardPrompt::custom(Label::from(message.to_owned()))
        .title(Label::from(action.to_owned()))
        .paragraph(label("Press Play Current Selection in the Actions panel to go on."));
    if allow_continue {
        prompt = prompt.button(PromptChoice::Proceed, label("Continue"));
    }
    open_or_say(app, StopMessage(prompt.button(PromptChoice::Cancel, label("Stop"))));
}

/// Closes a stop's message box as Stop does; the playback keeps waiting. The closer consumes the
/// Cancel it caused, so the pump does not read it as an answer.
pub(crate) fn close_message(app: &VectorcraftApp) {
    let (Some(dialogs), Some(ctx)) = (app.dialogs(), app.ui_ctx.clone()) else { return };
    dialogs.with(|host| {
        if host.cancel::<StopMessage>(&ctx) {
            let _ = host.take::<StopMessage>();
        }
    });
}
