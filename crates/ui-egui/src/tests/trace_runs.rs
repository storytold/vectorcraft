//! Image Trace in the app ([`crate::trace_runs`], #3098): a trace the user asks for runs off the UI
//! thread and is applied as one undo step once it is done, unless its image changed meanwhile. A
//! newer trace of the same object replaces it; Cancel stops it; its progress window shows it.

use std::time::{Duration, Instant};

use serde_json::json;
use vectorcraft_engine::{EngineError, Session};

use crate::{VectorcraftApp, trace_runs};

/// An app with a 100 × 100 pt document and a 40 × 40 px image placed in it, selected; traces run
/// off the UI thread.
fn app() -> VectorcraftApp {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let img = image::RgbaImage::from_fn(40, 40, |x, y| image::Rgba(if x < 20 && y < 30 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    app.run("file.place", json!({"name": "half.png", "dataBase64": data, "link": false})).unwrap();
    app.trace_runs.off_the_ui_thread = true;
    app
}

/// Applies the traces' results as the frames would, until none is left (at most 30 s).
fn wait(app: &mut VectorcraftApp) {
    let t0 = Instant::now();
    while !app.trace_runs.running.is_empty() && t0.elapsed() < Duration::from_secs(30) {
        trace_runs::poll(app);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.trace_runs.running.is_empty(), "still running");
}

/// Undo steps of the active document.
fn undos(app: &VectorcraftApp) -> usize {
    app.session.doc().unwrap().history.undo.len()
}

/// Draws the progress window at `time` seconds → the texts it drew.
fn progress(app: &mut VectorcraftApp, ctx: &egui::Context, time: f64) -> String {
    let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| trace_runs::show(app, ui.ctx()));
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

/// Object › Image Trace › Make traces in the background, the status bar saying so, and its result is
/// one undo step, the Image Trace object selected.
#[test]
fn make_traces_off_the_ui_thread_as_one_undo_step() {
    let mut app = app();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    assert_eq!(app.trace_runs.running.len(), 1);
    assert_eq!(app.ui.status, "Tracing the image…");
    assert_eq!(undos(&app), before, "nothing yet");
    wait(&mut app);
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), before + 1, "one undo step");
    assert_eq!(st.history.undo.last().unwrap().label, "Image Trace");
    let [id] = st.selection.objects[..] else { panic!("{:?}", st.selection.objects) };
    assert_eq!(st.doc.node(id).unwrap().name.as_deref(), Some("Image Trace"));
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
    // Make and Expand too.
    crate::menus::invoke(&mut app, "imageTrace.makeAndExpand", json!({"preset": "Default"}));
    assert_eq!(app.trace_runs.running.len(), 1);
    wait(&mut app);
    assert_eq!(app.session.doc().unwrap().history.undo.last().unwrap().label, "Image Trace (Make and Expand)");
}

/// Starting a trace asks for a frame, so its result is picked up without input: the frame that
/// started it may have asked for its next one before it did (#4617 review).
#[test]
fn starting_a_trace_asks_for_a_frame() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.adopt_context(&ctx);
    for _ in 0..5 {
        if !ctx.has_requested_repaint() {
            break;
        }
        let mut out = ctx.run_ui(Default::default(), |_| {});
        out.textures_delta.clear();
    }
    assert!(!ctx.has_requested_repaint(), "the context settled");
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    assert_eq!(app.trace_runs.running.len(), 1);
    assert!(ctx.has_requested_repaint());
    wait(&mut app);
}

/// Without threads (the web, `VECTORCRAFT_SYNC_OPEN`, unit tests that don't ask), Make traces at once.
#[test]
fn without_threads_make_traces_at_once() {
    let mut app = app();
    app.trace_runs.off_the_ui_thread = false;
    let before = undos(&app);
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    assert!(app.trace_runs.running.is_empty());
    assert_eq!(undos(&app), before + 1);
}

/// A trace's notes: the status bar names how many there are and the first of them, and keeps them
/// all, in the reply's order, for the message's hover. Make and Make and Expand, at once, off the
/// UI thread and from the Image Trace panel; a trace without notes leaves the status bar empty.
#[test]
fn a_trace_shows_how_many_notes_it_has() {
    let mut app = app();
    let img = image::RgbaImage::from_fn(1501, 20, |x, _| image::Rgba(if x < 700 { [0, 0, 0, 255] } else { [255, 255, 255, 255] }));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    let data = vectorcraft_format::base64_encode(&png);
    let p = json!({"params": {"mode": "color", "palette": "limited", "colors": 2, "keepColors": ["#ff00ff", "#00ffff"]}});
    for (command, threaded, from_panel) in
        [("imageTrace.make", false, false), ("imageTrace.make", true, false), ("imageTrace.makeAndExpand", true, true)]
    {
        app.run("file.place", json!({"name": "wide.png", "dataBase64": data, "link": false})).unwrap();
        app.trace_runs.off_the_ui_thread = threaded;
        app.ui.status.clear();
        trace_runs::start(&mut app, command, p.clone(), from_panel).unwrap();
        wait(&mut app);
        let notes = [
            "Nothing was traced: none of the colors the trace found matches keepColors (#ff00ff, #00ffff).",
            "The image is 1501 × 20 pixels, more than 1500 on a side: it is traced, but more slowly. A lower-fidelity preset or a smaller copy of the image traces faster.",
        ]
        .map(String::from)
        .to_vec();
        let status = format!("Traced with 2 notes: {}", notes[0]);
        assert_eq!(app.ui.status, status, "{command}, threaded {threaded}");
        assert_eq!(app.ui.status_notes, Some((status, notes)), "{command}, threaded {threaded}");
    }
    // The 40 × 40 px image of [`app`] traces without notes: the status bar stays empty.
    let mut app = self::app();
    for command in ["imageTrace.make", "imageTrace.makeAndExpand"] {
        app.ui.status.clear();
        let v = app.run(command, json!({"preset": "Default"})).unwrap();
        assert_eq!(v["warnings"], json!([]), "{command}");
        assert_eq!((app.ui.status.as_str(), &app.ui.status_notes), ("", &None), "{command}");
        app.run("edit.undo", json!({})).unwrap();
    }
}

/// Nothing to trace fails at once, before anything runs.
#[test]
fn nothing_to_trace_fails_at_once() {
    let mut app = app();
    app.run("select.none", json!({})).unwrap();
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    assert!(app.trace_runs.running.is_empty());
    assert!(!app.ui.status.is_empty(), "the status bar says why");
}

/// A second trace of the same object replaces the one running: only it is applied, and the first
/// ends without a word.
#[test]
fn a_new_trace_of_the_same_object_replaces_the_one_running() {
    let mut app = app();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Black and White Logo"}));
    assert_eq!(app.trace_runs.running.len(), 2);
    wait(&mut app);
    let st = app.session.doc().unwrap();
    assert_eq!(st.history.undo.len(), before + 1, "the newer trace alone");
    let [id] = st.selection.objects[..] else { panic!() };
    let stored = st.doc.node(id).unwrap().extras().trace.as_deref().cloned().unwrap();
    assert_eq!(stored["preset"], "Black and White Logo");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
}

/// Cancel (the progress window's, `imageTrace.cancelRuns`) changes nothing, even when the trace
/// had ended just before it.
#[test]
fn cancel_changes_nothing() {
    let mut app = app();
    let before = undos(&app);
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    assert_eq!(app.run("imageTrace.cancelRuns", json!({})).unwrap(), json!({"cancelled": 1}));
    wait(&mut app);
    assert_eq!(undos(&app), before);
    assert_eq!(app.ui.status, EngineError::Cancelled.to_string());
}

/// An image moved while it is traced doesn't take the trace: nothing changes, and the status bar
/// says to trace it again.
#[test]
fn a_trace_needs_its_image_as_it_was() {
    let mut app = app();
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    app.run("object.move", json!({"dx": 5, "dy": 0})).unwrap();
    let before = undos(&app);
    wait(&mut app);
    assert_eq!(undos(&app), before, "nothing changes");
    assert!(app.ui.status.contains("changed meanwhile: trace it again"), "{}", app.ui.status);
}

/// The progress window opens after its delay with Cancel and Continue in Background; Continue in
/// Background closes it and the trace goes on.
#[test]
fn the_progress_window_shows_a_trace() {
    let mut app = app();
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::menus::invoke(&mut app, "imageTrace.make", json!({"preset": "Default"}));
    assert!(!progress(&mut app, &ctx, 0.0).contains("Tracing the image"), "not before the delay");
    progress(&mut app, &ctx, 1.0); // a new window measures itself unseen first
    let shown = progress(&mut app, &ctx, 1.1);
    assert!(shown.contains("Tracing the image…") && shown.contains("Cancel") && shown.contains("Continue in Background"), "{shown}");
    assert_eq!(app.run("imageTrace.hideRuns", json!({})).unwrap(), json!({"hidden": 1}));
    assert!(!progress(&mut app, &ctx, 2.0).contains("Tracing the image"));
    let before = undos(&app);
    wait(&mut app);
    assert_eq!(undos(&app), before + 1, "it went on");
}
