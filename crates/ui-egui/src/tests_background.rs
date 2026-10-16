//! Background Save and Background Export: the file is written from a snapshot off the UI thread,
//! and the document counts as saved as of that snapshot.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, Node};
use vectorcraft_engine::Session;
use vectorcraft_geom::{Rect, shapes};

use crate::{Services, VectorcraftApp, background, io};

type Files = Arc<Mutex<Vec<(String, Vec<u8>)>>>;

/// Holds the background writer until opened.
#[derive(Clone, Default)]
struct Gate(Arc<AtomicBool>);

impl Gate {
    fn open(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    fn wait(&self) {
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

fn written(files: &Files) -> Vec<String> {
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

#[test]
fn bad_options_fail_before_going_to_the_background() {
    let (mut app, files) = app(Gate::default());
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    for bad in [json!({"path": "/docs/v.vectorcraft", "version": 9}), json!({"path": "/docs/v.vectorcraft", "version": 1, "compress": true})] {
        assert!(app.run("file.save", bad.clone()).is_err(), "{bad}");
    }
    assert!(app.background.jobs.is_empty() && written(&files).is_empty());
}
