//! A font file opened or dropped (File › Open, a drop, the web page's `openFile`, all through
//! `io::open_bytes`) is added to the session (`text.addFonts`) instead of opening as a document:
//! type set in it is drawn in it and the status bar names the faces added; a web font or a file
//! that is no font is refused with why. One test in its own process, so the process-wide font
//! database starts fresh, as when the app starts.
// Integration tests: unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use serde_json::json;
use vectorcraft_engine::Session;
use vectorcraft_ui_egui::{VectorcraftApp, io};

const FAMILY: &str = "Openfont Sans";

/// `text.fonts`' status for [`FAMILY`].
fn status(app: &mut VectorcraftApp) -> String {
    let listed = app.run("text.fonts", json!({})).unwrap();
    let row = listed.as_array().unwrap().iter().find(|f| f["family"] == FAMILY).cloned().unwrap();
    row["status"].as_str().unwrap().into()
}

#[test]
fn an_opened_font_file_joins_the_session() {
    let font = vectorcraft_testkit::fonts::renamed(FAMILY);
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 300, "height": 200})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 40, "text": "Open", "font": FAMILY})).unwrap();
    assert_eq!(status(&mut app), "missing");
    let docs = app.session.documents().len();

    assert!(io::open_bytes(&mut app, "Openfont.ttf", &font, None).unwrap().is_null());
    assert_eq!(app.session.documents().len(), docs, "no document opens");
    assert_eq!(app.ui.status, format!("Added the font {FAMILY} Regular"));
    assert_eq!(status(&mut app), "exact");

    // Again: the session has it already.
    io::open_bytes(&mut app, "Openfont.ttf", &font, None).unwrap();
    assert_eq!(app.ui.status, format!("The font {FAMILY} Regular is loaded already"));

    // A web font, or a file named as a font that is none, says why it isn't added.
    let mut woff2 = b"wOF2".to_vec();
    woff2.extend_from_slice(&[0; 44]);
    let e = io::open_bytes(&mut app, "Openfont.woff2", &woff2, None).unwrap_err();
    assert!(e.contains("WOFF"), "{e}");
    assert_eq!(io::open_bytes(&mut app, "notes.otf", b"not a font", None).unwrap_err(), "not a font file");
    // A file over the limit is refused before it is encoded (untouched zeroed pages: no memory used).
    let huge = vec![0u8; (256 << 20) + 1];
    assert_eq!(io::open_bytes(&mut app, "huge.ttf", &huge, None).unwrap_err(), "larger than 256 MB");
    assert_eq!(app.session.documents().len(), docs);
}
