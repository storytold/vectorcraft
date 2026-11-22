//! Fixtures the Actions panel's test files share. The sets live in one [`ActionList`]: set `i` is
//! `i` in [`ActionList::sets`] and its actions are its members in list order ([`model::members`]),
//! the order of the indices the commands take.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use craft::automation::actions::{Action, ActionList, ActionStep};
use craft::host::{PickKind, Picked, ScriptedFiles};
use craft::ui::dialog::{DialogHost, HeldOk, StackDialog};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::edits::ActionsQuestion;
use super::{INSERT_MENU_ITEM, INSERT_STOP, files, model, playback_drawn, tick_playback};
use crate::VectorcraftApp;

/// An app with the built-in set, in a window whose dialog host the tests drive and which has drawn its first frame, so
/// the window's keymap holds the app's commands.
pub(crate) fn new_app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    let ctx = app.ui_ctx.clone().expect("a window");
    crate::testing::app_frame(&mut app, &ctx, Default::default());
    app
}

/// An action named `name` with `steps`, each a command and its params, in no set yet.
pub(crate) fn action(name: &str, steps: &[(&str, Value)]) -> Action {
    Action::new(name, steps.iter().map(|(command, params)| ActionStep::new(*command, params.clone())).collect())
}

/// Adds the set `name` to `list` after the others, with `actions` in order.
fn push_into(list: &mut ActionList, name: &str, actions: Vec<Action>) {
    list.new_set(name).unwrap_or_else(|e| panic!("the set {name}: {e:?}"));
    for action in actions {
        let action = Action { set: Some(name.to_string()), ..action };
        let label = action.name.clone();
        list.push(action).unwrap_or_else(|e| panic!("the action {label}: {e:?}"));
    }
}

/// Adds the set `name` after the others, with `actions` in order.
pub(crate) fn push_set(app: &mut VectorcraftApp, name: &str, actions: Vec<Action>) {
    push_into(&mut app.ui.actions, name, actions);
}

/// Copies of the actions of the set `name` of `list`, in order.
fn members_of(list: &ActionList, name: &str) -> Vec<Action> {
    model::members(list, name).into_iter().filter_map(|i| list.action(i).cloned()).collect()
}

/// The name of set `set`.
fn name_of(app: &VectorcraftApp, set: usize) -> String {
    model::set_name(&app.ui.actions, set).unwrap_or_else(|| panic!("no set {set}")).to_string()
}

/// The actions of set `set`, in order, as copies a test keeps while it changes the app.
pub(crate) fn set_actions(app: &VectorcraftApp, set: usize) -> Vec<Action> {
    members_of(&app.ui.actions, &name_of(app, set))
}

/// The text of set `set` as Save Actions… writes it.
pub(crate) fn set_file(app: &VectorcraftApp, set: usize) -> String {
    files::write_set(&name_of(app, set), &set_actions(app, set))
}

/// Changes the actions of set `set` as `edit` changes a copy of them, and keeps the order of the
/// sets and of every other action: for what no command makes, such as a set at a file limit or an
/// exclusion past an action's last step. VectorCraft keeps every action in a set, so the list holds
/// no action outside one.
pub(crate) fn edit_set(app: &mut VectorcraftApp, set: usize, edit: impl FnOnce(&mut Vec<Action>)) {
    let list = &app.ui.actions;
    let mut edit = Some(edit);
    let mut rebuilt = ActionList::new();
    for (i, name) in list.sets().into_iter().enumerate() {
        let mut actions = members_of(list, name);
        if i == set
            && let Some(edit) = edit.take()
        {
            edit(&mut actions);
        }
        push_into(&mut rebuilt, name, actions);
    }
    assert!(edit.is_none(), "no set {set}");
    app.ui.actions.replace(rebuilt);
}

/// The actions of each set, in set order.
pub(crate) fn counts(app: &VectorcraftApp) -> Vec<usize> {
    let list = &app.ui.actions;
    list.sets().into_iter().map(|name| model::members(list, name).len()).collect()
}

/// An app with the default set and the sets Mine (two actions) and Other.
pub(crate) fn app_with_sets() -> VectorcraftApp {
    let mut app = new_app();
    let nudge = |name: &str| action(name, &[("object.move", json!({"dx": 1, "dy": 0}))]);
    push_set(&mut app, "Mine", vec![nudge("Nudge"), nudge("Nudge Again")]);
    push_set(&mut app, "Other", vec![]);
    app
}

pub(crate) fn set_names(app: &VectorcraftApp) -> Vec<String> {
    app.ui.actions.sets().into_iter().map(str::to_string).collect()
}

/// The sets Delete Selection's open question asks about, which its OK deletes.
pub(crate) fn asked(app: &VectorcraftApp) -> Option<Vec<String>> {
    app.dialogs()?.with(|h| h.get::<ActionsQuestion>().and_then(|q| q.0.tag_as::<Vec<String>>().cloned()))
}

pub(crate) fn action_names(app: &VectorcraftApp, set: usize) -> Vec<String> {
    set_actions(app, set).into_iter().map(|a| a.name).collect()
}

/// The sets as the preferences save them.
pub(crate) fn saved_sets(app: &VectorcraftApp) -> Value {
    model::prefs::serialize(&app.ui.actions, serde_json::value::Serializer).expect("the sets save")
}

/// Puts `picks` in place of the file service of `app`'s window.
fn install_picks(app: &mut VectorcraftApp, picks: ScriptedFiles) {
    crate::tests::picks::install_files(app, Arc::new(picks));
}

thread_local! {
    /// The file service of this test's [`with_files`] and the path its dialogs pick.
    static ANSWERING: RefCell<Option<(Arc<ScriptedFiles>, String)>> = const { RefCell::new(None) };
}

/// Files in memory for `app`: what it writes, and what it reads back. Every file dialog picks
/// `open`, a save dialog too, once [`poll_picks`] runs.
pub(crate) fn with_files(app: &mut VectorcraftApp, open: &str) -> Rc<RefCell<BTreeMap<String, Vec<u8>>>> {
    let files = Rc::new(RefCell::new(BTreeMap::<String, Vec<u8>>::new()));
    let (w, r) = (files.clone(), files.clone());
    // PLAN: hub-w3 keeps the `Services` readers and writer (`write`, `read`, `read_head`); only the pickers move to the host.
    app.services.write = Some(Box::new(move |path: &str, bytes: &[u8]| {
        w.borrow_mut().insert(path.to_string(), bytes.to_vec());
        Ok(())
    }));
    app.services.read = Some(Box::new(move |path: &str| r.borrow().get(path).cloned().ok_or_else(|| "no such file".to_string())));
    let picks = Arc::new(ScriptedFiles::held());
    ANSWERING.set(Some((picks.clone(), open.to_string())));
    crate::tests::picks::install_files(app, picks);
    files
}

/// Every file dialog of `app` is cancelled from now on.
pub(crate) fn cancel_picks(app: &mut VectorcraftApp) {
    ANSWERING.set(None);
    install_picks(app, ScriptedFiles::new());
}

/// Runs what a frame runs for the file dialogs that answered: the commands that waited for them.
pub(crate) fn poll_picks(app: &mut VectorcraftApp) {
    ANSWERING.with_borrow(|answering| {
        let Some((files, path)) = answering else { return };
        // Each poll answers every open dialog, so the open ones are the latest asked; the newest is answered first.
        for request in files.requests().iter().rev().take(files.open()) {
            files.answer(match request.kind {
                PickKind::Save => Picked::Saved { path: path.into(), file_type: None },
                _ => Picked::Paths(vec![path.into()]),
            });
        }
    });
    let ctx = app.ui_ctx.clone().expect("a test app has a window");
    crate::tests::picks::poll(app, &ctx);
}

/// An action set as another app saves it as text, written from scratch: the set `name` (none
/// when empty) and actions of steps, each a label and whether it is on, with no recorded values.
pub(crate) fn aia_text(name: &str, actions: &[(&str, &[(&str, bool)])]) -> String {
    let hex = |t: &str| format!("[ {} {} ]", t.len(), t.bytes().map(|b| format!("{b:02x}")).collect::<String>());
    let mut out = String::from("/version 3\n");
    if !name.is_empty() {
        out += &format!("/name {}\n", hex(name));
    }
    out += &format!("/isOpen 1\n/actionCount {}\n", actions.len());
    for (i, (action, steps)) in actions.iter().enumerate() {
        out += &format!("/action-{} {{\n\t/name {}\n\t/eventCount {}\n", i + 1, hex(action), steps.len());
        for (j, (label, on)) in steps.iter().enumerate() {
            out += &format!(
                "\t/event-{} {{\n\t\t/internalName (sample_step_{j})\n\t\t/localizedName {}\n\t\t/isOn {}\n\t\t/parameterCount 0\n\t}}\n",
                j + 1,
                hex(label),
                u8::from(*on)
            );
        }
        out += "}\n";
    }
    out
}

/// Runs `f` on the dialog host of `app`'s window, with the window's context.
fn with_host<R>(app: &VectorcraftApp, f: impl FnOnce(&egui::Context, &mut DialogHost) -> R) -> R {
    let ctx = app.ui_ctx.clone().expect("a test app has a window");
    app.dialogs().expect("the window's dialog host").with(|h| f(&ctx, h))
}

/// Runs the app's dialog pump, as each frame does after its passes.
fn pump(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a test app has a window");
    crate::dialogs::host::pump(app, &ctx);
}

/// Whether a `D` is open.
pub(crate) fn is_open<D: StackDialog>(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|d| d.with(|h| h.is_open::<D>()))
}

/// The key of the dialog on top, `None` when no dialog is open.
pub(crate) fn open_kind(app: &VectorcraftApp) -> Option<String> {
    app.dialogs()?.with(|h| h.open_dialogs().last().and_then(|d| d.key.clone()))
}

/// Sets `field` of the open `D` as a person editing it would (`ui.dialog.set`).
pub(crate) fn set_dialog<D: StackDialog>(app: &mut VectorcraftApp, field: &str, value: Value) {
    with_host(app, |ctx, h| {
        let id = h.get::<D>().map(|d| d.id()).expect("the dialog is open");
        h.set_json_field(ctx, id, field, value)
    })
    .unwrap_or_else(|e| panic!("{field}: {e}"));
}

/// Presses OK of the open `D` as its default button does, then runs the app's pump. A held OK
/// files its answer, the pump answers it, and a second OK closes the dialog with it: `Err` is the
/// reason the app refused it, and the dialog stays open then.
pub(crate) fn confirm_dialog<D: StackDialog>(app: &mut VectorcraftApp) -> Result<(), String> {
    let Some(id) = with_host(app, |_, h| h.get::<D>().map(|d| d.id())) else { return Err("the dialog is not open".into()) };
    if !with_host(app, |ctx, h| h.confirm::<D>(ctx)) {
        pump(app);
        if let Some(HeldOk::Refused(reason)) = with_host(app, |_, h| h.held_state(id)) {
            return Err(reason.as_str().to_string());
        }
        if !with_host(app, |ctx, h| h.confirm::<D>(ctx)) {
            return Err("OK was refused".into());
        }
    }
    pump(app);
    Ok(())
}

/// Closes the open `D` as Cancel does, then runs the app's pump.
pub(crate) fn cancel_dialog<D: StackDialog>(app: &mut VectorcraftApp) {
    assert!(with_host(app, |ctx, h| h.cancel::<D>(ctx)), "the dialog is open");
    pump(app);
}

/// A document with one rectangle selected and the set "Moves" with one action that moves it
/// right three times, by 1, 2 and 4 pt → (the app, the rectangle's left edge as it moves).
pub(crate) fn app_with_three_moves() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
    let mut app = new_app();
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let step = |dx: u32| ("object.move", json!({"dx": dx, "dy": 0}));
    app.ui.actions = ActionList::new();
    push_set(&mut app, "Moves", vec![action("Three Moves", &[step(1), step(2), step(4)])]);
    let x = move |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - 10.0;
    (app, x)
}

pub(crate) fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// One frame of a shown window at `now`: the logic pass, then a UI pass that draws → what the
/// logic pass's [`tick_playback`] returns.
pub(crate) fn frame(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    let wait = tick_playback(app, now);
    playback_drawn(app);
    wait
}

/// [`app_with_three_moves`] with a stop after the first move.
pub(crate) fn app_with_a_stop(allow_continue: bool) -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
    let (mut app, x) = app_with_three_moves();
    let params = json!({"set": 0, "action": 0, "step": 0, "message": "Paint it", "allowContinue": allow_continue});
    assert_eq!(app.run(INSERT_STOP, params).unwrap(), json!({"set": 0, "action": 0, "step": 1}));
    (app, x)
}

/// [`app_with_three_moves`] with View › Outline inserted after the first move: the moves of 1,
/// Outline, then the moves of 2 and 4.
pub(crate) fn app_with_a_view_step() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
    let (mut app, x) = app_with_three_moves();
    let doc = app.session.active().unwrap().doc.clone();
    let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "view.outline"})).unwrap();
    assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
    assert!(!app.outline_view(), "inserting runs nothing");
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
    assert_eq!(set_actions(&app, 0)[0].steps[1], ActionStep::new("view.outline", json!({})));
    (app, x)
}

/// The error of a played step that fails, with the step's place in the action (1 for the first)
/// and its command id.
pub(crate) fn assert_names_step(e: &str, step: usize, id: &str) {
    let prefix = format!("action step {step} (`{id}`) failed: ");
    assert!(e.starts_with(&prefix) && e.len() > prefix.len(), "{e}");
    assert!(!e.contains("batch step"), "{e}");
}

/// [`app_with_three_moves`] with a second rectangle, 100 pt to the right, and the first one
/// selected again → (the app, the left edge of the first one as it moves, the ids of both).
pub(crate) fn app_with_two_rectangles() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64, [u64; 2]) {
    let (mut app, x) = app_with_three_moves();
    let first = app.session.active().unwrap().selection.objects[0].0;
    let second = app.run("shape.rectangle", json!({"x": 110, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [first]})).unwrap();
    (app, x, [first, second])
}

pub(crate) fn left_of(app: &VectorcraftApp, id: u64) -> f64 {
    app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0
}

/// `n` actions with no steps, named A0, A1 …
pub(crate) fn empty_actions(n: usize) -> Vec<Action> {
    (0..n).map(|i| Action::new(format!("A{i}"), vec![])).collect()
}

/// The steps of action `action` in set `set`: each command, with its `dx` when it has one.
pub(crate) fn step_names(app: &VectorcraftApp, set: usize, action: usize) -> Vec<String> {
    set_actions(app, set)[action]
        .steps
        .iter()
        .map(|s| match s.params.get("dx") {
            Some(dx) => format!("{} {dx}", s.command),
            None => s.command.clone(),
        })
        .collect()
}

/// [`app_with_three_moves`] with a second rectangle whose Attributes note is "Badge front" →
/// (the app, the first rectangle's x offset, the second rectangle's x offset).
#[allow(clippy::type_complexity)]
pub(crate) fn app_with_a_noted_rect() -> (VectorcraftApp, Box<dyn Fn(&VectorcraftApp) -> f64>, Box<dyn Fn(&VectorcraftApp) -> f64>) {
    let (mut app, x) = app_with_three_moves();
    let first = app.session.active().unwrap().selection.objects.clone();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("attributes.set", json!({"ids": [id.0], "note": "Badge front"})).unwrap();
    app.run("select.set", json!({"ids": first.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let noted = move |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - 100.0;
    (app, Box::new(x), Box::new(noted))
}
