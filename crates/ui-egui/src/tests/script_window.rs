//! A script's ScriptUI dialog in the app: it opens when `show()` waits, takes clicks and changes
//! from the person and from `ui.dialog.confirm`, Enter and Escape, Stop, and quitting while it
//! waits.

use craft::automation::{ControlRequest, Reply};
use craft::ui::dialog::StackDialog as _;
use serde_json::{Value, json};
use vectorcraft_engine::script::{UiEvent, UiKind, UiValue};

use super::script_dialogs::{app, frame, is_open, key, open_id, pump};
use crate::dialogs::script_run::{CLOSE_KIND, CloseQuestion, GROWTH_KIND, GrowthQuestion, ScriptNotice, with_host};
use crate::dialogs::script_window::{self, KIND, Queued, ScriptWindow, with_window};
use crate::{VectorcraftApp, menus, script_runs};

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

const DRAW: &str = "vc.run('shape.rectangle', {x:0,y:0,width:10,height:10});";

const FORM: &str = "var w = new Window('dialog', 'Make'); var name = w.add('edittext', undefined, 'first', {name: 'name'}); \
    var bold = w.add('checkbox', undefined, 'Bold'); bold.onClick = function () { name.text = 'clicked'; }; \
    w.add('button', undefined, 'OK'); w.add('button', undefined, 'Cancel');";

fn start(app: &mut VectorcraftApp, source: &str) {
    menus::run_ui_command(app, "ui.runScript", &json!({"source": source, "name": "w.js"})).unwrap().unwrap();
}

/// Polls the run until it ends, or until its ScriptUI dialog is open and it waits (true).
fn until_window(app: &mut VectorcraftApp) -> bool {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return false;
        }
        script_runs::poll(app);
        if is_open::<ScriptWindow>(app) && app.session.script_window().is_some() {
            return true;
        }
    }
    panic!("the script neither ended nor opened its dialog");
}

/// Sets the dialog's fields as `ui.dialog.set` does, then gives the event as `ui.dialog.confirm`
/// does.
fn give(app: &mut VectorcraftApp, event: Value) -> Result<Value, String> {
    let id = open_id(app, KIND);
    for field in ["event", "control", "value"] {
        let value = event.get(field).cloned().unwrap_or(Value::Null);
        with_host(app, |host, ctx| host.set_json_field(ctx, id, field, value)).unwrap()?;
    }
    script_window::confirm(app)
}

/// One pass of the dialog host, then the frame's pump, which gives the run the next event that
/// waits.
fn step(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    frame(app, events);
    pump(app);
}

/// Closes the question quitting asks with Cancel, as Escape does, without the frame's pump.
fn cancel_close_question(app: &mut VectorcraftApp) {
    let id = open_id(app, CLOSE_KIND);
    with_host(app, |host, ctx| host.cancel_by_id(ctx, id)).unwrap();
    with_host(app, |host, _| host.take::<CloseQuestion>());
}

fn typed(app: &VectorcraftApp, i: usize) -> Option<String> {
    with_window(app, |d| d.typed(i).map(str::to_string)).flatten()
}

fn type_text(app: &VectorcraftApp, i: usize, text: &str) {
    with_window(app, |d| d.type_text(i, text)).unwrap();
}

fn queued(app: &VectorcraftApp) -> Vec<Queued> {
    with_window(app, |d| d.queued().iter().cloned().collect()).unwrap_or_default()
}

fn title(app: &VectorcraftApp) -> String {
    with_window(app, |d| d.json_fields().unwrap()["title"].as_str().unwrap_or_default().to_string()).unwrap()
}

fn log(app: &VectorcraftApp) -> Value {
    app.session.script_report().unwrap()["log"].clone()
}

#[test]
fn the_dialog_takes_events_and_stays_open_while_its_handlers_run() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} var r = w.show(); vc.log([r, name.text, bold.value]);"));
    assert!(until_window(&mut app));
    assert_eq!(title(&app), "Make");
    step(&mut app, Vec::new());
    give(&mut app, json!({"event": "click", "control": "Bold"})).unwrap();
    assert!(is_open::<ScriptWindow>(&app), "the dialog stays open for the next wait");
    assert!(until_window(&mut app));
    assert_eq!(app.session.script_window().unwrap().nodes[1].text, "clicked", "the handler changed the field");
    assert!(give(&mut app, json!({"event": "click", "control": 99})).is_err());
    assert!(give(&mut app, json!({"event": "wave"})).is_err());
    give(&mut app, json!({"event": "change", "control": "name", "value": "Ann"})).unwrap();
    assert!(until_window(&mut app));
    step(&mut app, vec![key(egui::Key::Enter)]);
    assert!(!until_window(&mut app), "Enter clicks OK, which closes the dialog");
    assert_eq!(log(&app), json!([[1, "Ann", true]]));
    assert!(!is_open::<ScriptWindow>(&app));
    assert_eq!(objects(&app), 1);
}

#[test]
fn text_still_in_a_field_reaches_the_script_before_a_click() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    let w = app.session.script_window().unwrap();
    with_window(&app, |d| {
        d.type_text(1, "Ann");
        let mut events = vec![UiEvent::Click(3)];
        d.commit_drafts(&w, &mut events).unwrap();
        assert_eq!(events, vec![UiEvent::Change(1, UiValue::Text("Ann".into())), UiEvent::Click(3)]);
        assert!(d.typed(1).is_none(), "the text is given once");
        for e in &events {
            d.queue(e);
        }
    })
    .unwrap();
    step(&mut app, Vec::new());
    assert!(until_window(&mut app), "the script takes the change and waits again");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "Ann"]]));
}

#[test]
fn escape_clicks_the_cancel_button_and_closes_a_dialog_without_one() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log(w.show());"));
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    step(&mut app, vec![key(egui::Key::Escape)]);
    assert!(!until_window(&mut app));
    // The next frame takes down the dialog's window.
    step(&mut app, Vec::new());
    assert!(!crate::dialogs::script_run::busy(&app), "the dialog Escape closed leaves nothing open");
    assert_eq!(log(&app), json!([2]));
    start(&mut app, "var w = new Window('dialog'); w.add('statictext', undefined, 'Hi'); vc.log(w.show());");
    assert!(until_window(&mut app));
    script_window::cancel(&mut app).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([0]));
}

#[test]
fn escape_keeps_the_dialog_open_until_the_script_closes_it() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('edittext', undefined, 'a'); w.onClose = function () { return false; }; vc.log(w.show());");
    assert!(until_window(&mut app));
    type_text(&app, 1, "typed");
    script_window::cancel(&mut app).unwrap();
    assert!(is_open::<ScriptWindow>(&app), "the dialog stays open");
    assert_eq!(queued(&app), vec![Queued::Close], "with the close after the typed text");
    assert!(until_window(&mut app), "the script takes the text and waits again");
    assert_eq!(app.session.script_window().unwrap().nodes[1].text, "typed");
    step(&mut app, Vec::new());
    assert!(until_window(&mut app), "onClose kept the dialog open");
    assert!(queued(&app).is_empty());
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn escape_gives_the_typed_text_before_the_cancel_click() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var e = w.add('edittext', undefined, 'old'); var c = w.add('button', undefined, 'Cancel'); \
        c.onClick = function () { vc.log(e.text); w.close(2); }; vc.log(w.show());",
    );
    assert!(until_window(&mut app));
    type_text(&app, 1, "typed");
    script_window::cancel(&mut app).unwrap();
    assert!(until_window(&mut app), "the script takes the text and waits again");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on Cancel closes the dialog");
    assert_eq!(log(&app), json!(["typed", 2]));
}

#[test]
fn a_second_dialog_replaces_the_first() {
    let mut app = app();
    start(
        &mut app,
        "var a = new Window('dialog', 'A'); a.add('edittext', undefined, 'x'); a.add('button', undefined, 'OK'); a.show(); \
        var b = new Window('dialog', 'B'); b.add('edittext', undefined, 'y'); b.add('button', undefined, 'OK'); vc.log([b.show(), b.children[0].text]);",
    );
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    type_text(&app, 1, "left over");
    assert!(until_window(&mut app));
    assert_eq!(title(&app), "B", "the second dialog's title");
    assert_eq!(typed(&app, 1), None, "without the first dialog's typed text");
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[1, "y"]]));
}

#[test]
fn a_dialog_function_after_show_opens_its_own_dialog() {
    let mut app = app();
    start(&mut app, &format!("{FORM} w.show(); alert('done');"));
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    for _ in 0..100 {
        script_runs::poll(&mut app);
        if is_open::<ScriptNotice>(&app) {
            break;
        }
    }
    assert!(is_open::<ScriptNotice>(&app), "the alert opens");
    assert!(!is_open::<ScriptWindow>(&app), "the dialog its show() ended is closed");
    assert!(app.session.script_dialog().is_some());
}

#[test]
fn an_alert_from_a_handler_ends_the_run_and_closes_the_dialog() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var e = w.add('edittext', undefined, 'a'); var b = w.add('button', undefined, 'Tell'); \
        b.onClick = function () { alert('told'); }; w.add('button', undefined, 'OK'); vc.log([w.show(), e.text]);",
    );
    assert!(until_window(&mut app));
    type_text(&app, 1, "typed");
    give(&mut app, json!({"event": "click", "control": "Tell"})).unwrap();
    assert!(until_window(&mut app), "the script takes the typed text first");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the handler's alert ends the run");
    let report = app.session.script_report().unwrap();
    assert!(report["message"].as_str().is_some_and(|m| m.contains("alert: a dialog cannot open inside a callback")), "{report}");
    assert!(!is_open::<ScriptWindow>(&app) && !is_open::<ScriptNotice>(&app), "no dialog of the run stays open");
    step(&mut app, Vec::new());
    assert!(!crate::dialogs::script_run::busy(&app), "the next frame takes down the dialog's window");
}

#[test]
fn stop_in_the_dialog_rolls_the_run_back() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} w.show(); vc.log('after');"));
    assert!(until_window(&mut app));
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
    assert_eq!(objects(&app), 0);
    assert_eq!(app.session.script_report().unwrap()["dialogs"][0]["answer"], "stop");
    assert!(!is_open::<ScriptWindow>(&app));
}

#[test]
fn quitting_while_the_dialog_waits_asks_and_cancel_shows_it_again() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} vc.log(w.show());"));
    assert!(until_window(&mut app));
    type_text(&app, 1, "typed");
    crate::dialogs::script_run::ask_to_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app));
    cancel_close_question(&mut app);
    assert!(until_window(&mut app), "the dialog shows again");
    assert_eq!(typed(&app, 1).as_deref(), Some("typed"), "with the text typed in its field");
    crate::dialogs::script_run::ask_to_close(&mut app);
    assert!(super::script_dialogs::confirm(&mut app, CLOSE_KIND));
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
    assert!(!is_open::<ScriptWindow>(&app));
}

#[test]
fn quitting_while_a_handler_runs_keeps_the_events_still_queued() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} name.onChange = function () {{ name.text = name.text + '!'; }}; vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "change", "control": "name", "value": "Ann"})).unwrap();
    with_window(&app, |d| d.queue(&UiEvent::Click(3))).unwrap();
    crate::dialogs::script_run::ask_to_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app));
    script_runs::poll(&mut app);
    cancel_close_question(&mut app);
    assert!(until_window(&mut app), "the dialog shows again");
    assert!(!queued(&app).is_empty(), "with the click still queued");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "Ann!"]]));
}

#[test]
fn a_click_from_ui_dialog_confirm_gives_the_typed_text_first() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    type_text(&app, 1, "typed");
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(until_window(&mut app), "the script takes the text and waits again");
    assert_eq!(typed(&app, 1), None, "the text is given once");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "typed"]]));
}

#[test]
fn a_typed_text_the_script_refuses_keeps_every_field_and_queues_nothing() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); w.add('edittext', undefined, 'a'); w.add('edittext', undefined, 'b'); \
        w.add('button', undefined, 'OK'); w.add('button', undefined, 'Cancel'); vc.log(w.show());",
    );
    assert!(until_window(&mut app));
    let long = "x".repeat(16 * 1024 + 1);
    type_text(&app, 1, &long);
    type_text(&app, 2, "fine");
    let kept = |app: &VectorcraftApp| {
        assert!(is_open::<ScriptWindow>(app), "the dialog stays open");
        assert_eq!(typed(app, 1).as_deref(), Some(long.as_str()), "the refused text stays in its field");
        assert_eq!(typed(app, 2).as_deref(), Some("fine"), "and so does the other one");
        assert!(queued(app).is_empty(), "nothing is queued");
    };
    assert!(give(&mut app, json!({"event": "click", "control": "OK"})).is_err(), "OK is an error");
    kept(&app);
    assert!(script_window::cancel(&mut app).is_err(), "Cancel is an error");
    kept(&app);
    let ctx = app.ui_ctx.clone().unwrap();
    for (method, params) in [("ui.dialog.cancel", json!({})), ("ui.dialog.set", json!({"fields": [], "action": "cancel"}))] {
        let (req, _) = ControlRequest::new(method, params);
        let Reply::Done(r) = crate::control::handle(&mut app, &ctx, &req) else { panic!("{method}: not done") };
        assert_eq!(r["ok"], false, "{method} answers with the error: {r}");
        kept(&app);
    }
    assert!(app.session.script_window().is_some(), "the script still waits");
    type_text(&app, 1, "a");
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(until_window(&mut app), "the script takes the other text and waits again");
    assert_eq!(app.session.script_window().unwrap().nodes[2].text, "fine");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([1]));
}

#[test]
fn a_typed_text_the_script_refuses_stays_in_its_field_after_it_loses_the_focus() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('edittext', undefined, 'a'); w.add('button', undefined, 'OK'); vc.log(w.show());");
    assert!(until_window(&mut app));
    let long = "x".repeat(16 * 1024 + 1);
    with_window(&app, |d| {
        let mut events = vec![UiEvent::Change(1, UiValue::Text(long.clone())), UiEvent::Change(1, UiValue::Text("fine".into()))];
        assert!(d.keep_refused_texts(&mut events).is_some(), "the long text is an error");
        assert_eq!(events, vec![UiEvent::Change(1, UiValue::Text("fine".into()))], "a text the script takes is still given");
    })
    .unwrap();
    assert_eq!(typed(&app, 1).as_deref(), Some(long.as_str()), "the refused text goes back in its field");
    frame(&app, Vec::new());
    frame(&app, Vec::new());
    assert_eq!(typed(&app, 1).as_deref(), Some(long.as_str()), "and stays there while the field has no focus");
    assert!(app.session.script_window().is_some(), "the script still waits");
    assert!(give(&mut app, json!({"event": "click", "control": "OK"})).is_err(), "OK is an error until it is fixed");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

/// One pass of the dialog host with accessibility on, drawn as part of the window. → what the pass
/// gave.
fn access_frame(app: &mut VectorcraftApp) -> egui::FullOutput {
    let ctx = app.ui_ctx.clone().unwrap();
    ctx.enable_accesskit();
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    crate::testing::part_frame(app, &ctx, egui::RawInput { screen_rect, ..Default::default() }, |app, ui| {
        if let Some(dialogs) = app.dialogs() {
            dialogs.with(|host| host.run(ui));
        }
    })
}

/// One pass of the dialog host with accessibility on. → the nodes it reports.
fn access_nodes(app: &mut VectorcraftApp) -> Vec<(egui::accesskit::NodeId, egui::accesskit::Node)> {
    access_frame(app).platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default()
}

#[test]
fn a_typed_text_the_script_refuses_stays_in_its_field_when_a_click_on_a_button_takes_the_focus() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('edittext', undefined, 'a'); w.add('button', undefined, 'OK'); vc.log(w.show());");
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    access_nodes(&mut app);
    let nodes = access_nodes(&mut app);
    let center = |found: Option<&egui::accesskit::Node>, what: &str| {
        let b = found.and_then(egui::accesskit::Node::bounds).unwrap_or_else(|| panic!("no {what}"));
        egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5)
    };
    let field = center(nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::TextInput), "edit field");
    let ok = center(nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::Button && n.label() == Some("OK")), "OK button");
    let click = |app: &mut VectorcraftApp, at: egui::Pos2| {
        let button =
            |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        step(app, vec![egui::Event::PointerMoved(at)]);
        step(app, vec![button(true)]);
        step(app, vec![button(false)]);
    };
    click(&mut app, field);
    let long = "x".repeat(16 * 1024 + 1);
    step(&mut app, vec![egui::Event::Text(long.clone())]);
    let typed_now = typed(&app, 1).expect("the typed text is kept while the field has the focus");
    assert!(typed_now.len() > 16 * 1024 && typed_now.contains(&long), "the field holds the long text");
    click(&mut app, ok);
    step(&mut app, Vec::new());
    assert!(is_open::<ScriptWindow>(&app), "the dialog stays open");
    assert_eq!(typed(&app, 1), Some(typed_now), "the refused text stays in its field");
    assert!(queued(&app).is_empty(), "nothing is queued");
    assert!(app.ui.status.contains("16 KiB"), "the error is in the status bar: {}", app.ui.status);
    assert!(app.session.script_window().is_some(), "the script still waits");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn screen_readers_read_a_group_by_its_name() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var g = w.add('group', undefined, {name: 'opts'}); g.add('checkbox', undefined, 'Bold'); vc.log(w.show());",
    );
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    access_nodes(&mut app);
    let nodes = access_nodes(&mut app);
    let group = nodes.iter().find(|(_, n)| n.role() == egui::accesskit::Role::Group && n.label() == Some("opts"));
    let (group, _) = group.unwrap_or_else(|| panic!("the group is in the accessibility tree"));
    let bold = nodes.iter().find(|(_, n)| n.role() == egui::accesskit::Role::CheckBox && n.label() == Some("Bold"));
    let (bold, _) = bold.unwrap_or_else(|| panic!("no Bold check box"));
    let parent_of = |id: egui::accesskit::NodeId| nodes.iter().find(|(_, n)| n.children().contains(&id)).map(|(p, _)| *p);
    let mut at = Some(*bold);
    let mut inside = false;
    for _ in 0..16 {
        at = at.and_then(parent_of);
        if at == Some(*group) {
            inside = true;
            break;
        }
    }
    assert!(inside, "the check box is inside the group in the accessibility tree");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn an_edit_field_takes_at_most_its_draft_limit_while_it_is_typed() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('edittext', undefined, ''); w.add('button', undefined, 'OK'); vc.log(w.show());");
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    access_nodes(&mut app);
    let nodes = access_nodes(&mut app);
    let b = nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::TextInput).and_then(egui::accesskit::Node::bounds);
    let b = b.unwrap_or_else(|| panic!("no edit field"));
    let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    step(&mut app, vec![egui::Event::PointerMoved(at)]);
    step(&mut app, vec![button(true)]);
    step(&mut app, vec![button(false)]);
    step(&mut app, vec![egui::Event::Paste("x".repeat(script_window::DRAFT_CHARS + 100))]);
    let typed_now = typed(&app, 1).expect("the pasted text is kept while the field has the focus");
    assert_eq!(typed_now.chars().count(), script_window::DRAFT_CHARS, "the draft stops at its limit");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn a_queued_click_reaches_its_button_after_a_handler_adds_a_control_before_it() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var p = w.add('panel'); var e = w.add('edittext', undefined, 'a'); w.add('button', undefined, 'OK'); \
        e.onChange = function () { p.add('statictext', undefined, 'Added'); }; vc.log([w.show(), e.text]);",
    );
    assert!(until_window(&mut app));
    type_text(&app, 2, "typed");
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(until_window(&mut app), "the script takes the text and waits again");
    assert_eq!(app.session.script_window().unwrap().nodes[4].text, "OK", "the handler added a control before OK");
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "typed"]]));
}

/// Polls the run until the question about its growth opens.
fn until_growth(app: &mut VectorcraftApp) {
    for _ in 0..10_000 {
        if is_open::<GrowthQuestion>(app) {
            return;
        }
        assert!(app.session.script_running(), "the run asks about its growth");
        script_runs::poll(app);
    }
    panic!("the run did not ask about its growth");
}

#[test]
fn continue_on_the_growth_question_shows_the_dialog_again_with_its_queued_click_and_typed_text() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var e = w.add('edittext', undefined, 'a'); var g = w.add('button', undefined, 'Grow'); \
        w.add('button', undefined, 'OK'); g.onClick = function () { vc.run('shape.rectangle', {x:0,y:0,width:10,height:10}); \
        for (var k = 0; k < 17; k++) { vc.run('select.all'); vc.run('edit.duplicate'); } }; vc.log([w.show(), e.text]);",
    );
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "Grow"})).unwrap();
    type_text(&app, 1, "typed");
    with_window(&app, |d| d.queue(&UiEvent::Click(3))).unwrap();
    until_growth(&mut app);
    assert!(is_open::<ScriptWindow>(&app), "the question opens over the dialog");
    assert_eq!(typed(&app, 1).as_deref(), Some("typed"), "the text typed in its field waits under the question");
    // Enter goes to the question over the dialog, whose default is Continue, and clicks nothing
    // in the dialog under it.
    frame(&app, Vec::new());
    frame(&app, vec![key(egui::Key::Enter)]);
    assert!(!is_open::<GrowthQuestion>(&app), "Enter answers the question");
    assert_eq!(queued(&app), vec![Queued::Click(4)], "the dialog under it takes no click from Enter");
    pump(&mut app);
    assert!(is_open::<ScriptWindow>(&app), "Continue shows the dialog again");
    // The run waits at its dialog again, takes the typed text, then the queued click on OK.
    run_to_end(&mut app);
    assert_eq!(log(&app), json!([[1, "typed"]]));
}

/// Takes the run through the events its dialog has queued, one wait at a time, to its end.
fn run_to_end(app: &mut VectorcraftApp) {
    for _ in 0..8 {
        if !until_window(app) {
            return;
        }
        step(app, Vec::new());
    }
    panic!("the run did not end with the events its dialog had queued");
}

#[test]
fn a_typed_text_stays_with_its_field_after_a_handler_adds_a_control_before_it_and_the_run_grows() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var p = w.add('panel'); var e = w.add('edittext', undefined, 'a'); \
        var g = w.add('button', undefined, 'Grow'); w.add('button', undefined, 'OK'); g.onClick = function () { \
        p.add('statictext', undefined, 'Added'); vc.run('shape.rectangle', {x:0,y:0,width:10,height:10}); \
        for (var k = 0; k < 17; k++) { vc.run('select.all'); vc.run('edit.duplicate'); } }; vc.log([w.show(), e.text]);",
    );
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    give(&mut app, json!({"event": "click", "control": "Grow"})).unwrap();
    type_text(&app, 2, "typed");
    with_window(&app, |d| d.queue(&UiEvent::Click(4))).unwrap();
    step(&mut app, Vec::new());
    until_growth(&mut app);
    assert!(super::script_dialogs::confirm(&mut app, GROWTH_KIND));
    assert!(until_window(&mut app), "the script waits at its dialog again");
    let w = app.session.script_window().unwrap();
    assert_eq!(w.nodes[2].text, "Added", "the handler added a control before the edit field");
    assert_eq!((w.nodes[3].kind, w.nodes[3].text.as_str()), (UiKind::EditText, "typed"), "the typed text reached its field, now one place later");
    run_to_end(&mut app);
    assert_eq!(log(&app), json!([[1, "typed"]]));
}

#[test]
fn queued_clicks_reach_their_buttons_after_a_handler_removes_a_control_before_them() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var g = w.add('group'); var x = g.add('button', undefined, 'x'); var y = g.add('button', undefined, 'y'); \
         var z = g.add('button', undefined, 'z'); var hit = []; x.onClick = function () { hit.push('x'); g.remove(x); }; \
         y.onClick = function () { hit.push('y'); }; z.onClick = function () { hit.push('z'); }; vc.log([w.show(), hit]);",
    );
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "x"})).unwrap();
    with_window(&app, |d| {
        d.queue(&UiEvent::Click(2));
        d.queue(&UiEvent::Click(3));
    })
    .unwrap();
    assert!(until_window(&mut app), "the script runs x's handler and waits again");
    assert_eq!(app.session.script_window().unwrap().nodes[2].text, "y", "the handler removed x");
    step(&mut app, Vec::new());
    assert!(until_window(&mut app), "the script runs y's handler and waits again");
    assert!(queued(&app).is_empty(), "nothing is left queued");
    give(&mut app, json!({"event": "close"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[0, ["x", "y"]]]), "the click on the removed x is dropped and the click on y reaches y");
}

#[test]
fn a_click_from_ui_dialog_confirm_waits_behind_the_events_still_queued() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var g = w.add('group'); var x = g.add('button', undefined, 'x'); var y = g.add('button', undefined, 'y'); \
         var hit = []; x.onClick = function () { hit.push('x'); }; y.onClick = function () { hit.push('y'); w.close(); }; \
         vc.log([w.show(), hit]);",
    );
    assert!(until_window(&mut app));
    let ids: Vec<u64> = app.session.script_window().unwrap().nodes.iter().map(|n| n.id).collect();
    with_window(&app, |d| d.queue(&UiEvent::Click(2))).unwrap();
    give(&mut app, json!({"event": "click", "control": "y"})).unwrap();
    assert_eq!(queued(&app), vec![Queued::Click(ids[2]), Queued::Click(ids[3])], "the confirmed click goes after the queued one");
    run_to_end(&mut app);
    assert_eq!(log(&app)[0][1], json!(["x", "y"]), "the handlers ran in the order the clicks were made");
}

#[test]
fn a_queued_list_change_is_dropped_after_a_handler_removes_an_item() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var rm = w.add('button', undefined, 'Remove'); var l = w.add('listbox', undefined, ['a', 'b', 'c']); \
         rm.onClick = function () { l.remove(0); }; vc.log([w.show(), l.selection === null]);",
    );
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "Remove"})).unwrap();
    with_window(&app, |d| d.queue(&UiEvent::Change(2, UiValue::Index(Some(1))))).unwrap();
    assert!(until_window(&mut app), "the script runs the handler and waits again");
    step(&mut app, Vec::new());
    assert!(queued(&app).is_empty(), "the change is dropped");
    assert_eq!(app.session.script_window().unwrap().nodes[2].selection, None);
    give(&mut app, json!({"event": "close"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[0, true]]));
}

#[test]
fn a_queued_list_change_is_dropped_after_a_handler_replaces_the_item_with_one_of_the_same_text() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var rm = w.add('button', undefined, 'Replace'); var l = w.add('listbox', undefined, ['a', 'b', 'c']); \
         rm.onClick = function () { l.remove(2); l.add('item', 'c'); }; vc.log([w.show(), l.selection === null]);",
    );
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "Replace"})).unwrap();
    with_window(&app, |d| d.queue(&UiEvent::Change(2, UiValue::Index(Some(2))))).unwrap();
    assert!(until_window(&mut app), "the script runs the handler and waits again");
    step(&mut app, Vec::new());
    assert!(queued(&app).is_empty(), "the change chose the item that was replaced, so it is dropped");
    assert_eq!(app.session.script_window().unwrap().nodes[2].selection, None);
    give(&mut app, json!({"event": "close"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[0, true]]));
}

#[test]
fn cancel_on_the_quit_question_does_not_show_a_dialog_whose_show_ended() {
    let mut app = app();
    start(
        &mut app,
        "var a = new Window('dialog', 'A'); a.add('button', undefined, 'OK'); a.show(); \
        var b = new Window('dialog', 'B'); b.add('button', undefined, 'OK'); vc.log(b.show());",
    );
    assert!(until_window(&mut app));
    let first = app.session.script_window().unwrap().shown;
    crate::dialogs::script_run::ask_to_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app));
    app.session.script_window_answer(UiEvent::Click(1)).unwrap();
    for _ in 0..10_000 {
        if app.session.script_window().is_some_and(|w| w.shown != first) {
            break;
        }
        script_runs::poll(&mut app);
    }
    assert_eq!(app.session.script_window().map(|w| w.title().to_string()), Some("B".to_string()), "the run waits at the second show()");
    crate::dialogs::script_run::settle_close(&mut app);
    assert!(is_open::<CloseQuestion>(&app));
    let open = with_host(&app, |host, _| host.open_dialogs()).unwrap();
    let question = open.iter().find(|d| d.key.as_deref() == Some(CLOSE_KIND)).unwrap();
    assert_eq!(question.parent, None, "the first dialog is forgotten");
    assert!(!is_open::<ScriptWindow>(&app));
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn ui_dialog_confirm_takes_a_whole_float_as_a_list_index_and_refuses_a_fraction() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var l = w.add('dropdownlist', undefined, ['a', 'b'], {name: 'pick'}); w.add('button', undefined, 'OK'); \
         vc.log([w.show(), l.selection.text]);",
    );
    assert!(until_window(&mut app));
    assert!(give(&mut app, json!({"event": "change", "control": "pick", "value": 1.5})).is_err());
    give(&mut app, json!({"event": "change", "control": "pick", "value": 1.0})).unwrap();
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[1, "b"]]));
}

#[test]
fn ui_dialog_set_with_an_action_gives_the_event() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    let ctx = app.ui_ctx.clone().unwrap();
    let run = |app: &mut VectorcraftApp, method: &str, params: Value| {
        let (req, _) = ControlRequest::new(method, params);
        let Reply::Done(r) = crate::control::handle(app, &ctx, &req) else { panic!("{method}: not done") };
        r
    };
    let fields = json!({"fields": [{"field": "event", "value": "change"}, {"field": "control", "value": "name"}, {"field": "value", "value": "Ann"}], "action": "confirm"});
    let r = run(&mut app, "ui.dialog.set", fields);
    assert_eq!(r["ok"], true, "{r}");
    assert!(until_window(&mut app));
    let r = run(&mut app, "ui.dialog.set", json!({"field": "title", "value": "x"}));
    assert_eq!(r["ok"], false, "the title is read only: {r}");
    let r = run(&mut app, "ui.dialog.set", json!({"field": "event", "value": "click", "action": "wave"}));
    assert_eq!(r["ok"], false, "{r}");
    assert_eq!(with_window(&app, |d| d.json_fields().unwrap()["event"].clone()), Some(Value::Null), "a refused action sets no field");
    run(&mut app, "ui.dialog.set", json!({"fields": [{"field": "event", "value": "click"}, {"field": "control", "value": "OK"}]}));
    let r = run(&mut app, "ui.dialog.confirm", json!({}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[1, "Ann"]]));
}

#[test]
fn an_unnamed_edit_field_is_called_by_the_static_text_before_it() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var g = w.add('group'); g.add('statictext', undefined, 'Width:'); g.add('edittext', undefined, '5'); \
         w.add('edittext', undefined, 'x', {name: 'height'}); w.add('edittext', undefined, 'y'); \
         var h = w.add('statictext', undefined, 'Secret'); h.visible = false; w.add('edittext', undefined, 'z'); \
         w.add('statictext', undefined, 'Account'); w.add('edittext', undefined, 'a', {name: 'user'}); w.add('button', undefined, 'OK'); w.show();",
    );
    assert!(until_window(&mut app));
    let w = app.session.script_window().unwrap();
    let name = |i: usize| script_window::screen_reader_name(&w.nodes[i], script_window::prompt_of(&w, i));
    assert_eq!(name(3), "Width:", "the static text before it in its group");
    assert_eq!(name(4), "height", "the name the script gave it");
    assert_eq!(name(5), "Text", "the control before it is no static text, and the script gave it no name");
    assert_eq!(name(7), "Text", "a hidden static text is no prompt");
    assert_eq!(name(9), "Account", "the visible text before it goes before the name the script gave");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn a_title_a_handler_changes_and_a_list_box_reach_the_dialog() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog', 'First'); w.add('statictext', undefined, 'Fruit:'); var l = w.add('listbox', undefined, ['apple', 'pear']); \
         l.selection = 1; w.add('statictext', undefined, 'Size:'); w.add('dropdownlist', undefined, ['small', 'large']); \
         var b = w.add('button', undefined, 'Rename'); b.onClick = function () { w.text = 'Renamed'; }; \
         w.add('button', undefined, 'OK'); w.show();",
    );
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "Rename"})).unwrap();
    assert!(until_window(&mut app));
    assert_eq!(title(&app), "Renamed", "the heading follows the handler's title");
    access_nodes(&mut app);
    let nodes = access_nodes(&mut app);
    let list = nodes.iter().any(|(_, n)| n.role() == egui::accesskit::Role::ListBox && n.label() == Some("Fruit:"));
    assert!(list, "screen readers read the list box by its prompt");
    let pear = nodes.iter().any(|(_, n)| n.label() == Some("pear") && n.is_selected() == Some(true));
    assert!(pear, "with its selected item");
    let drop = nodes.iter().any(|(_, n)| n.role() == egui::accesskit::Role::ComboBox && n.label() == Some("Size:"));
    assert!(drop, "screen readers read the drop-down list by its prompt");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn an_arrow_key_and_a_click_in_a_list_box_give_the_script_the_change() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var l = w.add('listbox', undefined, ['apple', 'pear', 'plum']); l.selection = 0; \
         var picked = []; l.onChange = function () { picked.push(l.selection.index); }; \
         w.add('button', undefined, 'OK'); vc.log([w.show(), picked]);",
    );
    assert!(until_window(&mut app));
    let mut on_list = false;
    for _ in 0..8 {
        step(&mut app, vec![key(egui::Key::Tab)]);
        let out = access_frame(&mut app);
        on_list = out.platform_output.accesskit_update.is_some_and(|u| {
            u.nodes.iter().any(|(id, n)| *id == u.focus && matches!(n.role(), egui::accesskit::Role::ListBox | egui::accesskit::Role::ListItem))
        });
        if on_list {
            break;
        }
    }
    assert!(on_list, "Tab puts the focus on the list box");
    step(&mut app, vec![key(egui::Key::ArrowDown)]);
    assert!(until_window(&mut app), "the change handler runs and the dialog waits again");
    step(&mut app, Vec::new());
    access_nodes(&mut app);
    let nodes = access_nodes(&mut app);
    let b = nodes
        .iter()
        .map(|(_, n)| n)
        .find(|n| n.role() == egui::accesskit::Role::ListItem && n.label() == Some("plum"))
        .and_then(egui::accesskit::Node::bounds)
        .unwrap_or_else(|| panic!("no plum row"));
    // The top half of the row: the OK button below the list starts in the row's lower half.
    let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, b.y0 as f32 + 4.0);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    step(&mut app, vec![egui::Event::PointerMoved(at)]);
    step(&mut app, vec![button(true)]);
    step(&mut app, vec![button(false)]);
    assert!(until_window(&mut app), "the change handler runs again");
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[1, [1, 2]]]), "the arrow key picked pear, the click picked plum, and the script was told each time");
}

#[test]
fn enter_with_the_focus_on_a_slider_clicks_the_default_button() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('slider', undefined, 2, 0, 5); w.add('button', undefined, 'OK'); vc.log(w.show());");
    assert!(until_window(&mut app));
    let mut on_slider = false;
    for _ in 0..8 {
        step(&mut app, vec![key(egui::Key::Tab)]);
        let out = access_frame(&mut app);
        on_slider = out
            .platform_output
            .accesskit_update
            .is_some_and(|u| u.nodes.iter().any(|(id, n)| *id == u.focus && n.role() == egui::accesskit::Role::Slider));
        if on_slider {
            break;
        }
    }
    assert!(on_slider, "Tab puts the focus on the slider");
    step(&mut app, vec![key(egui::Key::Enter)]);
    assert!(!until_window(&mut app), "Enter clicks OK, which closes the dialog");
    assert_eq!(log(&app), json!([1]));
}

#[test]
fn the_dialog_closes_when_its_show_ends_and_the_script_runs_on() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('button', undefined, 'OK'); w.show(); while (true) {}");
    assert!(until_window(&mut app));
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    for _ in 0..3 {
        script_runs::poll(&mut app);
    }
    assert!(app.session.script_running(), "the script still runs");
    assert!(!is_open::<ScriptWindow>(&app), "the dialog its show() ended is closed");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn stop_ends_the_run() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} w.show();"));
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    let nodes = access_nodes(&mut app);
    let stop = nodes.iter().map(|(_, n)| n).find(|n| n.role() == egui::accesskit::Role::Button && n.label() == Some("Stop"));
    let b = stop.and_then(egui::accesskit::Node::bounds).expect("a Stop button");
    let at = egui::pos2((b.x0 + b.x1) as f32 * 0.5, (b.y0 + b.y1) as f32 * 0.5);
    let button = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
    step(&mut app, vec![egui::Event::PointerMoved(at)]);
    step(&mut app, vec![button(true)]);
    step(&mut app, vec![button(false)]);
    assert!(!until_window(&mut app));
    assert_eq!(objects(&app), 0, "the run is put back");
    assert!(!is_open::<ScriptWindow>(&app));
}

#[test]
fn a_large_dialog_draws_in_a_small_window() {
    let mut app = app();
    let source = "var w = new Window('dialog', 'Big'); var p = w.add('panel', undefined, 'Lots'); \
        for (var i = 0; i < 200; i++) p.add('checkbox', undefined, 'Option ' + i); \
        var l = w.add('listbox', undefined, ['a', 'b']); var d = w.add('dropdownlist', undefined, ['x', 'y']); \
        w.add('slider', undefined, 2, 0, 5); w.add('progressbar', undefined, 1, 4); w.add('edittext', undefined, 'm', {multiline: true}); \
        w.add('button', undefined, 'OK'); vc.log(w.show());";
    start(&mut app, source);
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    step(&mut app, Vec::new());
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([1]));
}

#[test]
fn a_close_closes_the_dialog_after_a_click_on_a_control_it_lacks() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log(w.show());"));
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    assert!(give(&mut app, json!({"event": "click", "control": "missing"})).is_err());
    // Only `event` changes; `control` still names the missing control.
    let id = open_id(&app, KIND);
    with_host(&app, |host, ctx| host.set_json_field(ctx, id, "event", json!("close"))).unwrap().unwrap();
    script_window::confirm(&mut app).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([0]));
}

#[test]
fn a_cancel_while_a_handler_runs_reaches_the_dialog_s_next_wait() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    give(&mut app, json!({"event": "click", "control": "Bold"})).unwrap();
    assert!(app.session.script_window().is_none(), "the click's handler has not run yet");
    script_window::cancel(&mut app).unwrap();
    assert_eq!(queued(&app), vec![Queued::Cancel]);
    assert!(until_window(&mut app));
    step(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued cancel clicks Cancel");
    assert_eq!(log(&app), json!([[2, "clicked"]]));
}

#[test]
fn a_queued_text_the_script_refuses_goes_back_in_its_field() {
    let mut app = app();
    start(
        &mut app,
        "var w = new Window('dialog'); var name = w.add('edittext', undefined, 'first', {name: 'name'}); \
         var lock = w.add('checkbox', undefined, 'Lock'); lock.onClick = function () { name.enabled = false; }; \
         w.add('button', undefined, 'OK'); vc.log([w.show(), name.text]);",
    );
    assert!(until_window(&mut app));
    with_window(&app, |d| {
        d.queue(&UiEvent::Click(2));
        d.queue(&UiEvent::Change(1, UiValue::Text("Ann".into())));
    })
    .unwrap();
    step(&mut app, Vec::new());
    assert!(until_window(&mut app), "the click's handler disabled the field");
    step(&mut app, Vec::new());
    assert_eq!(typed(&app, 1).as_deref(), Some("Ann"), "the refused text stays in its field");
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([[1, "first"]]));
}
