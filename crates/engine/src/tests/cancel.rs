//! Cancelling a long export (#2165): it stops at the next checkpoint with `EngineError::Cancelled`,
//! writes nothing, leaves the document as it was, and the session goes on working.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::json;

use crate::cancel::{self, CancelToken, ProgressSink};
use crate::{EngineError, Session};

fn session(artboards: usize) -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 80, "artboards": artboards})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    s
}

fn revision(s: &Session) -> u64 {
    s.doc().unwrap().revision
}

#[test]
fn a_pdf_export_cancelled_between_pages_stops_writes_nothing_and_changes_nothing() {
    let path = vectorcraft_testkit::temp_dir("cancel-pdf").join("out.pdf");
    std::fs::remove_file(&path).ok();
    let mut s = session(5);
    let before = (revision(&s), s.doc().unwrap().history.undo.len());
    let token = CancelToken::new();
    let pages = Rc::new(Cell::new(0));
    // The sink runs inside the export, as the page loop reports each page: cancel on the third.
    let sink: ProgressSink = {
        let (token, pages) = (token.clone(), pages.clone());
        Rc::new(move |done, total, _| {
            // Stages before the pages report without a total.
            if total != 5 {
                return;
            }
            pages.set(done);
            if done == 3 {
                token.cancel();
            }
        })
    };
    let params = json!({"path": path.to_str().unwrap()});
    let r = cancel::scope(&token, Some(sink), || s.execute("document.exportPdf", &params));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!(pages.get(), 3, "the export stopped at the checkpoint after the cancel, not at the end");
    assert!(!path.exists(), "a cancelled export writes no file");
    assert_eq!((revision(&s), s.doc().unwrap().history.undo.len()), before, "the document is as it was");
    // The session is fine: the same export, not cancelled, completes.
    s.execute("document.exportPdf", &params).unwrap();
    assert!(path.exists());
}

#[test]
fn a_cancelled_flattening_export_stops_before_the_pages() {
    let mut s = session(2);
    let token = CancelToken::new();
    token.cancel();
    // PDF/X-1a flattens transparency first; a token raised beforehand stops at the first check.
    let r = cancel::scope(&token, None, || s.execute("document.exportPdf", &json!({"standard": "pdfX1a"})));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert!(s.execute("document.exportPdf", &json!({"standard": "pdfX1a"})).is_ok());
}

#[test]
fn raster_exports_and_svg_stop_when_cancelled_too() {
    let dir = vectorcraft_testkit::temp_dir("cancel-formats");
    let mut s = session(3);
    let token = CancelToken::new();
    token.cancel();
    for ext in ["png", "svg", "pdf"] {
        let path = dir.join(format!("out.{ext}"));
        std::fs::remove_file(&path).ok();
        let params = json!({"path": path.to_str().unwrap()});
        let r = cancel::scope(&token, None, || s.execute("document.export", &params));
        assert!(matches!(r, Err(EngineError::Cancelled)), "{ext}: {r:?}");
        assert!(!path.exists(), "{ext}: nothing is written");
    }
}

#[test]
fn nothing_is_cancelled_outside_a_scope() {
    let token = CancelToken::new();
    token.cancel();
    // The token only counts for work run under it, on its thread.
    let mut s = session(1);
    assert!(s.execute("document.exportPdf", &json!({})).is_ok());
}
