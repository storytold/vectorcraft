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
    let source = format!("Folder.selectDialog('Folder'); vc.run('document.export', {{path: {p:?}, format: 'svg'}}); while (true) {{}}");
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(app.session.script_picker().is_some(), "the run waits at its folder dialog");
    let paths = vec![dir.to_string_lossy().into_owned()];
    assert!(app.session.script_pick(vectorcraft_engine::script::files::PickAnswer::Allow { paths, always: false }));
    script_runs::poll(&mut app);
    assert!(path.exists());
    app.ui.dialog = Some(crate::state::Dialog::new(dialogs::script_run::CLOSE_KIND, json!({})));
    confirm(&mut app).unwrap();
    assert!(app.ui.status.contains(&p), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A script's open dialog with `multiSelect` shows the host's dialog for several files, with the
/// file types of the script's filter, and the script gets every file picked.
#[test]
fn a_scripts_open_dialog_picks_several_files_of_its_types() {
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let dir = folder("several");
    let paths: Vec<String> = ["a.txt", "b.txt"].iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect();
    for p in &paths {
        std::fs::write(p, "x").unwrap();
    }
    let (seen, given) = (asked.clone(), paths.clone());
    let services = crate::Services {
        pick_open: Some(Box::new(|_: &crate::FilePick| panic!("the dialog for one file"))),
        pick_open_files: Some(Box::new(move |pick: &crate::FilePick| {
            seen.borrow_mut().push(pick.filters.clone());
            given.clone()
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    let source = "var f = File.openDialog('Pick', 'Text:*.txt;All files:*.*', true); vc.log([f.length, f[0].name, f[1].name]);";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let mut d = app.ui.dialog.take().expect("the script's file dialog");
    assert_eq!(d.kind, dialogs::script_run::PICKER_KIND);
    dialogs::script_run::choose(&mut app, &mut d);
    assert_eq!(*asked.borrow(), vec![vec![("Text".to_string(), vec!["txt".to_string()]), ("All files".to_string(), vec!["*".to_string()])]]);
    assert_eq!(app.script_picked, paths);
    d.fields.insert("answer".into(), json!("once"));
    app.ui.dialog = Some(d);
    confirm(&mut app).unwrap();
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([[2, "a.txt", "b.txt"]]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder's `selectDlg` asks the host for a folder picker that starts in that folder, and
/// `Folder.selectDialog` for one that starts where the host starts it.
#[test]
fn a_scripts_folder_dialog_starts_in_the_folder_it_was_asked_from() {
    let dir = folder("selectstart");
    let requests = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = requests.clone();
    let picked = dir.to_string_lossy().into_owned();
    let services = crate::Services {
        start_pick: Some(Box::new(move |request: crate::picks::PickRequest| {
            seen.borrow_mut().push(request);
            None
        })),
        pick_folder: Some(Box::new(move || Some(picked.clone()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    let source = format!(
        "var a = new Folder({:?}).selectDlg('Pick'); var b = Folder.selectDialog('Pick'); vc.log([a !== null, b !== null]);",
        dir.to_string_lossy()
    );
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    for _ in 0..2 {
        script_runs::poll(&mut app);
        let mut d = app.ui.dialog.take().expect("the script's file dialog");
        assert_eq!(d.kind, dialogs::script_run::PICKER_KIND);
        dialogs::script_run::choose(&mut app, &mut d);
        d.fields.insert("answer".into(), json!("once"));
        app.ui.dialog = Some(d);
        confirm(&mut app).unwrap();
    }
    finish(&mut app);
    let start = app.session.script_report().map(|r| r["log"].clone());
    assert_eq!(start, Some(json!([[true, true]])));
    let r = requests.borrow();
    let starts_in_dir = |s: &str| std::fs::canonicalize(s).ok() == std::fs::canonicalize(&dir).ok();
    assert!(matches!(&r[..], [crate::picks::PickRequest::FolderAt(s), crate::picks::PickRequest::Folder] if starts_in_dir(s)), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Where the host shows file dialogs in line (Windows), a folder's `selectDlg` also opens the
/// folder picker in that folder, and `Folder.selectDialog` the one that starts where the host
/// starts it.
#[test]
fn a_scripts_folder_dialog_shown_in_line_starts_in_the_folder_it_was_asked_from() {
    let dir = folder("selectstartinline");
    let shown = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (at, plain) = (shown.clone(), shown.clone());
    let (picked_at, picked) = (dir.to_string_lossy().into_owned(), dir.to_string_lossy().into_owned());
    let services = crate::Services {
        pick_folder_at: Some(Box::new(move |start: &str| {
            at.borrow_mut().push(Some(start.to_string()));
            Some(picked_at.clone())
        })),
        pick_folder: Some(Box::new(move || {
            plain.borrow_mut().push(None);
            Some(picked.clone())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    let source = format!(
        "var a = new Folder({:?}).selectDlg('Pick'); var b = Folder.selectDialog('Pick'); vc.log([a !== null, b !== null]);",
        dir.to_string_lossy()
    );
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    for _ in 0..2 {
        script_runs::poll(&mut app);
        let mut d = app.ui.dialog.take().expect("the script's file dialog");
        assert_eq!(d.kind, dialogs::script_run::PICKER_KIND);
        dialogs::script_run::choose(&mut app, &mut d);
        d.fields.insert("answer".into(), json!("once"));
        app.ui.dialog = Some(d);
        confirm(&mut app).unwrap();
    }
    finish(&mut app);
    assert_eq!(app.session.script_report().map(|r| r["log"].clone()), Some(json!([[true, true]])));
    let r = shown.borrow();
    let starts_in_dir = |s: &str| std::fs::canonicalize(s).ok() == std::fs::canonicalize(&dir).ok();
    assert!(matches!(&r[..], [Some(s), None] if starts_in_dir(s)), "{r:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A control request, handled as the app handles one: while `answering_control` is set.
fn control(app: &mut VectorcraftApp, method: &str, params: Value) -> Value {
    let (req, _) = crate::control::ControlRequest::new(method, params);
    app.ui.answering_control = true;
    let outcome = crate::control::handle(app, &egui::Context::default(), &req);
    app.ui.answering_control = false;
    let crate::control::Outcome::Done(r) = outcome else { panic!("expected an immediate reply") };
    r
}

const BY_PERSON: &str = "Only the person at the keyboard can allow a script's file access";

/// The paths a script's file dialog allows come from Choose… alone: `ui.dialog.set` cannot set
/// them, and fields set on the dialog do not let Always Allow keep a grant the run cannot keep.
#[test]
fn a_scripts_file_dialog_takes_no_paths_from_automation() {
    let dir = folder("automated");
    let secret = dir.join("secret.txt");
    std::fs::write(&secret, "secret").unwrap();
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = "var f = File.openDialog('Pick'); vc.log(f === null ? null : f.fsName);";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::script_run::PICKER_KIND));
    let p = secret.to_string_lossy().into_owned();
    let r = control(&mut app, "ui.dialog.set", json!({"field": "paths", "value": [p]}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("no field `paths`")), "{r}");
    let always = json!({"fields": [{"field": "canAlways", "value": true}, {"field": "answer", "value": "always"}], "action": "confirm"});
    let r = control(&mut app, "ui.dialog.set", always.clone());
    assert_eq!(r["error"], BY_PERSON, "{r}");
    let d = app.ui.dialog.as_mut().expect("the dialog stays open");
    d.fields.insert("answer".into(), json!("once"));
    assert_eq!(confirm(&mut app).unwrap_err(), "choose a file or folder first, or deny");
    app.script_picked = vec![p];
    let r = control(&mut app, "ui.dialog.set", always);
    assert_eq!(r["error"], BY_PERSON, "{r}");
    let d = app.ui.dialog.as_mut().expect("the dialog stays open");
    d.fields.insert("canAlways".into(), json!(true));
    d.fields.insert("answer".into(), json!("always"));
    assert_eq!(confirm(&mut app).unwrap_err(), "this script's file access can be allowed only once");
    let r = control(&mut app, "ui.dialog.set", json!({"fields": [{"field": "answer", "value": "deny"}], "action": "confirm"}));
    assert!(r.get("error").is_none_or(Value::is_null), "{r}");
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Allow Once and Always Allow This Script are taken only from the person at the keyboard: a control
/// request and synthetic input are refused and the dialog stays open, after a person picked a file
/// with Choose… too, while Deny from automation is taken.
#[test]
fn a_scripts_file_access_is_allowed_only_by_the_person() {
    let dir = folder("byperson");
    let picked = dir.join("picked.txt");
    std::fs::write(&picked, "picked").unwrap();
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = "var f = File.openDialog('Pick'); vc.log(f === null ? null : f.name);";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    app.script_picked = vec![picked.to_string_lossy().into_owned()];
    let r = control(&mut app, "ui.dialog.set", json!({"fields": [{"field": "answer", "value": "once"}], "action": "confirm"}));
    assert_eq!(r["error"], BY_PERSON, "{r}");
    assert!(app.session.script_picker().is_some(), "the run still waits");
    let r = control(&mut app, "ui.dialog.set", json!({"fields": [{"field": "answer", "value": "deny"}], "action": "confirm"}));
    assert!(r.get("error").is_none_or(Value::is_null), "Deny is taken from automation: {r}");
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([null]));

    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    app.script_picked = vec![picked.to_string_lossy().into_owned()];
    app.ui.dialog.as_mut().expect("the file dialog").fields.insert("answer".into(), json!("once"));
    app.synthetic_frame = true;
    assert_eq!(confirm(&mut app).unwrap_err(), BY_PERSON);
    app.synthetic_frame = false;
    assert!(app.session.script_picker().is_some(), "the run still waits");
    confirm(&mut app).unwrap();
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["picked.txt"]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The texts the dialog layer draws, once a few frames have let it settle.
fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut v = vec![];
    for _ in 0..3 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
        v.clear();
        out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    }
    v
}

/// The dialog says what the run asks for and shows the script's prompt, also after `ui.dialog.set`
/// changed the dialog's `kind` and `prompt` fields.
#[test]
fn a_scripts_file_dialog_says_what_the_run_asks_for() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = "vc.log(Folder.selectDialog('Pick the export folder'));";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let r =
        control(&mut app, "ui.dialog.set", json!({"fields": [{"field": "kind", "value": "open"}, {"field": "prompt", "value": "Nothing to see"}]}));
    assert!(r.get("error").is_none_or(Value::is_null), "{r}");
    let texts = dialog_texts(&mut app);
    let folder = "The script asks to read and write a folder you choose, with everything in it.";
    assert!(texts.iter().any(|t| t == folder), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Pick the export folder"), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Nothing to see" || t == "The script asks to read a file you choose."), "{texts:?}");
    dialogs::cancel(&mut app);
    finish(&mut app);
}

/// Enter in the file dialog gives Deny, its default answer, also after the person picked a folder.
#[test]
fn enter_in_a_scripts_file_dialog_denies() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = "vc.log(Folder.selectDialog('Pick'));";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let dir = folder("enterdeny");
    app.script_picked = vec![dir.to_string_lossy().into_owned()];
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let enter = egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    for events in [vec![], vec![], vec![enter]] {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| dialogs::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(app.ui.dialog.as_ref().is_none_or(|d| d.kind != dialogs::script_run::PICKER_KIND), "the dialog closes");
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Enter on the file dialog's focused Choose… button goes to that button, which opens the folder
/// picker once, and does not deny.
#[test]
fn enter_on_a_focused_choose_button_does_not_deny() {
    let asked = std::rc::Rc::new(std::cell::Cell::new(0usize));
    let counted = asked.clone();
    let pick_folder: Box<dyn FnMut() -> Option<String>> = Box::new(move || {
        counted.set(counted.get() + 1);
        None
    });
    let mut app = VectorcraftApp::new(Session::new(), crate::Services { pick_folder: Some(pick_folder), ..Default::default() });
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": "vc.log(Folder.selectDialog('Pick'));" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let key = |key| egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
    let frame = |app: &mut VectorcraftApp, events| {
        let mut out = ctx.run_ui(egui::RawInput { events, ..Default::default() }, |ui| dialogs::show(app, ui.ctx()));
        out.textures_delta.clear();
    };
    for events in [vec![], vec![], vec![key(egui::Key::Tab)], vec![]] {
        frame(&mut app, events);
    }
    assert!(ctx.memory(|m| m.focused().is_some()), "Tab gives a button the keyboard focus");
    assert_eq!(asked.get(), 0);
    frame(&mut app, vec![key(egui::Key::Enter)]);
    frame(&mut app, vec![]);
    assert_eq!(asked.get(), 1, "Enter chose the focused Choose… button");
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::script_run::PICKER_KIND), "the dialog stays open");
    assert!(app.session.script_picker().is_some(), "the run still waits");
    dialogs::cancel(&mut app);
    finish(&mut app);
}

/// Choose… keeps at most the 2,048 files a script is given, and the dialog says the files picked
/// after them were left out.
#[test]
fn a_scripts_file_dialog_keeps_at_most_2048_picked_files() {
    let given: Vec<String> = (0..2050).map(|n| format!("/picked/{n}.txt")).collect();
    let all = given.clone();
    let services = crate::Services { pick_open_files: Some(Box::new(move |_: &crate::FilePick| all.clone())), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({})).unwrap();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": "File.openDialog('Pick', '', true);" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let mut d = app.ui.dialog.take().expect("the script's file dialog");
    dialogs::script_run::choose(&mut app, &mut d);
    app.ui.dialog = Some(d);
    assert_eq!(app.script_picked, given[..2048]);
    let texts = dialog_texts(&mut app);
    let note = "A script is given at most 2,048 files: the files picked after them were left out";
    assert!(texts.iter().any(|t| t == note), "{texts:?}");
    dialogs::cancel(&mut app);
    finish(&mut app);
}

#[test]
fn a_new_app_shows_the_warning_by_default() {
    assert!(Session::new().prefs.show_external_script_warning);
}
