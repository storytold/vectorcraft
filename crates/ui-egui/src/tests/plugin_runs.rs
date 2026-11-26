//! Heavy plug-in filters in the app ([`crate::plugin_runs`]): installing one asks first, its dialog
//! has no Preview, and it runs off the UI thread, its result applied as one undo step once it is
//! done, unless the document changed meanwhile. Cancel stops it; its progress window shows it. A
//! light filter that reports its progress runs there too.

use std::sync::Arc;
use std::time::{Duration, Instant};

use craft::host::{Host, ManualJobs};
use craft::ui::dialog::DialogId;
use craft::ui::patterns::form_dialogs::HostedParamDialog;
use egui_kittest::Harness;
use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::{registry, wat as tpl};

use super::plugins::{approve, window};
use crate::dialogs::plugin::InstallPrompt;
use crate::i18n::LangExt as _;
use crate::{VectorcraftApp, dialogs, export_progress, plugin_runs};

/// An app with a new 200 × 200 pt document.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.allow_plugin_grants_from_requests();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
}

/// The window of an [`app`] whose background work waits until the test runs it: a run that OK
/// started is still running when the frame that started it ends. On threads a fast run could end
/// before a later pass of that frame, which applies it. The app's clock stands still, so the
/// progress window, which would hold the results back, opens only when a test moves the clock.
fn held_window() -> (Harness<'static, VectorcraftApp>, Arc<ManualJobs>) {
    let mut app = app();
    app.now_override = Some(0.0);
    let jobs = Arc::new(ManualJobs::new());
    app.host = Arc::new(Host::builder(app.host.app().clone()).i18n(crate::i18n::shared()).jobs(jobs.clone()).build());
    (window(app), jobs)
}

/// A heavy test filter `id` (named "Test `id`") with `params` that counts `steps` down, then adds
/// a triangle.
fn heavy(id: &str, steps: u64, params: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"filter","budget":"heavy","params":{params}}}"#);
    let output = r#"{"objects":[{"path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[30,0]},{"p":[15,20]}]}]}}]}"#;
    wat::parse_str(tpl::spinning(&manifest, steps, output)).unwrap()
}

/// Installs [`heavy`] filter `id` as the user would once they agreed.
fn install(app: &mut VectorcraftApp, id: &str, steps: u64, params: &str) {
    let data = vectorcraft_format::base64_encode(&heavy(id, steps, params));
    app.run("plugin.install", json!({"dataBase64": data, "allowHeavy": true})).unwrap();
}

/// Applies the runs' results as the frames would, until none is left (at most 30 s).
fn wait(app: &mut VectorcraftApp) {
    let t0 = Instant::now();
    while !app.plugin_runs.running.is_empty() && t0.elapsed() < Duration::from_secs(30) {
        plugin_runs::poll(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.plugin_runs.running.is_empty(), "still running");
}

/// The id of the dialog the host shows, if one is open.
fn open_id(app: &VectorcraftApp) -> Option<DialogId> {
    // PLAN: assumed `OpenDialog::id: DialogId`
    app.dialogs().and_then(|d| d.with(|host| host.open_dialogs().first().map(|o| o.id)))
}

/// Whether the host shows no dialog.
fn none_open(app: &VectorcraftApp) -> bool {
    open_id(app).is_none()
}

/// Whether the install question is open.
fn asking(app: &VectorcraftApp) -> bool {
    app.dialogs().is_some_and(|d| d.with(|host| host.is_open::<InstallPrompt>()))
}

/// Whether the open filter dialog previews, `None` without one.
fn previewing(app: &VectorcraftApp) -> Option<bool> {
    app.dialogs().and_then(|d| d.with(|host| host.get::<HostedParamDialog>().map(HostedParamDialog::is_previewing)))
}

/// One whole app frame of the window, and the pump of the dialogs' answers.
fn frame(h: &mut Harness<'static, VectorcraftApp>) {
    super::plugins::app_step(h);
    let ctx = h.state().ui_ctx.clone().unwrap();
    dialogs::host::pump(h.state_mut(), &ctx);
}

/// Answers the open dialog: OK when `ok`, else Cancel; then a frame, so the pump acts on it.
fn answer(h: &mut Harness<'static, VectorcraftApp>, ok: bool) {
    let ctx = h.state().ui_ctx.clone().unwrap();
    let id = open_id(h.state()).unwrap();
    h.state().dialogs().unwrap().with(|host| if ok { host.confirm_by_id(&ctx, id) } else { host.cancel_by_id(&ctx, id) });
    frame(h);
}

/// Sets the Preview check box of the open dialog.
fn set_preview(h: &mut Harness<'static, VectorcraftApp>, on: bool) {
    let ctx = h.state().ui_ctx.clone().unwrap();
    let id = open_id(h.state()).unwrap();
    h.state().dialogs().unwrap().with(|host| host.set_json_field(&ctx, id, "preview", json!(on))).unwrap();
    frame(h);
}

/// Install Plug-in asks before it installs a filter that asks for a heavy budget, and OK installs
/// it; File › Open of the `.wasm` asks too, and Cancel installs nothing.
#[test]
fn installing_a_heavy_filter_asks_first() {
    let mut h = window(app());
    let id = "test.ui.heavy.install";
    let bytes = heavy(id, 1, "{}");
    let r = h.state_mut().run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "route.wasm"})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    frame(&mut h);
    assert!(asking(h.state()));
    let pending = h.state().pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.message, format!("Install “Test {id}”, which asks for a heavy budget?"));
    assert!(pending.detail.contains("up to 5 minutes and do 20 times the work"), "{}", pending.detail);
    assert!(registry::get(id).is_none(), "not before the user agrees");
    approve(&mut h);
    frame(&mut h);
    assert_eq!(registry::get(id).map(|p| p.manifest().name.clone()), Some(format!("Test {id}")));
    assert_eq!(h.state().ui.status, format!("Installed plug-in Test {id}"));
    // File › Open of the .wasm asks too, and Cancel installs nothing.
    registry::remove(id);
    crate::io::open_bytes(h.state_mut(), "route.wasm", &bytes, None).unwrap();
    frame(&mut h);
    assert!(asking(h.state()));
    answer(&mut h, false);
    assert!(registry::get(id).is_none());
    assert!(h.state().pending_plugin_install.is_none());
}

/// The heavy-budget question gives the filter's ID and file SHA-256, and also the manifest's own
/// name when the name it asks with is a translation.
#[test]
fn the_heavy_budget_question_shows_the_id_hash_and_manifest_name() {
    let mut h = window(app());
    let id = "test.ui.heavy.translated";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Route","version":"1","kind":"filter","budget":"heavy","translations":{{"de":{{"name":"Fräsen"}}}}}}"#);
    let bytes = wat::parse_str(tpl::spinning(&manifest, 1, r#"{"objects":[]}"#)).unwrap();
    let hash = vectorcraft_plugins::permissions::sha256_hex(&bytes);
    let data = vectorcraft_format::base64_encode(&bytes);
    let ask = |h: &mut Harness<'static, VectorcraftApp>, lang: &str| {
        crate::i18n::set_current(crate::i18n::Lang::from_code(lang).unwrap());
        let r = h.state_mut().run("ui.installPlugin", json!({ "dataBase64": data }));
        let pending = h.state().pending_plugin_install.as_ref().map(|p| (p.message.clone(), p.detail.clone()));
        crate::i18n::set_current(crate::i18n::Lang::EN);
        frame(h);
        answer(h, false);
        assert_eq!(r.unwrap()["pending"], dialogs::plugin::INSTALL_KIND);
        pending.unwrap()
    };
    let (message, detail) = ask(&mut h, "de");
    assert!(message.contains("Fräsen") && !message.contains("Route"), "{message}");
    for shown in [id, "Route", hash.as_str()] {
        assert!(detail.contains(shown), "{shown}: {detail}");
    }
    let (message, detail) = ask(&mut h, "en");
    assert_eq!(message, "Install “Route”, which asks for a heavy budget?");
    assert!(detail.starts_with(&format!("ID: {id}\nFile SHA-256: {hash}\n\n")), "no second name line: {detail}");
    assert!(registry::get(id).is_none());
}

/// The dialog has no Preview; OK closes it and the filter runs in the background, the status bar
/// saying so, and its result is one undo step named after it, selected.
#[test]
fn a_heavy_filter_runs_off_the_ui_thread_and_applies_as_one_undo_step() {
    let (mut h, jobs) = held_window();
    let id = "test.ui.heavy.run";
    install(h.state_mut(), id, 1000, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#);
    h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut h);
    assert_eq!(previewing(h.state()), Some(false), "no Preview: a run may take minutes");
    assert!(!h.state().session.in_interaction(), "nothing previewed");
    let undo = h.state().session.doc().unwrap().history.undo.len();
    answer(&mut h, true);
    assert!(none_open(h.state()));
    assert_eq!(h.state().plugin_runs.running.len(), 1);
    assert_eq!(h.state().ui.status, format!("Running “Test {id}”…"));
    let app = h.state_mut();
    finish_held(app, &jobs);
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(st.history.undo.last().unwrap().label, format!("Test {id}"));
    assert_eq!(st.selection.objects.len(), 1, "the triangle it added, selected");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
}

/// The progress window opens once the run is [`export_progress::DELAY`] old on the app's clock,
/// however long the frames take; after Continue in Background closes it the result applies.
#[test]
fn the_progress_window_opens_by_the_app_clock() {
    use egui_kittest::kittest::Queryable;
    let (mut h, jobs) = held_window();
    let id = "test.ui.heavy.clock";
    install(h.state_mut(), id, 1, "{}");
    let undo = h.state().session.doc().unwrap().history.undo.len();
    assert_eq!(h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap()["background"], true);
    // Only the progress window shows its step.
    let step = "The plug-in is computing…";
    frame(&mut h);
    assert!(h.query_by_label(step).is_none(), "not before the delay");
    h.state_mut().now_override = Some(export_progress::DELAY);
    frame(&mut h);
    assert!(h.query_by_label(step).is_some(), "the progress window, after the delay");
    assert_eq!(h.state_mut().run("plugin.hideRuns", json!({})).unwrap(), json!({"hidden": 1}));
    frame(&mut h);
    assert!(h.query_by_label(step).is_none(), "Continue in Background closed it");
    finish_held(h.state_mut(), &jobs);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1);
}

/// A document edited while the filter runs doesn't take its result: nothing changes, and the status
/// bar says to run it again.
#[test]
fn a_heavy_filter_s_result_needs_the_document_as_it_was() {
    let mut app = app();
    let id = "test.ui.heavy.stale";
    install(&mut app, id, 1, "{}");
    // Without parameters it runs at once, in the background.
    assert_eq!(app.run("plugin.dialog", json!({"id": id})).unwrap()["background"], true);
    // An edit made while it runs: its result no longer fits.
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    let doc = app.session.doc().unwrap().doc.clone();
    wait(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, doc, "nothing changed");
    assert_eq!(app.ui.status, format!("“Test {id}” finished, but the document or the plug-in changed meanwhile: run it again"));
}

/// The progress window is the run's: headed by it, with Cancel and Continue in Background; Cancel
/// stops the run, which changes nothing, and the window goes.
#[test]
fn cancel_stops_a_heavy_filter_shown_in_its_progress_window() {
    let mut app = app();
    let id = "test.ui.heavy.cancel";
    // It would run for minutes.
    install(&mut app, id, 1 << 40, "{}");
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    let shown = plugin_runs::shown(&app).expect("the run's window");
    assert_eq!(shown.title, format!("Running “Test {id}”…"));
    assert_eq!((shown.cancel, shown.hide), ("plugin.cancelRuns", Some("plugin.hideRuns")));
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    assert!(plugin_runs::shown(&app).is_none(), "a cancelled run isn't shown");
    wait(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, doc);
    assert_eq!(app.ui.status, vectorcraft_engine::EngineError::Cancelled.to_string());
}

/// Continue in Background closes the progress window while the run goes on; Cancel still stops it.
#[test]
fn continue_in_background_hides_the_progress_window() {
    let mut app = app();
    let id = "test.ui.heavy.hide";
    install(&mut app, id, 1 << 40, "{}");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert!(plugin_runs::shown(&app).is_some());
    assert_eq!(app.run("plugin.hideRuns", json!({})).unwrap(), json!({"hidden": 1}));
    assert!(plugin_runs::shown(&app).is_none());
    assert!(app.plugin_runs.running.first().is_some_and(|r| r.hidden), "still running");
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap(), json!({"cancelled": 1}));
    wait(&mut app);
    assert_eq!(app.ui.status, vectorcraft_engine::EngineError::Cancelled.to_string());
}

/// A heavy filter of plug-in API level 2 that reports its progress gets a bar in its progress window,
/// with the share it reported, clamped; Cancel makes its `vc_cancelled` return 1 and it stops,
/// changing nothing.
#[test]
fn a_filter_that_reports_progress_shows_a_bar_and_sees_cancel() {
    let mut app = app();
    let id = "test.ui.heavy.progress";
    let manifest =
        format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","budget":"heavy","api":2,"permissions":["doc.read","doc.write","progress"]}}"#);
    let first = "(call $vc_progress (i64.const 5000) (i64.const 1000)) (call $vc_progress (i64.const 250) (i64.const 1000))
        (block $stop (loop $l (br_if $stop (call $vc_cancelled)) (br $l)))";
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::reporting(&manifest, first, r#"{"objects":[]}"#)).unwrap());
    app.run("plugin.install", json!({"dataBase64": data, "allowHeavy": true, "grant": ["doc.read", "doc.write", "progress"]})).unwrap();
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    // Waits for the run's thread to report the share (a handshake: the thread never stops before
    // it is cancelled, and a run that ended leaves the loop).
    loop {
        plugin_runs::poll(&mut app);
        match app.plugin_runs.running.first() {
            Some(r) if r.progress.counts() != (250, 1000) => std::thread::yield_now(),
            _ => break,
        }
    }
    assert_eq!(app.plugin_runs.running.first().map(|r| r.progress.counts()), Some((250, 1000)), "the second report, after the first was clamped");
    assert_eq!(plugin_runs::shown(&app).and_then(|s| s.fraction), Some(0.25));
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    while !app.plugin_runs.running.is_empty() {
        plugin_runs::poll(&mut app);
        std::thread::yield_now();
    }
    assert_eq!(app.session.doc().unwrap().doc, doc);
    assert_eq!(app.ui.status, vectorcraft_engine::EngineError::Cancelled.to_string());
}

/// The triangle the light test filters return.
const TRIANGLE: &str = r#"{"objects":[{"path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[30,0]},{"p":[15,20]}]}]}}]}"#;

/// Installs a light filter `id` of plug-in API level 2 with `params` that returns [`TRIANGLE`],
/// granted as the user would grant it. With `progress` it lists the `progress` permission, and its
/// `vc_run` first runs `first`, which may report its progress.
fn install_light(app: &mut VectorcraftApp, id: &str, progress: bool, params: &str, first: &str) {
    let may: &[&str] = if progress { &["doc.read", "doc.write", "progress"] } else { &["doc.read", "doc.write"] };
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{},"params":{params}}}"#, json!(may));
    // Without `progress` the module may not import the host functions.
    let src = if progress { tpl::reporting(&manifest, first, TRIANGLE) } else { tpl::returning(&manifest, TRIANGLE) };
    let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
    app.run("plugin.install", json!({"dataBase64": data, "grant": may})).unwrap();
}

/// Applies the runs' results as the frames would, until none is left (each run ends: it returns,
/// or its budget stops it).
fn finish_runs(app: &mut VectorcraftApp) {
    while !app.plugin_runs.running.is_empty() {
        plugin_runs::poll(app);
        std::thread::yield_now();
    }
}

/// Runs the jobs [`held_window`] held, then applies the runs' results as the frames would.
fn finish_held(app: &mut VectorcraftApp, jobs: &ManualJobs) {
    jobs.run_all();
    finish_runs(app);
}

/// A light filter that may report its progress runs off the UI thread when it runs at once, as a
/// heavy one does: the status bar says so, and its result is one undo step named after it.
#[test]
fn a_light_filter_that_reports_progress_runs_off_the_ui_thread() {
    let mut app = app();
    let id = "test.ui.light.progress.run";
    install_light(&mut app, id, true, "{}", "(call $vc_progress (i64.const 1) (i64.const 2))");
    let undo = app.session.doc().unwrap().history.undo.len();
    let r = app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert_eq!(r["background"], true, "{r}");
    assert_eq!(app.ui.status, format!("Running “Test {id}”…"));
    finish_runs(&mut app);
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(st.history.undo.last().unwrap().label, format!("Test {id}"));
    assert_eq!(st.selection.objects.len(), 1, "the triangle it added, selected");
}

/// In the dialog of a light filter that reports its progress, OK with Preview off runs it in the
/// background, also right after Preview was turned off; with Preview on, OK keeps the previewed
/// run, as for any light filter.
#[test]
fn ok_runs_a_light_progress_filter_in_the_background_without_a_preview() {
    let (mut h, jobs) = held_window();
    let id = "test.ui.light.progress.dialog";
    install_light(h.state_mut(), id, true, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#, "");
    let undo = h.state().session.doc().unwrap().history.undo.len();
    h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut h);
    set_preview(&mut h, false);
    assert!(!h.state().session.in_interaction(), "nothing previewed");
    answer(&mut h, true);
    assert!(none_open(h.state()));
    assert_eq!(h.state().plugin_runs.running.len(), 1, "running in the background");
    finish_held(h.state_mut(), &jobs);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 1);
    // Preview on: OK keeps the previewed run, and nothing runs in the background.
    h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut h);
    assert_eq!(previewing(h.state()), Some(true));
    assert!(h.state().session.in_interaction(), "previewed");
    answer(&mut h, true);
    assert!(h.state().plugin_runs.running.is_empty());
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 2);
    // Preview turned off after a preview: the next frame rolls it back and OK goes to the background.
    h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut h);
    assert!(h.state().session.in_interaction(), "previewed");
    set_preview(&mut h, false);
    assert!(!h.state().session.in_interaction(), "the preview was rolled back");
    answer(&mut h, true);
    assert_eq!(h.state().plugin_runs.running.len(), 1, "running in the background");
    finish_held(h.state_mut(), &jobs);
    assert_eq!(h.state().session.doc().unwrap().history.undo.len(), undo + 3);
}

/// In the dialog of a light filter that reports its progress, a failed preview is not kept: OK
/// rolls it back and runs the filter in the background instead of again on the UI thread.
#[test]
fn ok_after_a_failed_preview_runs_a_light_progress_filter_in_the_background() {
    let (mut h, jobs) = held_window();
    let id = "test.ui.light.progress.fails";
    install_light(h.state_mut(), id, true, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#, "(unreachable)");
    let doc = h.state().session.doc().unwrap().doc.clone();
    h.state_mut().run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut h);
    assert!(h.state().session.in_interaction(), "the preview's interaction");
    answer(&mut h, true);
    assert!(none_open(h.state()));
    // Only a refused preview is not kept; a kept one would have committed and left no run.
    assert!(!h.state().session.in_interaction(), "the failed preview was rolled back");
    assert_eq!(h.state().plugin_runs.running.len(), 1, "running in the background");
    finish_held(h.state_mut(), &jobs);
    assert_eq!(h.state().session.doc().unwrap().doc, doc);
}

/// A light filter without the `progress` permission still runs at once, on the UI thread.
#[test]
fn a_light_filter_that_does_not_report_progress_runs_at_once() {
    let mut app = app();
    let id = "test.ui.light.plain";
    install_light(&mut app, id, false, "{}", "");
    let r = app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert!(r.get("background").is_none(), "{r}");
    assert!(app.plugin_runs.running.is_empty());
    assert_eq!(app.session.doc().unwrap().selection.objects.len(), 1, "the triangle, added at once");
}
