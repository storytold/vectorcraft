//! Heavy plug-in filters in the app ([`crate::plugin_runs`]): installing one asks first, its dialog
//! has no Preview, and it runs off the UI thread, its result applied as one undo step once it is
//! done, unless the document changed meanwhile. Cancel stops it; its progress window shows it. A
//! light filter that reports its progress runs there too.

use std::time::{Duration, Instant};

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::{registry, wat as tpl};

use crate::{VectorcraftApp, dialogs, plugin_runs, theme};

/// An app with a new 200 × 200 pt document.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.session.allow_plugin_grants_from_requests();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app
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

/// One headless frame of the dialog layer.
fn frame(app: &mut VectorcraftApp) {
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(Default::default(), |ui| dialogs::show(app, ui.ctx()));
    out.textures_delta.clear();
}

/// Draws the progress window at `time` seconds → the texts it drew.
fn progress(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> String {
    let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| plugin_runs::show(app, ui.ctx()));
    out.textures_delta.clear();
    let mut texts = String::new();
    for shape in out.shapes {
        if let egui::epaint::Shape::Text(t) = shape.shape {
            texts.push_str(t.galley.text());
            texts.push('\n');
        }
    }
    texts
}

/// Install Plug-in asks before it installs a filter that asks for a heavy budget, and OK installs
/// it; File › Open of the `.wasm` asks too, and Cancel installs nothing.
#[test]
fn installing_a_heavy_filter_asks_first() {
    let mut app = app();
    let id = "test.ui.heavy.install";
    let bytes = heavy(id, 1, "{}");
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&bytes), "name": "route.wasm"})).unwrap();
    assert_eq!(r["pending"], dialogs::plugin::INSTALL_KIND);
    let pending = app.pending_plugin_install.as_ref().unwrap();
    assert_eq!(pending.message, format!("Install “Test {id}”, which asks for a heavy budget?"));
    assert!(pending.detail.contains("up to 5 minutes and do 20 times the work"), "{}", pending.detail);
    assert!(registry::get(id).is_none(), "not before the user agrees");
    super::plugins::show_approval(&mut app);
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(registry::get(id).map(|p| p.manifest().name.clone()), Some(format!("Test {id}")));
    assert_eq!(app.ui.status, format!("Installed plug-in Test {id}"));
    // File › Open of the .wasm asks too, and Cancel installs nothing.
    registry::remove(id);
    crate::io::open_bytes(&mut app, "route.wasm", &bytes, None).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::plugin::INSTALL_KIND));
    dialogs::cancel(&mut app);
    assert!(registry::get(id).is_none());
}

/// The dialog has no Preview; OK closes it and the filter runs in the background, the status bar
/// saying so, and its result is one undo step named after it, selected.
#[test]
fn a_heavy_filter_runs_off_the_ui_thread_and_applies_as_one_undo_step() {
    let mut app = app();
    let id = "test.ui.heavy.run";
    install(&mut app, id, 1000, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#);
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert!(app.ui.dialog.as_ref().unwrap().fields.get("preview").is_none(), "no Preview: a run may take minutes");
    frame(&mut app);
    assert!(!app.session.in_interaction(), "nothing previewed");
    let undo = app.session.doc().unwrap().history.undo.len();
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.plugin_runs.running.len(), 1);
    assert_eq!(app.ui.status, format!("Running “Test {id}”…"));
    wait(&mut app);
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    assert_eq!(st.history.undo.last().unwrap().label, format!("Test {id}"));
    assert_eq!(st.selection.objects.len(), 1, "the triangle it added, selected");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
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

/// The progress window opens after the delay, headed by the run; Cancel stops the run, which
/// changes nothing, and the window closes.
#[test]
fn cancel_stops_a_heavy_filter_shown_in_its_progress_window() {
    let mut app = app();
    let id = "test.ui.heavy.cancel";
    // It would run for minutes.
    install(&mut app, id, 1 << 40, "{}");
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    assert!(!progress(&mut app, &ctx, 0.0).contains("Running"), "not before the delay");
    progress(&mut app, &ctx, 1.0); // a new window measures itself unseen first
    let shown = progress(&mut app, &ctx, 1.1);
    let heading = format!("Running “Test {id}”…");
    assert!(shown.contains(&heading) && shown.contains("The plug-in is computing…") && shown.contains("Cancel"), "{shown}");
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    assert!(!progress(&mut app, &ctx, 2.0).contains(&heading), "a cancelled run isn't shown");
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
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    progress(&mut app, &ctx, 0.0);
    progress(&mut app, &ctx, 1.0);
    let heading = format!("Running “Test {id}”…");
    assert!(progress(&mut app, &ctx, 1.1).contains(&heading));
    assert_eq!(app.run("plugin.hideRuns", json!({})).unwrap(), json!({"hidden": 1}));
    assert!(!progress(&mut app, &ctx, 2.0).contains(&heading));
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
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    progress(&mut app, &ctx, 0.0);
    progress(&mut app, &ctx, 1.0);
    let shown = progress(&mut app, &ctx, 1.1);
    assert!(shown.contains("The plug-in is computing…") && shown.contains("25 %"), "{shown}");
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

/// A light filter that reports its progress shows it in the progress window, and Cancel there
/// makes its `vc_cancelled` return 1: it stops, changing nothing.
#[test]
fn a_light_filter_s_progress_window_shows_its_share_and_cancel_stops_it() {
    let mut app = app();
    let id = "test.ui.light.progress.cancel";
    let first = "(call $vc_progress (i64.const 250) (i64.const 1000)) (block $stop (loop $l (br_if $stop (call $vc_cancelled)) (br $l)))";
    install_light(&mut app, id, true, "{}", first);
    let doc = app.session.doc().unwrap().doc.clone();
    assert_eq!(app.run("plugin.dialog", json!({"id": id})).unwrap()["background"], true);
    // Waits for the run's thread to report the share (a handshake: the thread never stops before
    // it is cancelled or its budget runs out, and a run that ended leaves the loop).
    loop {
        plugin_runs::poll(&mut app);
        match app.plugin_runs.running.first() {
            Some(r) if r.progress.counts() != (250, 1000) => std::thread::yield_now(),
            _ => break,
        }
    }
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    progress(&mut app, &ctx, 0.0);
    progress(&mut app, &ctx, 1.0);
    let shown = progress(&mut app, &ctx, 1.1);
    assert!(shown.contains(&format!("Running “Test {id}”…")) && shown.contains("25 %") && shown.contains("Cancel"), "{shown}");
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    finish_runs(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, doc);
    assert_eq!(app.ui.status, vectorcraft_engine::EngineError::Cancelled.to_string());
}

/// In the dialog of a light filter that reports its progress, OK with Preview off runs it in the
/// background, also right after Preview was turned off; with Preview on, OK keeps the previewed
/// run, as for any light filter.
#[test]
fn ok_runs_a_light_progress_filter_in_the_background_without_a_preview() {
    let mut app = app();
    let id = "test.ui.light.progress.dialog";
    install_light(&mut app, id, true, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#, "");
    let undo = app.session.doc().unwrap().history.undo.len();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
    frame(&mut app);
    assert!(!app.session.in_interaction(), "nothing previewed");
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.plugin_runs.running.len(), 1, "running in the background");
    finish_runs(&mut app);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
    // Preview on: OK keeps the previewed run, and nothing runs in the background.
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().fields.get("preview"), Some(&json!(true)));
    frame(&mut app);
    assert!(app.session.in_interaction(), "previewed");
    dialogs::confirm(&mut app).unwrap();
    assert!(app.plugin_runs.running.is_empty());
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 2);
    // Preview turned off after a preview and OK before the next frame: the preview is rolled back
    // and the run goes to the background.
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut app);
    assert!(app.session.in_interaction(), "previewed");
    app.ui.dialog.as_mut().unwrap().fields.insert("preview".into(), json!(false));
    dialogs::confirm(&mut app).unwrap();
    assert!(!app.session.in_interaction(), "the preview was rolled back");
    assert_eq!(app.plugin_runs.running.len(), 1, "running in the background");
    finish_runs(&mut app);
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 3);
}

/// In the dialog of a light filter that reports its progress, a failed preview is not kept: OK
/// rolls it back and runs the filter in the background instead of again on the UI thread.
#[test]
fn ok_after_a_failed_preview_runs_a_light_progress_filter_in_the_background() {
    let mut app = app();
    let id = "test.ui.light.progress.fails";
    install_light(&mut app, id, true, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#, "(unreachable)");
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut app);
    assert!(app.session.in_interaction(), "the preview's interaction");
    assert!(app.ui.dialog.as_ref().unwrap().fields.contains_key("__refused"), "the preview failed");
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert!(!app.session.in_interaction(), "the failed preview was rolled back");
    assert_eq!(app.plugin_runs.running.len(), 1, "running in the background");
    finish_runs(&mut app);
    assert_eq!(app.session.doc().unwrap().doc, doc);
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

/// The workers a test drives by hand, as the web build's would run: each one a
/// [`vectorcraft_plugins::worker::Worker`] that handles what the page posted only when the test
/// says ([`Fake::serve`]), on a clock the test sets.
#[derive(Clone, Default)]
struct Fake {
    clock: std::rc::Rc<std::cell::Cell<f64>>,
    started: std::rc::Rc<std::cell::RefCell<Vec<FakeWorker>>>,
}

#[derive(Clone)]
struct FakeWorker {
    inbox: plugin_runs::SharedInbox,
    sent: std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>,
    terminated: std::rc::Rc<std::cell::Cell<bool>>,
    worker: std::rc::Rc<std::cell::RefCell<vectorcraft_plugins::worker::Worker>>,
}

struct FakeLink(FakeWorker);

impl plugin_runs::WorkerLink for FakeLink {
    fn post(&mut self, msg: Vec<u8>) -> Result<(), String> {
        self.0.sent.borrow_mut().push(msg);
        Ok(())
    }
    fn terminate(&mut self) {
        self.0.terminated.set(true);
    }
}

impl plugin_runs::Workers for Fake {
    fn start(&self, inbox: plugin_runs::SharedInbox) -> Result<Box<dyn plugin_runs::WorkerLink>, String> {
        let to_page = std::rc::Rc::clone(&inbox);
        let worker = vectorcraft_plugins::worker::Worker::new(std::rc::Rc::new(move |m: Vec<u8>| to_page.borrow_mut().push(&m)));
        let w = FakeWorker { inbox, sent: Default::default(), terminated: Default::default(), worker: std::rc::Rc::new(worker.into()) };
        self.started.borrow_mut().push(w.clone());
        Ok(Box::new(FakeLink(w)))
    }
    fn now_ms(&self) -> f64 {
        self.clock.get()
    }
}

impl Fake {
    fn worker(&self, k: usize) -> FakeWorker {
        self.started.borrow()[k].clone()
    }
    /// Worker `k` says it is ready.
    fn ready(&self, k: usize) {
        self.worker(k).worker.borrow().ready();
    }
    /// Worker `k` handles everything the page posted to it so far.
    fn serve(&self, k: usize) {
        let w = self.worker(k);
        let sent: Vec<_> = std::mem::take(&mut *w.sent.borrow_mut());
        for m in sent {
            w.worker.borrow_mut().handle(&m);
        }
    }
}

/// An app whose filters run in [`Fake`] workers, which keeps the modules it loads.
fn web_app() -> (VectorcraftApp, Fake) {
    vectorcraft_plugins::keep_modules(true);
    let mut app = app();
    let fake = Fake::default();
    app.services.plugin_workers = Some(Box::new(fake.clone()));
    (app, fake)
}

/// On the web a filter without a progress report runs in a worker too: nothing runs on the page,
/// and the result applies as one undo step once the worker posts it.
#[test]
fn on_the_web_a_filter_runs_in_a_worker_and_applies_as_one_undo_step() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.run";
    install_light(&mut app, id, false, "{}", "");
    let undo = app.session.doc().unwrap().history.undo.len();
    assert_eq!(app.run("plugin.dialog", json!({"id": id})).unwrap()["background"], true);
    plugin_runs::poll(&mut app);
    assert!(fake.worker(0).sent.borrow().is_empty(), "nothing is posted before the worker is ready");
    fake.ready(0);
    plugin_runs::poll(&mut app);
    assert_eq!(fake.worker(0).sent.borrow().len(), 1, "the run, once it is ready");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo, "nothing applied before the worker answers");
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty());
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, format!("Test {id}"));
    assert_eq!(st.selection.objects.len(), 1, "the triangle it added");
    assert!(fake.worker(0).terminated.get(), "the worker ends with the run");
}

/// On the web a filter's dialog has no Preview, which would run the plug-in on the page: OK runs it
/// in a worker.
#[test]
fn on_the_web_a_filter_dialog_has_no_preview_and_ok_runs_it_in_a_worker() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.dialog";
    install_light(&mut app, id, false, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#, "");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    frame(&mut app);
    assert!(!app.session.in_interaction(), "nothing previewed");
    dialogs::confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.plugin_runs.running.len(), 1);
    fake.ready(0);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty());
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, format!("Test {id}"));
}

/// A worker that never answers is terminated once the run's time budget and the grace after it
/// have passed on the page's clock; until then the page goes on, and the run changes nothing.
#[test]
fn on_the_web_a_plugin_that_never_answers_is_ended_at_its_time_budget() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.hung";
    install_light(&mut app, id, false, "{}", "");
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(0);
    let budget = vectorcraft_plugins::Limits::default().wall_time_ms as f64;
    fake.clock.set(budget);
    plugin_runs::poll(&mut app);
    assert_eq!(app.plugin_runs.running.len(), 1, "still within its budget and grace");
    fake.clock.set(budget + plugin_runs::GRACE_MS);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty());
    assert!(fake.worker(0).terminated.get());
    assert!(app.ui.status.contains("time budget"), "{}", app.ui.status);
    assert_eq!(app.session.doc().unwrap().doc, doc);
}

/// On the web a plug-in command runs in a worker too, and applies as the command: one undo step
/// named after it.
#[test]
fn on_the_web_a_plugin_command_runs_in_a_worker() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.command";
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read","doc.write"],"commands":[{{"id":"same","label":"Same"}}]}}"#
    );
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::returning(&manifest, TRIANGLE)).unwrap());
    app.run("plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"]})).unwrap();
    let cmd = format!("plugin.{id}.same");
    assert_eq!(app.run(&cmd, json!({})).unwrap()["background"], true);
    fake.ready(0);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty());
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Same");
    assert_eq!(app.session.doc().unwrap().selection.objects.len(), 1, "the triangle it added");
}

/// On the web a plug-in command's dialog has no Preview, and OK runs the command in a worker, also
/// while an interaction is open.
#[test]
fn on_the_web_a_command_dialog_runs_its_command_in_a_worker() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.commanddialog";
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read","doc.write"],"commands":[{{"id":"same","label":"Same","params":{{"size":{{"type":"number","min":1,"max":9,"default":3}}}}}}]}}"#
    );
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::returning(&manifest, TRIANGLE)).unwrap());
    app.run("plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"]})).unwrap();
    assert!(dialogs::plugin::open_command(&mut app, &format!("plugin.{id}.same")));
    frame(&mut app);
    assert!(!app.session.in_interaction(), "nothing previewed");
    app.session.begin_interaction("Move").unwrap();
    let doc = app.session.doc().unwrap().doc.clone();
    assert_eq!(dialogs::confirm(&mut app).unwrap()["background"], true);
    assert!(app.ui.dialog.is_none());
    assert_eq!(app.session.doc().unwrap().doc, doc, "nothing ran on the page");
    assert_eq!(app.plugin_runs.running.len(), 1);
    fake.ready(0);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty());
    assert!(!app.session.in_interaction());
    assert_ne!(app.session.doc().unwrap().doc, doc, "the worker's output applied");
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Same", "as an undo step of its own");
}

/// While a dialog is open a worker still gets its run, and Cancel and the time budget still end it;
/// only an output waits for the dialog to close.
#[test]
fn on_the_web_a_dialog_holds_back_the_output_but_not_cancel_or_the_time_budget() {
    let (mut app, fake) = web_app();
    let (id, dialog_id) = ("test.ui.web.underdialog", "test.ui.web.thedialog");
    install_light(&mut app, id, false, "{}", "");
    install_light(&mut app, dialog_id, false, r#"{"size":{"type":"number","min":1,"max":9,"default":3}}"#, "");
    let budget = vectorcraft_plugins::Limits::default().wall_time_ms as f64;
    // Overdue under a dialog.
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    app.run("plugin.dialog", json!({"id": dialog_id})).unwrap();
    assert!(app.ui.dialog.is_some());
    fake.ready(0);
    plugin_runs::poll(&mut app);
    assert_eq!(fake.worker(0).sent.borrow().len(), 1, "the run is posted under the dialog");
    fake.clock.set(budget + plugin_runs::GRACE_MS);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && fake.worker(0).terminated.get());
    // Canceled under a dialog.
    app.ui.dialog = None;
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    app.run("plugin.dialog", json!({"id": dialog_id})).unwrap();
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    assert!(app.plugin_runs.running.is_empty() && fake.worker(1).terminated.get());
    // An output waits for the dialog to close.
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(2);
    plugin_runs::poll(&mut app);
    fake.serve(2);
    plugin_runs::poll(&mut app);
    assert_eq!(app.plugin_runs.running.len(), 1, "the dialog is open");
    app.ui.dialog = None;
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty());
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, format!("Test {id}"));
}

/// Cancel terminates the worker at once, and an edit made while it runs stops it too: neither
/// changes the document.
#[test]
fn on_the_web_cancel_and_an_edit_end_the_worker_at_once() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.cancel";
    install_light(&mut app, id, false, "{}", "");
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(0);
    plugin_runs::poll(&mut app);
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    assert!(fake.worker(0).terminated.get());
    assert!(app.plugin_runs.running.is_empty());
    assert_eq!(app.session.doc().unwrap().doc, doc);
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(1);
    plugin_runs::poll(&mut app);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    plugin_runs::poll(&mut app);
    assert!(fake.worker(1).terminated.get(), "stopped once the document changed");
    assert_eq!(app.ui.status, format!("“Test {id}” stopped: the document or the plug-in changed meanwhile: run it again"));
}

/// A filter that calls commands runs in a worker that sends each call to the page: the page makes
/// it and answers, other commands wait, the progress window shows at once with Cancel only, and
/// the calls and the output are one undo step. Cancel during such a run undoes its calls.
#[test]
fn on_the_web_a_filter_that_calls_commands_holds_the_documents_until_it_ends() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.calls";
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"]}}"#);
    let imports = r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (data (i32.const 0x2000) "select.all")
  (data (i32.const 0x2100) "{}")"#;
    let first = "(drop (call $vc_call (i32.const 0x2000) (i32.const 10) (i32.const 0x2100) (i32.const 2)))";
    let wasm = wat::parse_str(tpl::with_imports(&manifest, imports, first, r#"{"objects":[]}"#)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant})).unwrap();
    let rect =
        |app: &mut VectorcraftApp, x: f64| app.run("shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10})).unwrap()["id"].clone();
    let (a, b) = (rect(&mut app, 10.0), rect(&mut app, 40.0));
    app.run("select.set", json!({"ids": [a]})).unwrap();
    let undo = app.session.doc().unwrap().history.undo.len();
    let before = app.session.doc().unwrap().doc.clone();
    // Cancel while it waits for the worker: what its calls did is undone.
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(0);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert_eq!(app.session.doc().unwrap().selection.objects.len(), 2, "the page made the call");
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    assert!(!app.session.plugin_calls_open());
    assert_eq!(app.session.doc().unwrap().doc, before);
    assert_eq!(app.session.doc().unwrap().selection.objects.len(), 1);
    // Run to its end.
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    let e = app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap_err();
    assert_eq!(e, vectorcraft_engine::EngineError::PluginBusy.to_string());
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    progress(&mut app, &ctx, 0.0); // a new window measures itself unseen first
    let texts = progress(&mut app, &ctx, 0.1);
    assert!(texts.contains("Cancel") && !texts.contains("Continue in Background"), "shown before the delay, with Cancel only: {texts}");
    assert_eq!(app.run("plugin.hideRuns", json!({})).unwrap_err(), e, "only Cancel goes through");
    fake.ready(1);
    plugin_runs::poll(&mut app);
    fake.serve(1);
    plugin_runs::poll(&mut app);
    fake.serve(1);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && !app.session.plugin_calls_open());
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1, "the call and the output are one undo step");
    let id_of = |v: &serde_json::Value| vectorcraft_doc::NodeId(v.as_u64().unwrap());
    assert!(st.doc.node(id_of(&a)).is_none() && st.doc.node(id_of(&b)).is_some());
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 5, "height": 5})).unwrap();
}

/// A run that ends while another run holds the documents waits for it, and doesn't end it.
#[test]
fn on_the_web_a_run_that_ends_while_another_holds_the_documents_waits() {
    let (mut app, fake) = web_app();
    let (plain, caller) = ("test.ui.web.waits", "test.ui.web.holds");
    install_light(&mut app, plain, false, "{}", "");
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest =
        format!(r#"{{"id":"{caller}","name":"Test {caller}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"]}}"#);
    let imports = r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (data (i32.const 0x2000) "select.all")
  (data (i32.const 0x2100) "{}")"#;
    let first = "(drop (call $vc_call (i32.const 0x2000) (i32.const 10) (i32.const 0x2100) (i32.const 2)))";
    let wasm = wat::parse_str(tpl::with_imports(&manifest, imports, first, TRIANGLE)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant})).unwrap();
    app.run("plugin.dialog", json!({"id": plain})).unwrap();
    app.run("plugin.dialog", json!({"id": caller})).unwrap();
    assert!(app.session.plugin_calls_open());
    fake.ready(0);
    fake.ready(1);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(app.session.plugin_calls_open(), "the other run's end leaves the holder running");
    fake.serve(1);
    plugin_runs::poll(&mut app);
    fake.serve(1);
    plugin_runs::poll(&mut app);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && !app.session.plugin_calls_open());
    let labels: Vec<String> = app.session.doc().unwrap().history.undo.iter().map(|e| e.label.clone()).collect();
    assert!(labels.contains(&format!("Test {caller}")), "{labels:?}");
}

/// Install Plug-in on the web checks the module in a worker first: the page installs it once the
/// worker has read its manifest, and a module that fails there is not installed, the status bar
/// saying why.
#[test]
fn on_the_web_a_module_is_checked_in_a_worker_before_it_installs() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.install";
    let manifest = tpl::manifest(id, "filter", "{}");
    let good = wat::parse_str(tpl::returning(&manifest, TRIANGLE)).unwrap();
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&good)})).unwrap();
    assert_eq!(r, json!({"pending": true}));
    assert!(registry::get(id).is_none(), "not before the worker checked it");
    fake.ready(0);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(registry::get(id).is_some());
    assert!(fake.worker(0).terminated.get());
    let bad_id = "test.ui.web.badinstall";
    let traps = wat::parse_str(tpl::module(&tpl::manifest(bad_id, "filter", "{}"), tpl::ECHO, "(start $s) (func $s unreachable)", 1)).unwrap();
    app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&traps)})).unwrap();
    fake.ready(1);
    plugin_runs::poll(&mut app);
    fake.serve(1);
    plugin_runs::poll(&mut app);
    assert!(registry::get(bad_id).is_none());
    assert!(app.ui.status.contains("trapped"), "{}", app.ui.status);
    registry::remove(id);
}

/// Two checked installs that both ask for approval ask one at a time: the second waits while the
/// first one's question is open.
#[test]
fn on_the_web_checked_installs_ask_one_at_a_time() {
    let (mut app, fake) = web_app();
    let module = |id: &str| {
        let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read"]}}"#);
        vectorcraft_format::base64_encode(&wat::parse_str(tpl::returning(&manifest, TRIANGLE)).unwrap())
    };
    let (first, second) = ("test.ui.web.askfirst", "test.ui.web.asksecond");
    app.run("ui.installPlugin", json!({"dataBase64": module(first)})).unwrap();
    app.run("ui.installPlugin", json!({"dataBase64": module(second)})).unwrap();
    // Both answers come before the same frame.
    fake.ready(0);
    fake.ready(1);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    fake.serve(1);
    plugin_runs::poll(&mut app);
    let asks = |app: &VectorcraftApp| app.pending_plugin_install.as_ref().map(|p| p.message.clone()).unwrap_or_default();
    assert!(asks(&app).contains(first), "{}", asks(&app));
    plugin_runs::poll(&mut app);
    assert!(asks(&app).contains(first), "the second waits: {}", asks(&app));
    app.ui.dialog = None;
    app.pending_plugin_install = None;
    plugin_runs::poll(&mut app);
    assert!(asks(&app).contains(second), "{}", asks(&app));
}

/// A worker's inbox keeps its latest progress report and a few other messages; one more, a message
/// the page can't read, or a worker of another protocol ends what it is doing.
#[test]
fn a_worker_inbox_is_bounded() {
    use vectorcraft_plugins::worker::FromWorker;
    let mut inbox = plugin_runs::Inbox::default();
    for done in 0..10_000 {
        inbox.push(&FromWorker::Progress { done, total: 0 }.encode());
    }
    for _ in 0..plugin_runs::MAX_QUEUED {
        inbox.push(&FromWorker::Call { command: "select.all".into(), args: json!({}) }.encode());
    }
    assert!(!inbox.failed(), "progress reports replace each other");
    inbox.push(&FromWorker::Call { command: "select.all".into(), args: json!({}) }.encode());
    assert!(inbox.failed());
    let (mut app, fake) = web_app();
    let id = "test.ui.web.flood";
    install_light(&mut app, id, false, "{}", "");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    let w = fake.worker(0);
    for _ in 0..=plugin_runs::MAX_QUEUED {
        w.inbox.borrow_mut().push(&FromWorker::Ready { protocol: vectorcraft_plugins::worker::PROTOCOL }.encode());
    }
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && w.terminated.get());
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.worker(1).inbox.borrow_mut().push(&FromWorker::Ready { protocol: 0 }.encode());
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && fake.worker(1).terminated.get(), "another protocol");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.worker(2).inbox.borrow_mut().push(b"junk");
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && fake.worker(2).terminated.get(), "a message it can't read");
}

/// Installs filter `id`, which calls `select.all` and then returns `output`, granted as the user
/// would grant it.
fn install_caller(app: &mut VectorcraftApp, id: &str, output: &str) {
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"]}}"#);
    let imports = r#"(import "env" "vc_call" (func $vc_call (param i32 i32 i32 i32) (result i32)))
  (data (i32.const 0x2000) "select.all")
  (data (i32.const 0x2100) "{}")"#;
    let first = "(drop (call $vc_call (i32.const 0x2000) (i32.const 10) (i32.const 0x2100) (i32.const 2)))";
    let wasm = wat::parse_str(tpl::with_imports(&manifest, imports, first, output)).unwrap();
    app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant})).unwrap();
}

/// A dialog that some other part of the app opened.
fn open_dialog(app: &mut VectorcraftApp) {
    app.ui.dialog = Some(crate::state::Dialog::new("test.other", json!({})));
}

/// While a dialog is open, a command call waiting in the inbox is not answered, so it doesn't keep
/// the run, and its hold on the documents, past the run's deadline.
#[test]
fn on_the_web_a_call_waiting_under_a_dialog_ends_at_the_time_budget() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.callunderdialog";
    install_caller(&mut app, id, TRIANGLE);
    let doc = app.session.doc().unwrap().doc.clone();
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    assert!(app.session.plugin_calls_open());
    open_dialog(&mut app);
    fake.ready(0);
    plugin_runs::poll(&mut app);
    // The worker runs it to its command call, which waits for the dialog.
    fake.serve(0);
    fake.clock.set(vectorcraft_plugins::Limits::default().wall_time_ms as f64 + plugin_runs::GRACE_MS);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && fake.worker(0).terminated.get());
    assert!(!app.session.plugin_calls_open(), "the documents are free again");
    assert_eq!(app.session.doc().unwrap().doc, doc);
}

/// A run whose document changes is stopped at once, also while a dialog is open.
#[test]
fn on_the_web_an_edit_ends_the_worker_while_a_dialog_is_open() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.staleunderdialog";
    install_light(&mut app, id, false, "{}", "");
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(0);
    plugin_runs::poll(&mut app);
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    open_dialog(&mut app);
    plugin_runs::poll(&mut app);
    assert!(app.plugin_runs.running.is_empty() && fake.worker(0).terminated.get(), "stopped once the document changed");
    assert_eq!(app.ui.status, format!("“Test {id}” stopped: the document or the plug-in changed meanwhile: run it again"));
}

/// A module checked while a run holds the documents installs once the run ends, without asking
/// for it again.
#[test]
fn on_the_web_a_checked_install_waits_for_the_run_that_holds_the_documents() {
    let (mut app, fake) = web_app();
    let (id, caller) = ("test.ui.web.installwaits", "test.ui.web.installwaits.caller");
    install_caller(&mut app, caller, TRIANGLE);
    let module = wat::parse_str(tpl::returning(&tpl::manifest(id, "filter", "{}"), TRIANGLE)).unwrap();
    let r = app.run("ui.installPlugin", json!({"dataBase64": vectorcraft_format::base64_encode(&module)})).unwrap();
    assert_eq!(r, json!({"pending": true}));
    app.run("plugin.dialog", json!({"id": caller})).unwrap();
    assert!(app.session.plugin_calls_open());
    fake.ready(0);
    plugin_runs::poll(&mut app);
    fake.serve(0);
    plugin_runs::poll(&mut app);
    assert!(registry::get(id).is_none(), "not while the run holds the documents");
    assert_eq!(app.run("plugin.cancelRuns", json!({})).unwrap()["cancelled"], 1);
    plugin_runs::poll(&mut app);
    assert!(registry::get(id).is_some(), "{}", app.ui.status);
    registry::remove(id);
}

/// The progress window shows the run that holds the documents at once, with Cancel only, also when
/// a run that started before it is running.
#[test]
fn on_the_web_the_progress_window_shows_the_run_that_holds_the_documents_first() {
    let (mut app, _fake) = web_app();
    let (plain, caller) = ("test.ui.web.showplain", "test.ui.web.showholder");
    install_light(&mut app, plain, false, "{}", "");
    install_caller(&mut app, caller, TRIANGLE);
    app.run("plugin.dialog", json!({"id": plain})).unwrap();
    app.run("plugin.dialog", json!({"id": caller})).unwrap();
    let ctx = egui::Context::default();
    theme::install_fonts(&ctx);
    progress(&mut app, &ctx, 0.0); // a new window measures itself unseen first
    let texts = progress(&mut app, &ctx, 0.1);
    assert!(texts.contains(&format!("Running “Test {caller}”…")), "{texts}");
    assert!(texts.contains("Cancel") && !texts.contains("Continue in Background"), "{texts}");
    app.run("plugin.cancelRuns", json!({})).unwrap();
}

/// While a script runs, a worker run still gets its request and its time budget still ends it, and
/// no other run starts in a worker.
#[test]
fn on_the_web_a_worker_run_is_ended_at_its_time_budget_while_a_script_runs() {
    let (mut app, fake) = web_app();
    let id = "test.ui.web.underscript";
    install_light(&mut app, id, false, "{}", "");
    app.session.prefs.show_external_script_warning = false;
    app.run("plugin.dialog", json!({"id": id})).unwrap();
    fake.ready(0);
    crate::menus::run_ui_command(&mut app, "ui.runScript", &json!({"source": "while (true) {}"})).unwrap().unwrap();
    assert!(app.session.script_running());
    app.poll_finished_work();
    assert_eq!(fake.worker(0).sent.borrow().len(), 1, "the run is posted while the script runs");
    assert!(app.run("plugin.dialog", json!({"id": id})).is_err(), "no new worker run");
    fake.clock.set(vectorcraft_plugins::Limits::default().wall_time_ms as f64 + plugin_runs::GRACE_MS);
    app.poll_finished_work();
    assert!(app.plugin_runs.running.is_empty() && fake.worker(0).terminated.get());
    app.run("script.stop", json!({})).unwrap();
    while app.session.script_running() {
        crate::script_runs::poll(&mut app);
    }
}

/// A recovery round waits while a run holds the documents, since a run that fails or is canceled
/// undoes its edits, and runs once the run ends.
#[test]
fn recovery_copies_wait_for_the_run_that_holds_the_documents() {
    let store = std::sync::Arc::new(vectorcraft_engine::cmd::recovery::MemoryStore::default());
    let services = crate::Services { recovery_store: Some(store), ..Default::default() };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.session.allow_plugin_grants_from_requests();
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})).unwrap();
    let id = "test.ui.recovery.holds";
    install_caller(&mut app, id, r#"{"objects":[]}"#);
    app.session.prefs.autosave_interval = 2;
    assert_eq!(crate::recovery::tick(&mut app, 10.0), 0);
    vectorcraft_engine::cmd::plugin::FilterRun::begin_calls(&mut app.session, "plugin.run", &json!({"id": id})).unwrap();
    assert_eq!(crate::recovery::tick(&mut app, 130.0), 0, "not while the run holds the documents");
    app.session.end_plugin_calls();
    assert_eq!(crate::recovery::tick(&mut app, 131.0), 1);
    registry::remove(id);
}

/// `CRAFT_UI_HEAVY_FILTER_WASM=<folder>` writes a heavy test filter to install in the app
/// (`order-paths-test-filter.wasm`): it computes until it is cancelled or its budget runs out, so
/// its progress window stays up for a picture.
#[test]
fn heavy_test_filter_for_pictures() {
    let Some(dir) = std::env::var_os("CRAFT_UI_HEAVY_FILTER_WASM") else { return };
    let manifest = r#"{"id":"org.example.order","name":"Order Paths for Plotting (test plug-in)","version":"1","kind":"filter",
        "budget":"heavy","description":"Orders the selected paths so the pen travels as little as possible between them.",
        "params":{"passes":{"type":"int","min":1,"max":100,"default":20,"label":"Improvement passes"},
        "start":{"type":"choice","options":["top left","nearest"],"default":"top left","label":"Start at"}}}"#;
    let src = tpl::spinning(manifest, 1 << 40, r#"{"objects":[]}"#);
    std::fs::write(std::path::PathBuf::from(dir).join("order-paths-test-filter.wasm"), wat::parse_str(src).unwrap()).unwrap();
}
