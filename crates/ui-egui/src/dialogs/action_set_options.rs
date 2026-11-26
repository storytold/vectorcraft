//! Actions panel › Create New Set (and ≡ › New Set…), ≡ › Set Options… and ≡ › Action Options…:
//! the name of a new action set, of the current one, or of the selected action.
//!
//! Fields: `set` (the index of the set renamed, or of the action's set; not for a new set),
//! `action` (the index of the action renamed, in `set`; only for Action Options) and `name`. Action
//! Options also holds the action's function key and color (`key`, `cmd`, `shift`, `color`,
//! `automaticAlignTo`, see [`ActionOptions`]). OK adds the set and makes it the current one,
//! renames the set, or changes the action's name, function key and color (`ui.action.options`).
//! An invalid `set`, `action` or function key is an error, and so is a name the list refuses. OK
//! then changes no set or action, and the dialog stays open.

use craft::ui::contract::{Bind, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogId, DialogResult, DialogStack, FieldRefusal, HeldOk, StackDialog, dialog_id};
use craft::ui::form::{Field, FieldGroup};
use craft::ui::runtime::env::Label;
use craft::ui::text::TextField;
use egui::{Context, Ui};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::actions::{
    ActionDialog, ActionOptions, Form, OPTION_KEYS, automatic_align_options, key_and_color, key_and_color_params, key_reason, label, merged,
    open_or_say, set_index,
};
use crate::VectorcraftApp;
use crate::panels::actions::{ACTION_OPTIONS, edits, model};

/// The key the dialog answers for.
const KEY: &str = "actionSetOptions";

/// What the dialog is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SetOptionsKind {
    /// A set to add.
    NewSet,
    /// The name of a set.
    Set,
    /// The name, function key and color of an action.
    Action,
}

/// What the dialog edits.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetOptionsFields {
    /// The set's or the action's name.
    pub name: String,
    /// The function key, color and Align choice; only an action has them.
    #[serde(flatten)]
    pub options: ActionOptions,
}

/// Why OK is off: no name, or a function key the system keeps.
fn check(fields: &SetOptionsFields) -> Option<Label> {
    if fields.name.trim().is_empty() {
        return Some(label("Enter a name"));
    }
    key_reason(&fields.options)
}

/// The New Set, Set Options and Action Options dialog on the dialog host.
pub struct SetOptionsDialog {
    id: DialogId,
    /// What the dialog is for.
    pub kind: SetOptionsKind,
    /// The index of the set: of the one renamed, or of the action's; none for a new set.
    pub set: Option<usize>,
    /// The index of the action in its set; only for Action Options.
    pub action: Option<usize>,
    form: Form<SetOptionsFields>,
}

impl SetOptionsDialog {
    fn new(kind: SetOptionsKind, set: Option<usize>, action: Option<usize>, fields: SetOptionsFields) -> Self {
        SetOptionsDialog { id: dialog_id(KEY), kind, set, action, form: Form::new(fields, check) }
    }
}

/// Open New Set, named "Set N" with the first number no set uses.
pub fn open_new(app: &mut VectorcraftApp) {
    let sets = app.ui.actions.sets();
    let n = (1..).find(|n| !sets.contains(&format!("Set {n}").as_str())).unwrap_or(1);
    let fields = SetOptionsFields { name: format!("Set {n}"), ..SetOptionsFields::default() };
    open_or_say(app, SetOptionsDialog::new(SetOptionsKind::NewSet, None, None, fields));
}

/// Open Set Options for the set at `set`, if there is one.
pub fn open(app: &mut VectorcraftApp, set: usize) {
    if let Some(name) = model::set_name(&app.ui.actions, set) {
        let fields = SetOptionsFields { name: name.to_owned(), ..SetOptionsFields::default() };
        open_or_say(app, SetOptionsDialog::new(SetOptionsKind::Set, Some(set), None, fields));
    }
}

/// Open Action Options for the action at `action` in set `set`, if there is one, with its name,
/// function key and color.
pub fn open_action(app: &mut VectorcraftApp, set: usize, action: usize) {
    let Some(a) = model::flat(&app.ui.actions, set, action).and_then(|flat| app.ui.actions.action(flat)) else { return };
    let key = model::function_key(a);
    let options = ActionOptions {
        key: key.map_or(0, |k| k.key),
        cmd: key.is_some_and(|k| k.cmd),
        shift: key.is_some_and(|k| k.shift),
        color: a.color,
        automatic_align_to: model::automatic_align_to(a),
    };
    let fields = SetOptionsFields { name: a.name.clone(), options };
    open_or_say(app, SetOptionsDialog::new(SetOptionsKind::Action, Some(set), Some(action), fields));
}

impl ActionDialog for SetOptionsDialog {
    /// What OK files, with the kind and place the dialog had.
    type Answer = (SetOptionsKind, Option<usize>, Option<usize>, SetOptionsFields);

    fn take_ok(&mut self) -> Option<Self::Answer> {
        self.form.take().map(|fields| (self.kind, self.set, self.action, fields))
    }

    fn answer_ok(&mut self, answer: Result<(), Label>) {
        self.form.answer(answer);
    }

    fn run(app: &mut VectorcraftApp, (kind, set, action, fields): Self::Answer) -> Result<(), String> {
        match kind {
            SetOptionsKind::NewSet => edits::new_set(app, &fields.name),
            SetOptionsKind::Set => {
                let current = set.and_then(|set| model::set_name(&app.ui.actions, set)).ok_or("no such action set")?.to_owned();
                edits::rename_set(app, &current, &fields.name)
            }
            SetOptionsKind::Action => {
                let mut params = key_and_color_params(&fields.options);
                params["set"] = json!(set);
                params["action"] = json!(action);
                params["name"] = json!(fields.name.trim());
                app.run(ACTION_OPTIONS, params).map(|_| ())
            }
        }
    }
}

impl StackDialog for SetOptionsDialog {
    type Result = SetOptionsFields;

    fn id(&self) -> DialogId {
        self.id
    }

    fn key(&self) -> Option<&str> {
        Some(KEY)
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<SetOptionsFields>> {
        let kind = self.kind;
        let title = match kind {
            SetOptionsKind::Action => label("Action Options"),
            SetOptionsKind::Set => label("Set Options"),
            SetOptionsKind::NewSet => label("New Set"),
        };
        self.form.show(ui, title.clone(), None, |ui, fields| {
            FieldGroup::new(title.clone()).show(ui, |ui, _| {
                Field::new(label("Name:")).show(ui, |ui, _| {
                    TextField::new(label("Name:"), Bind::bound(&mut fields.name)).show(ui);
                });
                if kind == SetOptionsKind::Action {
                    key_and_color(ui, &mut fields.options);
                    automatic_align_options(ui, &mut fields.options);
                }
            });
        })
    }

    fn confirm(&mut self, _: &Context) -> Option<SetOptionsFields> {
        self.form.confirm()
    }

    fn held_state(&self) -> Option<HeldOk> {
        self.form.held_state()
    }

    /// The fields the kind has: the place and name, and the action's options for Action Options.
    fn json_fields(&self) -> Option<Value> {
        let mut fields = self.form.json_fields()?;
        if self.kind != SetOptionsKind::Action
            && let Some(map) = fields.as_object_mut()
        {
            for key in OPTION_KEYS {
                map.remove(key);
            }
        }
        let mut place = serde_json::Map::new();
        if self.kind != SetOptionsKind::NewSet {
            place.insert("set".into(), json!(self.set));
        }
        if self.kind == SetOptionsKind::Action {
            place.insert("action".into(), json!(self.action));
        }
        Some(merged(fields, Value::Object(place)))
    }

    fn set_json_field(&mut self, ctx: &Context, field: &str, value: Value) -> Result<(), String> {
        match field {
            "set" if self.kind != SetOptionsKind::NewSet => set_index(&mut self.set, field, &value),
            "action" if self.kind == SetOptionsKind::Action => set_index(&mut self.action, field, &value),
            _ if OPTION_KEYS.contains(&field) && self.kind != SetOptionsKind::Action => Err(FieldRefusal::Inactive.message(field)),
            _ => self.form.set_field(ctx, field, value),
        }
    }
}
