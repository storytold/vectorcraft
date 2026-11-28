use std::collections::BTreeSet;

use craft::automation::actions::{ActionColor, ActionList, ActionStep};
use craft::automation::ui::actions::LibraryPrompt;
use craft::ui::dialog::{PromptChoice, StackDialog};
use serde_json::{Value, json};

use super::edits::{self, ActionsQuestion};
use super::tests_support::*;
use super::*;
use crate::dialogs::action_batch::BatchDialog;
use crate::dialogs::action_insert_menu_item::InsertMenuItemDialog;
use crate::dialogs::action_new::NewActionDialog;
use crate::dialogs::action_set_options::SetOptionsDialog;
use crate::dialogs::action_stop::RecordStopDialog;

/// The index in `ActionList::actions` of action `action` of set `set` (both in panel order).
fn global(app: &VectorcraftApp, set: usize, action: usize) -> usize {
    model::flat(&app.ui.actions, set, action).expect("the action exists")
}

fn at(app: &VectorcraftApp, set: usize, action: usize) -> &Action {
    app.ui.actions.action(global(app, set, action)).expect("the action exists")
}

/// The steps of action `action` in set `set`, each as its command and params.
fn steps(app: &VectorcraftApp, set: usize, action: usize) -> Vec<(String, Value)> {
    at(app, set, action).steps.iter().map(|s| (s.command.clone(), s.params.clone())).collect()
}

/// Excludes the given steps of an action that has none excluded.
fn exclude(app: &mut VectorcraftApp, set: usize, action: usize, steps: &[usize]) {
    let at = global(app, set, action);
    for &step in steps {
        app.ui.actions.set_step(at, step, false).expect("the step exists");
    }
}

fn excluded(app: &VectorcraftApp, set: usize, action: usize) -> BTreeSet<usize> {
    at(app, set, action).disabled.clone()
}

fn clear_steps(app: &mut VectorcraftApp, set: usize, action: usize) {
    let at = global(app, set, action);
    while app.ui.actions.step_count(at) > 0 {
        app.ui.actions.delete_step(at, 0).expect("the step exists");
    }
}

/// The function key of an action as `(key, cmd, shift)`; none for no key.
fn key_of(app: &VectorcraftApp, set: usize, action: usize) -> Option<(u8, bool, bool)> {
    model::function_key(at(app, set, action)).map(|k| (k.key, k.cmd, k.shift))
}

/// The built-in set as a new app has it.
fn built_in() -> Value {
    saved_sets(&new_app())
}

/// The current set: where a recording goes.
fn current_set(app: &VectorcraftApp) -> Option<String> {
    app.ui.action_set.clone()
}

/// Clicks the header of the set `name`: it is selected and current.
fn select_set(app: &mut VectorcraftApp, name: &str) {
    app.ui.actions_panel.selected = Some(ActionSelection::Set(name.to_string()));
    app.ui.action_set = Some(name.to_string());
}

/// The name of the set the recording in progress goes into.
fn recording_set(app: &VectorcraftApp) -> Option<String> {
    let (set, ..) = app.ui.recording.as_ref()?;
    model::set_name(&app.ui.actions, *set).map(str::to_string)
}

/// Field `field` of the open `D`, as `ui.inspect` reports it.
fn dialog_field<D: StackDialog>(app: &VectorcraftApp, field: &str) -> Value {
    let fields = app.dialogs().and_then(|d| d.with(|h| h.get::<D>().and_then(StackDialog::json_fields)));
    fields.expect("the dialog is open")[field].clone()
}

/// The index field `field` of the open `D`.
fn dialog_index<D: StackDialog>(app: &VectorcraftApp, field: &str) -> Option<usize> {
    dialog_field::<D>(app, field).as_u64().and_then(|i| usize::try_from(i).ok())
}

/// The name of the set the open `D` holds.
fn dialog_set<D: StackDialog>(app: &VectorcraftApp) -> Option<String> {
    set_names(app).get(dialog_index::<D>(app, "set")?).cloned()
}

/// The name of the action the open `D` holds.
fn dialog_action<D: StackDialog>(app: &VectorcraftApp) -> Option<String> {
    let flat = model::flat(&app.ui.actions, dialog_index::<D>(app, "set")?, dialog_index::<D>(app, "action")?)?;
    app.ui.actions.action(flat).map(|a| a.name.clone())
}

/// What the open Actions question asks, when it is Reset's or Clear's.
fn library_question(app: &VectorcraftApp) -> Option<LibraryPrompt> {
    app.dialogs()?.with(|h| h.get::<ActionsQuestion>().and_then(|q| q.0.tag_as::<LibraryPrompt>().cloned()))
}

/// Answers the open Actions question as its buttons do, and takes the answer as the frame's pump does.
fn answer(app: &mut VectorcraftApp, choice: PromptChoice) {
    if choice == PromptChoice::Cancel {
        cancel_dialog::<ActionsQuestion>(app);
    } else {
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|h| {
                if let Some(q) = h.get_mut::<ActionsQuestion>() {
                    q.0.set_choice(choice);
                }
            });
        }
        confirm_dialog::<ActionsQuestion>(app).expect("the question takes the answer");
    }
    edits::take_answers(app);
}

/// The panel menu's Delete.
fn menu_delete(app: &mut VectorcraftApp) {
    let ran = menu::run(app, &CommandRef::from(ActionsPanelCommands::Delete).id, &Value::Null);
    ran.expect("Delete is a row of the panel menu").expect("the menu runs it");
}

fn new_doc(app: &mut VectorcraftApp) {
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
}

/// #3712: New Set makes a set that recordings go into, numbered within it, and the sets and the
/// current one are saved with the UI preferences.
#[test]
fn create_new_set_names_a_set_that_recordings_go_into() {
    let mut app = new_app();
    new_doc(&mut app);
    crate::dialogs::action_set_options::open_new(&mut app);
    set_dialog::<SetOptionsDialog>(&mut app, "name", json!("Print"));
    confirm_dialog::<SetOptionsDialog>(&mut app).unwrap();
    assert_eq!((set_names(&app), counts(&app)), (vec!["Default Actions".to_string(), "Print".into()], vec![6, 0]));
    assert_eq!(current_set(&app).as_deref(), Some("Print"), "the new set is current");
    let rect = json!({"x": 10, "y": 10, "width": 20, "height": 20});
    app.run(START_RECORDING, json!({})).unwrap();
    app.run("shape.rectangle", rect.clone()).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(counts(&app), [6, 1], "recorded into the new set");
    assert_eq!(action_names(&app, 1), ["Action 1"], "numbered within its set");
    select_set(&mut app, "Default Actions");
    app.run(START_RECORDING, json!({})).unwrap();
    app.run("shape.rectangle", rect).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(counts(&app), [7, 1], "recorded into the set made current");
    let saved = serde_json::to_value(&app.ui).unwrap();
    let mut restored = new_app();
    restored.ui = serde_json::from_value::<crate::state::UiState>(saved).unwrap();
    assert_eq!(set_names(&restored), ["Default Actions", "Print"]);
    assert_eq!(current_set(&restored).as_deref(), Some("Default Actions"));
}

/// `ui.action.rename` renames the action at its set and action indexes and refuses a missing
/// index or a name with nothing but spaces, renaming nothing.
#[test]
fn rename_action_renames_one_action_and_refuses_bad_params() {
    let mut app = app_with_sets();
    assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "Push"})).unwrap(), json!({"name": "Push"}));
    assert_eq!(action_names(&app, 1), ["Push", "Nudge Again"]);
    for p in [
        json!({"set": 1, "action": 2, "name": "X"}),
        json!({"set": 3, "action": 0, "name": "X"}),
        json!({"set": 1, "name": "X"}),
        json!({"action": 0, "name": "X"}),
        json!({"set": 4_294_967_297u64, "action": 0, "name": "X"}),
    ] {
        assert_eq!(app.run(RENAME_ACTION, p.clone()).unwrap_err(), "no such action", "{p}");
    }
    assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0, "name": "   "})).unwrap_err(), "name is empty");
    assert_eq!(app.run(RENAME_ACTION, json!({"set": 1, "action": 0})).unwrap_err(), "name is empty");
    assert_eq!(action_names(&app, 1), ["Push", "Nudge Again"], "a refused rename renames nothing");
}

/// OK in Action Options or Set Options whose `set` or `action` field `ui.dialog.set` changed to
/// something that is not a set or action is an error: it renames no set or action, adds no set,
/// and keeps the dialog open.
#[test]
fn options_with_a_field_that_is_not_an_index_change_nothing() {
    use crate::dialogs::action_set_options::{open, open_action};
    let mut app = app_with_sets();
    for (of_action, field, value) in
        [(true, "action", json!(null)), (true, "set", json!(null)), (true, "action", json!(9)), (false, "set", json!(null)), (false, "set", json!(9))]
    {
        if of_action {
            open_action(&mut app, 1, 0);
        } else {
            open(&mut app, 1);
        }
        set_dialog::<SetOptionsDialog>(&mut app, "name", json!("X"));
        set_dialog::<SetOptionsDialog>(&mut app, field, value.clone());
        assert!(confirm_dialog::<SetOptionsDialog>(&mut app).is_err(), "{field}={value}");
        assert!(is_open::<SetOptionsDialog>(&app), "the dialog stays open: {field}={value}");
        assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "{field}={value}");
        assert_eq!(action_names(&app, 1), ["Nudge", "Nudge Again"], "{field}={value}");
        cancel_dialog::<SetOptionsDialog>(&mut app);
    }
}

#[test]
fn stale_action_options_do_not_rename_the_copy_that_took_its_index() {
    let mut app = app_with_sets();
    crate::dialogs::action_set_options::open_action(&mut app, 1, 1);
    set_dialog::<SetOptionsDialog>(&mut app, "name", json!("Wrong target"));
    assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 0})).unwrap(), json!({"set": 1, "action": 1}));
    let before = action_names(&app, 1);
    assert!(confirm_dialog::<SetOptionsDialog>(&mut app).is_err(), "an index taken by a copy must refuse OK");
    assert!(is_open::<SetOptionsDialog>(&app));
    assert_eq!(action_names(&app, 1), before);
}

#[test]
fn stale_set_options_do_not_rename_the_set_that_took_its_index() {
    let mut app = app_with_sets();
    crate::dialogs::action_set_options::open(&mut app, 1);
    set_dialog::<SetOptionsDialog>(&mut app, "name", json!("Wrong target"));
    app.ui.actions.delete_set("Mine").unwrap();
    assert!(confirm_dialog::<SetOptionsDialog>(&mut app).is_err(), "an index taken by the next set must refuse OK");
    assert!(is_open::<SetOptionsDialog>(&app));
    assert_eq!(set_names(&app), ["Default Actions", "Other"]);
}

/// `ui.actionSet.delete` moves the panel's selected action down with its set, and clears it
/// when its set is deleted.
#[test]
fn deleting_sets_moves_or_clears_the_selected_action() {
    let mut app = app_with_sets();
    let named = |app: &VectorcraftApp| match app.ui.actions_panel.selected {
        Some(ActionSelection::Action(i)) => app.ui.actions.action(i).map(|a| a.name.clone()),
        _ => None,
    };
    let nudge = app.ui.actions.actions().iter().position(|a| a.name == "Nudge Again").unwrap();
    app.ui.actions_panel.selected = Some(ActionSelection::Action(nudge));
    app.run(DELETE_SETS, json!({"names": ["Default Actions"]})).unwrap();
    assert_eq!(named(&app).as_deref(), Some("Nudge Again"), "Nudge Again is still selected, at its new index");
    push_set(&mut app, "Last", vec![action("Spin", &[])]);
    let spin = app.ui.actions.actions().iter().position(|a| a.name == "Spin").unwrap();
    app.ui.actions_panel.selected = Some(ActionSelection::Action(spin));
    app.run(DELETE_SETS, json!({"names": ["Last"]})).unwrap();
    assert_eq!(app.ui.actions_panel.selected, None, "the deleted set's action is no longer selected");
    edits::new_set(&mut app, "Set 1").unwrap();
    assert_eq!(named(&app), None, "a new set starts with no action selected");
}

/// Delete Selection with a set selected asks first: Cancel keeps every set, Proceed deletes the
/// selected set with its actions and keeps the others.
#[test]
fn delete_selection_asks_and_then_deletes_the_selected_set() {
    let mut app = app_with_sets();
    select_set(&mut app, "Mine");
    menu_delete(&mut app);
    assert!(is_open::<ActionsQuestion>(&app), "Delete Selection asks before deleting a set");
    assert_eq!(asked(&app), Some(vec!["Mine".to_string()]));
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "nothing is deleted before the answer");
    answer(&mut app, PromptChoice::Cancel);
    assert!(!is_open::<ActionsQuestion>(&app));
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "Cancel deletes nothing");
    menu_delete(&mut app);
    answer(&mut app, PromptChoice::Proceed);
    assert_eq!(set_names(&app), ["Default Actions", "Other"], "Proceed deletes the selected set and its actions");
    assert_eq!(current_set(&app), None, "no set is current after the current one is deleted");
    menu_delete(&mut app);
    assert!(!is_open::<ActionsQuestion>(&app), "nothing selected: nothing to ask about");
}

/// After New Set makes another set current, the panel menu's Delete asks about that set and
/// leaves the action selected before it.
#[test]
fn delete_after_new_set_targets_the_new_set() {
    let mut app = app_with_sets();
    let nudge = app.ui.actions.actions().iter().position(|a| a.name == "Nudge").unwrap();
    app.ui.actions_panel.selected = Some(ActionSelection::Action(nudge));
    edits::new_set(&mut app, "Set 1").unwrap();
    menu_delete(&mut app);
    assert_eq!(asked(&app), Some(vec!["Set 1".to_string()]), "the new set, not the action");
    assert_eq!(action_names(&app, 1).len(), 2);
}

/// The question holds the names of the sets it asks about: when another set is deleted while it
/// is open, Proceed deletes the asked set; when the asked set is gone, it deletes nothing.
#[test]
fn ok_deletes_the_asked_set_after_an_earlier_set_is_deleted_under_the_question() {
    let mut app = app_with_sets();
    push_set(&mut app, "Third", vec![]);
    select_set(&mut app, "Other");
    menu_delete(&mut app);
    assert_eq!(asked(&app), Some(vec!["Other".to_string()]));
    app.run(DELETE_SETS, json!({"names": ["Mine"]})).unwrap();
    assert_eq!(set_names(&app), ["Default Actions", "Other", "Third"]);
    answer(&mut app, PromptChoice::Proceed);
    assert_eq!(set_names(&app), ["Default Actions", "Third"], "Proceed deletes Other, the asked set");
    select_set(&mut app, "Third");
    menu_delete(&mut app);
    app.run(DELETE_SETS, json!({"names": ["Third"]})).unwrap();
    answer(&mut app, PromptChoice::Proceed);
    assert_eq!(set_names(&app), ["Default Actions"], "the asked set is gone: Proceed deletes nothing else");
}

/// The default set can be deleted, and so can the last set: the panel is then empty, no set is
/// current, and New Set makes a set again.
#[test]
fn deleting_the_default_and_the_last_set_leaves_an_empty_panel() {
    let mut app = new_app();
    app.run(DELETE_SETS, json!({"names": ["Default Actions"]})).unwrap();
    assert!(app.ui.actions.actions().is_empty() && app.ui.actions.sets().is_empty());
    assert_eq!(current_set(&app), None);
    crate::dialogs::action_set_options::open_new(&mut app);
    confirm_dialog::<SetOptionsDialog>(&mut app).unwrap();
    assert!(!is_open::<SetOptionsDialog>(&app));
    assert_eq!((set_names(&app), current_set(&app)), (vec!["Set 1".to_string()], Some("Set 1".to_string())), "New Set works on an empty panel");
}

/// `ui.actionSet.delete` keeps a recording into a later set going in the same set, stops one
/// into a deleted set, and refuses a set that is not there.
#[test]
fn deleting_sets_keeps_a_recording_in_its_set() {
    let mut app = app_with_sets();
    new_doc(&mut app);
    select_set(&mut app, "Other");
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(DELETE_SETS, json!({"names": ["Mine"]})).unwrap(), json!({"deleted": 1}));
    assert_eq!(set_names(&app), ["Default Actions", "Other"]);
    assert_eq!((recording_set(&app).as_deref(), current_set(&app).as_deref()), (Some("Other"), Some("Other")), "still recording into Other");
    assert_eq!(app.run(DELETE_SETS, json!({"names": ["Gone"]})).unwrap_err(), "the action sets changed; nothing was deleted");
    assert_eq!(set_names(&app), ["Default Actions", "Other"], "a refused name deletes nothing");
    assert_eq!(app.run(DELETE_SETS, json!({})).unwrap(), json!({"deleted": 1}), "the current set by default");
    assert_eq!(set_names(&app), ["Default Actions"]);
    assert_eq!(recording_set(&app), None, "the recording into the deleted set stops");
    assert_eq!(app.session.journal.held(), None);
}

/// `ui.actionSet.delete` closes a Batch dialog open on an action of a deleted set, and keeps
/// one open on another set.
#[test]
fn deleting_a_set_closes_its_batch_dialog() {
    let mut app = app_with_sets();
    crate::dialogs::action_batch::open(&mut app, 1, 0).unwrap();
    app.run(DELETE_SETS, json!({"names": ["Mine"]})).unwrap();
    assert!(!is_open::<BatchDialog>(&app), "the Batch dialog of the deleted set closes");
    let mut app = app_with_sets();
    crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();
    app.run(DELETE_SETS, json!({"names": ["Default Actions"]})).unwrap();
    assert!(is_open::<BatchDialog>(&app), "the Batch dialog of a kept set stays open");
}

/// `ui.actionSet.duplicate` puts the copy of an action or a set right after its source, refuses
/// indexes past the last set or action, and keeps a recording and an open Batch dialog on their
/// own set and action.
#[test]
fn the_duplicate_command_copies_by_index_and_keeps_recordings_and_dialogs_in_place() {
    let mut app = app_with_sets();
    assert_eq!(app.run(DUPLICATE, json!({})).unwrap_err(), "no action set is current");
    assert_eq!(app.run(DUPLICATE, json!({"set": 3})).unwrap_err(), "no such action set");
    assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 2})).unwrap_err(), "no such action");
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "a refused index copies nothing");
    crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();
    assert_eq!(dialog_action::<BatchDialog>(&app).as_deref(), Some("Nudge Again"));
    assert_eq!(app.run(DUPLICATE, json!({"set": 1, "action": 0})).unwrap(), json!({"set": 1, "action": 1}));
    let names = action_names(&app, 1);
    assert_eq!((names.len(), names[0].as_str(), names[2].as_str()), (3, "Nudge", "Nudge Again"), "the copy follows its source");
    assert!(is_open::<BatchDialog>(&app), "the Batch dialog stays open");
    assert_eq!(dialog_action::<BatchDialog>(&app).as_deref(), Some("Nudge Again"), "on the same action");
    cancel_dialog::<BatchDialog>(&mut app);
    new_doc(&mut app);
    select_set(&mut app, "Other");
    app.run(START_RECORDING, json!({})).unwrap();
    crate::dialogs::action_set_options::open(&mut app, 2);
    assert_eq!(app.run(DUPLICATE, json!({"set": 0})).unwrap(), json!({"set": 1}));
    let names = set_names(&app);
    assert_eq!((names.len(), names[0] != names[1], &names[2..]), (4, true, &["Mine".to_string(), "Other".to_string()][..]));
    assert_eq!(recording_set(&app).as_deref(), Some("Other"), "still recording into Other");
    assert_eq!(dialog_set::<SetOptionsDialog>(&app).as_deref(), Some("Other"), "Set Options still on Other");
}

/// `ui.actionSet.reset` replaces every set, edited ones included, with the built-in set, makes it
/// current, closes a dialog on a replaced set, and records no undo step.
#[test]
fn reset_replaces_every_set_with_the_built_in_set_and_clears_what_named_the_old_ones() {
    let mut app = app_with_sets();
    new_doc(&mut app);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let undo = undo_steps(&app);
    app.ui.actions.rename_set("Default Actions", "Renamed").unwrap();
    select_set(&mut app, "Other");
    crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();

    assert_eq!(app.run(RESET, json!({})).unwrap(), json!({"changed": true, "mode": "replace", "set": 0}));
    assert_eq!(saved_sets(&app), built_in(), "only the built-in set, as it comes");
    assert_eq!(current_set(&app).as_deref(), Some("Default Actions"), "it is current");
    assert!(!is_open::<BatchDialog>(&app), "the Batch dialog of a replaced set closes");
    assert_eq!(undo_steps(&app), undo, "no undo step");

    app.ui.actions.clear_all();
    assert_eq!(app.run(RESET, Value::Null).unwrap(), json!({"changed": true, "mode": "replace", "set": 0}), "from no sets");
    assert_eq!(saved_sets(&app), built_in());
    assert_eq!(current_set(&app).as_deref(), Some("Default Actions"));
}

/// Reset Actions on the built-in set as it comes still makes it current when no set is, and
/// changes nothing once it is current.
#[test]
fn reset_on_the_built_in_set_makes_it_current_then_changes_nothing() {
    let mut app = new_app();
    assert_eq!(current_set(&app), None, "a new app has no current set");
    assert_eq!(app.run(RESET, json!({"mode": "replace"})).unwrap()["changed"], true, "the set becomes current");
    assert_eq!(current_set(&app).as_deref(), Some("Default Actions"));
    let revision = app.ui.actions.revision();
    assert_eq!(app.run(RESET, json!({})).unwrap(), json!({"changed": false, "mode": "replace", "set": 0}));
    assert_eq!(app.ui.actions.revision(), revision, "the list stays as it was");
    crate::dialogs::action_set_options::open(&mut app, 0);
    assert_eq!(app.run(RESET, json!({})).unwrap()["changed"], true, "Set Options for the set closes");
    assert!(!is_open::<SetOptionsDialog>(&app));
}

/// Append adds a copy of the built-in set after the sets there are, with the first free number,
/// and keeps every set, the current set and an open dialog as they are.
#[test]
fn reset_append_adds_the_built_in_set_with_a_free_number_and_keeps_the_rest() {
    let mut app = app_with_sets();
    let last = global(&app, 0, 5);
    app.ui.actions.delete_action(last).unwrap();
    app.ui.actions.rename_set("Other", "Default Actions 3").unwrap();
    assert_eq!(current_set(&app), None);
    assert_eq!(app.run(RESET, json!({"mode": "append"})).unwrap(), json!({"changed": true, "mode": "append", "set": 3}));
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Default Actions 3", "Default Actions 2"]);
    assert_eq!(counts(&app), [5, 2, 0, 6], "the sets there were stay as they were, the built-in actions are added");
    assert_eq!(current_set(&app), None, "still no current set");
    select_set(&mut app, "Mine");
    crate::dialogs::action_batch::open(&mut app, 1, 0).unwrap();
    assert_eq!(app.run(RESET, json!({"mode": "append"})).unwrap()["set"], 4);
    assert_eq!(set_names(&app)[4], "Default Actions 4");
    assert_eq!(current_set(&app).as_deref(), Some("Mine"), "the current set stays");
    assert!(is_open::<BatchDialog>(&app), "the Batch dialog stays open");
}

/// `ui.actionSet.reset` refuses params that are not an object, another key or mode, and a reset
/// while an action records or plays, waiting at a stop included, and then changes nothing.
#[test]
fn reset_refuses_bad_params_and_a_running_action_and_changes_nothing() {
    let mut app = app_with_sets();
    let before = saved_sets(&app);
    for (p, error) in [
        (json!(1), "the params must be an object, not 1"),
        (json!("append"), "the params must be an object, not \"append\""),
        (json!([]), "the params must be an object, not []"),
        (json!({"mode": null}), "mode must be replace or append"),
        (json!({"mode": 1}), "mode must be replace or append"),
        (json!({"mode": "clear"}), "mode must be replace or append"),
        (json!({"mode": "append", "set": 0}), "unknown parameter `set`; the command takes: mode"),
    ] {
        assert_eq!(app.run(RESET, p.clone()).unwrap_err(), error, "{p}");
        assert_eq!(saved_sets(&app), before, "{p} changes nothing");
        assert_eq!(current_set(&app), None);
    }

    new_doc(&mut app);
    app.run(START_RECORDING, json!({})).unwrap();
    for mode in ["replace", "append"] {
        assert_eq!(app.run(RESET, json!({ "mode": mode })).unwrap_err(), "an action is already recording");
    }
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "a recording keeps its sets");
    app.run(STOP_RECORDING, json!({})).unwrap();

    let (mut app, _) = app_with_a_stop(true);
    let before = saved_sets(&app);
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    for mode in ["replace", "append"] {
        assert_eq!(app.run(RESET, json!({ "mode": mode })).unwrap_err(), "an action is already playing");
    }
    assert_eq!(saved_sets(&app), before, "the playing set stays");
}

/// `ui.actionSet.clear` deletes every set, leaves no set current, closes a dialog on a deleted
/// set and records no undo step. A clear with no sets changes nothing, the empty list stays empty
/// after a save and a restart, a dialog that names no set stays open, and Reset Actions adds the
/// built-in set again. Thousands of sets go in one clear.
#[test]
fn clear_deletes_every_set_and_clears_what_named_them() {
    let mut app = app_with_sets();
    new_doc(&mut app);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    let undo = undo_steps(&app);
    app.ui.actions.rename_set("Default Actions", "Renamed").unwrap();
    push_set(&mut app, "Loaded", vec![]);
    assert_eq!(set_names(&app).len(), 4);
    select_set(&mut app, "Other");
    let nudge = app.ui.actions.actions().iter().position(|a| a.name == "Nudge Again").unwrap();
    app.ui.actions_panel.selected = Some(ActionSelection::Action(nudge));
    crate::dialogs::action_batch::open(&mut app, 1, 1).unwrap();

    assert_eq!(app.run(CLEAR, json!({})).unwrap(), json!({"cleared": 4}));
    assert!(app.ui.actions.actions().is_empty() && app.ui.actions.sets().is_empty());
    assert_eq!((current_set(&app), &app.ui.actions_panel.selected), (None, &None), "no set is current and no action selected");
    assert!(!is_open::<BatchDialog>(&app), "the Batch dialog of a deleted set closes");
    assert_eq!(undo_steps(&app), undo, "no undo step");

    assert_eq!(app.run(CLEAR, Value::Null).unwrap(), json!({"cleared": 0}), "nothing left to clear");
    assert_eq!(app.run(CLEAR, json!({"names": []})).unwrap(), json!({"cleared": 0}));
    let mut restarted = new_app();
    restarted.ui = serde_json::from_value::<crate::state::UiState>(serde_json::to_value(&app.ui).unwrap()).unwrap();
    assert!(set_names(&restarted).is_empty(), "no sets after a restart either");
    assert_eq!(current_set(&restarted), None);

    push_set(&mut app, "Mine", vec![]);
    crate::dialogs::action_set_options::open_new(&mut app);
    assert_eq!(app.run(CLEAR, json!({})).unwrap(), json!({"cleared": 1}));
    assert!(is_open::<SetOptionsDialog>(&app), "a New Set dialog names no set and stays open");
    cancel_dialog::<SetOptionsDialog>(&mut app);
    assert_eq!(app.run(RESET, json!({"mode": "append"})).unwrap()["set"], 0);
    assert_eq!(saved_sets(&app), built_in(), "Reset Actions adds the built-in set again");

    for i in 0..5000 {
        app.ui.actions.new_set(&format!("Set {i}")).unwrap();
    }
    assert_eq!(app.run(CLEAR, json!({})).unwrap(), json!({"cleared": 5001}));
    assert!(app.ui.actions.sets().is_empty());
}

/// `ui.actionSet.clear` refuses params that are not an object, another key, `names` that are not a
/// list of names (`null` included), `names` that are not exactly the panel's sets in order (a
/// set added after the question opened or a gone name included), and a clear while an action
/// records or plays, waiting at a stop included, and then deletes no set and keeps the current set.
#[test]
fn clear_refuses_bad_params_changed_sets_and_a_running_action() {
    let mut app = app_with_sets();
    select_set(&mut app, "Mine");
    let names = set_names(&app);
    let before = saved_sets(&app);
    let changed = "the action sets changed; nothing was deleted";
    let not_names = "the action set names are not a list";
    for (p, error) in [
        (json!(1), "the params must be an object, not 1"),
        (json!("all"), "the params must be an object, not \"all\""),
        (json!([]), "the params must be an object, not []"),
        (json!({"sets": [0]}), "unknown parameter `sets`; the command takes: names"),
        (json!({"names": 5}), not_names),
        (json!({"names": null}), not_names),
        (json!({"names": [names[0], 7]}), not_names),
        (json!({"names": [-1]}), not_names),
        (json!({"names": []}), changed),
        (json!({"names": [names[0], names[1]]}), changed),
        (json!({"names": [names[2], names[1], names[0]]}), changed),
        (json!({"names": [names[0], names[1], names[2], names[2]]}), changed),
        (json!({"names": [names[0], names[1], "Gone"]}), changed),
    ] {
        assert_eq!(app.run(CLEAR, p.clone()).unwrap_err(), error, "{p}");
        assert_eq!(saved_sets(&app), before, "{p} changes nothing");
        assert_eq!(current_set(&app).as_deref(), Some("Mine"));
    }
    let asked = json!({ "names": names });
    push_set(&mut app, "Later", vec![]);
    assert_eq!(app.run(CLEAR, asked.clone()).unwrap_err(), changed);
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other", "Later"], "a set added after the question keeps every set");
    app.ui.actions.delete_set("Later").unwrap();
    assert_eq!(app.run(CLEAR, asked).unwrap(), json!({"cleared": 3}), "the sets the question named");

    let mut app = app_with_sets();
    new_doc(&mut app);
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(CLEAR, json!({})).unwrap_err(), "an action is already recording");
    assert_eq!(set_names(&app), ["Default Actions", "Mine", "Other"], "a recording keeps its set");
    app.run(STOP_RECORDING, json!({})).unwrap();

    let (mut app, _) = app_with_a_stop(true);
    let before = saved_sets(&app);
    play(&mut app, 0, 0).unwrap();
    assert!(waiting_at_stop(&app));
    assert_eq!(app.run(CLEAR, json!({})).unwrap_err(), "an action is already playing");
    assert_eq!(saved_sets(&app), before, "the playing set stays");
}

/// Reset and Clear ask before they change the sets: Cancel and Escape change nothing, Append
/// adds the built-in set, OK replaces; Clear changes nothing until Proceed.
#[test]
fn reset_and_clear_ask_before_they_change_the_sets() {
    let mut app = new_app();
    push_set(&mut app, "Mine", vec![]);
    select_set(&mut app, "Mine");
    let cases: [(PromptChoice, bool, Vec<&str>, Option<&str>); 4] = [
        (PromptChoice::Cancel, false, vec!["Default Actions", "Mine"], Some("Mine")),
        (PromptChoice::Cancel, true, vec!["Default Actions", "Mine"], Some("Mine")),
        (PromptChoice::Alternate, false, vec!["Default Actions", "Mine", "Default Actions 2"], Some("Mine")),
        (PromptChoice::Proceed, false, vec!["Default Actions"], Some("Default Actions")),
    ];
    for (choice, escape, want, current) in cases {
        let sets_before = set_names(&app);
        edits::ask_reset(&mut app);
        assert_eq!(library_question(&app), Some(LibraryPrompt::Reset), "Reset asks first: {choice:?}");
        assert_eq!(set_names(&app), sets_before, "nothing changes before the answer");
        if escape {
            cancel_dialog::<ActionsQuestion>(&mut app);
            edits::take_answers(&mut app);
        } else {
            answer(&mut app, choice);
        }
        assert!(!is_open::<ActionsQuestion>(&app), "{choice:?} closes the question");
        assert_eq!(set_names(&app), want, "{choice:?} escape={escape}");
        assert_eq!(current_set(&app).as_deref(), current, "{choice:?} escape={escape}");
    }
    assert_eq!(saved_sets(&app), built_in(), "OK leaves the built-in set as it comes");

    push_set(&mut app, "Mine", vec![]);
    edits::ask_clear(&mut app);
    assert_eq!(library_question(&app), Some(LibraryPrompt::ClearAll), "Clear asks first");
    assert_eq!(set_names(&app), ["Default Actions", "Mine"], "nothing changes before the answer");
    answer(&mut app, PromptChoice::Cancel);
    assert_eq!(set_names(&app), ["Default Actions", "Mine"], "Cancel clears nothing");
    edits::ask_clear(&mut app);
    answer(&mut app, PromptChoice::Proceed);
    assert!(set_names(&app).is_empty(), "Proceed clears every set");
}

/// New Action asks for a name and set, and OK adds the action there and records into it.
#[test]
fn new_action_asks_for_a_name_and_set_and_records_into_the_new_action() {
    let mut app = app_with_sets();
    new_doc(&mut app);
    select_set(&mut app, "Mine");
    crate::dialogs::action_new::open(&mut app).unwrap();
    assert!(is_open::<NewActionDialog>(&app));
    let fields = (
        dialog_set::<NewActionDialog>(&app),
        dialog_field::<NewActionDialog>(&app, "name"),
        dialog_field::<NewActionDialog>(&app, "key"),
        dialog_field::<NewActionDialog>(&app, "color"),
    );
    assert_eq!(fields, (Some("Mine".to_string()), json!("Action 3"), json!(0), json!("")), "the current set, a free name, no key or color");
    cancel_dialog::<NewActionDialog>(&mut app);
    assert_eq!(counts(&app), [6, 2, 0], "Cancel adds nothing");
    assert_eq!(recording_set(&app), None, "and records nothing");
    crate::dialogs::action_new::open(&mut app).unwrap();
    set_dialog::<NewActionDialog>(&mut app, "set", json!(2));
    set_dialog::<NewActionDialog>(&mut app, "name", json!("  Box  "));
    confirm_dialog::<NewActionDialog>(&mut app).unwrap();
    assert!(!is_open::<NewActionDialog>(&app));
    assert_eq!(action_names(&app, 2), ["Box"], "added to the set chosen, with the name trimmed");
    assert_eq!(current_set(&app).as_deref(), Some("Other"), "its set is current");
    assert_eq!(recording_set(&app).as_deref(), Some("Other"), "Record starts recording into it");
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    let commands: Vec<String> = steps(&app, 2, 0).into_iter().map(|(c, _)| c).collect();
    assert_eq!(commands, ["shape.rectangle"], "the recorded steps, in the new action");
    assert_eq!(counts(&app), [6, 2, 1], "Stop adds no other action");
    app.run(NEW_ACTION, json!({"set": 2, "name": "Empty"})).unwrap();
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(action_names(&app, 2), ["Box", "Empty"]);
    assert!(steps(&app, 2, 1).is_empty(), "nothing recorded: an empty action");
}

/// `ui.action.new` refuses an index past the last set, an empty name, a key or color that is
/// not one, and a second recording, and then adds nothing. New Action does not open without a
/// set or while recording, and closes when its set is deleted. On Windows F1, Ctrl+F4 and
/// Ctrl+F6 are refused.
#[test]
fn new_action_refuses_bad_params_and_adds_nothing() {
    let mut app = app_with_sets();
    for (p, want) in [
        (json!({"set": 3, "name": "A"}), "no such action set"),
        (json!({"name": "A"}), "no such action set"),
        (json!({"set": 1, "name": "  "}), "name is empty"),
        (json!({"set": 1}), "name is empty"),
        (json!({"set": 1, "name": "A", "key": 16}), "no such function key"),
        (json!({"set": 1, "name": "A", "key": "F5"}), "no such function key"),
        (json!({"set": 1, "name": "A", "color": "pink"}), "no such color"),
    ] {
        assert_eq!(app.run(NEW_ACTION, p.clone()).unwrap_err(), want, "{p}");
    }
    if crate::keymap::platform() == craft::keymap::Platform::Windows {
        for (key, cmd) in [(1, false), (1, true), (4, true), (6, true)] {
            let want = format!("{} cannot be used on Windows", FunctionKey { key, cmd, shift: false }.text());
            assert_eq!(app.run(NEW_ACTION, json!({"set": 1, "name": "A", "key": key, "cmd": cmd})).unwrap_err(), want);
        }
    }
    assert_eq!(counts(&app), [6, 2, 0], "nothing added");
    assert_eq!(recording_set(&app), None, "nothing recording");
    // The dialog follows its set down, and closes with it.
    crate::dialogs::action_new::open(&mut app).unwrap();
    set_dialog::<NewActionDialog>(&mut app, "set", json!(2));
    app.run(DELETE_SETS, json!({"names": ["Mine"]})).unwrap();
    assert_eq!(dialog_set::<NewActionDialog>(&app).as_deref(), Some("Other"), "New Action follows Other");
    app.run(DELETE_SETS, json!({"names": ["Other"]})).unwrap();
    assert!(!is_open::<NewActionDialog>(&app), "New Action closes with its set");
    crate::dialogs::action_new::open(&mut app).unwrap();
    set_dialog::<NewActionDialog>(&mut app, "set", json!(9));
    assert!(confirm_dialog::<NewActionDialog>(&mut app).is_err());
    assert!(is_open::<NewActionDialog>(&app), "a set that is gone: the dialog stays open");
    cancel_dialog::<NewActionDialog>(&mut app);
    new_doc(&mut app);
    app.run(NEW_ACTION, json!({"set": 0, "name": "A"})).unwrap();
    assert_eq!(app.run(NEW_ACTION, json!({"set": 0, "name": "B"})).unwrap_err(), "an action is already recording");
    assert_eq!(crate::dialogs::action_new::open(&mut app).unwrap_err(), "an action is already recording");
    assert!(!crate::dialogs::action_new::can_open(&app));
    assert_eq!(counts(&app), [7], "one action added");
    app.run(STOP_RECORDING, json!({})).unwrap();
    app.ui.actions.clear_all();
    assert_eq!(crate::dialogs::action_new::open(&mut app).unwrap_err(), "no such action set");
    assert!(!is_open::<NewActionDialog>(&app));
}

/// Action Options shows the action's function key and color; OK sets them and keeps the name's
/// steps, Cancel keeps them as they were. A rename keeps them, and a copy has no key.
#[test]
fn action_options_sets_the_function_key_and_color_and_keeps_the_steps() {
    use crate::dialogs::action_set_options::open_action;
    let mut app = app_with_sets();
    open_action(&mut app, 1, 1);
    let fill = |app: &mut VectorcraftApp| {
        assert_eq!(
            (
                dialog_field::<SetOptionsDialog>(app, "key"),
                dialog_field::<SetOptionsDialog>(app, "shift"),
                dialog_field::<SetOptionsDialog>(app, "color")
            ),
            (json!(0), json!(false), json!(""))
        );
        set_dialog::<SetOptionsDialog>(app, "key", json!(15));
        set_dialog::<SetOptionsDialog>(app, "shift", json!(true));
        set_dialog::<SetOptionsDialog>(app, "color", json!("green"));
    };
    fill(&mut app);
    cancel_dialog::<SetOptionsDialog>(&mut app);
    assert_eq!((key_of(&app, 1, 1), at(&app, 1, 1).color), (None, ActionColor::default()), "Cancel keeps them");
    open_action(&mut app, 1, 1);
    fill(&mut app);
    confirm_dialog::<SetOptionsDialog>(&mut app).unwrap();
    let green = ActionColor::from_name("green");
    assert_eq!((at(&app, 1, 1).name.as_str(), key_of(&app, 1, 1), at(&app, 1, 1).color), ("Nudge Again", Some((15, false, true)), green));
    assert_eq!(steps(&app, 1, 1), [("object.move".to_string(), json!({"dx": 1, "dy": 0}))], "the steps stay");
    open_action(&mut app, 1, 1);
    let shown = (
        dialog_field::<SetOptionsDialog>(&app, "key"),
        dialog_field::<SetOptionsDialog>(&app, "shift"),
        dialog_field::<SetOptionsDialog>(&app, "color"),
    );
    assert_eq!(shown, (json!(15), json!(true), json!("green")), "shown when it opens again");
    cancel_dialog::<SetOptionsDialog>(&mut app);
    app.run(RENAME_ACTION, json!({"set": 1, "action": 1, "name": "Push"})).unwrap();
    assert_eq!((key_of(&app, 1, 1), at(&app, 1, 1).color), (Some((15, false, true)), green), "a rename keeps them");
    app.run(DUPLICATE, json!({"set": 1, "action": 1})).unwrap();
    assert_eq!((key_of(&app, 1, 2), at(&app, 1, 2).color), (None, green), "the copy has no key");
    assert_eq!(key_of(&app, 1, 1), Some((15, false, true)), "the original keeps it");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "key": 0, "color": null})).unwrap();
    assert_eq!(
        (at(&app, 1, 1).name.as_str(), key_of(&app, 1, 1), at(&app, 1, 1).color),
        ("Push", None, ActionColor::default()),
        "cleared; the name stays"
    );
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "key": 15, "cmd": true, "shift": true})).unwrap();
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "key": 14})).unwrap();
    assert_eq!(key_of(&app, 1, 1), Some((14, true, true)), "a new key keeps the modifiers left out");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 1, "shift": false})).unwrap();
    assert_eq!(key_of(&app, 1, 1), Some((14, true, false)), "shift alone changes Shift");
    app.run(ACTION_OPTIONS, json!({"set": 1, "action": 2, "shift": true})).unwrap();
    assert_eq!(key_of(&app, 1, 2), None, "a modifier alone gives no key");
    for (p, want) in [
        (json!({"set": 1, "action": 9}), "no such action"),
        (json!({"set": 1, "action": 1, "name": " "}), "name is empty"),
        (json!({"set": 1, "action": 1, "color": "pink"}), "no such color"),
        (json!({"set": 1, "action": 1, "key": 99}), "no such function key"),
    ] {
        assert_eq!(app.run(ACTION_OPTIONS, p.clone()).unwrap_err(), want, "{p}");
    }
}

#[test]
fn insert_stop_goes_after_the_given_step_or_at_the_end_and_keeps_exclusions() {
    let (mut app, _) = app_with_three_moves();
    exclude(&mut app, 0, 0, &[0, 2]);
    app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0, "message": "After the first"})).unwrap();
    app.run(INSERT_STOP, json!({"set": 0, "action": 0})).unwrap();
    let steps = steps(&app, 0, 0);
    let ids: Vec<&str> = steps.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["object.move", STOP_STEP, "object.move", "object.move", STOP_STEP]);
    assert_eq!(steps[1].1, json!({"message": "After the first", "allowContinue": false}));
    assert_eq!(steps[4].1, json!({"message": "", "allowContinue": false}), "the message and Allow Continue are optional");
    assert_eq!(excluded(&app, 0, 0), BTreeSet::from([0, 3]), "the step excluded after the insertion point moved with it");

    let (mut app, _) = app_with_three_moves();
    exclude(&mut app, 0, 0, &[1]);
    app.run(INSERT_STOP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    assert_eq!(excluded(&app, 0, 0), BTreeSet::from([2]), "a later exclusion moves with its step");
}

#[test]
fn insert_stop_refuses_bad_params_and_changes_nothing() {
    let (mut app, _) = app_with_three_moves();
    for (params, want) in [
        (json!({"message": "m"}), "no such action"),
        (json!({"set": 0}), "no such action"),
        (json!({"action": 0}), "no such action"),
        (json!({"step": 0}), "no such action"),
        (json!({"set": 1, "action": 0}), "no such action"),
        (json!({"set": 0, "action": 1}), "no such action"),
        (json!({"set": "0", "action": 0}), "no such action"),
        (json!({"set": 0, "action": null}), "no such action"),
        (json!({"set": 0, "action": 0, "step": 3}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": -1}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": null}), "no such action step"),
        (json!({"set": 0, "action": 0, "message": 5}), "message must be text"),
        (json!({"set": 0, "action": 0, "message": null}), "message must be text"),
        (json!({"set": 0, "action": 0, "allowContinue": "yes"}), "allowContinue must be true or false"),
        (json!(5), "`params` must be an object"),
        (json!([1]), "`params` must be an object"),
        (json!("message"), "`params` must be an object"),
        (json!(null), "`params` must be an object"),
    ] {
        assert_eq!(app.run(INSERT_STOP, params.clone()).unwrap_err(), want, "{params}");
        assert_eq!(steps(&app, 0, 0).len(), 3, "{params}");
    }
    app.run(START_RECORDING, json!({})).unwrap();
    assert_eq!(app.run(INSERT_STOP, json!({"message": 5})).unwrap_err(), "message must be text");
    for params in [json!(5), json!([1]), json!(true), json!(null)] {
        assert_eq!(app.run(INSERT_STOP, params.clone()).unwrap_err(), "`params` must be an object", "{params}");
    }
    app.run(STOP_RECORDING, json!({})).unwrap();
    assert_eq!(app.ui.actions.actions().last().map(|a| a.steps.len()), Some(0), "the refused stop is not recorded");
}

#[test]
fn insert_menu_item_goes_after_the_given_step_or_at_the_end_runs_nothing_and_plays_as_the_menu_runs_it() {
    let (mut app, x) = app_with_three_moves();
    exclude(&mut app, 0, 0, &[0, 2]);
    let doc = app.session.active().unwrap().doc.clone();
    let journal = app.session.journal.len();
    let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "object.group"})).unwrap();
    assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
    let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "type.changeCase", "params": {"case": "upper"}})).unwrap();
    assert_eq!(r, json!({"set": 0, "action": 0, "step": 4}));
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "inserting runs nothing");
    assert_eq!(app.session.journal.len(), journal, "and records nothing");
    let held = steps(&app, 0, 0);
    let ids: Vec<&str> = held.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["object.move", "object.group", "object.move", "object.move", "type.changeCase"]);
    assert_eq!((&held[1].1, &held[4].1), (&json!({}), &json!({"case": "upper"})));
    assert_eq!(excluded(&app, 0, 0), BTreeSet::from([0, 3]), "the step excluded after the insertion point moved with it");

    exclude(&mut app, 0, 0, &[4]);
    let before = undo_steps(&app);
    let r = play(&mut app, 0, 0).unwrap();
    let results = r["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "the group and the included move play, and the excluded change of case does not: {r}");
    assert!(results[0]["id"].is_u64(), "the group step made a group: {r}");
    assert_eq!(undo_steps(&app), before + 1, "the action plays as one undo step");
    assert_eq!(x(&app), 2.0, "the move of 2 moved the group");
}

/// Batch… refuses an action with an included View menu step, which acts on a document window,
/// and plays one whose View menu step is excluded.
#[test]
fn batch_refuses_an_action_with_a_view_step() {
    let (mut app, _) = app_with_a_view_step();
    assert_eq!(crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap_err(), "Batch cannot play an action with a View menu step");
    assert!(!is_open::<BatchDialog>(&app));
    exclude(&mut app, 0, 0, &[1]);
    crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap();
}

#[test]
fn insert_menu_item_refuses_what_no_menu_item_runs_and_changes_nothing() {
    let (mut app, _) = app_with_three_moves();
    let place = |extra: Value| {
        let mut p = json!({"set": 0, "action": 0});
        if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        p
    };
    let none = "no menu item that can be inserted runs this command";
    for (params, want) in [
        (place(json!({})), "command must be text"),
        (place(json!({"command": 5})), "command must be text"),
        (place(json!({"command": "no.such.command"})), none),
        // Undo, a File menu item, an item that asks for values first, the app's own commands and
        // an action's own command.
        (place(json!({"command": "edit.undo"})), none),
        (place(json!({"command": "file.save"})), none),
        (place(json!({"command": "object.move"})), none),
        (place(json!({"command": "object.move", "params": {"dx": 1, "dy": 0}})), none),
        (place(json!({"command": INSERT_MENU_ITEM})), none),
        (place(json!({"command": "edit.paste"})), none),
        // The app's own commands outside the View menu, a saved-view slot and the View menu's New
        // View…, which asks for a name first.
        (place(json!({"command": "window.panel", "params": {"panel": "layers", "toggle": true}})), none),
        (place(json!({"command": "view.goto1"})), none),
        (place(json!({"command": "view.saved.new"})), none),
        // Align To is the step's own: only the menu item's params are taken.
        (place(json!({"command": "object.align", "params": {"horizontal": "left", "to": "selection"}})), none),
        // Image Trace, whose menu items trace off the UI thread with progress and Cancel.
        (place(json!({"command": "imageTrace.make", "params": {"preset": "Default"}})), none),
        (place(json!({"command": "imageTrace.makeAndExpand", "params": {"preset": "Default"}})), none),
        // Pattern's Make and Edit Pattern, whose menu items open the Pattern Options panel.
        (place(json!({"command": "object.pattern.make"})), none),
        (place(json!({"command": "object.pattern.edit"})), none),
        // Params that no menu item gives its command.
        (place(json!({"command": "type.changeCase"})), none),
        (place(json!({"command": "type.changeCase", "params": {"case": "middle"}})), none),
        (place(json!({"command": "object.group", "params": {"ids": [1]}})), none),
        (place(json!({"command": "object.group", "params": [1]})), "`params` must be an object"),
        (place(json!({"command": "object.group", "params": null})), "`params` must be an object"),
        (place(json!({"command": "object.group", "step": 3})), "no such action step"),
        (place(json!({"command": "object.group", "step": null})), "no such action step"),
        (json!({"command": "object.group"}), "no such action"),
        (json!({"command": "object.group", "set": 0}), "no such action"),
        (json!({"command": "object.group", "set": 0, "action": 1}), "no such action"),
        (json!({"command": "object.group", "set": 1, "action": 0}), "no such action"),
        (json!(5), "`params` must be an object"),
        (json!(null), "`params` must be an object"),
    ] {
        assert_eq!(app.run(INSERT_MENU_ITEM, params.clone()).unwrap_err(), want, "{params}");
        assert_eq!(steps(&app, 0, 0).len(), 3, "{params}");
    }
}

/// A listed menu item runs from the menu what its step runs in playback: the click opens no
/// dialog, and it journals the item's own command with the params its step holds. An Object ›
/// Align item's step holds the Align To choice that the click sends, an Effect › Pathfinder
/// item's step holds the Pathfinder Options that the click applies, and the app adds the active
/// artboard to both.
#[test]
fn every_listed_menu_item_runs_from_the_menu_what_its_step_holds() {
    // The View menu's proof items change the whole process's proof view.
    let _view = crate::testing::DefaultProofView::new();
    for item in crate::menus::insertable_items(false, &crate::menus::Listed::default()) {
        let (mut app, _) = app_with_three_moves();
        app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": item.command, "params": item.params})).unwrap();
        let held = steps(&app, 0, 0)[3].1.clone();
        let start = app.session.journal.len();
        crate::menus::invoke(&mut app, &item.command, item.params.clone());
        assert!(app.dialogs().is_some_and(|d| d.with(|h| h.is_empty())), "{item:?} opens a dialog");
        let ran = app.session.journal_for_action(start);
        let without_artboard = |p: &Value| {
            let mut p = p.clone();
            if let Some(o) = p.as_object_mut() {
                o.remove("artboard");
            }
            p
        };
        assert!(ran.iter().all(|(c, p)| *c != item.command || without_artboard(p) == held), "{item:?} ran {ran:?}, its step holds {held}");
    }
}

/// Object › Align items can be inserted, and the step keeps the Align To that the click sends at
/// insertion: a later Align To choice does not change it.
#[test]
fn an_inserted_align_item_keeps_the_align_to_of_its_insertion() {
    let (mut app, _, ids) = app_with_two_rectangles();
    app.run("select.set", json!({ "ids": ids })).unwrap();
    app.ui.align_to = super::super::align::AlignTo::Artboard;
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
    app.ui.align_to = super::super::align::AlignTo::Selection;
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
    let steps = steps(&app, 0, 0);
    assert_eq!(steps[3], ("object.align".to_string(), json!({"horizontal": "left", "to": "artboard"})));
    assert_eq!(steps[4], ("object.align".to_string(), json!({"horizontal": "left", "to": "selection"})));
}

/// An Effect › Pathfinder item's step holds the Pathfinder Options in force at insertion, as the
/// click applies the effect with them: a later Pathfinder Options change does not change the step,
/// and playback applies the effect with the step's options.
#[test]
fn an_inserted_pathfinder_effect_item_keeps_the_pathfinder_options_of_its_insertion() {
    let (mut app, _, ids) = app_with_two_rectangles();
    app.run("select.set", json!({ "ids": ids })).unwrap();
    app.run("object.pathfinder.options", json!({"precision": 0.5, "removeRedundantPoints": true})).unwrap();
    clear_steps(&mut app, 0, 0);
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "effect.apply", "params": {"effect": "pathfinder.add"}})).unwrap();
    app.run("object.pathfinder.options", json!({"precision": 0.01, "removeRedundantPoints": false})).unwrap();
    let options = json!({"precision": 0.5, "removeRedundantPoints": true, "removeUnpainted": false});
    let held = json!({"effect": "pathfinder.add", "params": options});
    assert_eq!(steps(&app, 0, 0), [("effect.apply".to_string(), held)]);
    app.run("select.set", json!({ "ids": ids })).unwrap();
    app.run("object.group", json!({})).unwrap();
    play(&mut app, 0, 0).unwrap();
    let group = app.session.doc().unwrap().selection.objects[0];
    let effects = &app.session.doc().unwrap().doc.node(group).unwrap().appearance.effects;
    assert_eq!((effects.len(), effects[0].id.as_str(), &effects[0].params), (1, "pathfinder.add", &options));
}

/// Every Effect › Pathfinder item's step holds the Pathfinder Options that clicking the item
/// journals, with options other than the defaults in force.
#[test]
fn every_pathfinder_effect_item_step_holds_the_options_its_click_applies() {
    let options = json!({"precision": 0.25, "removeRedundantPoints": false, "removeUnpainted": true});
    let items: Vec<_> = crate::menus::insertable_items(false, &crate::menus::Listed::default())
        .into_iter()
        .filter(|item| {
            item.command == "effect.apply"
                && item.params.get("effect").and_then(Value::as_str).is_some_and(vectorcraft_effects::takes_pathfinder_options)
        })
        .collect();
    assert!(!items.is_empty());
    for item in items {
        let (mut app, _, ids) = app_with_two_rectangles();
        app.run("object.pathfinder.options", options.clone()).unwrap();
        clear_steps(&mut app, 0, 0);
        app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": item.command, "params": item.params})).unwrap();
        let held = steps(&app, 0, 0)[0].1.clone();
        assert_eq!(held["params"], options, "{item:?}");
        app.run("select.set", json!({ "ids": ids })).unwrap();
        let start = app.session.journal.len();
        crate::menus::invoke(&mut app, &item.command, item.params.clone());
        let ran: Vec<_> = app.session.journal_for_action(start).into_iter().filter(|(c, _)| *c == item.command).collect();
        assert!(!ran.is_empty(), "{item:?} did not run");
        for (_, mut p) in ran {
            if let Some(o) = p.as_object_mut() {
                o.remove("artboard");
            }
            assert_eq!(p, held, "{item:?}");
        }
    }
}

/// With anchor points selected, an Object › Align item sends its params unchanged, and the points
/// align to their own bounds: its step holds the same params, without the Align To choice.
#[test]
fn an_inserted_align_item_with_anchors_selected_keeps_the_menu_params() {
    let (mut app, _, [first, _]) = app_with_two_rectangles();
    app.ui.align_to = super::super::align::AlignTo::Artboard;
    app.run("select.anchors", json!({"id": first, "anchors": [[0, 0], [0, 1]]})).unwrap();
    assert!(!app.session.active().unwrap().selection.anchors.is_empty());
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
    assert_eq!(steps(&app, 0, 0)[3], ("object.align".to_string(), json!({"horizontal": "left"})));
}

/// A step that aligns to the key object fails in playback when the selection has no key then:
/// it does not align to the selection or the artboard instead, and the steps after it do not run.
#[test]
fn an_align_step_to_the_key_object_fails_without_a_key() {
    let (mut app, _, [first, second]) = app_with_two_rectangles();
    app.run("select.set", json!({"ids": [first, second]})).unwrap();
    app.run("select.key", json!({"id": second})).unwrap();
    clear_steps(&mut app, 0, 0);
    app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "command": "object.align", "params": {"horizontal": "left"}})).unwrap();
    assert_eq!(steps(&app, 0, 0)[0].1["to"], json!("key"));
    let at = global(&app, 0, 0);
    app.ui.actions.insert_step(at, 1, ActionStep::new("object.move", json!({"dx": 5, "dy": 0}))).unwrap();
    app.run("select.set", json!({"ids": [first, second]})).unwrap();
    let before = (left_of(&app, first), left_of(&app, second));
    assert!(play(&mut app, 0, 0).unwrap_err().contains("no key object"));
    assert_eq!((left_of(&app, first), left_of(&app, second)), before, "nothing moved");
}

#[test]
fn insert_select_path_stores_the_selected_path_changes_nothing_and_plays_by_selecting_it() {
    let (mut app, x, [first, second]) = app_with_two_rectangles();
    exclude(&mut app, 0, 0, &[0, 2]);
    let doc = app.session.active().unwrap().doc.clone();
    let journal = app.session.journal.len();
    let undo = undo_steps(&app);
    let r = app.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc), "inserting edits nothing");
    assert_eq!(app.session.journal.len(), journal, "and records nothing");
    assert_eq!(undo_steps(&app), undo, "and adds no undo step");
    let held = steps(&app, 0, 0);
    assert_eq!(held[1].0, SELECT_PATH_STEP);
    assert_eq!(held[1].1["id"], json!(first));
    assert!(held[1].1["fingerprint"].as_str().is_some_and(|f| f.starts_with("v1:")), "{:?}", held[1]);
    let off = excluded(&app, 0, 0);
    assert_eq!(off, BTreeSet::from([0, 3]), "the step excluded after the insertion point moved with it");
    assert!(!off.contains(&1), "the new step is included");

    app.run("select.set", json!({"ids": [second]})).unwrap();
    let before = undo_steps(&app);
    play(&mut app, 0, 0).unwrap();
    assert_eq!(app.session.active().unwrap().selection.objects, [vectorcraft_doc::NodeId(first)], "the step selected the stored path");
    assert_eq!(x(&app), 2.0, "the move after it moved the stored path");
    assert_eq!(left_of(&app, second), 110.0, "and not the path selected before");
    assert_eq!(undo_steps(&app), before + 1, "the action plays as one undo step");
}

#[test]
fn insert_select_path_refuses_what_is_not_one_whole_path_and_changes_nothing() {
    let (mut app, _, [first, second]) = app_with_two_rectangles();
    let place = json!({"set": 0, "action": 0});
    let refused = |app: &mut VectorcraftApp, params: Value, want: &str| {
        let selection = app.session.active().map(|st| st.selection.objects.clone());
        assert_eq!(app.run(INSERT_SELECT_PATH, params.clone()).unwrap_err(), want, "{params}");
        assert_eq!(steps(app, 0, 0).len(), 3, "{params}");
        assert_eq!(app.session.active().map(|st| st.selection.objects.clone()), selection, "{params}");
    };
    app.run("select.set", json!({"ids": [first, second]})).unwrap();
    refused(&mut app, place.clone(), "select only one path");
    app.run("select.none", json!({})).unwrap();
    refused(&mut app, place.clone(), "select a path");
    app.run("select.anchors", json!({"id": first, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    refused(&mut app, place.clone(), "select one whole path or compound path");
    app.run("select.set", json!({"ids": [first, second]})).unwrap();
    let group = app.run("object.group", json!({})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [group]})).unwrap();
    refused(&mut app, place.clone(), "select one whole path or compound path");
    app.run("select.set", json!({"ids": [first]})).unwrap();
    for (params, want) in [
        (json!({}), "no such action"),
        (json!({"set": 0}), "no such action"),
        (json!({"set": 1, "action": 0}), "no such action"),
        (json!({"set": 0, "action": 1}), "no such action"),
        (json!({"set": "0", "action": 0}), "no such action"),
        (json!({"set": 0, "action": 0, "step": 3}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": null}), "no such action step"),
        (json!(5), "`params` must be an object"),
        (json!(null), "`params` must be an object"),
    ] {
        refused(&mut app, params, want);
    }
    let mut empty = app_with_sets();
    assert_eq!(empty.run(INSERT_SELECT_PATH, json!({"set": 0, "action": 0})).unwrap_err(), "no document open");
}

#[test]
fn delete_step_removes_one_step_and_moves_the_exclusions_up() {
    let (mut app, _) = app_with_a_stop(false);
    exclude(&mut app, 0, 0, &[0, 1, 3]);
    assert_eq!(app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 1})).unwrap(), json!({"set": 0, "action": 0, "step": 1}));
    let held = steps(&app, 0, 0);
    let ids: Vec<&str> = held.iter().map(|(c, _)| c.as_str()).collect();
    assert_eq!(ids, ["object.move", "object.move", "object.move"]);
    assert_eq!(excluded(&app, 0, 0), BTreeSet::from([0, 2]), "the deleted step's exclusion goes and a later one moves up");
}

#[test]
fn delete_step_refuses_bad_params_and_changes_nothing() {
    let (mut app, _) = app_with_three_moves();
    for (params, want) in [
        (json!({}), "no such action"),
        (json!(5), "no such action"),
        (json!([1]), "no such action"),
        (json!({"set": 1, "action": 0, "step": 0}), "no such action"),
        (json!({"set": 0, "action": 1, "step": 0}), "no such action"),
        (json!({"set": "0", "action": 0, "step": 0}), "no such action"),
        (json!({"set": 0, "action": 0}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": 3}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": -1}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": null}), "no such action step"),
    ] {
        assert_eq!(app.run(DELETE_STEP, params.clone()).unwrap_err(), want, "{params}");
        assert_eq!(steps(&app, 0, 0).len(), 3, "{params}");
    }
}

/// An open Record Stop dialog follows its step up when a step above it is deleted, and closes
/// when its own step is deleted.
#[test]
fn delete_step_keeps_an_open_record_stop_on_its_place() {
    let (mut app, _) = app_with_three_moves();
    crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(2));
    app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    assert_eq!(dialog_field::<RecordStopDialog>(&app, "step"), json!(1), "a deleted step above moves it up");
    app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 1})).unwrap();
    assert!(!is_open::<RecordStopDialog>(&app), "deleting its step closes it");
}

#[test]
fn move_step_within_action_updates_order_exclusions_and_selection() {
    let (mut app, _) = app_with_three_moves();
    let at = global(&app, 0, 0);
    app.ui.actions.insert_step(at, 3, ActionStep::new("object.move", json!({"dx": 8, "dy": 0}))).unwrap();
    exclude(&mut app, 0, 0, &[1]);
    app.ui.actions_panel.selected = Some(ActionSelection::Step(at, 3));
    assert_eq!(
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 1, "before": 4})).unwrap(),
        json!({"set": 0, "action": 0, "step": 3, "moved": true})
    );
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 4", "object.move 8", "object.move 2"]);
    assert_eq!(excluded(&app, 0, 0), BTreeSet::from([3]), "the moved step stays excluded");
    assert_eq!(app.ui.actions_panel.selected, Some(ActionSelection::Step(at, 2)), "the selected step stays selected");
    assert_eq!(
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 2, "before": 0})).unwrap(),
        json!({"set": 0, "action": 0, "step": 0, "moved": true})
    );
    assert_eq!(step_names(&app, 0, 0), ["object.move 8", "object.move 1", "object.move 4", "object.move 2"]);
    assert_eq!(excluded(&app, 0, 0), BTreeSet::from([3]));
    assert_eq!(app.ui.actions_panel.selected, Some(ActionSelection::Step(at, 0)), "the moved selected step is selected where it went");
    for before in [1, 2] {
        assert_eq!(
            app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 1, "before": before})).unwrap(),
            json!({"set": 0, "action": 0, "step": 1, "moved": false})
        );
    }
    assert_eq!(step_names(&app, 0, 0), ["object.move 8", "object.move 1", "object.move 4", "object.move 2"], "a drop on itself moves nothing");
    crate::dialogs::action_stop::open_record(&mut app, Some((0, 0)), Some(3));
    app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 3, "before": 0})).unwrap();
    assert_eq!(dialog_field::<RecordStopDialog>(&app, "step"), json!(0), "an open Record Stop follows its step");
}

#[test]
fn move_step_between_actions_and_sets_keeps_file_roundtrip() {
    let (mut app, _) = app_with_three_moves();
    exclude(&mut app, 0, 0, &[2]);
    push_set(&mut app, "Other", vec![action("Empty", &[]), action("One", &[("object.rotate", json!({"angle": 5}))])]);
    select_set(&mut app, "Moves");
    let (from, empty) = (global(&app, 0, 0), global(&app, 1, 0));
    app.ui.actions_panel.selected = Some(ActionSelection::Step(from, 2));
    assert_eq!(
        app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 2, "toSet": 1, "toAction": 0, "before": 0})).unwrap(),
        json!({"set": 1, "action": 0, "step": 0, "moved": true})
    );
    assert_eq!(step_names(&app, 0, 0), ["object.move 1", "object.move 2"]);
    assert!(excluded(&app, 0, 0).is_empty());
    assert_eq!(step_names(&app, 1, 0), ["object.move 4"], "an empty action takes a step");
    assert_eq!(excluded(&app, 1, 0), BTreeSet::from([0]), "the moved step stays excluded");
    assert_eq!(app.ui.actions_panel.selected, Some(ActionSelection::Step(empty, 0)));
    assert_eq!(current_set(&app).as_deref(), Some("Other"), "the step's new set is current");
    app.run(MOVE_STEP, json!({"set": 0, "action": 0, "step": 0, "toSet": 1, "toAction": 1, "before": 0})).unwrap();
    assert_eq!(step_names(&app, 1, 1), ["object.move 1", "object.rotate"]);
    assert_eq!(app.ui.actions_panel.selected, Some(ActionSelection::Step(empty, 0)), "an unmoved selected step stays");
    let loaded = ActionList::from_file(app.ui.actions.to_file());
    assert_eq!(loaded.actions().len(), app.ui.actions.actions().len());
    for (a, b) in loaded.actions().iter().zip(app.ui.actions.actions()) {
        assert_eq!((&a.name, &a.steps, &a.disabled), (&b.name, &b.steps, &b.disabled));
    }
}

#[test]
fn move_step_rejects_invalid_and_oversize_destinations_without_changes() {
    let (mut app, _) = app_with_three_moves();
    let big = |text: String| action("Big", &[("document.note", json!({ "s": text }))]);
    push_set(&mut app, "Full", vec![action("Full", &vec![("object.move", json!({"dx": 0})); files::MAX_ACTION_STEPS])]);
    push_set(&mut app, "Big", vec![big(String::new())]);
    let room = files::MAX_SET_FILE - set_file(&app, 2).len() - 4;
    app.ui.actions.delete_set("Big").unwrap();
    push_set(&mut app, "Big", vec![big("x".repeat(room))]);
    assert!(set_file(&app, 2).len() <= files::MAX_SET_FILE);
    let before = saved_sets(&app);
    for (params, want) in [
        (json!({}), "no such action"),
        (json!({"set": 0, "action": 1, "step": 0, "before": 0}), "no such action"),
        (json!({"set": 3, "action": 0, "step": 0, "before": 0}), "no such action"),
        (json!({"set": 0, "action": 0, "step": 3, "before": 0}), "no such action step"),
        (json!({"set": 0, "action": 0, "before": 0}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": 0}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": 0, "before": 4}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": 0, "before": "1"}), "no such action step"),
        (json!({"set": 0, "action": 0, "step": 0, "toSet": 3, "before": 0}), "no such action set"),
        (json!({"set": 0, "action": 0, "step": 0, "toSet": null, "before": 0}), "no such action set"),
        (json!({"set": 0, "action": 0, "step": 0, "toSet": 1, "toAction": 1, "before": 0}), "no such action"),
        (json!({"set": 0, "action": 0, "step": 0, "toSet": 1, "toAction": 0, "before": 0}), "the action already has 10,000 steps"),
        (json!({"set": 0, "action": 0, "step": 0, "toSet": 2, "toAction": 0, "before": 1}), "the action set would be larger than 8 MiB"),
    ] {
        assert_eq!(app.run(MOVE_STEP, params.clone()).unwrap_err(), want, "{params}");
        assert_eq!(saved_sets(&app), before, "{params}");
    }
}

/// An open Insert Menu Item dialog stays on its action and follows its step through step edits,
/// and closes with its step or its set.
#[test]
fn step_and_set_edits_keep_an_open_insert_menu_item_on_its_place() {
    let (mut app, _) = app_with_three_moves();
    crate::dialogs::action_insert_menu_item::open(&mut app, Some((0, 0)), Some(2));
    assert_eq!(dialog_action::<InsertMenuItemDialog>(&app).as_deref(), Some("Three Moves"));
    app.run(DUPLICATE, json!({"set": 0, "action": 0})).unwrap();
    assert_eq!(dialog_action::<InsertMenuItemDialog>(&app).as_deref(), Some("Three Moves"), "a copied action keeps it on its action");
    app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 0})).unwrap();
    assert_eq!(dialog_field::<InsertMenuItemDialog>(&app, "step"), json!(1), "a deleted step above moves it up");
    app.run(DELETE_STEP, json!({"set": 0, "action": 0, "step": 1})).unwrap();
    assert!(!is_open::<InsertMenuItemDialog>(&app), "deleting its step closes it");
    crate::dialogs::action_insert_menu_item::open(&mut app, Some((0, 0)), None);
    app.run(DELETE_SETS, json!({"names": ["Moves"]})).unwrap();
    assert!(!is_open::<InsertMenuItemDialog>(&app), "deleting its set closes it");
}

#[test]
fn an_excluded_stop_does_not_hold_and_batch_refuses_an_included_one() {
    let (mut app, x) = app_with_a_stop(false);
    assert_eq!(crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap_err(), "Batch cannot play an action with a stop");
    assert!(!is_open::<BatchDialog>(&app));
    app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap();
    crate::dialogs::action_batch::open(&mut app, 0, 0).unwrap();
    app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": true})).unwrap();
    assert_eq!(confirm_dialog::<BatchDialog>(&mut app).unwrap_err(), "Batch cannot play an action with a stop", "a stop included since it opened");
    assert!(is_open::<BatchDialog>(&app), "the refusal keeps the dialog open");
    cancel_dialog::<BatchDialog>(&mut app);
    app.run(INCLUDE_STEPS, json!({"set": 0, "action": 0, "step": 1, "included": false})).unwrap();
    play(&mut app, 0, 0).unwrap();
    assert!(!waiting_at_stop(&app));
    assert_eq!(x(&app), 7.0, "the excluded stop does not hold the playback");
}

/// An open Record Stop dialog stays on its action through set edits, and closes with its set.
#[test]
fn set_edits_keep_an_open_record_stop_on_its_action() {
    let mut app = app_with_sets();
    crate::dialogs::action_stop::open_record(&mut app, Some((1, 0)), None);
    assert_eq!(dialog_set::<RecordStopDialog>(&app).as_deref(), Some("Mine"));
    app.run(DUPLICATE, json!({"set": 0})).unwrap();
    assert_eq!(dialog_set::<RecordStopDialog>(&app).as_deref(), Some("Mine"), "a copied set above keeps it on its set");
    app.run(DELETE_SETS, json!({"names": ["Default Actions"]})).unwrap();
    assert_eq!(dialog_set::<RecordStopDialog>(&app).as_deref(), Some("Mine"), "a deleted set above keeps it on its set");
    app.run(DELETE_SETS, json!({"names": ["Mine"]})).unwrap();
    assert!(!is_open::<RecordStopDialog>(&app), "deleting its set closes it");
}

/// A Select Object step goes after the given step, or at the action's end, runs nothing when it
/// goes in, and plays as `select.byNote`: the steps after it act on the objects it selects.
#[test]
fn insert_select_object_goes_after_the_given_step_and_plays_as_a_selection_by_note() {
    let (mut app, x, noted) = app_with_a_noted_rect();
    let selection = app.session.active().unwrap().selection.clone();
    let r = app.run(INSERT_SELECT_OBJECT, json!({"set": 0, "action": 0, "step": 0, "note": "  badge "})).unwrap();
    assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
    assert_eq!(app.session.active().unwrap().selection, selection, "inserting runs nothing");
    assert_eq!(steps(&app, 0, 0)[1], (SELECT_OBJECT_STEP.to_string(), json!({"note": "badge", "caseSensitive": false, "wholeWord": false})));
    app.run(INSERT_SELECT_OBJECT, json!({"set": 0, "action": 0, "note": "Badge", "caseSensitive": true, "wholeWord": true})).unwrap();
    assert_eq!(steps(&app, 0, 0)[4].1, json!({"note": "Badge", "caseSensitive": true, "wholeWord": true}));
    play(&mut app, 0, 0).unwrap();
    assert_eq!((x(&app), noted(&app)), (1.0, 6.0), "the moves after the step move the object with the note");
    assert_eq!(app.session.active().unwrap().selection.objects.len(), 1);
}

#[test]
fn insert_select_object_refuses_bad_params_and_changes_nothing() {
    let (mut app, _, _) = app_with_a_noted_rect();
    let place = |extra: Value| {
        let mut p = json!({"set": 0, "action": 0});
        if let (Some(p), Some(extra)) = (p.as_object_mut(), extra.as_object()) {
            p.extend(extra.clone());
        }
        p
    };
    for (params, want) in [
        (place(json!({})), "the note is empty"),
        (place(json!({"note": " \t "})), "the note is empty"),
        (place(json!({"note": 5})), "`note` must be a string"),
        (place(json!({"note": "x".repeat(1025)})), "the note is longer than 1,024 bytes"),
        (place(json!({"note": "x", "caseSensitive": "yes"})), "`caseSensitive` must be true or false"),
        (place(json!({"note": "x", "wholeWord": 0})), "`wholeWord` must be true or false"),
        (place(json!({"note": "x", "action": 5})), "no such action"),
        (place(json!({"note": "x", "step": 9})), "no such action step"),
        (json!({"note": "x"}), "no such action"),
        (json!(["note"]), "`params` must be an object"),
    ] {
        assert_eq!(app.run(INSERT_SELECT_OBJECT, params.clone()).unwrap_err(), want, "{params}");
        assert_eq!(steps(&app, 0, 0).len(), 3, "{params}");
    }
    assert!(app.run(INSERT_SELECT_OBJECT, place(json!({"note": "x".repeat(1024)}))).is_ok(), "1,024 bytes goes in");
}
