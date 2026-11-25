//! SVGZ in the app: Place takes it, and a .svgz name (Export As SVG, Save As) writes it.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::{Host, Picked, ScriptedFiles};
use serde_json::{Value, json};
use vectorcraft_engine::Session;

use super::nativeoptions::answer;
use super::picks::{open_dialogs, poll};
use crate::{Services, VectorcraftApp, io};

pub(crate) type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The most save panels one test answers.
const SAVES: usize = 32;

/// An app with a window and a rectangle whose save panels answer `picked`; the writer records its files.
pub(crate) fn app(picked: &str) -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    let ctx = crate::testing::ui_context();
    app.ui_ctx = Some(ctx.clone());
    let saved = Picked::Saved { path: PathBuf::from(picked), file_type: None };
    app.host = Arc::new(app.host.with_files(Arc::new(ScriptedFiles::answering(std::iter::repeat_n(saved, SAVES)))));
    Host::clone(&app.host).install(&ctx);
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    (app, written)
}

/// Runs what a frame runs for an answered save panel: its continuation, then the save.
pub(crate) fn picked(app: &mut VectorcraftApp) {
    let ctx = app.ui_ctx.clone().expect("a window");
    poll(app, &ctx);
}

/// Answers each step a save or an export leaves open, in whatever order it asks: the save panel
/// with the path [`app`] picks and each options dialog with OK.
pub(crate) fn finish(app: &mut VectorcraftApp) {
    for _ in 0..4 {
        picked(app);
        if open_dialogs(app).is_empty() {
            return;
        }
        answer(app, &[], true);
    }
}

#[test]
fn svgz_places_like_svg() {
    let (mut app, _) = app("/tmp/unused.svg");
    let svg = app.session.execute("document.serialize", &json!({"format": "svg"})).unwrap()["text"].as_str().unwrap().to_string();
    app.run("file.new", json!({"width": 100, "height": 80})).unwrap();
    io::place_bytes(&mut app, "art.svgz", &vectorcraft_svg::compress(&svg)).unwrap();
    assert_eq!(app.session.doc().unwrap().selection.objects.len(), 1, "the rectangle was placed");
}

#[test]
fn a_svgz_name_writes_compressed_svg() {
    let (mut app, written) = app("/tmp/art.svgz");
    // Export As SVG, named .svgz in the save panel.
    app.run("file.export.svg", Value::Null).unwrap();
    finish(&mut app);
    let (path, bytes) = written.borrow()[0].clone();
    assert_eq!(path, "/tmp/art.svgz");
    assert!(vectorcraft_svg::text_of(&bytes).unwrap().contains("<style>"), "the SVG Options apply");
    assert!(vectorcraft_svg::is_svgz(&bytes));
    // Save As .svgz asks for SVG Options, then saves compressed and keeps the path.
    assert_eq!(app.run("file.saveAs", Value::Null).unwrap()["pending"], "fileDialog");
    picked(&mut app);
    assert_eq!(open_dialogs(&app).len(), 1, "SVG Options");
    answer(&mut app, &[], true);
    assert!(vectorcraft_svg::is_svgz(&written.borrow()[1].1));
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/art.svgz"));
}
