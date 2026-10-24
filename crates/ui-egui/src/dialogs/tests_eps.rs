//! Export As → EPS Options, drawn headlessly.

use serde_json::json;

use super::tests_export::{app, frame, kind, names, set};
use super::*;

/// The PostScript of an EPS file.
fn ps(bytes: &[u8]) -> String {
    String::from_utf8(vectorcraft_eps::sections(bytes).unwrap().0.to_vec()).unwrap()
}

#[test]
fn export_as_eps_shows_eps_options_then_writes_and_remembers_them() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "eps"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("epsOptions"), "EPS Options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!(
        (d.str("path"), d.str("previewFormat"), d.str("flattenerPreset")),
        ("/out/Untitled-1.eps".into(), "tiffColor".into(), "Medium Resolution".into())
    );
    frame(&mut app);
    // Every control draws: the black-and-white preview greys out Transparent / Opaque.
    set(&mut app, "previewFormat", json!("tiffBw"));
    set(&mut app, "level", json!(2));
    set(&mut app, "cmykPostScript", json!(false));
    frame(&mut app);
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("level"), "2", "a number reads as the level");
    set(&mut app, "flattenerPreset", json!("High Resolution"));
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    // The central file, then one file per artboard, named with an underscore.
    assert_eq!(names(&written), ["/out/Untitled-1.eps", "/out/Untitled-1_Artboard-1.eps", "/out/Untitled-1_Artboard-2.eps"]);
    assert_eq!(r["files"].as_array().map(Vec::len), Some(3));
    let bytes = &written.borrow()[0].1;
    let (text, tiff) = vectorcraft_eps::sections(bytes).unwrap();
    assert!(tiff.is_some() && String::from_utf8_lossy(text).contains("%%LanguageLevel: 2"));
    assert!(ps(bytes).contains(" rg\n"), "RGB PostScript");
    // The next EPS Options start from these choices, but not from this export's path.
    crate::menus::invoke(&mut app, "ui.epsOptionsDialog", json!({}));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("previewFormat"), d.str("level"), d.str("flattenerPreset")), ("tiffBw".into(), "2".into(), "High Resolution".into()));
    assert!(!d.bool("cmykPostScript") && !d.fields.contains_key("path"));
    assert!(app.ui.eps_options.get("useArtboards").is_none());
}

#[test]
fn eps_options_take_saved_flattener_presets_and_refuse_bad_ones() {
    let (mut app, written) = app(1);
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 0})).unwrap();
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "flattenerPreset": "medium"})).unwrap();
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("flattenerPreset"), "Medium Resolution", "an id shows as its name");
    frame(&mut app);
    set(&mut app, "flattenerPreset", json!("Nope"));
    let e = confirm(&mut app).unwrap_err();
    assert!(e.contains("preset"), "{e}");
    assert_eq!(kind(&app), Some("epsOptions"));
    set(&mut app, "flattenerPreset", json!("Mine"));
    set(&mut app, "previewFormat", json!("none"));
    confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/out/a.eps"]);
    assert!(written.borrow()[0].1.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"), "{:?}", &written.borrow()[0].1[..8]);
}

#[test]
fn the_eps_options_dialog_needs_a_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(app.run("ui.epsOptionsDialog", json!({})).is_err());
    assert!(!crate::menus::enabled(&app, "ui.epsOptionsDialog"));
}

/// Opening an EPS file whose PostScript reads only partly says why, in the status bar and in
/// `file.open`'s result (what `app.open` and MCP's `open_file` return): the error, the operator
/// and the procedure it ran in.
#[test]
fn opening_an_eps_that_reads_partly_says_why() {
    let dir = std::env::temp_dir().join(format!("vc-eps-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("partial.eps");
    std::fs::write(&path, "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n0 0 10 10 rectfill /p { frobnicate } def p\n%%EOF\n")
        .unwrap();
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    app.services.read = Some(Box::new(|p: &str| std::fs::read(p).map_err(|e| e.to_string())));
    let r = app.run("file.open", json!({"path": path.to_string_lossy()})).unwrap();
    let w = r["warnings"][0].as_str().unwrap();
    assert!(w.contains("`frobnicate`") && w.contains("in `p`"), "{r}");
    assert!(app.ui.status.contains("Opened with 1 note(s)") && app.ui.status.contains("frobnicate"), "{}", app.ui.status);
    std::fs::remove_dir_all(dir).ok();
}
