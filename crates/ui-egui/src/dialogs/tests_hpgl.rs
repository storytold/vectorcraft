//! Export As → HP-GL, drawn headlessly: listed with the other formats, written straight away (no
//! options dialog), one plot per artboard with Use Artboards.

use serde_json::json;

use super::tests_export::{app, frame, kind, names, set};
use super::*;

#[test]
fn export_as_lists_hpgl_and_writes_plots() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("format"), "hpgl");
    frame(&mut app);
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none(), "no options dialog follows");
    assert_eq!(r["path"], "/out/Untitled-1.plt");
    assert!(written.borrow()[0].1.starts_with(b"IN;"));
    assert!(written.borrow()[0].1.ends_with(b"PU;SP0;"));
    // One plot per artboard.
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    set(&mut app, "useArtboards", json!(true));
    frame(&mut app);
    confirm(&mut app).unwrap();
    let n = names(&written);
    assert_eq!(&n[1..], ["/out/Untitled-1-Artboard-1.plt", "/out/Untitled-1-Artboard-2.plt"]);
}
