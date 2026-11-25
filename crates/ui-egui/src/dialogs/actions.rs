//! What the Actions panel's dialogs share: the form loop with its held OK, the place a step dialog
//! writes into, the rows Action Options and New Action have in common, and the pump that answers
//! every OK of the area.
//!
//! OK of an action dialog files its values and keeps the dialog open. [`pump`] runs the command the
//! values stand for once a frame; the dialog closes when the command accepts them and shows the
//! reason under its rows when the command refuses them.

use craft::automation::actions::ActionColor;
use craft::ui::choice::Checkbox;
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogResult, FieldRefusal, HeldAnswer, HeldOk, PromptChoice, StackDialog, serde_fields, set_serde_field};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{ListWords, label_color_label};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::runtime::overlay::CloseReason;
use egui::{Context, Ui};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::action_batch::BatchDialog;
use super::action_insert_menu_item::InsertMenuItemDialog;
use super::action_new::NewActionDialog;
use super::action_select_object::SelectObjectDialog;
use super::action_set_options::SetOptionsDialog;
use super::action_stop::{RecordStopDialog, StopMessage};
use crate::VectorcraftApp;
// PLAN: `edits` and `playback` are private modules of `panels::actions` (menus-w13); the sheet calls `edits::take_answers`, `new_set` and `rename_set`, and menus-w15's `playback::included_commands`, from here, so both need `pub(crate) mod`.
use crate::panels::actions::{CONTINUE_PLAYBACK, FUNCTION_KEYS, FunctionKey, edits, key_error};

/// The translated text of `english` as a label, without the colon a field label ends with: the form
/// puts its own beside the control.
pub(crate) fn label(english: &str) -> Label {
    Label::from(crate::i18n::t(english).trim_end_matches([':', '：']).trim_end().to_owned())
}

/// What an action dialog's controls edit: a value the form can keep, compare, show as JSON and read
/// back from it.
pub(crate) trait Fields: Clone + PartialEq + Default + Send + Sync + Serialize + DeserializeOwned + 'static {}

impl<T> Fields for T where T: Clone + PartialEq + Default + Send + Sync + Serialize + DeserializeOwned + 'static {}

/// The form loop every action dialog runs: the values, the values it opened with (Reset), the
/// held OK and the check that keeps OK off.
pub(crate) struct Form<V: Fields> {
    /// What the dialog's controls edit.
    pub(crate) values: V,
    opening: V,
    held: HeldAnswer<V>,
    /// Why OK is off for these values; `None` when they can be accepted.
    check: fn(&V) -> Option<Label>,
}

impl<V: Fields> Form<V> {
    /// A form opening on `values`, whose OK is off while `check` gives a reason.
    pub(crate) fn new(values: V, check: fn(&V) -> Option<Label>) -> Self {
        Form { opening: values.clone(), values, held: HeldAnswer::new(), check }
    }

    /// Shows one pass. `ok` names the accepting button when it is not OK; `body` draws the rows and
    /// edits the values. OK files the values for [`pump`]; the pass after the app accepted them
    /// closes the dialog with them, and a refusal shows under the rows until a value changes.
    pub(crate) fn show(&mut self, ui: &mut Ui, title: Label, ok: Option<Label>, mut body: impl FnMut(&mut Ui, &mut V)) -> Option<DialogResult<V>> {
        // A measuring run closes nothing.
        let finish = self.held.is_accepted() && !craft::ui::runtime::measure::active(ui.ctx());
        let refusal = self.held.refusal().map(|(reason, refused)| (reason.clone(), refused.clone()));
        let reason = (self.check)(&self.values);
        let check = self.check;
        let mut dialog = FormDialog::new(title, Bind::bound(&mut self.values))
            .opening(self.opening.clone())
            .accept_stays_open(true)
            .ok_enabled(reason.is_none() && !self.held.is_waiting());
        if let Some(ok) = ok {
            dialog = dialog.ok_label(ok);
        }
        if let Some(reason) = reason {
            dialog = dialog.ok_reason(reason);
        }
        let shown = dialog.show(ui, |ui, form, values| {
            body(ui, values);
            if let Some((reason, _)) = refusal.as_ref().filter(|(_, refused)| *refused == *values) {
                Text::new(reason.clone()).shade(Shade::Muted).show(ui);
            }
            if finish {
                form.request_close_as(CloseReason::Activated);
            }
            check(values).is_none()
        });
        if self.held.refusal().is_some_and(|(_, refused)| *refused != self.values) {
            self.held.clear_refusal();
        }
        // A pass reports a press only in the pass that took the input, so a second pass of the frame files nothing.
        if shown.picked == Some(0) && !self.held.is_waiting() {
            self.held.file(self.values.clone());
        }
        if finish && shown.closed == Some(CloseReason::Activated) {
            return Some(self.held.finish().map_or(DialogResult::Cancel, DialogResult::Ok));
        }
        shown.closed.map(|_| DialogResult::Cancel)
    }

    /// OK as a command gives it: files the values, or answers them once the app accepted them. `None`
    /// while OK is off or the app has not answered.
    pub(crate) fn confirm(&mut self) -> Option<V> {
        if self.held.is_accepted() {
            return self.held.finish();
        }
        if self.held.is_waiting() || (self.check)(&self.values).is_some() {
            return None;
        }
        self.held.file(self.values.clone());
        None
    }

    /// The values OK filed, once.
    pub(crate) fn take(&mut self) -> Option<V> {
        self.held.take()
    }

    /// The app's answer to the filed values.
    pub(crate) fn answer(&mut self, answer: Result<(), Label>) {
        self.held.answer(answer);
    }

    /// Where the held OK stands.
    pub(crate) fn held_state(&self) -> Option<HeldOk> {
        self.held.state()
    }

    /// The values as JSON, by their serde names.
    pub(crate) fn json_fields(&self) -> Option<Value> {
        serde_fields(&self.values)
    }

    /// Sets the value `field` names as the control would; a value of another type is refused.
    pub(crate) fn set_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        set_serde_field(&mut self.values, field, value)?;
        ctx.request_repaint();
        Ok(())
    }
}

/// An index a dialog field holds: `null` for none, else a whole number.
pub(crate) fn set_index(slot: &mut Option<usize>, field: &str, value: &Value) -> Result<(), String> {
    let index = match value {
        Value::Null => None,
        other => Some(other.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or_else(|| FieldRefusal::Rejects(value).message(field))?),
    };
    *slot = index;
    Ok(())
}

/// Adds the entries of the object `extra` to the object `fields`.
pub(crate) fn merged(mut fields: Value, extra: Value) -> Value {
    if let (Some(into), Value::Object(extra)) = (fields.as_object_mut(), extra) {
        into.extend(extra);
    }
    fields
}

/// Where the new step of a step dialog goes: the action at (`set`, `action`) after step `step`, else
/// at its end; without a place, into the recording in progress. It stays out of the form's values so
/// Reset keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Target {
    /// The index of the set.
    pub set: Option<usize>,
    /// The index of the action in the set.
    pub action: Option<usize>,
    /// The step the new one goes after.
    pub step: Option<usize>,
}

impl Target {
    /// The place `open` gets: `step` means nothing without an action.
    pub(crate) fn new(place: Option<(usize, usize)>, step: Option<usize>) -> Self {
        Target { set: place.map(|(set, _)| set), action: place.map(|(_, action)| action), step: place.and(step) }
    }

    /// The place as JSON fields.
    pub(crate) fn json(self) -> Value {
        json!({"set": self.set, "action": self.action, "step": self.step})
    }

    /// Sets the place field `field` names; `None` when `field` is not one.
    pub(crate) fn set_field(&mut self, field: &str, value: &Value) -> Option<Result<(), String>> {
        let slot = match field {
            "set" => &mut self.set,
            "action" => &mut self.action,
            "step" => &mut self.step,
            _ => return None,
        };
        Some(set_index(slot, field, value))
    }

    /// Adds the keys the place has to the params of the insert command.
    pub(crate) fn add_to(self, params: &mut Value) {
        for (key, index) in [("set", self.set), ("action", self.action), ("step", self.step)] {
            if let Some(index) = index {
                params[key] = json!(index);
            }
        }
    }
}

/// The function key, color and Align To choice of an action: what New Action and Action Options
/// share.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionOptions {
    /// 0 for none, 1 to 15 for F1 to F15.
    pub key: u8,
    /// Ctrl, Command on macOS.
    pub cmd: bool,
    /// Shift.
    pub shift: bool,
    /// The Button Mode color; its file name in JSON.
    pub color: ActionColor,
    /// Whether Align at playback takes its reference from the live selection.
    pub automatic_align_to: bool,
}

/// The JSON names of the fields [`ActionOptions`] adds to a dialog's values.
pub(crate) const OPTION_KEYS: [&str; 5] = ["key", "cmd", "shift", "color", "automaticAlignTo"];

/// Why OK is off for the function key chosen, when the system or the platform keeps it.
pub(crate) fn key_reason(options: &ActionOptions) -> Option<Label> {
    if options.key == 0 {
        return None;
    }
    key_error(FunctionKey { key: options.key, cmd: options.cmd, shift: options.shift }).map(|message| Label::from(crate::i18n::msg(&message)))
}

/// The Function Key row (the key, Ctrl or ⌘, Shift) and the Color row.
pub(crate) fn key_and_color(ui: &mut Ui, options: &mut ActionOptions) {
    let keys: Vec<Entry<u8>> =
        std::iter::once(Entry::new(0, label("None"))).chain((1..=FUNCTION_KEYS).map(|n| Entry::new(n, Label::from(format!("F{n}"))))).collect();
    let modifier = if crate::keymap::platform() == craft::keymap::Platform::Mac { "⌘" } else { "Ctrl" };
    Field::new(label("Function Key:")).show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            if let Some(key) = Select::new(label("Function Key:"), Bind::given(options.key), &keys).show(ui).value().copied() {
                options.key = key;
            }
            Checkbox::new(Bind::bound(&mut options.cmd)).label(modifier).show(ui);
            Checkbox::new(Bind::bound(&mut options.shift)).label(label("Shift")).show(ui);
        });
    });
    let env = Env::of(ui);
    let words = env.words::<ListWords>();
    let colors: Vec<Entry<ActionColor>> =
        ActionColor::ALL.into_iter().map(|color| Entry::new(color, label_color_label(color.label_color(), &words))).collect();
    Field::new(label("Color:")).show(ui, |ui, _| {
        if let Some(color) = Select::new(label("Color:"), Bind::given(options.color), &colors).show(ui).value().copied() {
            options.color = color;
        }
    });
}

/// The Align playback choice and what it does.
pub(crate) fn automatic_align_options(ui: &mut Ui, options: &mut ActionOptions) {
    Checkbox::new(Bind::bound(&mut options.automatic_align_to)).label(label("Choose Align To at playback")).show(ui);
    for line in [
        "Align steps use the key object, key anchor or key guide when one is set.",
        "With one whole object selected and no key, they use the artboard.",
        "Otherwise they use the selection.",
    ] {
        Text::new(label(line)).shade(Shade::Muted).show(ui);
    }
}

/// The function key, color and Align params `ui.action.new` and `ui.action.options` take.
pub(crate) fn key_and_color_params(options: &ActionOptions) -> Value {
    json!({
        "key": options.key,
        "cmd": options.cmd,
        "shift": options.shift,
        "color": options.color,
        "automaticAlignTo": options.automatic_align_to,
    })
}

/// Opens `dialog` on the window's dialog host; `Err` when the app has no window to show it in.
pub(crate) fn open<D: StackDialog>(app: &VectorcraftApp, dialog: D) -> Result<(), String> {
    let dialogs = app.dialogs().ok_or("no window")?;
    dialogs.with(|host| host.open(dialog));
    Ok(())
}

/// [`open`] for the openers that return nothing: a missing window is said in the status line.
pub(crate) fn open_or_say<D: StackDialog>(app: &mut VectorcraftApp, dialog: D) {
    if let Err(message) = open(app, dialog) {
        app.status(message);
    }
}

/// A dialog of this area whose filed OK [`pump`] answers by running a command.
pub(crate) trait ActionDialog: StackDialog + Sized {
    /// What OK files, with the place the dialog holds when it was filed.
    type Answer;

    /// The answer OK filed, once.
    fn take_ok(&mut self) -> Option<Self::Answer>;

    /// The app's answer: `Ok` closes the dialog, `Err` keeps it open with the reason.
    fn answer_ok(&mut self, answer: Result<(), Label>);

    /// Runs the command the answer stands for.
    fn run(app: &mut VectorcraftApp, answer: Self::Answer) -> Result<(), String>;
}

/// Answers the filed OK of the open `D`, if there is one; whether it answered one. A finished
/// dialog's result carries nothing the app needs, so it is taken to keep the host tidy.
fn settle<D: ActionDialog>(app: &mut VectorcraftApp) -> bool {
    let Some(dialogs) = app.dialogs() else { return false };
    let filed = dialogs.with(|host| {
        let _ = host.take::<D>();
        host.get_mut::<D>().and_then(D::take_ok)
    });
    let Some(answer) = filed else { return false };
    // The host is not locked while the command runs: a command may follow or close dialogs.
    let outcome = D::run(app, answer).map_err(Label::from);
    dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<D>() {
            dialog.answer_ok(outcome);
        }
    });
    true
}

/// Gives the open dialogs the app data they show: the set names, the Batch warning, the Insert Menu
/// Item rows.
fn push_app_data(app: &VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let sets: Vec<String> = app.ui.actions.sets().into_iter().map(str::to_owned).collect();
    let batch = dialogs.with(|host| {
        if let Some(dialog) = host.get_mut::<NewActionDialog>() {
            dialog.set_sets(&sets);
        }
        host.get::<BatchDialog>().map(|dialog| (dialog.set, dialog.action, dialog.fields().clone()))
    });
    if let Some((set, action, fields)) = batch {
        let warn = super::action_batch::overwrites_one_file(app, set, action, &fields);
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut::<BatchDialog>() {
                dialog.warn = warn;
            }
        });
    }
    let key = InsertMenuItemDialog::rows_key(app);
    if dialogs.with(|host| host.get::<InsertMenuItemDialog>().is_some_and(|dialog| dialog.built_from() != Some(key))) {
        let rows =
            super::action_insert_menu_item::build(crate::i18n::current(), app.session.prefs.font_names_in_english, &crate::menus::Listed::of(app));
        dialogs.with(|host| {
            if let Some(dialog) = host.get_mut::<InsertMenuItemDialog>() {
                dialog.set_rows(key, rows);
            }
        });
    }
}

/// The folder picks the open Batch dialog asked for: each goes to the app's folder request, whose
/// answer is written into the dialog.
fn ask_batch_folders(app: &mut VectorcraftApp) {
    let Some(dialogs) = app.dialogs() else { return };
    let asked = dialogs.with(|host| host.get_mut::<BatchDialog>().map(BatchDialog::take_asked).unwrap_or_default());
    for which in asked {
        super::action_batch::ask_folder(app, which);
    }
}

/// Runs once a frame after the shell pass: gives the open dialogs the app data they show, answers
/// every OK the dialogs of the Actions panel filed, and takes the answer of a stop's message box.
/// A pass answers only the OK filed in its own pass, so a second pass of the frame finds none.
pub(crate) fn pump(app: &mut VectorcraftApp, ctx: &Context) {
    push_app_data(app);
    ask_batch_folders(app);
    let mut answered = settle::<NewActionDialog>(app);
    answered |= settle::<SetOptionsDialog>(app);
    answered |= settle::<RecordStopDialog>(app);
    answered |= settle::<InsertMenuItemDialog>(app);
    answered |= settle::<SelectObjectDialog>(app);
    answered |= settle::<super::action_playback::PlaybackDialog>(app);
    answered |= settle::<BatchDialog>(app);
    if let Some(dialogs) = app.dialogs() {
        let proceed =
            dialogs.with(|host| matches!(host.take::<StopMessage>(), Some(DialogResult::Ok(answer)) if answer.choice == PromptChoice::Proceed));
        if proceed {
            if let Err(message) = app.run(CONTINUE_PLAYBACK, json!({})) {
                app.status(message);
            }
            answered = true;
        }
    }
    edits::take_answers(app);
    if answered {
        // The dialog that was answered closes (or shows its reason) in the next pass.
        ctx.request_repaint();
    }
}
