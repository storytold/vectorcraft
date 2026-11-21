//! Export As → EMF and WMF, drawn headlessly: listed with the other formats, written straight
//! away (no options dialog), one file per artboard with Use Artboards.

use serde_json::json;

use super::tests_export::{app, confirm, fields, frame, kind, names, set};

/// Does `b` carry the EMF header's signature, or start with the placeable WMF key?
fn is_emf(b: &[u8]) -> bool {
    b.get(40..44) == Some(&b" EMF"[..])
}

fn is_wmf(b: &[u8]) -> bool {
    b.get(..4) == Some(&[0xD7, 0xCD, 0xC6, 0x9A][..])
}

#[test]
fn export_as_lists_emf_and_wmf_and_writes_them() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "emf"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(fields(&app)["format"], "emf");
    frame(&mut app);
    confirm(&mut app);
    assert!(kind(&app).is_none(), "no options dialog follows");
    assert_eq!(names(&written), ["/out/Untitled-1.emf"]);
    assert!(is_emf(&written.borrow()[0].1));
    // WMF, one file per artboard.
    app.run("file.exportAs", json!({"format": "wmf"})).unwrap();
    set(&mut app, "useArtboards", json!(true));
    frame(&mut app);
    confirm(&mut app);
    assert!(kind(&app).is_none());
    let n = names(&written);
    assert_eq!(&n[1..], ["/out/Untitled-1-Artboard-1.wmf", "/out/Untitled-1-Artboard-2.wmf"]);
    assert!(written.borrow()[1..].iter().all(|(_, b)| is_wmf(b)));
}
