use super::*;
use vectorcraft_engine::Session;

/// An app with the default set and the sets Mine (two actions) and Other.
pub(super) fn app_with_sets() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    let action = |name: &str| Action::new(name, vec![("object.move".into(), json!({"dx": 1, "dy": 0}))]);
    app.ui.action_sets.push(ActionSet::new("Mine", vec![action("Nudge"), action("Nudge Again")]));
    app.ui.action_sets.push(ActionSet::new("Other", vec![]));
    app
}

pub(super) fn set_names(app: &VectorcraftApp) -> Vec<String> {
    app.ui.action_sets.iter().map(|s| s.name.clone()).collect()
}

/// The bottom bar's Delete Selection button (stop, record, play, new set, new action, delete).
pub(super) fn trash(ctx: &egui::Context, app: &mut VectorcraftApp) -> egui::Pos2 {
    super::super::tests_appearance::frame_events(ctx, app, vec![], show);
    let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [_, _, _, _, _, delete] = bar[..] else { panic!("the bottom bar: {bar:?}") };
    delete.center()
}

/// A click on the name of set `name` with `modifiers` held.
pub(super) fn click_set(ctx: &egui::Context, app: &mut VectorcraftApp, name: &str, modifiers: egui::Modifiers) {
    use super::super::tests_appearance::{frame_events, text_rect};
    let pos = text_rect(&frame_events(ctx, app, vec![], show), &format!("▾ {name}")).center();
    let button = |pressed| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
    for e in [egui::Event::PointerMoved(pos), button(true), button(false)] {
        frame_events(ctx, app, vec![egui::Event::ModifiersChanged(modifiers), e], show);
    }
}

/// The question Delete Selection asks about sets, with the sets OK deletes.
pub(super) fn asked(app: &VectorcraftApp) -> Option<(String, Value)> {
    let d = app.ui.dialog.as_ref()?;
    (d.kind == crate::dialogs::confirm::KIND && d.str("__command") == DELETE_SETS).then(|| (d.str("message"), d.fields["__params"]["sets"].clone()))
}

pub(super) fn action_names(app: &VectorcraftApp, set: usize) -> Vec<String> {
    app.ui.action_sets[set].actions.iter().map(|a| a.name.clone()).collect()
}

/// The sets as Save Actions… writes them, without the ids.
pub(super) fn saved_sets(sets: &[ActionSet]) -> Value {
    serde_json::to_value(sets).unwrap()
}

/// Files in memory for `app`: what it writes, and what it reads back. Save dialogs pick
/// `/sets/` and the suggested name, open dialogs pick `open`.
pub(super) fn with_files(app: &mut VectorcraftApp, open: &str) -> std::rc::Rc<std::cell::RefCell<std::collections::BTreeMap<String, Vec<u8>>>> {
    let files = std::rc::Rc::new(std::cell::RefCell::new(std::collections::BTreeMap::<String, Vec<u8>>::new()));
    let (w, r) = (files.clone(), files.clone());
    app.services.write = Some(Box::new(move |path: &str, bytes: &[u8]| {
        w.borrow_mut().insert(path.to_string(), bytes.to_vec());
        Ok(())
    }));
    app.services.read = Some(Box::new(move |path: &str| r.borrow().get(path).cloned().ok_or_else(|| "no such file".to_string())));
    app.services.pick_save = Some(Box::new(|pick: &crate::FilePick| Some(format!("/sets/{}", pick.name))));
    let open = open.to_string();
    app.services.pick_open = Some(Box::new(move |_: &crate::FilePick| Some(open.clone())));
    files
}

/// An action set as another app saves it as text, written from scratch: the set `name` (none
/// when empty) and actions of steps, each a label and whether it is on, with no recorded values.
pub(super) fn aia_text(name: &str, actions: &[(&str, &[(&str, bool)])]) -> String {
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

/// The bottom bar's stop and Create New Action buttons (stop, record, play, new set, new
/// action, delete).
pub(super) fn stop_and_new_action(ctx: &egui::Context, app: &mut VectorcraftApp) -> (egui::Pos2, egui::Pos2) {
    super::super::tests_appearance::frame_events(ctx, app, vec![], show);
    let mut bar = crate::toolbar::tests::widget_rects(ctx, egui::vec2(24.0, 24.0));
    bar.sort_by(|a, b| a.left().total_cmp(&b.left()));
    let [stop, _, _, _, new_action, _] = bar[..] else { panic!("the bottom bar: {bar:?}") };
    (stop.center(), new_action.center())
}

/// The texts the dialog layer draws once the open dialog has settled.
pub(super) fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
    let ctx = egui::Context::default();
    let mut texts = vec![];
    for _ in 0..3 {
        texts = super::super::tests_appearance::frame_events(&ctx, app, vec![], |app, ui| crate::dialogs::show(app, ui.ctx()));
    }
    texts.into_iter().map(|(t, _)| t).collect()
}

/// A document with one rectangle selected and the set "Moves" with one action that moves it
/// right three times, by 1, 2 and 4 pt → (the app, the rectangle's left edge as it moves).
pub(super) fn app_with_three_moves() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("select.set", json!({"ids": [id.0]})).unwrap();
    let step = |dx: u32| ("object.move".to_string(), json!({"dx": dx, "dy": 0}));
    app.ui.action_sets = vec![ActionSet::new("Moves", vec![Action::new("Three Moves", vec![step(1), step(2), step(4)])])];
    let x = move |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - 10.0;
    (app, x)
}

pub(super) fn undo_steps(app: &VectorcraftApp) -> usize {
    app.session.active().unwrap().history.undo.len()
}

/// One frame of a shown window at `now`: the logic pass, then a UI pass that draws → what the
/// logic pass's [`tick_playback`] returns.
pub(super) fn frame(app: &mut VectorcraftApp, now: f64) -> Option<f64> {
    let wait = tick_playback(app, now);
    playback_drawn(app);
    wait
}

/// [`app_with_three_moves`] with a stop after the first move.
pub(super) fn app_with_a_stop(allow_continue: bool) -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
    let (mut app, x) = app_with_three_moves();
    let params = json!({"set": 0, "action": 0, "step": 0, "message": "Paint it", "allowContinue": allow_continue});
    assert_eq!(app.run(INSERT_STOP, params).unwrap(), json!({"set": 0, "action": 0, "step": 1}));
    (app, x)
}

pub(super) fn open_kind(app: &VectorcraftApp) -> Option<&str> {
    app.ui.dialog.as_ref().map(|d| d.kind.as_str())
}

/// [`app_with_three_moves`] with View › Outline inserted after the first move: the moves of 1,
/// Outline, then the moves of 2 and 4.
pub(super) fn app_with_a_view_step() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64) {
    let (mut app, x) = app_with_three_moves();
    let doc = app.session.active().unwrap().doc.clone();
    let r = app.run(INSERT_MENU_ITEM, json!({"set": 0, "action": 0, "step": 0, "command": "view.outline"})).unwrap();
    assert_eq!(r, json!({"set": 0, "action": 0, "step": 1}));
    assert!(!app.outline_view(), "inserting runs nothing");
    assert!(std::sync::Arc::ptr_eq(&app.session.active().unwrap().doc, &doc));
    assert_eq!(app.ui.action_sets[0].actions[0].steps[1], ("view.outline".to_string(), json!({})));
    (app, x)
}

/// The error of a played step that fails, with the step's place in the action (1 for the first)
/// and its command id.
pub(super) fn assert_names_step(e: &str, step: usize, id: &str) {
    let prefix = format!("action step {step} (`{id}`) failed: ");
    assert!(e.starts_with(&prefix) && e.len() > prefix.len(), "{e}");
    assert!(!e.contains("batch step"), "{e}");
}

/// [`app_with_three_moves`] with a second rectangle, 100 pt to the right, and the first one
/// selected again → (the app, the left edge of the first one as it moves, the ids of both).
pub(super) fn app_with_two_rectangles() -> (VectorcraftApp, impl Fn(&VectorcraftApp) -> f64, [u64; 2]) {
    let (mut app, x) = app_with_three_moves();
    let first = app.session.active().unwrap().selection.objects[0].0;
    let second = app.run("shape.rectangle", json!({"x": 110, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
    app.run("select.set", json!({"ids": [first]})).unwrap();
    (app, x, [first, second])
}

pub(super) fn left_of(app: &VectorcraftApp, id: u64) -> f64 {
    app.session.active().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().geometric_bounds().unwrap().x0
}

/// `n` actions with no steps, named A0, A1 …
pub(super) fn empty_actions(n: usize) -> Vec<Action> {
    (0..n).map(|i| Action::new(format!("A{i}"), vec![])).collect()
}

/// The steps of action `action` in set `set`: each command, with its `dx` when it has one.
pub(super) fn step_names(app: &VectorcraftApp, set: usize, action: usize) -> Vec<String> {
    app.ui.action_sets[set].actions[action]
        .steps
        .iter()
        .map(|(c, p)| match p.get("dx") {
            Some(dx) => format!("{c} {dx}"),
            None => c.clone(),
        })
        .collect()
}

/// Selects the first step of "Three Moves" by clicking its row, with the action open.
pub(super) fn click_first_step(ctx: &egui::Context, app: &mut VectorcraftApp) {
    use super::super::tests_appearance::{click, frame_events};
    set_pstate(ctx, "actions-expanded", BTreeSet::from([app.ui.action_sets[0].actions[0].id]));
    let texts = frame_events(ctx, app, vec![], show);
    let first = texts.iter().filter(|(s, _)| s == "object.move").map(|(_, r)| *r).min_by(|a, b| a.top().total_cmp(&b.top())).unwrap();
    click(ctx, app, first.center(), show);
}

/// [`app_with_three_moves`] with a second rectangle whose Attributes note is "Badge front" →
/// (the app, the first rectangle's x offset, the second rectangle's x offset).
#[allow(clippy::type_complexity)]
pub(super) fn app_with_a_noted_rect() -> (VectorcraftApp, Box<dyn Fn(&VectorcraftApp) -> f64>, Box<dyn Fn(&VectorcraftApp) -> f64>) {
    let (mut app, x) = app_with_three_moves();
    let first = app.session.active().unwrap().selection.objects.clone();
    let id =
        vectorcraft_doc::NodeId(app.run("shape.rectangle", json!({"x": 100, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap());
    app.run("attributes.set", json!({"ids": [id.0], "note": "Badge front"})).unwrap();
    app.run("select.set", json!({"ids": first.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
    let noted = move |app: &VectorcraftApp| app.session.active().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap().x0 - 100.0;
    (app, Box::new(x), Box::new(noted))
}
