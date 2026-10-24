//! Background Save and Background Export: the file is written from a snapshot off the UI thread,
//! and the document counts as saved as of that snapshot.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Rect, shapes};

use crate::control::ControlRequest;
use crate::{Services, VectorcraftApp, background, io};

pub(super) type Files = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Holds the background writer until opened.
#[derive(Clone, Default)]
pub(super) struct Gate(Arc<AtomicBool>);

impl Gate {
    pub(super) fn open(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub(super) fn wait(&self) {
        while !self.0.load(Ordering::SeqCst) {
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

/// An app whose thread-safe writer records files once `gate` opens.
fn app(gate: Gate) -> (VectorcraftApp, Files) {
    let files = Files::default();
    let f = files.clone();
    let services = Services {
        write: Some(Box::new(|_: &str, _: &[u8]| Err("the UI thread must not write here".into()))),
        write_shared: Some(Arc::new(move |p: &str, b: &[u8]| {
            gate.wait();
            f.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    (VectorcraftApp::new(Session::new(), services), files)
}

/// A document of `n` paths.
fn paths(n: usize) -> Document {
    let mut d = Document::new(1000.0, 1000.0);
    let layer = d.layers[0].id;
    for i in 0..n {
        let id = d.alloc_id();
        let (x, y) = ((i % 250) as f64 * 4.0, (i / 250) as f64 * 4.0);
        d.insert(Some(layer), i, Node::path(id, shapes::ellipse(Rect::new(x, y, x + 3.0, y + 3.0)), Appearance::default_art())).unwrap();
    }
    d
}

pub(super) fn written(files: &Files) -> Vec<String> {
    files.lock().unwrap().iter().map(|(p, _)| p.clone()).collect()
}

#[test]
fn a_50k_path_save_returns_at_once_and_the_file_appears_later() {
    let gate = Gate::default();
    let (mut app, files) = app(gate.clone());
    app.session.add_document(paths(50_000), None);
    let r = app.run("file.save", json!({"path": "/docs/big.vectorcraft"})).unwrap();
    assert_eq!((r["background"].as_bool(), r["path"].as_str()), (Some(true), Some("/docs/big.vectorcraft")));
    assert!(written(&files).is_empty(), "nothing written yet");
    assert_eq!(app.background.jobs.len(), 1);
    assert!(app.ui.status.starts_with("Saving big.vectorcraft"));
    // The file is the document's once it is written (and its dates are stamped then, #1721: until
    // then this document, never edited, is as it was).
    assert!(!app.session.doc().unwrap().is_dirty() && app.session.doc().unwrap().path.is_none());
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/docs/big.vectorcraft"]);
    let st = app.session.doc().unwrap();
    assert!(!st.is_dirty());
    assert_eq!(st.path.as_deref(), Some("/docs/big.vectorcraft"));
    assert_eq!(app.ui.status, "Saved /docs/big.vectorcraft");
    let bytes = files.lock().unwrap()[0].1.clone();
    assert_eq!(vectorcraft_format::load(&bytes).unwrap().node_count(), 50_001);
}

#[test]
fn the_dirty_state_follows_the_snapshot() {
    let gate = Gate::default();
    let (mut app, files) = app(gate.clone());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    app.run("file.save", json!({"path": "/docs/a.vectorcraft"})).unwrap();
    // Edited while the file is written: still modified afterwards.
    app.run("shape.rectangle", json!({"x": 20, "y": 0, "width": 10, "height": 10})).unwrap();
    gate.open();
    background::wait_all(&mut app);
    assert!(app.session.doc().unwrap().is_dirty(), "the edit came after the snapshot");
    let saved = vectorcraft_format::load(&files.lock().unwrap()[0].1).unwrap();
    assert_eq!(saved.node_count(), 2, "the file holds the snapshot");
    // Undoing back to the snapshot is clean again.
    app.run("edit.undo", json!({})).unwrap();
    assert!(!app.session.doc().unwrap().is_dirty());
}

#[test]
fn exports_run_in_the_background_and_the_preference_turns_both_off() {
    let gate = Gate::default();
    let (mut app, files) = app(gate.clone());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let r = io::export(&mut app, Some("svg"), Some("/out/a.svg".into()), &json!({})).unwrap();
    assert_eq!((r["background"].as_bool(), r["path"].as_str()), (Some(true), Some("/out/a.svg")));
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/out/a.svg"]);
    assert_eq!(app.ui.status, "Exported /out/a.svg");
    // Preferences off: at once, through the ordinary writer (which this app refuses).
    app.session.execute("prefs.set", &json!({"values": {"backgroundSave": false, "backgroundExport": false}})).unwrap();
    assert!(io::export(&mut app, Some("svg"), Some("/out/b.svg".into()), &json!({})).is_err());
    assert!(app.run("file.save", json!({"path": "/docs/x.vectorcraft"})).is_err());
    assert!(app.background.jobs.is_empty());
}

/// A save that returns before the file is written still says what it writes and which options it
/// does not use, as `app.save` (MCP's `save_file`) replies with it: what the format loses first, as
/// the reply of a save written at once.
#[test]
fn a_background_save_names_its_format_and_unused_options_at_once() {
    let gate = Gate::default();
    let (mut app, files) = app(gate.clone());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let r = app.run("file.save", json!({"path": "/out/a.svg", "compress": true})).unwrap();
    assert_eq!((r["background"].as_bool(), r["format"].as_str()), (Some(true), Some("svg")), "{r}");
    let warnings = r["warnings"].as_array().unwrap();
    assert_eq!(warnings.len(), 2, "{r}");
    assert!(warnings[0].as_str().unwrap().starts_with("SVG keeps the artwork but not everything"), "{r}");
    assert_eq!(warnings[1], "`compress` was not used: SVG files don't read it", "{r}");
    gate.open();
    background::wait_all(&mut app);
    let r = app.run("file.save", json!({"path": "/out/b.vectorcraft", "compress": true})).unwrap();
    assert_eq!((r["background"].as_bool(), r["format"].as_str()), (Some(true), Some("vectorcraft")), "{r}");
    assert_eq!(r["warnings"], json!([]), "{r}");
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/out/a.svg", "/out/b.vectorcraft"]);
}

#[test]
fn a_cancelled_export_writes_nothing() {
    let (gate, writing) = (Gate::default(), Gate::default());
    let files = Files::default();
    let (f, g, w) = (files.clone(), gate.clone(), writing.clone());
    let services = Services {
        write_shared: Some(Arc::new(move |p: &str, b: &[u8]| {
            w.open();
            g.wait();
            f.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    // The first export holds the worker at the gate, past its last check: it still writes.
    io::export(&mut app, Some("svg"), Some("/out/first.svg".into()), &json!({})).unwrap();
    writing.wait();
    io::export(&mut app, Some("png"), Some("/out/second.png".into()), &json!({})).unwrap();
    assert_eq!(app.run("file.cancelExport", json!({})).unwrap()["cancelled"], 2);
    assert_eq!(app.run("file.cancelExport", json!({})).unwrap()["cancelled"], 0, "nothing left to cancel");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/out/first.svg"]);
    assert_eq!(app.ui.status, io::EXPORT_CANCELLED);
}

#[test]
fn quitting_waits_for_background_saves() {
    let gate = Gate::default();
    let (mut app, files) = app(gate.clone());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.save", json!({"path": "/docs/q.vectorcraft"})).unwrap();
    let opener = gate.clone();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(50));
        opener.open();
    });
    // Without waiting, the document would still count as modified and Quit would ask about it.
    let r: Value = app.run("app.quit", json!({})).unwrap();
    release.join().unwrap();
    assert!(r.is_null(), "{r}");
    assert_eq!(app.ui.status, "quit");
    assert_eq!(written(&files), ["/docs/q.vectorcraft"]);
}

/// The status bar's texts in one headless frame.
pub(super) fn status_bar_texts(app: &mut VectorcraftApp) -> Vec<String> {
    fn texts(s: &egui::Shape, out: &mut Vec<String>) {
        match s {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(v) => v.iter().for_each(|s| texts(s, out)),
            _ => {}
        }
    }
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| crate::chrome::status_bar(app, ui));
    out.textures_delta.clear();
    let mut v = vec![];
    out.shapes.iter().for_each(|c| texts(&c.shape, &mut v));
    v
}

#[test]
fn the_status_bar_shows_progress() {
    let gate = Gate::default();
    let (mut app, _) = app(gate.clone());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("file.save", json!({"path": "/docs/p.vectorcraft"})).unwrap();
    io::export(&mut app, Some("svg"), Some("/out/p.svg".into()), &json!({})).unwrap();
    let busy = status_bar_texts(&mut app);
    assert!(busy.iter().any(|t| t == "Saving p.vectorcraft… (+1)"), "{busy:?}");
    gate.open();
    background::wait_all(&mut app);
    let done = status_bar_texts(&mut app);
    assert!(done.iter().any(|t| t == "Exported /out/p.svg") && !done.iter().any(|t| t.contains('…')), "{done:?}");
}

/// An app with a control channel: its UI-thread writer records files (or fails with `fail`), and
/// its thread-safe writer does the same once `gate` opens.
fn controlled(gate: Gate, fail: Option<&'static str>) -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>, Files) {
    controlled_with(gate, fail, None)
}

/// [`controlled`], with `wrote` told each time the UI-thread writer has recorded a file.
fn controlled_with(
    gate: Gate,
    fail: Option<&'static str>,
    wrote: Option<std::sync::mpsc::Sender<()>>,
) -> (VectorcraftApp, std::sync::mpsc::Sender<ControlRequest>, Files) {
    let files = Files::default();
    let (f, g) = (files.clone(), files.clone());
    let services = Services {
        write: Some(Box::new(move |p: &str, b: &[u8]| match fail {
            Some(e) => Err(e.into()),
            None => {
                f.lock().unwrap().push((p.to_string(), b.to_vec()));
                if let Some(w) = &wrote {
                    let _ = w.send(());
                }
                Ok(())
            }
        })),
        write_shared: Some(Arc::new(move |p: &str, b: &[u8]| {
            gate.wait();
            if let Some(e) = fail {
                return Err(e.into());
            }
            g.lock().unwrap().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let (tx, rx) = std::sync::mpsc::channel();
    let mut app = VectorcraftApp::new(Session::new(), services).with_control(rx);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})).unwrap();
    (app, tx, files)
}

/// One request through the control channel, as the app's frame drains it.
fn request(app: &mut VectorcraftApp, tx: &std::sync::mpsc::Sender<ControlRequest>, method: &str, params: Value) -> Value {
    let (req, reply) = ControlRequest::new(method, params);
    tx.send(req).unwrap();
    app.drain_control(&egui::Context::default());
    reply.try_recv().unwrap()
}

/// `app.save` and `app.export` with `wait: true` (MCP's `save_file` in the app) write the file before
/// they reply, with Background Save and Background Export on; without it they still reply at once.
#[test]
fn control_saves_and_exports_with_wait_reply_once_written() {
    let gate = Gate::default();
    let (mut app, tx, files) = controlled(gate.clone(), None);
    let r = request(&mut app, &tx, "app.save", json!({"path": "/docs/w.vectorcraft", "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["result"].get("background").is_none() && r["result"]["path"] == "/docs/w.vectorcraft", "{r}");
    assert_eq!(written(&files), ["/docs/w.vectorcraft"], "written before the reply");
    let st = app.session.doc().unwrap();
    assert!(!st.is_dirty() && st.path.as_deref() == Some("/docs/w.vectorcraft"));
    assert_eq!(app.ui.status, "Saved /docs/w.vectorcraft");
    let r = request(&mut app, &tx, "app.export", json!({"path": "/out/w.svg", "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(r["result"].get("background").is_none() && r["result"]["path"] == "/out/w.svg", "{r}");
    assert_eq!(written(&files), ["/docs/w.vectorcraft", "/out/w.svg"]);
    assert!(app.background.jobs.is_empty());

    for (method, path) in [("app.save", "/docs/later.vectorcraft"), ("app.export", "/out/later.svg")] {
        let r = request(&mut app, &tx, method, json!({"path": path, "wait": false}));
        assert_eq!(r["result"]["background"], true, "{method}: {r}");
    }
    assert_eq!(written(&files).len(), 2, "the gate holds the worker");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/docs/w.vectorcraft", "/out/w.svg", "/docs/later.vectorcraft", "/out/later.svg"]);
}

/// An export with `wait: true` to the file a background export is still writing replies after both
/// are written, so the older one doesn't write over it afterwards.
#[test]
fn control_exports_with_wait_follow_the_jobs_already_running() {
    let gate = Gate::default();
    let (wrote, waiting_wrote) = std::sync::mpsc::channel();
    let (mut app, tx, files) = controlled_with(gate.clone(), None, Some(wrote));
    let r = request(&mut app, &tx, "app.export", json!({"path": "/out/same.svg"}));
    assert_eq!(r["result"]["background"], true, "{r}");
    // The waiting export draws one more rectangle than the background export's snapshot.
    app.run("shape.rectangle", json!({"x": 50, "y": 50, "width": 30, "height": 30})).unwrap();
    // The background export is let go as soon as the waiting export has written its file, or after
    // a second while the waiting export waits for it.
    let opener = gate.clone();
    let release = std::thread::spawn(move || {
        let _ = waiting_wrote.recv_timeout(std::time::Duration::from_secs(1));
        opener.open();
    });
    let r = request(&mut app, &tx, "app.export", json!({"path": "/out/same.svg", "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert!(app.background.jobs.is_empty(), "the background export finished before the reply");
    release.join().unwrap();
    background::wait_all(&mut app);
    let both = files.lock().unwrap().clone();
    assert_eq!(written(&files), ["/out/same.svg", "/out/same.svg"]);
    assert_ne!(both[0].1, both[1].1, "the two exports draw different art");
    let r = request(&mut app, &tx, "app.export", json!({"path": "/out/check.svg", "wait": true}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(files.lock().unwrap()[2].1, both[1].1, "the waiting export wrote last");
}

/// A panic during a save with `wait: true`, caught further up, leaves Background Save on.
#[test]
fn a_panic_while_waiting_leaves_background_saves_on() {
    let gate = Gate::default();
    let (mut app, files) = app(gate.clone());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| background::in_foreground(&mut app, |_| panic!("an encoder bug"))));
    assert!(r.is_err());
    let r = app.run("file.save", json!({"path": "/docs/after.vectorcraft"})).unwrap();
    assert_eq!(r["background"], true, "{r}");
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/docs/after.vectorcraft"]);
}

/// A save or export with `wait: true` that fails replies with the error.
#[test]
fn control_saves_and_exports_with_wait_reply_with_the_error() {
    let gate = Gate::default();
    gate.open();
    let (mut app, tx, files) = controlled(gate, Some("disk full"));
    for (method, path) in [("app.save", "/docs/f.vectorcraft"), ("app.export", "/out/f.svg")] {
        let r = request(&mut app, &tx, method, json!({"path": path, "wait": true}));
        assert_eq!(r["ok"], false, "{method}: {r}");
        assert!(r["error"].as_str().unwrap().contains("disk full"), "{method}: {r}");
    }
    assert!(app.session.doc().unwrap().path.is_none(), "the document keeps no file");
    // Without it the reply comes first, and the failure only reaches the status bar.
    let r = request(&mut app, &tx, "app.save", json!({"path": "/docs/f.vectorcraft"}));
    assert_eq!(r["result"]["background"], true, "{r}");
    background::wait_all(&mut app);
    assert!(written(&files).is_empty());
}

/// A `wait` that is not true or false is refused before anything is saved or exported; null is the
/// same as leaving it out.
#[test]
fn control_saves_and_exports_refuse_a_wait_that_is_not_a_boolean() {
    let gate = Gate::default();
    let (mut app, tx, files) = controlled(gate.clone(), None);
    for (method, path) in [("app.save", "/docs/y.vectorcraft"), ("app.export", "/out/y.svg")] {
        for wait in [json!("yes"), json!(1), json!({})] {
            let r = request(&mut app, &tx, method, json!({"path": path, "wait": wait}));
            assert_eq!(r["ok"], false, "{method} {wait}: {r}");
            assert_eq!(r["error"], "`wait` must be true or false", "{method} {wait}: {r}");
        }
    }
    assert!(app.background.jobs.is_empty() && written(&files).is_empty(), "nothing saved or exported");
    assert!(app.session.doc().unwrap().path.is_none());
    for (method, path) in [("app.save", "/docs/n.vectorcraft"), ("app.export", "/out/n.svg")] {
        let r = request(&mut app, &tx, method, json!({"path": path, "wait": null}));
        assert_eq!(r["result"]["background"], true, "{method}: {r}");
    }
    gate.open();
    background::wait_all(&mut app);
    assert_eq!(written(&files), ["/docs/n.vectorcraft", "/out/n.svg"]);
}

#[test]
fn bad_options_fail_before_going_to_the_background() {
    let (mut app, files) = app(Gate::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    for bad in [json!({"path": "/docs/v.vectorcraft", "version": 9}), json!({"path": "/docs/v.vectorcraft", "version": 1, "compress": true})] {
        assert!(app.run("file.save", bad.clone()).is_err(), "{bad}");
    }
    assert!(app.background.jobs.is_empty() && written(&files).is_empty());
}
