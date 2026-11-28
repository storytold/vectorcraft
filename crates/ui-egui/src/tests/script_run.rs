//! File › Scripts › Other Script… (`ui.runScript`): the chooser, the warning and its preference,
//! a run advanced frame by frame, Stop, and the questions a run and quitting ask.

use craft::host::{PickKind, Picked};
use craft::ui::dialog::{DialogResult, StackDialog};
use serde_json::{Value, json};
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::scripts::TOO_LARGE;
use vectorcraft_engine::script::MAX_SOURCE_BYTES;
use vectorcraft_engine::script::folder::Clock;

use super::script_dialogs::{cancel, confirm, frame, is_open, key, open_id, pump, set};
use crate::dialogs::script_run::{
    self, BY_PERSON, CLOSE_KIND, FilesAnswer, GROWTH_KIND, GrowthQuestion, KIND, PICKER_KIND, RunQuestion, ScriptFiles,
};
use crate::tests::picks::{Picker, scripted};
use crate::{VectorcraftApp, menus, script_runs};

/// An app with a document, whose folder listings take no time.
fn app() -> VectorcraftApp {
    let mut session = Session::new();
    session.set_script_listing_clock(Clock::Fixed(std::time::Duration::ZERO));
    let mut app = crate::testing::windowed(VectorcraftApp::new(session, crate::Services::default()));
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
fn a_script_file_reply_does_not_populate_a_reopened_dialog() {
    let mut app = app();
    let picker = scripted(&mut app);
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": "File.openDialog('Choose');"})).unwrap().unwrap();
    script_runs::poll(&mut app);
    script_run::with_host(&app, |host, _| host.get_mut::<ScriptFiles>().unwrap().choose = true).unwrap();
    pump(&mut app);
    assert_eq!(picker.requests().len(), 1);

    script_run::forget::<ScriptFiles>(&app);
    script_run::open_picker(&app, app.session.script_picker().unwrap());
    let old_path = std::env::temp_dir().join("original-script-picker.txt");
    picker.answer(&[&old_path.to_string_lossy()]);
    let ctx = app.ui_ctx.clone().unwrap();
    crate::tests::picks::poll(&mut app, &ctx);

    assert!(picked(&app).is_empty(), "the old file chooser must not populate the new open dialog");
    set(&app, PICKER_KIND, "answer", json!("deny")).unwrap();
    assert!(confirm(&mut app, PICKER_KIND));
    finish(&mut app);
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

/// The warning automation asked while confined to a folder: Run runs the script as automation's,
/// with that folder still the only place it may read.
#[test]
fn the_warning_runs_the_script_under_the_access_it_was_asked_with() {
    let mut app = app();
    let (inside, outside) = (folder("scope-in"), folder("scope-out"));
    let path = outside.join("draw.js");
    std::fs::write(&path, DRAW).unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap();
    vectorcraft_engine::file_access::confine(roots.as_ref(), || {
        crate::file_asks::as_automated(&mut app, true, |app| {
            menus::run_ui_command(app, "ui.runScript", &json!({"path": path.to_string_lossy()})).unwrap().unwrap();
        })
    });
    assert!(confirm(&mut app, KIND));
    assert!(app.ui.status.contains("outside the read root"), "the script outside the folder is refused: {}", app.ui.status);
    assert!(!app.session.script_running());
    finish(&mut app);
    assert_eq!(objects(&app), 0);
    let _ = std::fs::remove_dir_all(&inside);
    let _ = std::fs::remove_dir_all(&outside);
}

/// The warning a person asked, answered by automation confined to a folder: the script runs with
/// that folder still the only place it may read.
#[test]
fn the_warning_answered_under_narrower_access_keeps_it() {
    let mut app = app();
    let (inside, outside) = (folder("narrow-in"), folder("narrow-out"));
    let path = outside.join("draw.js");
    std::fs::write(&path, DRAW).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({"path": path.to_string_lossy()})).unwrap().unwrap();
    let roots = vectorcraft_engine::file_access::AutomationRoots::new(&[inside.as_path()], &[inside.as_path()]).unwrap();
    assert!(vectorcraft_engine::file_access::confine(roots.as_ref(), || confirm(&mut app, KIND)));
    assert!(app.ui.status.contains("outside the read root"), "the script outside the folder is refused: {}", app.ui.status);
    assert!(!app.session.script_running());
    let _ = std::fs::remove_dir_all(&inside);
    let _ = std::fs::remove_dir_all(&outside);
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

/// The selection a script changes reaches a plug-in that reacts to `selection.changed` in the
/// frame the script ends in: once it has ended, nothing asks for another frame.
#[test]
fn a_finished_run_delivers_its_plugin_events_in_the_frame_it_ends() {
    use vectorcraft_plugins::wat as tpl;

    /// Removes the plug-in when the test ends, failed or not: the registry serves every session in
    /// the process.
    struct Remove(&'static str);
    impl Drop for Remove {
        fn drop(&mut self) {
            vectorcraft_plugins::registry::remove(self.0);
        }
    }

    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    app.session.allow_plugin_grants_from_requests();
    let id = "test.UI.script_run_events";
    let _remove = Remove(id);
    let grant = json!(["progress", "events"]);
    let manifest = format!(r#"{{"id":"{id}","name":"Run Events","kind":"filter","api":2,"permissions":{grant},"events":["selection.changed"]}}"#);
    // The handler writes the event it is given as a warning, which the status bar shows.
    let warn = "(drop (call $vc_log (i32.const 1) (local.get $ep) (local.get $el))) (i32.const 0)";
    let wasm = wat::parse_str(tpl::reacting(&manifest, tpl::HOST_IMPORTS, warn)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true})).unwrap();
    app.poll_finished_work();
    let source = format!("{DRAW} vc.run('select.all', {{}});");
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": source })).unwrap().unwrap();
    let ctx = egui::Context::default();
    for _ in 0..10_000 {
        if !app.session.script_running() {
            break;
        }
        app.begin_host_iteration();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.tick(ui.ctx()));
        out.textures_delta.clear();
    }
    assert!(!app.session.script_running(), "the script did not end");
    assert!(app.ui.status.starts_with("Run Events: "), "{}", app.ui.status);
    assert!(app.ui.status.contains("selection.changed"), "{}", app.ui.status);
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
fn a_startup_script_stopped_by_quitting_stays_in_the_status_bar() {
    let mut app = app();
    let dir = folder("startup-quit");
    std::fs::write(dir.join("a.jsx"), "for (;;) {}").unwrap();
    app.session.attach_startup_scripts_dir(dir.clone());
    app.session.prefs.show_external_script_warning = false;
    app.run("prefs.set", json!({"key": "runStartupScripts", "value": true})).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": DRAW, "name": "chosen" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    assert!(app.session.startup_script_running());
    script_run::ask_to_close(&mut app);
    assert!(confirm(&mut app, CLOSE_KIND));
    finish(&mut app);
    script_runs::poll(&mut app);
    let stopped = "The script was stopped and its changes were undone";
    assert_eq!(app.ui.status, stopped);
    assert_eq!(app.ui.status_notes.as_ref().and_then(|(_, notes)| notes.first()).map(String::as_str), Some(stopped));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn quitting_between_a_startup_script_and_the_chosen_script_stops_the_chosen_script() {
    let mut app = app();
    let dir = folder("startup-quit-between");
    // A first startup script that cannot start leaves a note for the round.
    std::fs::write(dir.join("0.jsx"), [0xff, 0xfe, 0x00]).unwrap();
    std::fs::write(dir.join("a.jsx"), "vc.log(1);").unwrap();
    app.session.attach_startup_scripts_dir(dir.clone());
    app.session.prefs.show_external_script_warning = false;
    app.run("prefs.set", json!({"key": "runStartupScripts", "value": true})).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": DRAW, "name": "chosen" })).unwrap().unwrap();
    for _ in 0..10_000 {
        if app.session.script_name().is_none() {
            break;
        }
        script_runs::poll(&mut app);
    }
    assert!(app.session.script_running() && app.session.script_name().is_none(), "the chosen script waits for the next frame");
    script_run::ask_to_close(&mut app);
    // Frames drawn while the question is open do not start the chosen script.
    for _ in 0..3 {
        script_runs::poll(&mut app);
    }
    assert!(app.session.script_running() && app.session.script_name().is_none(), "the chosen script waits for the answer");
    // A frame takes the answer before it would start the next run, then pumps the dialogs.
    let id = open_id(&app, CLOSE_KIND);
    assert!(script_run::with_host(&app, |host, ctx| host.confirm_by_id(ctx, id)).unwrap());
    script_runs::poll(&mut app);
    assert!(!app.session.script_running());
    pump(&mut app);
    script_runs::poll(&mut app);
    let message = "The scripts waiting to run were not run";
    assert_eq!(app.ui.status, message);
    let notes = app.ui.status_notes.clone().map(|(_, notes)| notes).unwrap_or_default();
    assert_eq!(notes.len(), 2, "{notes:?}");
    assert_eq!(notes[0], message);
    assert!(notes[1].starts_with("0.jsx: "), "{notes:?}");
    assert_eq!(objects(&app), 0, "the chosen script did not run");
    // A later round stopped the same way, with no notes of its own, does not show the earlier round's.
    std::fs::remove_file(dir.join("0.jsx")).unwrap();
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": DRAW, "name": "chosen" })).unwrap().unwrap();
    for _ in 0..10_000 {
        if app.session.script_name().is_none() {
            break;
        }
        script_runs::poll(&mut app);
    }
    assert!(app.session.script_running() && app.session.script_name().is_none());
    script_run::ask_to_close(&mut app);
    assert!(confirm(&mut app, CLOSE_KIND));
    script_runs::poll(&mut app);
    assert_eq!(app.ui.status, message);
    assert_eq!(app.ui.status_notes, None);
    assert_eq!(objects(&app), 0);
    let _ = std::fs::remove_dir_all(&dir);
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

/// With two files picked, the file dialog lists them in a scrolled group named Picked files.
#[test]
fn a_scripts_file_dialog_lists_several_picked_files_in_a_named_group() {
    let mut app = app();
    app.session.prefs.show_external_script_warning = false;
    menus::run_ui_command(&mut app, "ui.runScript", &json!({ "source": "File.openDialog('Pick', '', true);" })).unwrap().unwrap();
    script_runs::poll(&mut app);
    let given = vec!["/picked/a.txt".to_string(), "/picked/b.txt".to_string()];
    script_run::with_host(&app, |host, _| host.get_mut::<ScriptFiles>().map(|d| d.add(given, true, false))).unwrap();
    frame(&app, vec![]);
    let out = frame(&app, vec![]);
    let update = out.platform_output.accesskit_update.expect("an accessibility tree");
    let groups: Vec<_> =
        update.nodes.iter().filter(|(_, node)| node.role() == egui::accesskit::Role::Group).filter_map(|(_, node)| node.label()).collect();
    assert!(groups.contains(&"Picked files"), "a group named Picked files: {groups:?}");
    let nodes: std::collections::HashMap<_, _> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
    let (_, group) = update
        .nodes
        .iter()
        .find(|(_, node)| node.role() == egui::accesskit::Role::Group && node.label() == Some("Picked files"))
        .expect("the Picked files group");
    let mut texts = Vec::new();
    let mut pending = group.children().to_vec();
    while let Some(id) = pending.pop() {
        if let Some(node) = nodes.get(&id) {
            texts.extend(node.label().into_iter().chain(node.value()).map(str::to_owned));
            pending.extend_from_slice(node.children());
        }
    }
    for path in ["/picked/a.txt", "/picked/b.txt"] {
        assert!(texts.iter().any(|text| text == path), "{path} is inside the Picked files group: {texts:?}");
    }
    cancel(&mut app, PICKER_KIND);
    finish(&mut app);
}
