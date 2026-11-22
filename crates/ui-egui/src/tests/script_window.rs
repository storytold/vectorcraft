//! A script's ScriptUI dialog in the app: it opens when `show()` waits, takes clicks and changes
//! from the person and from `ui.dialog.confirm`, Enter and Escape, Stop, and quitting while it
//! waits.

use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::script::{UiEvent, UiValue};

use crate::dialogs::script_run::CLOSE_KIND;
use crate::dialogs::script_window::{self, KIND};
use crate::{VectorcraftApp, dialogs, menus, script_runs};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    app
}

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
        if app.ui.dialog.as_ref().is_some_and(|d| d.kind == KIND) && app.session.script_window().is_some() {
            return true;
        }
    }
    panic!("the script neither ended nor opened its dialog");
}

/// Gives the dialog an event as `ui.dialog.set` and `ui.dialog.confirm` do.
fn give(app: &mut VectorcraftApp, event: Value) -> Result<Value, String> {
    let d = app.ui.dialog.as_mut().unwrap();
    for (k, v) in event.as_object().unwrap() {
        d.fields.insert(k.clone(), v.clone());
    }
    dialogs::confirm(app)
}

thread_local! {
    static CTX: egui::Context = {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    };
}

/// One headless frame of the dialog layer and the shortcuts. Each key pressed in it is let go in a
/// second frame.
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>) {
    let ctx = CTX.with(Clone::clone);
    let released: Vec<egui::Event> = events
        .iter()
        .filter_map(|e| match e {
            egui::Event::Key { key, physical_key, modifiers, pressed: true, .. } => {
                Some(egui::Event::Key { key: *key, physical_key: *physical_key, pressed: false, repeat: false, modifiers: *modifiers })
            }
            _ => None,
        })
        .collect();
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    let pressed = !released.is_empty();
    for events in std::iter::once(events).chain(pressed.then_some(released)) {
        let mut out = ctx.run_ui(egui::RawInput { events, screen_rect, ..Default::default() }, |ui| {
            crate::shortcuts::handle(app, ui.ctx());
            dialogs::show(app, ui.ctx());
        });
        out.textures_delta.clear();
    }
}

fn key(k: egui::Key) -> egui::Event {
    egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }
}

fn log(app: &VectorcraftApp) -> Value {
    app.session.script_report().unwrap()["log"].clone()
}

#[test]
fn the_dialog_takes_events_and_stays_open_while_its_handlers_run() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} var r = w.show(); vc.log([r, name.text, bold.value]);"));
    assert!(until_window(&mut app));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("title"), "Make");
    frame(&mut app, Vec::new());
    give(&mut app, json!({"event": "click", "control": "Bold"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(KIND), "the dialog stays open for the next wait");
    assert!(until_window(&mut app));
    assert_eq!(app.session.script_window().unwrap().nodes[1].text, "clicked", "the handler changed the field");
    assert!(give(&mut app, json!({"event": "click", "control": 99})).is_err());
    assert!(give(&mut app, json!({"event": "wave"})).is_err());
    give(&mut app, json!({"event": "change", "control": "name", "value": "Ann"})).unwrap();
    assert!(until_window(&mut app));
    frame(&mut app, vec![key(egui::Key::Enter)]);
    assert!(!until_window(&mut app), "Enter clicks OK, which closes the dialog");
    assert_eq!(log(&app), json!([[1, "Ann", true]]));
    assert!(app.ui.dialog.is_none());
    assert_eq!(objects(&app), 1);
}

#[test]
fn text_still_in_a_field_reaches_the_script_before_a_click() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    let w = app.session.script_window().unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    d.fields.insert("text1".into(), json!("Ann"));
    let mut events = vec![UiEvent::Click(3)];
    script_window::commit_drafts(&w, d, &mut events);
    assert_eq!(events, vec![UiEvent::Change(1, UiValue::Text("Ann".into())), UiEvent::Click(3)]);
    assert!(!d.fields.contains_key("text1"), "the text is given once");
    d.fields.insert("queued".into(), json!([{"event": "change", "control": 1, "value": "Ann"}, {"event": "click", "control": 3, "value": null}]));
    frame(&mut app, Vec::new());
    assert!(until_window(&mut app), "the script takes the change and waits again");
    frame(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "Ann"]]));
}

#[test]
fn escape_clicks_the_cancel_button_and_closes_a_dialog_without_one() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log(w.show());"));
    assert!(until_window(&mut app));
    frame(&mut app, vec![key(egui::Key::Escape)]);
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([2]));
    start(&mut app, "var w = new Window('dialog'); w.add('statictext', undefined, 'Hi'); vc.log(w.show());");
    assert!(until_window(&mut app));
    dialogs::cancel(&mut app);
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([0]));
}

#[test]
fn escape_keeps_the_dialog_open_until_the_script_closes_it() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('edittext', undefined, 'a'); w.onClose = function () { return false; }; vc.log(w.show());");
    assert!(until_window(&mut app));
    start_typing(&mut app);
    dialogs::cancel(&mut app);
    let d = app.ui.dialog.as_ref().expect("the dialog stays open");
    assert_eq!(d.fields["queued"], json!([{"event": "close", "control": null, "value": null}]), "with the close after the typed text");
    assert!(until_window(&mut app), "the script takes the text and waits again");
    assert_eq!(app.session.script_window().unwrap().nodes[1].text, "typed");
    frame(&mut app, Vec::new());
    assert!(until_window(&mut app), "onClose kept the dialog open");
    assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("queued"));
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

fn start_typing(app: &mut VectorcraftApp) {
    app.ui.dialog.as_mut().unwrap().fields.insert("text1".into(), json!("typed"));
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
    start_typing(&mut app);
    dialogs::cancel(&mut app);
    assert!(until_window(&mut app), "the script takes the text and waits again");
    frame(&mut app, Vec::new());
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
    app.ui.dialog.as_mut().unwrap().fields.insert("text1".into(), json!("left over"));
    assert!(until_window(&mut app));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("title"), "B", "the second dialog's title");
    assert!(!d.fields.contains_key("text1"), "without the first dialog's typed text");
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
        if app.ui.dialog.as_ref().is_some_and(|d| d.kind != KIND) {
            break;
        }
    }
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::script_run::NOTICE_KIND), "the alert opens");
    assert!(app.session.script_dialog().is_some());
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
    assert!(app.ui.dialog.is_none());
}

#[test]
fn quitting_while_the_dialog_waits_asks_and_cancel_shows_it_again() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} vc.log(w.show());"));
    assert!(until_window(&mut app));
    app.ui.dialog.as_mut().unwrap().fields.insert("text1".into(), json!("typed"));
    dialogs::script_run::ask_to_close(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(CLOSE_KIND));
    dialogs::cancel(&mut app);
    assert!(until_window(&mut app), "the dialog opens again");
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("text1"), "typed", "with the text typed in its field");
    dialogs::script_run::ask_to_close(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
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
    frame(&mut app, Vec::new());
    frame(&mut app, Vec::new());
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(!until_window(&mut app));
    assert_eq!(log(&app), json!([1]));
}
