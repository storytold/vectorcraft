//! File › Scripts › Other Script… (`ui.runScript`): the chooser, the warning and its preference,
//! a run advanced frame by frame, Stop, and the questions a run and quitting ask.

use serde_json::{Value, json};
use vectorcraft_engine::Session;

use crate::{VectorcraftApp, dialogs, menus, script_runs};

fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), crate::Services::default());
    app.run("file.new", json!({})).unwrap();
    app
}

fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vc-script-run-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn objects(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, Vec::len)).sum()
}

fn finish(app: &mut VectorcraftApp) {
    for _ in 0..10_000 {
        if !app.session.script_running() {
            return;
        }
        script_runs::poll(app);
    }
    panic!("the script did not end");
}

const DRAW: &str = "vc.run('shape.rectangle', {x:0,y:0,width:10,height:10}); vc.run('shape.rectangle', {x:20,y:0,width:10,height:10});";

fn confirm(app: &mut VectorcraftApp) -> Result<Value, String> {
    dialogs::confirm(app)
}

#[test]
fn the_warning_comes_before_the_run_and_cancel_runs_nothing() {
    let mut app = app();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": DRAW, "name": "draw.js"})).unwrap().unwrap();
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.fields["name"].as_str()), (dialogs::script_run::KIND, Some("draw.js")));
    assert!(!app.session.script_running());
    dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    assert!(!app.session.script_running());
    finish(&mut app);
    assert_eq!(objects(&app), 0);
}

#[test]
fn a_run_goes_frame_by_frame_and_is_one_undo_step() {
    let mut app = app();
    let dir = folder("draw");
    let path = dir.join("Dessin (été) #1.JSX");
    std::fs::write(&path, DRAW).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"path": path.to_string_lossy()})).unwrap().unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields["name"], "Dessin (été) #1.JSX");
    confirm(&mut app).unwrap();
    assert!(app.session.script_running());
    assert_eq!(objects(&app), 0, "nothing shows until the run commits");
    finish(&mut app);
    assert_eq!(objects(&app), 2);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(app.ui.status, "Ran Dessin (été) #1.JSX");
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(objects(&app), 0);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn do_not_show_again_is_remembered_and_the_script_still_runs() {
    let mut app = app();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": DRAW})).unwrap().unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("dontShow".into(), json!(true));
    confirm(&mut app).unwrap();
    finish(&mut app);
    assert!(!app.session.prefs.show_external_script_warning);
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": DRAW})).unwrap().unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(app.session.script_running());
    finish(&mut app);
    assert_eq!(objects(&app), 4);
}

#[test]
fn only_js_and_jsx_files_are_taken() {
    let mut app = app();
    for name in ["a.txt", "a.jsxbin", "a"] {
        let e = menus::run_ui_command(&mut app, "ui.runScript", &json!({ "path": name })).unwrap().unwrap_err();
        assert!(e.contains(".js or .jsx"), "{name}: {e}");
    }
    assert!(app.ui.dialog.is_none());
}

#[test]
fn stop_puts_the_document_back_and_says_so() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = format!("{DRAW} while (true) {{}}");
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(app.session.script_running());
    assert!(app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 1, "height": 1})).unwrap_err().contains("a script is running"));
    app.run("script.stop", json!({})).unwrap();
    finish(&mut app);
    assert_eq!(objects(&app), 0);
    assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
}

#[test]
fn a_script_error_goes_to_the_status_bar() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": "throw new Error('nope');", "name": "x.js"})).unwrap().unwrap();
    finish(&mut app);
    assert_eq!(app.ui.status, "x.js: line 1, column 1: Error: nope (thrown)");
}

/// An app whose script picker is the web's: picks are counted and files arrive through its inbox.
fn app_with_picker() -> (VectorcraftApp, dialogs::script_run::ScriptInbox, std::rc::Rc<std::cell::Cell<u32>>) {
    let mut app = app();
    let inbox = dialogs::script_run::ScriptInbox::default();
    let picks = std::rc::Rc::new(std::cell::Cell::new(0));
    let counted = picks.clone();
    app.services.script_async = Some(Box::new(move || counted.set(counted.get() + 1)));
    app.services.script_inbox = Some(inbox.clone());
    (app, inbox, picks)
}

#[test]
fn a_canceled_web_pick_does_not_take_a_dropped_script() {
    let (mut app, _inbox, picks) = app_with_picker();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({})).unwrap().unwrap();
    assert_eq!(picks.get(), 1);
    // A pick that was canceled delivers nothing, and the next file that opens is an ordinary open.
    script_runs::poll(&mut app);
    let _ = crate::io::open_bytes(&mut app, "dropped.js", DRAW.as_bytes(), None);
    assert!(app.ui.dialog.as_ref().is_none_or(|d| d.kind != dialogs::script_run::KIND));
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
}

#[test]
fn a_web_pick_goes_to_the_warning_and_waits_for_an_open_dialog() {
    let (mut app, inbox, _picks) = app_with_picker();
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::CLOSE_KIND, json!({})));
    inbox.lock().unwrap().push(("picked.js".to_string(), Some(DRAW.as_bytes().to_vec())));
    script_runs::poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().kind, dialogs::script_run::CLOSE_KIND);
    assert_eq!(inbox.lock().unwrap().len(), 1, "the file waits");
    app.ui.dialog = None;
    script_runs::poll(&mut app);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.kind.as_str(), d.fields["name"].as_str()), (dialogs::script_run::KIND, Some("picked.js")));
    confirm(&mut app).unwrap();
    finish(&mut app);
    assert_eq!(objects(&app), 2);
}

#[test]
fn a_web_pick_that_is_not_text_is_reported() {
    let (mut app, inbox, _picks) = app_with_picker();
    inbox.lock().unwrap().push(("bad.js".to_string(), Some(vec![0xff, 0xfe])));
    script_runs::poll(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.ui.status, "the script is not UTF-8 text");
}

#[test]
fn a_web_pick_too_large_to_read_is_reported() {
    let (mut app, inbox, _picks) = app_with_picker();
    inbox.lock().unwrap().push(("big.js".to_string(), None));
    script_runs::poll(&mut app);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.ui.status, "the script is too large to run");
}

/// A run that never ends, with the growth question up.
fn at_the_growth_question(app: &mut VectorcraftApp) {
    app.session.prefs.show_external_script_warning = false;
    let source = format!("{DRAW} while (true) {{}}");
    menus::run_ui_command(app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(app);
    assert!(app.session.script_running());
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::GROWTH_KIND, json!({})));
}

#[test]
fn the_growth_questions_stop_button_stops_the_run() {
    let mut app = app();
    at_the_growth_question(&mut app);
    dialogs::cancel(&mut app);
    assert!(app.ui.dialog.is_none());
    finish(&mut app);
    assert_eq!(objects(&app), 0);
    assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
}

#[test]
fn escape_on_the_growth_question_stops_the_run() {
    let mut app = app();
    at_the_growth_question(&mut app);
    let key = egui::Key::Escape;
    let events = vec![egui::Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }];
    let ctx = egui::Context::default();
    let mut output = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| crate::shortcuts::handle(&mut app, ui.ctx()));
    output.textures_delta.clear();
    assert!(app.ui.dialog.is_none());
    finish(&mut app);
    assert_eq!(objects(&app), 0);
    assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
}

#[test]
fn a_stopped_run_closes_its_growth_question() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": "while (true) {}"})).unwrap().unwrap();
    script_runs::poll(&mut app);
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::GROWTH_KIND, json!({})));
    app.run("script.stop", json!({})).unwrap();
    finish(&mut app);
    assert!(app.ui.dialog.is_none());
}

#[test]
fn finished_background_work_waits_for_the_run_to_end() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    app.opening.activate.push(("no such document".to_string(), vectorcraft_engine::file_access::ScopeHandle::capture()));
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": "while (true) {}"})).unwrap().unwrap();
    app.poll_finished_work();
    assert_eq!(app.opening.activate.len(), 1);
    app.run("script.stop", json!({})).unwrap();
    finish(&mut app);
    app.poll_finished_work();
    assert!(app.opening.activate.is_empty());
}

#[test]
fn quitting_asks_to_stop_and_stop_undoes_the_run() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = format!("{DRAW} while (true) {{}}");
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(app.session.script_running());
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::CLOSE_KIND, json!({})));
    confirm(&mut app).unwrap();
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.ui.status, "The script was stopped and its changes were undone");
}

#[test]
fn a_stop_names_the_files_the_run_left_written() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let dir = folder("written");
    let path = dir.join("out.svg");
    let p = path.to_string_lossy().into_owned();
    let source = format!("vc.run('document.export', {{path: {p:?}, format: 'svg'}}); while (true) {{}}");
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(path.exists());
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::CLOSE_KIND, json!({})));
    confirm(&mut app).unwrap();
    assert!(app.ui.status.contains(&p), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_new_app_shows_the_warning_by_default() {
    assert!(Session::new().prefs.show_external_script_warning);
}
