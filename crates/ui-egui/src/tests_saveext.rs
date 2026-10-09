//! A name typed in a save panel without an extension gets the chosen format's: Save, Save As,
//! Save a Copy, Export, Export Selection and other writes through a save panel.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_engine::Session;

use crate::{FilePick, Services, VectorcraftApp, io};

type Written = Rc<RefCell<Vec<String>>>;

/// An app with a document whose save panel answers `typed` and whose writer records the paths.
fn saving_app(typed: &'static str) -> (VectorcraftApp, Written) {
    let written = Written::default();
    let w = written.clone();
    let services = Services {
        write: Some(Box::new(move |p: &str, _: &[u8]| {
            w.borrow_mut().push(p.to_string());
            Ok(())
        })),
        pick_save: Some(Box::new(move |_: &FilePick| Some(typed.to_string()))),
        ..Default::default()
    };
    let mut app = VectorcraftApp::new(Session::new(), services);
    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 30, "height": 20})).unwrap();
    (app, written)
}

fn last(w: &Written) -> String {
    w.borrow().last().cloned().unwrap_or_default()
}

#[test]
fn saves_add_the_format_extension_to_a_bare_name() {
    let (mut app, w) = saving_app("/tmp/Untitled-1");
    app.run("file.save", json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/Untitled-1.ai");
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/tmp/Untitled-1.ai"));
    let (mut app, w) = saving_app("/tmp/copy");
    app.run("file.saveCopy", json!({"format": "svg", "svg": {}})).unwrap();
    assert_eq!(last(&w), "/tmp/copy.svg");
    let (mut app, w) = saving_app("/tmp/as");
    app.run("file.saveAs", json!({"format": "pdf", "options": {}})).unwrap();
    assert_eq!(last(&w), "/tmp/as.pdf");
}

#[test]
fn exports_add_the_format_extension_to_a_bare_name() {
    let (mut app, w) = saving_app("/tmp/art");
    io::export(&mut app, Some("png"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/art.png");
    app.run("select.all", json!({})).unwrap();
    app.run("document.exportSelection", json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/art.png", "Export Selection: its default format");
    io::save_command_output(&mut app, "document.export", "txt", json!({"format": "txt"})).unwrap();
    assert_eq!(last(&w), "/tmp/art.txt");
}

#[test]
fn typed_extensions_are_kept() {
    let (mut app, w) = saving_app("/tmp/art.svg");
    io::export(&mut app, Some("svg"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/art.svg");
    let (mut app, w) = saving_app("/tmp/My.Poster.SVG");
    io::export(&mut app, Some("svg"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/My.Poster.SVG");
    // A dot in the name isn't an extension.
    let (mut app, w) = saving_app("/tmp/v1.2");
    io::export(&mut app, Some("png"), None, &json!({})).unwrap();
    assert_eq!(last(&w), "/tmp/v1.2.png");
}
