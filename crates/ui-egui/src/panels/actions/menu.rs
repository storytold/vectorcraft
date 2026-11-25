//! The Actions panel's menu: its rows, their states and what each one runs.

use std::hash::{Hash, Hasher};

use craft::automation::ui::actions::{ActionSelection, ActionsPanelCommands};
use craft::keymap::CommandRef;
use craft::ui::menu::PanelRow;
use craft::ui::runtime::commands::{CommandState, CommandStates};
use serde_json::{Value, json};

use super::edits::{self, BUTTON_MODE, INSERT_SELECT_PATH};
use super::files::{LOAD_SET, REPLACE_SET, SAVE_SET, can_load};
use super::playback;
use crate::VectorcraftApp;

/// The command that opens Set Selection on the selected step or action.
const SELECT_OBJECT: &str = "ui.action.selectObjectDialog";
/// The command that opens the selected action's options.
const OPTIONS: &str = "ui.action.optionsDialog";

/// A row of the menu: one of the framework's commands, or one it has no command for.
#[derive(Clone, Copy, PartialEq)]
enum Row {
    Panel(ActionsPanelCommands),
    SelectObject,
    Options,
}

impl Row {
    fn command(self) -> CommandRef {
        match self {
            Row::Panel(c) => CommandRef::from(c),
            Row::SelectObject => CommandRef::new(SELECT_OBJECT),
            Row::Options => CommandRef::new(OPTIONS),
        }
    }
}

/// The rows in menu order, in groups the menu separates. Start and stop recording are the panel's
/// buttons, not menu rows.
const GROUPS: [&[Row]; 3] = {
    use ActionsPanelCommands as C;
    [
        &[Row::Panel(C::ButtonMode)],
        &[
            Row::Panel(C::NewAction),
            Row::Panel(C::NewSet),
            Row::Panel(C::Duplicate),
            Row::Panel(C::Delete),
            Row::Panel(C::SetOptions),
            Row::Panel(C::Batch),
            Row::Panel(C::Play),
            Row::Panel(C::InsertStop),
            Row::Panel(C::InsertMenuItem),
            Row::Panel(C::InsertPath),
            Row::SelectObject,
            Row::Options,
            Row::Panel(C::PlaybackOptions),
        ],
        &[Row::Panel(C::Save), Row::Panel(C::Load), Row::Panel(C::Replace), Row::Panel(C::ClearAll), Row::Panel(C::Reset)],
    ]
};

/// The rows of the panel menu.
pub(crate) fn menu_rows(_: &VectorcraftApp) -> Vec<PanelRow> {
    let mut rows = Vec::new();
    for (i, group) in GROUPS.iter().enumerate() {
        if i > 0 {
            rows.push(PanelRow::Separator);
        }
        rows.extend(group.iter().map(|row| PanelRow::Command(row.command())));
    }
    rows
}

/// What the menu's states read from the app.
struct Facts {
    buttons: bool,
    recording: bool,
    idle: bool,
    waiting: bool,
    /// The set the panel shows as current, none in Button Mode.
    current: Option<usize>,
    /// The selected action as (set, index in the set), only while its set is the current one.
    selected: Option<(usize, usize)>,
    /// The selected step of that action, only while the panel shows its steps.
    step: Option<usize>,
    sets: usize,
    selected_sets: bool,
    path: bool,
}

impl Facts {
    fn of(app: &VectorcraftApp) -> Self {
        let panel = &app.ui.actions_panel;
        let buttons = panel.settings.button_mode;
        let list = &app.ui.actions;
        let names = list.sets();
        let sets = names.len();
        let recording = app.ui.recording.is_some();
        let current = app.ui.action_set.as_deref().and_then(|set| names.iter().position(|n| *n == set)).filter(|_| !buttons);
        // A selected action counts only while its set is the current one, as the panel shows it.
        let (flat, step) = match panel.selected {
            Some(ActionSelection::Action(i)) => (Some(i), None),
            Some(ActionSelection::Step(i, s)) => (Some(i), Some(s)),
            _ => (None, None),
        };
        let place = flat.and_then(|i| super::model::place(list, i)).filter(|&(s, _)| current == Some(s));
        let step = step.filter(|&s| {
            flat.and_then(|i| list.action(i)).is_some_and(|a| panel.expanded.contains(&a.name)) && flat.is_some_and(|i| s < list.step_count(i))
        });
        Facts {
            buttons,
            recording,
            idle: !recording && app.ui.action_playing.is_none(),
            waiting: playback::waiting_at_stop(app),
            current,
            selected: place,
            step: step.filter(|_| place.is_some()),
            sets,
            selected_sets: !panel.selected_sets.is_empty(),
            path: app.session.active().is_some_and(|st| vectorcraft_engine::storable_selection(st).is_ok()),
        }
    }

    fn has(&self) -> bool {
        self.selected.is_some()
    }
}

/// The menu's states: production's rules, set into the frame's one `CommandStates`.
pub(crate) fn menu_states(app: &VectorcraftApp, states: &mut CommandStates) {
    use ActionsPanelCommands as C;
    let f = Facts::of(app);
    let (has, recording) = (f.has(), f.recording);
    // Button Mode is off while an action records, so the Stop button the list shows stays in reach.
    states.set(Row::Panel(C::ButtonMode).command(), CommandState::new().enabled(f.buttons || !recording).checked(f.buttons));
    let mut set = |row: Row, enabled: bool| states.set(row.command(), CommandState::new().enabled(enabled));
    set(Row::Panel(C::NewAction), crate::dialogs::action_new::can_open(app));
    set(Row::Panel(C::NewSet), true);
    set(Row::Panel(C::Duplicate), f.current.is_some());
    set(Row::Panel(C::Delete), has || (!f.buttons && (f.current.is_some() || f.selected_sets)));
    set(Row::Panel(C::SetOptions), f.current.is_some());
    set(Row::Panel(C::Batch), has);
    set(Row::Panel(C::InsertStop), recording || has);
    set(Row::Panel(C::InsertMenuItem), recording || has);
    set(Row::Panel(C::InsertPath), (recording || has) && f.path);
    set(Row::SelectObject, recording || has);
    set(Row::Options, has);
    set(Row::Panel(C::PlaybackOptions), true);
    set(Row::Panel(C::Save), f.current.is_some());
    set(Row::Panel(C::Load), can_load(app));
    set(Row::Panel(C::Replace), can_load(app) && f.idle);
    set(Row::Panel(C::ClearAll), f.idle && f.sets > 0);
    set(Row::Panel(C::Reset), f.idle);
    // The framework's labels differ from the panel's own words on these rows.
    let titled = |row: Row, title: &str, enabled: bool| (row.command(), CommandState::new().enabled(enabled).title(title.to_string()));
    for (command, state) in [
        titled(Row::Panel(C::Play), "Play Current Selection", has || f.waiting),
        titled(Row::Panel(C::InsertPath), "Insert Select Path", (recording || has) && f.path),
        titled(Row::Panel(C::ClearAll), "Clear Actions", f.idle && f.sets > 0),
        titled(Row::Panel(C::Reset), "Reset Actions", f.idle),
    ] {
        states.set(command, state);
    }
}

/// A hash of what [`menu_states`] reads, for the state key that decides when the menu's states are read again.
pub(crate) fn key(app: &VectorcraftApp) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    let panel = &app.ui.actions_panel;
    app.ui.actions.revision().hash(&mut h);
    (&panel.selected, &panel.selected_sets, panel.expanded.len(), panel.settings.button_mode).hash(&mut h);
    (app.ui.recording.is_some(), app.ui.action_playing.is_some(), playback::waiting_at_stop(app), &app.ui.action_set).hash(&mut h);
    app.session.active().is_some_and(|st| vectorcraft_engine::storable_selection(st).is_ok()).hash(&mut h);
    h.finish()
}

/// Runs the panel-menu command `id`; `None` for an id that is not this panel's.
pub(crate) fn run(app: &mut VectorcraftApp, id: &str, _params: &Value) -> Option<Result<Value, String>> {
    use ActionsPanelCommands as C;
    let row = GROUPS.iter().flat_map(|g| g.iter()).copied().find(|row| row.command().id == id)?;
    let f = Facts::of(app);
    // The target of a new step: the selected step, else the end of the selected action, else the recording.
    let target = if f.recording { None } else { f.selected };
    let step = f.step.filter(|_| target.is_some());
    let result = match row {
        Row::Panel(C::ButtonMode) => app.run(BUTTON_MODE, json!({})),
        Row::Panel(C::NewAction) => crate::dialogs::action_new::open(app).map(|_| Value::Null),
        Row::Panel(C::NewSet) => {
            crate::dialogs::action_set_options::open_new(app);
            Ok(Value::Null)
        }
        Row::Panel(C::Duplicate) => {
            super::duplicate_selection(app);
            Ok(Value::Null)
        }
        Row::Panel(C::Delete) => {
            super::delete_selection(app);
            Ok(Value::Null)
        }
        Row::Panel(C::SetOptions) => {
            if let Some(s) = f.current {
                crate::dialogs::action_set_options::open(app, s);
            }
            Ok(Value::Null)
        }
        Row::Panel(C::Batch) => match f.selected {
            Some((s, a)) => crate::dialogs::action_batch::open(app, s, a).map(|_| Value::Null),
            None => Ok(Value::Null),
        },
        Row::Panel(C::Play) if f.waiting => playback::continue_playback(app, &json!({})),
        Row::Panel(C::Play) => f.selected.map_or(Ok(Value::Null), |(s, a)| playback::play(app, s, a)),
        Row::Panel(C::InsertStop) => {
            crate::dialogs::action_stop::open_record(app, target, step);
            Ok(Value::Null)
        }
        Row::Panel(C::InsertMenuItem) => {
            crate::dialogs::action_insert_menu_item::open(app, target, step);
            Ok(Value::Null)
        }
        Row::Panel(C::InsertPath) => {
            let place = match (target, step) {
                (None, _) => json!({}),
                (Some((s, a)), None) => json!({"set": s, "action": a}),
                (Some((s, a)), Some(step)) => json!({"set": s, "action": a, "step": step}),
            };
            app.run(INSERT_SELECT_PATH, place)
        }
        Row::SelectObject => {
            crate::dialogs::action_select_object::open(app, target, step);
            Ok(Value::Null)
        }
        Row::Options => {
            if let Some((s, a)) = f.selected {
                crate::dialogs::action_set_options::open_action(app, s, a);
            }
            Ok(Value::Null)
        }
        Row::Panel(C::PlaybackOptions) => {
            crate::dialogs::action_playback::open(app);
            Ok(Value::Null)
        }
        // As commands, so a file dialog shown off the UI thread saves, loads or replaces when it answers.
        Row::Panel(C::Save) => {
            let name = f.current.and_then(|s| super::model::set_name(&app.ui.actions, s));
            app.run(SAVE_SET, json!({ "name": name }))
        }
        Row::Panel(C::Load) => app.run(LOAD_SET, json!({})),
        Row::Panel(C::Replace) => app.run(REPLACE_SET, json!({})),
        // Both ask first: OK deletes every set, Reset's OK or Append replaces or adds the built-in sets.
        Row::Panel(C::ClearAll) => {
            edits::ask_clear(app);
            Ok(Value::Null)
        }
        Row::Panel(C::Reset) => {
            edits::ask_reset(app);
            Ok(Value::Null)
        }
        // Not rows of this menu.
        Row::Panel(_) => Ok(Value::Null),
    };
    // A cancelled file dialog is not an error to show.
    if let Err(e) = &result
        && e != "cancelled"
    {
        app.status(e.clone());
    }
    Some(Ok(Value::Null))
}
