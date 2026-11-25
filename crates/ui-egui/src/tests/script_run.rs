//! File › Scripts › Other Script… (`ui.runScript`): the chooser, the warning and its preference,
//! a run advanced frame by frame, Stop, and the questions a run and quitting ask.

use craft::host::{PickKind, Picked};
use craft::ui::dialog::{DialogResult, StackDialog};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::scripts::TOO_LARGE;
use vectorcraft_engine::script::MAX_SOURCE_BYTES;

use super::script_dialogs::{cancel, confirm, fields, frame, is_open, key, open_id, pump, set};
use crate::dialogs::script_check;
use crate::dialogs::script_run::{
    self, BY_PERSON, CLOSE_KIND, FilesAnswer, GROWTH_KIND, GrowthQuestion, KIND, PICKER_KIND, RunQuestion, ScriptFiles,
};
use crate::tests::picks::{Picker, scripted};
use crate::{VectorcraftApp, menus, script_runs};

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), crate::Services::default()));
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

/// The open file dialog's fields, as automation lists them.
fn files_fields(app: &VectorcraftApp) -> Option<Value> {
    script_run::with_host(app, |host, _| host.get::<ScriptFiles>().and_then(|d| d.json_fields())).unwrap()
}

/// The paths the open file dialog holds.
fn picked(app: &VectorcraftApp) -> Vec<String> {
    files_fields(app).and_then(|f| serde_json::from_value(f["paths"].clone()).ok()).unwrap_or_default()
}

/// Presses Choose… in the open file dialog, answers the host's file dialog with `paths`, and lets
/// the answer arrive.
fn choose(app: &mut VectorcraftApp, picker: &Picker, paths: &[&str]) {
    script_run::with_host(app, |host, _| host.get_mut::<ScriptFiles>().map(|d| d.choose = true)).unwrap().expect("the script's file dialog");
    pump(app);
    picker.answer(paths);
    crate::tests::picks::poll(app, &app.ui_ctx.clone().unwrap());
}

/// The person's click on Allow Once with what the dialog holds: the dialog closes with that answer.
fn allow_once(app: &mut VectorcraftApp) {
    let paths = picked(app);
    script_run::forget::<ScriptFiles>(app);
    script_run::settle_files(app, DialogResult::Ok(FilesAnswer { answer: "once", paths, left_out: false, automated: false }));
}

#[test]
fn the_warning_comes_before_the_run_and_cancel_runs_nothing() {
    let mut app = app();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": DRAW, "name": "draw.js"})).unwrap().unwrap();
    assert!(is_open::<RunQuestion>(&app));
    assert!(!app.session.script_running());
    cancel(&mut app, KIND);
    assert!(!is_open::<RunQuestion>(&app));
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
    assert!(confirm(&mut app, KIND));
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
    set(&app, KIND, "skip", json!(true)).unwrap();
    assert!(confirm(&mut app, KIND));
    finish(&mut app);
    assert!(!app.session.prefs.show_external_script_warning);
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": DRAW})).unwrap().unwrap();
    assert!(!is_open::<RunQuestion>(&app));
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
    assert!(!app.any_dialog_open());
    assert!(!is_open::<RunQuestion>(&app));
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
fn a_canceled_pick_does_not_take_a_dropped_script() {
    let mut app = app();
    let picker = scripted(&mut app);
    let r = menus::run_ui_command(&mut app, "ui.runScript", &json!({})).unwrap().unwrap();
    assert_eq!(r["pending"], "fileDialog");
    picker.cancel();
    let ctx = app.ui_ctx.clone().unwrap();
    crate::tests::picks::poll(&mut app, &ctx);
    // The next file that opens is an ordinary open.
    let _ = crate::io::open_bytes(&mut app, "dropped.js", DRAW.as_bytes(), None);
    assert!(!is_open::<RunQuestion>(&app));
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
}

#[test]
fn a_picked_script_goes_to_the_warning() {
    let mut app = app();
    let dir = folder("picked");
    let path = dir.join("picked.js");
    std::fs::write(&path, DRAW).unwrap();
    let picker = scripted(&mut app);
    menus::run_ui_command(&mut app, "ui.runScript", &json!({})).unwrap().unwrap();
    picker.answer(&[&path.to_string_lossy()]);
    let ctx = app.ui_ctx.clone().unwrap();
    crate::tests::picks::poll(&mut app, &ctx);
    assert!(is_open::<RunQuestion>(&app));
    assert!(confirm(&mut app, KIND));
    finish(&mut app);
    assert_eq!(objects(&app), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_web_pick_that_is_not_text_is_reported() {
    let mut app = app();
    let e = script_run::picked_script(&mut app, Picked::Contents(vec![("bad.js".into(), vec![0xff, 0xfe])])).unwrap_err();
    assert_eq!(e, "the script is not UTF-8 text");
    assert!(!is_open::<RunQuestion>(&app));
}

#[test]
fn a_web_pick_too_large_to_read_is_reported() {
    let mut app = app();
    let e = script_run::picked_script(&mut app, Picked::Contents(vec![("big.js".into(), vec![b' '; MAX_SOURCE_BYTES + 1])])).unwrap_err();
    assert_eq!(e, TOO_LARGE);
    assert!(!is_open::<RunQuestion>(&app));
}

/// A run that never ends, with the growth question up.
fn at_the_growth_question(app: &mut VectorcraftApp) {
    app.session.prefs.show_external_script_warning = false;
    let source = format!("{DRAW} while (true) {{}}");
    menus::run_ui_command(app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(app);
    assert!(app.session.script_running());
    script_run::open_growth(app);
}

#[test]
fn the_growth_questions_stop_button_stops_the_run() {
    let mut app = app();
    at_the_growth_question(&mut app);
    cancel(&mut app, GROWTH_KIND);
    assert!(!is_open::<GrowthQuestion>(&app));
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
    script_run::open_growth(&app);
    app.run("script.stop", json!({})).unwrap();
    finish(&mut app);
    assert!(!is_open::<GrowthQuestion>(&app));
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
    script_run::ask_to_close(&mut app);
    assert!(confirm(&mut app, CLOSE_KIND));
    assert!(!app.session.script_running());
    assert_eq!(objects(&app), 0);
    assert!(!app.any_dialog_open());
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
    assert!(is_open::<ScriptFiles>(&app), "the run waits at its folder dialog");
    let paths = vec![dir.to_string_lossy().into_owned()];
    assert!(app.session.script_pick(vectorcraft_engine::script::files::PickAnswer::Allow { paths, always: false }));
    script_runs::poll(&mut app);
    assert!(path.exists());
    script_run::ask_to_close(&mut app);
    assert!(confirm(&mut app, CLOSE_KIND));
    assert!(app.ui.status.contains(&p), "{}", app.ui.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// While a run waits at its file dialog, the app's frame logic does not ask for a frame on every
/// frame.
#[test]
fn a_run_waiting_at_its_file_dialog_lets_the_app_idle() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": "Folder.selectDialog('Folder');" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(app.session.script_picker().is_some(), "the run waits at its folder dialog");
    assert!(is_open::<ScriptFiles>(&app));
    let ctx = crate::testing::ui_context();
    let mut wait = std::time::Duration::ZERO;
    for frame in 0..4 {
        let time = f64::from(frame);
        app.now_override = Some(time);
        let screen_rect = Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let raw = egui::RawInput { screen_rect, time: Some(time), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
        wait = out.viewport_output.get(&egui::ViewportId::ROOT).map_or(std::time::Duration::MAX, |v| v.repaint_delay);
    }
    assert!(app.session.script_picker().is_some(), "the run still waits");
    assert!(wait > std::time::Duration::from_millis(100), "{wait:?}: {:?}", ctx.repaint_causes());
    assert!(app.session.script_request_stop());
    finish(&mut app);
}

/// A script's open dialog with `multiSelect` asks the host for several files, with the file types
/// of the script's filter, and the script gets every file picked.
#[test]
fn a_scripts_open_dialog_picks_several_files_of_its_types() {
    let dir = folder("several");
    let paths: Vec<String> = ["a.txt", "b.txt"].iter().map(|n| dir.join(n).to_string_lossy().into_owned()).collect();
    for p in &paths {
        std::fs::write(p, "x").unwrap();
    }
    let mut app = app();
    let picker = scripted(&mut app);
    app.session.prefs.show_external_script_warning = false;
    let source = "var f = File.openDialog('Pick', 'Text:*.txt;All files:*.*', true); vc.log([f.length, f[0].name, f[1].name]);";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(is_open::<ScriptFiles>(&app));
    let refs: Vec<&str> = paths.iter().map(String::as_str).collect();
    choose(&mut app, &picker, &refs);
    let requests = picker.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].kind, PickKind::OpenMany);
    let filters: Vec<(String, Vec<String>)> = requests[0].filters.iter().map(|f| (f.name.clone(), f.extensions.clone())).collect();
    assert_eq!(filters, vec![("Text".to_string(), vec!["txt".to_string()]), ("All files".to_string(), vec!["*".to_string()])]);
    assert_eq!(picked(&app), paths);
    allow_once(&mut app);
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([[2, "a.txt", "b.txt"]]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder's `selectDlg` asks the host for a folder picker that starts in that folder, and
/// `Folder.selectDialog` for one that starts where the host starts it.
#[test]
fn a_scripts_folder_dialog_starts_in_the_folder_it_was_asked_from() {
    let dir = folder("selectstart");
    let mut app = app();
    let picker = scripted(&mut app);
    app.session.prefs.show_external_script_warning = false;
    let source = format!(
        "var a = new Folder({:?}).selectDlg('Pick'); var b = Folder.selectDialog('Pick'); vc.log([a !== null, b !== null]);",
        dir.to_string_lossy()
    );
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    for _ in 0..2 {
        script_runs::poll(&mut app);
        choose(&mut app, &picker, &[&dir.to_string_lossy()]);
        allow_once(&mut app);
    }
    finish(&mut app);
    assert_eq!(app.session.script_report().map(|r| r["log"].clone()), Some(json!([[true, true]])));
    let requests = picker.requests();
    assert!(requests.iter().all(|r| r.kind == PickKind::Folder), "{requests:?}");
    let starts_in_dir = |s: &Option<std::path::PathBuf>| s.as_ref().and_then(|s| std::fs::canonicalize(s).ok()) == std::fs::canonicalize(&dir).ok();
    assert!(matches!(requests.as_slice(), [first, second] if starts_in_dir(&first.directory) && second.directory.is_none()), "{requests:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The paths a script's file dialog allows come from Choose… alone, and no field of the dialog
/// sets them or changes what it asks.
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
    assert!(is_open::<ScriptFiles>(&app));
    let p = secret.to_string_lossy().into_owned();
    for (field, value) in [("paths", json!([p])), ("kind", json!("open")), ("prompt", json!("Nothing")), ("canAlways", json!(true))] {
        let e = set(&app, PICKER_KIND, field, value).unwrap_err();
        // The dialog host's refusal of a field the dialog does not have.
        assert!(e.contains(&format!("unknown field `{field}`")), "{field}: {e}");
    }
    assert_eq!(set(&app, PICKER_KIND, "answer", json!("once")).unwrap_err(), BY_PERSON);
    assert_eq!(set(&app, PICKER_KIND, "answer", json!("always")).unwrap_err(), BY_PERSON);
    assert!(app.session.script_picker().is_some(), "the run still waits");
    set(&app, PICKER_KIND, "answer", json!("deny")).unwrap();
    assert!(confirm(&mut app, PICKER_KIND));
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Allow Once is taken only for paths the person picked: paths from a pick automation asked for are
/// refused and the dialog opens again with them; a person's pick is allowed.
#[test]
fn a_scripts_file_access_is_allowed_only_by_the_person() {
    let dir = folder("byperson");
    let picked_file = dir.join("picked.txt");
    std::fs::write(&picked_file, "picked").unwrap();
    let paths = vec![picked_file.to_string_lossy().into_owned()];
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    let source = "var f = File.openDialog('Pick'); vc.log(f === null ? null : f.name);";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    script_run::forget::<ScriptFiles>(&app);
    let automated = FilesAnswer { answer: "once", paths: paths.clone(), left_out: false, automated: true };
    script_run::settle_files(&mut app, DialogResult::Ok(automated));
    assert_eq!(app.ui.status, BY_PERSON);
    assert!(app.session.script_picker().is_some(), "the run still waits");
    assert_eq!(picked(&app), paths, "the dialog opens again with the same paths");
    set(&app, PICKER_KIND, "answer", json!("deny")).unwrap();
    assert!(confirm(&mut app, PICKER_KIND), "Deny is taken from automation");
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([null]));

    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    script_runs::poll(&mut app);
    script_run::with_host(&app, |host, _| host.get_mut::<ScriptFiles>().map(|d| d.add(paths.clone(), false, false))).unwrap();
    allow_once(&mut app);
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!(["picked.txt"]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Enter never allows file access, also after the person picked a folder; Deny answers null.
#[test]
fn enter_in_a_scripts_file_dialog_denies() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": "vc.log(Folder.selectDialog('Pick'));" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let dir = folder("enterdeny");
    script_run::with_host(&app, |host, _| host.get_mut::<ScriptFiles>().map(|d| d.add(vec![dir.to_string_lossy().into_owned()], false, false)))
        .unwrap();
    frame(&app, vec![]);
    frame(&app, vec![]);
    frame(&app, vec![key(egui::Key::Enter)]);
    pump(&mut app);
    assert!(is_open::<ScriptFiles>(&app), "Enter answers nothing");
    assert!(app.session.script_picker().is_some(), "the run still waits");
    let id = open_id(&app, PICKER_KIND);
    script_run::with_host(&app, |host, ctx| host.cancel_by_id(ctx, id)).unwrap();
    pump(&mut app);
    finish(&mut app);
    assert_eq!(app.session.script_report().unwrap()["log"], json!([null]));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Choose… keeps at most the 2,048 files a script is given, and the dialog says the files picked
/// after them were left out.
#[test]
fn a_scripts_file_dialog_keeps_at_most_2048_picked_files() {
    let given: Vec<String> = (0..2050).map(|n| format!("/picked/{n}.txt")).collect();
    let mut app = app();
    let picker = scripted(&mut app);
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": "File.openDialog('Pick', '', true);" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let refs: Vec<&str> = given.iter().map(String::as_str).collect();
    choose(&mut app, &picker, &refs);
    assert_eq!(picked(&app), given[..2048]);
    assert_eq!(files_fields(&app).unwrap()["leftOut"], true);
    cancel(&mut app, PICKER_KIND);
    finish(&mut app);
}

/// The texts the open warning draws, after a few frames of the whole app.
fn dialog_texts(app: &mut VectorcraftApp) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = app.ui_ctx.clone().unwrap();
    let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1600.0, 4000.0));
    let mut out = None;
    for i in 0..4 {
        let raw = egui::RawInput { time: Some(0.1 * f64::from(i)), screen_rect: Some(screen), ..Default::default() };
        out = Some(crate::testing::app_frame(app, &ctx, raw));
    }
    let mut v = vec![];
    out.unwrap().shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

/// The warning before a run lists the files the script writes, read without running it.
#[test]
fn the_warning_lists_the_files_a_script_writes() {
    let mut app = app();
    let source = "var f = new File(Folder.temp + '/out.txt');\nf.open('w');\nf.write('x');\nf.close();\n";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "write.js"})).unwrap().unwrap();
    assert!(!app.session.script_running());
    let texts = dialog_texts(&mut app);
    let writes = script_check::outside_label("filesWritten").unwrap();
    assert!(texts.iter().any(|t| t.starts_with(writes) && t.contains("(w)")), "{texts:?}");
    assert!(texts.iter().any(|t| t == script_check::need_label("files").unwrap()), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Outside the document:"), "{texts:?}");
    // The warning lists the report as `reaches`, which `ui.dialog.set` cannot change.
    assert_eq!(fields(&app, KIND)["reaches"]["outside"][0]["id"], "filesWritten");
    assert!(set(&app, KIND, "reaches", json!(null)).is_err());
    assert_eq!(fields(&app, KIND)["reaches"]["outside"][0]["id"], "filesWritten");
    cancel(&mut app, KIND);
    finish(&mut app);
}

/// A script that reaches nothing outside the document gets the warning with nothing more.
#[test]
fn the_warning_for_a_script_that_reaches_nothing_lists_nothing_more() {
    let mut app = app();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": DRAW, "name": "draw.js"})).unwrap().unwrap();
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|t| t.starts_with("A script can change the open documents")), "{texts:?}");
    for heading in ["Outside the document:", "Includes:", "VectorCraft read this script without running it."] {
        assert!(!texts.iter().any(|t| t == heading), "{heading}: {texts:?}");
    }
    assert!(confirm(&mut app, KIND));
    finish(&mut app);
    assert_eq!(objects(&app), 2);
}

/// A script file's warning also lists what the files its `#include` lines name reach.
#[test]
fn the_warning_lists_what_a_scripts_included_files_reach() {
    let mut app = app();
    let dir = folder("includes");
    std::fs::create_dir_all(dir.join("lib")).unwrap();
    std::fs::write(dir.join("main.jsx"), "#include 'lib/net.jsxinc'\n").unwrap();
    std::fs::write(dir.join("lib/net.jsxinc"), "var s = new Socket();\n").unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"path": dir.join("main.jsx").to_string_lossy()})).unwrap().unwrap();
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|t| t.starts_with("lib/net.jsxinc · ")), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Includes:"), "{texts:?}");
    assert!(texts.iter().any(|t| t == script_check::need_label("hostOnly").unwrap()), "{texts:?}");
    cancel(&mut app, KIND);
    finish(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A script that opens a dialog and reaches nothing outside the document lists the need without the
/// outside-the-document heading.
#[test]
fn the_warning_lists_a_need_without_the_outside_heading() {
    let mut app = app();
    let source = "var w = new Window('dialog', 'Title');";
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "dialog.js"})).unwrap().unwrap();
    assert_eq!(fields(&app, KIND)["reaches"]["outside"], json!([]));
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|t| t == script_check::need_label("dialogs").unwrap()), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Outside the document:"), "{texts:?}");
    cancel(&mut app, KIND);
    finish(&mut app);
}

/// A script file that changes what it reaches while its warning is open does not run on Run: the
/// warning opens again with the new report, and the next Run runs it.
#[test]
fn run_lists_a_script_again_when_it_changed_after_its_warning_opened() {
    let mut app = app();
    let dir = folder("changed");
    let path = dir.join("changed.jsx");
    std::fs::write(&path, DRAW).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"path": path.to_string_lossy()})).unwrap().unwrap();
    assert_eq!(fields(&app, KIND)["reaches"]["outside"], json!([]));
    std::fs::write(&path, format!("{DRAW}\nvar s = new Socket();\n")).unwrap();
    assert!(confirm(&mut app, KIND));
    assert!(!app.session.script_running());
    assert!(is_open::<RunQuestion>(&app));
    assert_eq!(app.ui.status, script_run::CHANGED_SINCE_WARNING);
    assert_eq!(fields(&app, KIND)["reaches"]["outside"][0]["id"], "network");
    assert!(confirm(&mut app, KIND));
    assert!(app.session.script_running());
    finish(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A script with more includes than the warning lists gives the count of the rest.
#[test]
fn the_warning_counts_the_includes_it_does_not_list() {
    let mut app = app();
    let dir = folder("many-includes");
    let names: Vec<String> = (0..script_run::SHOWN_INCLUDES + 3).map(|n| format!("i{n}.jsxinc")).collect();
    for name in &names {
        std::fs::write(dir.join(name), "var x = 1;\n").unwrap();
    }
    let main: String = names.iter().map(|n| format!("#include '{n}'\n")).collect();
    std::fs::write(dir.join("main.jsx"), main).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"path": dir.join("main.jsx").to_string_lossy()})).unwrap().unwrap();
    let texts = dialog_texts(&mut app);
    let listed = |name: &str| texts.iter().any(|t| t.starts_with(&format!("{name} · ")));
    assert!(listed(&names[script_run::SHOWN_INCLUDES - 1]), "{texts:?}");
    assert!(!listed(&names[script_run::SHOWN_INCLUDES]), "{texts:?}");
    assert!(texts.iter().any(|t| t == "Included files not listed: 3"), "{texts:?}");
    assert_eq!(fields(&app, KIND)["reaches"]["includes"].as_array().map(Vec::len), Some(names.len()));
    cancel(&mut app, KIND);
    finish(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A script the checker could not read as a script gets a warning that says so, not the warning of
/// a script that was checked and reaches nothing.
#[test]
fn the_warning_says_when_a_script_was_not_checked() {
    let missing = std::env::temp_dir().join(format!("vc-script-run-ui-missing-{}.js", std::process::id()));
    for (params, text) in [
        (json!({"source": "export const b = 2;", "name": "module.js"}), "Not a script for the reference app"),
        (json!({"source": "var a = 1;\u{0}", "name": "binary.js"}), "Not a script for the reference app"),
        (json!({"source": "@JSXBIN@ES@2.0@MyBbyBn0ABJAnA", "name": "compiled.js"}), "Compiled script: ask its author for the .jsx file"),
        (json!({"path": missing.to_string_lossy()}), "Not read"),
    ] {
        let mut app = app();
        menus::run_ui_command(&mut app, "ui.runScript", &params).unwrap().unwrap();
        let texts = dialog_texts(&mut app);
        assert!(texts.iter().any(|t| t == text), "{params}: {texts:?}");
        cancel(&mut app, KIND);
        finish(&mut app);
    }
}

/// A script whose report was cut short at a list's limit says so, also when nothing else is listed.
#[test]
fn the_warning_says_when_a_list_was_cut_short() {
    let mut app = app();
    let source: String = (0..=1000).map(|n| format!("x = app.m{n};\n")).collect();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": source, "name": "many.js"})).unwrap().unwrap();
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|t| t == "Some lists were cut short at their limit."), "{texts:?}");
    cancel(&mut app, KIND);
    finish(&mut app);
}

/// A script over the checker's size limit gets a warning that says it was too large to check.
#[test]
fn the_warning_says_when_a_script_is_too_large_to_check() {
    let mut app = app();
    let dir = folder("large");
    let path = dir.join("large.js");
    std::fs::write(&path, vec![b' '; MAX_SOURCE_BYTES + 1]).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"path": path.to_string_lossy()})).unwrap().unwrap();
    let texts = dialog_texts(&mut app);
    assert!(texts.iter().any(|t| t == "Too large to check"), "{texts:?}");
    cancel(&mut app, KIND);
    finish(&mut app);
    let _ = std::fs::remove_dir_all(&dir);
}
