//! A script's `alert`, `confirm` and `prompt` in the app: the dialog the run waits at, its buttons,
//! Enter and Escape, Stop, another dialog in the way, quitting while it waits, and the same answers
//! given by a run with nobody to ask.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::dialogs::script_run::{CLOSE_KIND, NOTICE_KIND};
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

fn start(app: &mut VectorcraftApp, source: &str) {
    menus::run_ui_command(app, "ui.runScript", &json!({"source": source, "name": "d.js"})).unwrap().unwrap();
}

/// Polls the run until it ends, or until its dialog is open (true).
fn until_dialog(app: &mut VectorcraftApp) -> bool {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return false;
        }
        script_runs::poll(app);
        if app.ui.dialog.as_ref().is_some_and(|d| d.kind == NOTICE_KIND) {
            return true;
        }
    }
    panic!("the script neither ended nor opened a dialog");
}

fn answer(app: &mut VectorcraftApp, answer: &str) {
    app.ui.dialog.as_mut().unwrap().fields.insert("answer".into(), json!(answer));
    dialogs::confirm(app).unwrap();
}

thread_local! {
    // One context per test, so frames follow one another as in the app.
    static CTX: egui::Context = {
        let ctx = egui::Context::default();
        crate::theme::install_fonts(&ctx);
        ctx
    };
}

/// One headless frame of the dialog layer and the shortcuts, in a window of `size`. Each key pressed
/// in it is let go in a second frame, as a person lets go of it.
fn frame(app: &mut VectorcraftApp, events: Vec<egui::Event>, size: egui::Vec2) {
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
    let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size));
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

const WINDOW: egui::Vec2 = egui::vec2(1200.0, 800.0);

fn log(app: &VectorcraftApp) -> Value {
    app.session.script_report().unwrap()["log"].clone()
}

#[test]
fn alert_waits_and_stop_rolls_back_first_dialog() {
    for stop_with_escape in [false, true] {
        let mut app = app();
        start(&mut app, &format!("{DRAW} while (1) {{ alert('x'); }}"));
        assert!(until_dialog(&mut app));
        let d = app.ui.dialog.as_ref().unwrap();
        assert_eq!((d.fields["kind"].as_str(), d.fields["message"].as_str(), d.fields["title"].clone()), (Some("alert"), Some("x"), Value::Null));
        for _ in 0..5 {
            script_runs::poll(&mut app);
        }
        assert!(app.session.script_running(), "the run waits at the dialog");
        assert_eq!(objects(&app), 0);
        if stop_with_escape {
            frame(&mut app, vec![key(egui::Key::Escape)], WINDOW);
        } else {
            answer(&mut app, "stop");
        }
        assert!(app.ui.dialog.is_none());
        assert!(!until_dialog(&mut app));
        assert_eq!(objects(&app), 0);
        assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
        assert_eq!(app.session.script_report().unwrap()["dialogs"][0]["answer"], "stop");
    }
}

#[test]
fn confirm_enter_uses_no_as_default() {
    for (no_as_default, want) in [(true, false), (false, true)] {
        let mut app = app();
        start(&mut app, &format!("vc.log(confirm('Go?', {no_as_default}));"));
        assert!(until_dialog(&mut app));
        frame(&mut app, vec![], WINDOW);
        frame(&mut app, vec![key(egui::Key::Enter)], WINDOW);
        assert!(app.ui.dialog.is_none(), "Enter answers the dialog: noAsDflt {no_as_default}");
        assert!(!until_dialog(&mut app));
        assert_eq!(log(&app), json!([want]), "noAsDflt {no_as_default}");
    }
    let mut app = app();
    start(&mut app, "vc.log(confirm('Go?'));");
    assert!(until_dialog(&mut app));
    frame(&mut app, vec![key(egui::Key::Escape)], WINDOW);
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!([false]), "Escape is No");
}

#[test]
fn prompt_ok_and_cancel_return_values() {
    let mut app = app();
    start(&mut app, "vc.log(prompt('Name', 'pre', 'Ask')); vc.log(prompt('Again', 'x'));");
    assert!(until_dialog(&mut app));
    assert_eq!(dialogs::script_run::notice_heading(app.ui.dialog.as_ref().unwrap()), "Ask");
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["value"], "pre");
    app.ui.dialog.as_mut().unwrap().fields.insert("value".into(), json!("typed"));
    dialogs::confirm(&mut app).unwrap();
    assert!(until_dialog(&mut app));
    dialogs::cancel(&mut app);
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!(["typed", null]));
    let mut app = self::app();
    start(&mut app, "vc.log(prompt('Name', ''));");
    assert!(until_dialog(&mut app));
    let e = {
        app.ui.dialog.as_mut().unwrap().fields.insert("answer".into(), json!("yes"));
        dialogs::confirm(&mut app).unwrap_err()
    };
    assert!(e.contains("ok, cancel, stop"), "{e}");
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, NOTICE_KIND, "a wrong answer leaves the dialog open");
    answer(&mut app, "ok");
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!([""]), "OK with an empty field gives an empty string");
}

#[test]
fn the_dialog_opens_in_the_frame_the_run_waits() {
    let mut app = app();
    start(&mut app, "alert('x');");
    script_runs::poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(NOTICE_KIND), "no later frame is drawn while the run waits");
}

#[test]
fn a_prompt_text_past_the_limit_keeps_the_dialog_open() {
    let mut app = app();
    start(&mut app, "vc.log(prompt('Name', ''));");
    assert!(until_dialog(&mut app));
    let long = "x".repeat(vectorcraft_engine::script::dialogs::MAX_PROMPT_BYTES + 1);
    app.ui.dialog.as_mut().unwrap().fields.insert("value".into(), json!(long));
    assert!(dialogs::confirm(&mut app).is_err());
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, NOTICE_KIND, "the refused answer leaves the dialog open");
    assert!(app.session.script_dialog().is_some(), "the run still waits");
    app.ui.dialog.as_mut().unwrap().fields.insert("value".into(), json!("ok"));
    answer(&mut app, "ok");
    assert!(!until_dialog(&mut app));
    assert_eq!(log(&app), json!(["ok"]));
}

#[test]
fn dialog_wait_survives_another_modal() {
    let mut app = app();
    start(&mut app, "alert('x');");
    script_runs::poll(&mut app);
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::GROWTH_KIND, json!({})));
    let other = app.ui.dialog.clone();
    for _ in 0..3 {
        script_runs::poll(&mut app);
    }
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.clone()), other.map(|d| d.kind), "the open dialog stays");
    assert!(app.session.script_running());
    app.ui.dialog = None;
    assert!(until_dialog(&mut app), "the script's dialog opens once the other one closes");
}

#[test]
fn close_while_notification_waits_abandons_or_resumes() {
    let mut app = app();
    start(&mut app, &format!("{DRAW} vc.log(prompt('Name', ''));"));
    assert!(until_dialog(&mut app));
    app.ui.dialog.as_mut().unwrap().fields.insert("value".into(), json!("half"));
    dialogs::script_run::ask_to_close(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, CLOSE_KIND);
    script_runs::poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, CLOSE_KIND, "the run goes on waiting");
    dialogs::cancel(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.fields["value"].as_str()), (NOTICE_KIND, Some("half")), "Cancel shows the dialog again as it was");
    dialogs::script_run::ask_to_close(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
    assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
}

#[test]
fn notification_layout_with_long_text_and_small_window() {
    let mut app = app();
    let long = "word ".repeat(3_000);
    start(&mut app, &format!("alert({long:?} + '\\u0007\\r\\n', 'T\\nitle', true);"));
    assert!(until_dialog(&mut app));
    for _ in 0..3 {
        frame(&mut app, vec![], egui::vec2(320.0, 240.0));
    }
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, NOTICE_KIND);
    assert_eq!(dialogs::script_run::notice_heading(app.ui.dialog.as_ref().unwrap()), "T\u{240a}itle");
    answer(&mut app, "ok");
    assert!(!until_dialog(&mut app));
    assert_eq!(app.session.script_report().unwrap()["dialogs"][0]["errorIcon"], true);
}

#[test]
fn the_app_and_a_run_with_nobody_to_ask_give_the_same_results() {
    let source = "vc.log(confirm('a', true, 'T')); vc.log(prompt('b', 'p')); alert('c', undefined, false); vc.log('done');";
    let mut app = app();
    start(&mut app, source);
    for a in ["yes", "ok", "ok"] {
        assert!(until_dialog(&mut app));
        answer(&mut app, a);
    }
    assert!(!until_dialog(&mut app));
    let framed = app.session.script_report().unwrap().clone();
    let mut s = Session::new();
    let given = json!([{"kind": "confirm", "value": true}, {"kind": "prompt", "value": "p"}, {"kind": "alert", "value": "ok"}]);
    let headless = s.execute("script.run", &json!({"source": source, "name": "d.js", "answers": given})).unwrap();
    assert_eq!(framed["log"], headless["log"]);
    assert_eq!(framed["dialogs"], headless["dialogs"]);
    assert_eq!(framed["log"], json!([true, "p", "done"]));
}
