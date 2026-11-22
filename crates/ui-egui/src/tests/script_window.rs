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
fn quitting_while_a_handler_runs_keeps_the_events_still_queued() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} {FORM} name.onChange = function () {{ name.text = name.text + '!'; }}; vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    app.ui.dialog.as_mut().unwrap().fields.insert("queued".into(), json!([{"event": "click", "control": 3, "value": null}]));
    give(&mut app, json!({"event": "change", "control": "name", "value": "Ann"})).unwrap();
    dialogs::script_run::ask_to_close(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(CLOSE_KIND));
    script_runs::poll(&mut app);
    dialogs::cancel(&mut app);
    assert!(until_window(&mut app), "the dialog opens again");
    assert!(app.ui.dialog.as_ref().unwrap().fields.contains_key("queued"), "with the click still queued");
    frame(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "Ann!"]]));
}

#[test]
fn a_click_from_ui_dialog_confirm_gives_the_typed_text_first() {
    let mut app = app();
    start(&mut app, &format!("{FORM} vc.log([w.show(), name.text]);"));
    assert!(until_window(&mut app));
    start_typing(&mut app);
    give(&mut app, json!({"event": "click", "control": "OK"})).unwrap();
    assert!(until_window(&mut app), "the script takes the text and waits again");
    assert!(!app.ui.dialog.as_ref().unwrap().fields.contains_key("text1"), "the text is given once");
    frame(&mut app, Vec::new());
    assert!(!until_window(&mut app), "the queued click on OK closes the dialog");
    assert_eq!(log(&app), json!([[1, "typed"]]));
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
    assert_eq!(name(5), "", "the control before it is no static text");
    assert_eq!(name(7), "", "a hidden static text is no prompt");
    assert_eq!(name(9), "user", "a name the script gave goes before the text before it");
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
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("title"), "Renamed", "the heading follows the handler's title");
    let ctx = CTX.with(Clone::clone);
    ctx.enable_accesskit();
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    let mut nodes = Vec::new();
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        nodes = out.platform_output.accesskit_update.map(|u| u.nodes).unwrap_or_default();
    }
    let list = nodes.iter().any(|(_, n)| n.role() == egui::accesskit::Role::ListBox && n.label() == Some("Fruit:") && n.value() == Some("pear"));
    assert!(list, "screen readers read the list box by its prompt, with its selected item");
    let drop = nodes.iter().any(|(_, n)| n.role() == egui::accesskit::Role::ComboBox && n.label() == Some("Size:"));
    assert!(drop, "screen readers read the drop-down list by its prompt");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
}

#[test]
fn enter_with_the_focus_on_a_slider_clicks_the_default_button() {
    let mut app = app();
    start(&mut app, "var w = new Window('dialog'); w.add('slider', undefined, 2, 0, 5); w.add('button', undefined, 'OK'); vc.log(w.show());");
    assert!(until_window(&mut app));
    let ctx = CTX.with(Clone::clone);
    ctx.enable_accesskit();
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0)));
    let mut on_slider = false;
    for _ in 0..8 {
        frame(&mut app, vec![key(egui::Key::Tab)]);
        let mut out = ctx.run_ui(egui::RawInput { screen_rect, ..Default::default() }, |ui| dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        on_slider = out
            .platform_output
            .accesskit_update
            .is_some_and(|u| u.nodes.iter().any(|(id, n)| *id == u.focus && n.role() == egui::accesskit::Role::Slider));
        if on_slider {
            break;
        }
    }
    assert!(on_slider, "Tab puts the focus on the slider");
    frame(&mut app, vec![key(egui::Key::Enter)]);
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
    assert!(app.ui.dialog.as_ref().is_none_or(|d| d.kind != KIND), "the dialog its show() ended is closed");
    app.session.script_request_stop();
    assert!(!until_window(&mut app));
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
