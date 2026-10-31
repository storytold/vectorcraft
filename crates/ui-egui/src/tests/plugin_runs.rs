//! Heavy plug-in filters in the app ([`crate::plugin_runs`]): installing one asks first, its dialog
//! has no Preview, and it runs off the UI thread, its result applied as one undo step once it is
//! done, unless the document changed meanwhile. Cancel stops it; its progress window shows it.

use std::time::{Duration, Instant};

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_plugins::{registry, wat as tpl};

use crate::{VectorcraftApp, dialogs, plugin_runs, theme};

/// An app with a new 200 × 200 pt document.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
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
    assert_eq!(r["pending"], dialogs::confirm::KIND);
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(d.str("message"), format!("Install “Test {id}”, which asks for a heavy budget?"));
    assert!(d.str("detail").contains("up to 5 minutes and do 20 times the work"), "{}", d.str("detail"));
    assert!(registry::get(id).is_none(), "not before the user agrees");
    dialogs::confirm(&mut app).unwrap();
    assert_eq!(registry::get(id).map(|p| p.manifest().name.clone()), Some(format!("Test {id}")));
    assert_eq!(app.ui.status, format!("Installed plug-in Test {id}"));
    // File › Open of the .wasm asks too, and Cancel installs nothing.
    registry::remove(id);
    crate::io::open_bytes(&mut app, "route.wasm", &bytes, None).unwrap();
    assert_eq!(app.ui.dialog.as_ref().map(|d| d.kind.as_str()), Some(dialogs::confirm::KIND));
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
