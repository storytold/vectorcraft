//! Export As → EPS Options, drawn headlessly.

use craft::ui::patterns::form_dialogs::DialogFields;
use serde_json::json;

use super::tests_export::{app, cancel, confirm, frame, kind, names, set, try_confirm, try_set};
use super::tests_print::press;
use super::tests_raster_formats::fields;
use super::*;
use crate::VectorcraftApp;

/// The PostScript of an EPS file.
fn ps(bytes: &[u8]) -> String {
    String::from_utf8(vectorcraft_eps::sections(bytes).unwrap().0.to_vec()).unwrap()
}

/// A text value of the open dialog.
fn text(d: &DialogFields, key: &str) -> String {
    d.text(key).unwrap_or_default().to_string()
}

/// The custom flattener options an EPS Options dialog holds.
fn flattener(d: &DialogFields) -> serde_json::Value {
    d.get("flattener").and_then(|v| serde_json::to_value(v).ok()).unwrap_or_default()
}

/// A dialog's values as JSON, to compare two states.
fn as_json(d: &DialogFields) -> serde_json::Value {
    serde_json::to_value(d).unwrap_or_default()
}

#[test]
fn export_as_eps_shows_eps_options_then_writes_and_remembers_them() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "eps"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("epsOptions"), "EPS Options follow");
    let d = fields(&app);
    assert_eq!(
        (text(&d, "path"), text(&d, "previewFormat"), text(&d, "flattenerPreset")),
        ("/out/Untitled-1.eps".into(), "tiffColor".into(), "Medium Resolution".into())
    );
    frame(&mut app);
    // Every control draws: the black-and-white preview greys out Transparent / Opaque.
    set(&mut app, "previewFormat", json!("tiffBw"));
    set(&mut app, "level", json!("2"));
    set(&mut app, "cmykPostScript", json!(false));
    frame(&mut app);
    assert_eq!(text(&fields(&app), "level"), "2", "the level's id");
    set(&mut app, "flattenerPreset", json!("High Resolution"));
    confirm(&mut app);
    assert!(kind(&app).is_none());
    // The central file, then one file per artboard, named with an underscore.
    assert_eq!(names(&written), ["/out/Untitled-1.eps", "/out/Untitled-1_Artboard-1.eps", "/out/Untitled-1_Artboard-2.eps"]);
    let bytes = &written.borrow()[0].1;
    let (text_part, tiff) = vectorcraft_eps::sections(bytes).unwrap();
    assert!(tiff.is_some() && String::from_utf8_lossy(text_part).contains("%%LanguageLevel: 2"));
    assert!(ps(bytes).contains(" rg\n"), "RGB PostScript");
    // The next EPS Options start from these choices, but not from this export's path.
    crate::menus::invoke(&mut app, "ui.epsOptionsDialog", json!({}));
    let d = fields(&app);
    assert_eq!((text(&d, "previewFormat"), text(&d, "level"), text(&d, "flattenerPreset")), ("tiffBw".into(), "2".into(), "High Resolution".into()));
    assert!(d.bool("cmykPostScript") != Some(true) && !d.contains("path"));
    assert!(app.ui.eps_options.get("useArtboards").is_none());
}

#[test]
fn eps_options_take_saved_flattener_presets_and_refuse_bad_ones() {
    let (mut app, written) = app(1);
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 0})).unwrap();
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "flattenerPreset": "medium"})).unwrap();
    assert_eq!(text(&fields(&app), "flattenerPreset"), "Medium Resolution", "an id shows as its name");
    frame(&mut app);
    set(&mut app, "flattenerPreset", json!("Nope"));
    let e = try_confirm(&mut app).unwrap_err();
    assert!(e.contains("preset"), "{e}");
    assert_eq!(kind(&app).as_deref(), Some("epsOptions"));
    set(&mut app, "flattenerPreset", json!("Mine"));
    set(&mut app, "previewFormat", json!("none"));
    confirm(&mut app);
    assert_eq!(names(&written), ["/out/a.eps"]);
    assert!(written.borrow()[0].1.starts_with(b"%!PS-Adobe-3.0 EPSF-3.0"), "{:?}", &written.borrow()[0].1[..8]);
}

/// Two overlapping squares, the second half transparent, so that an EPS export flattens them.
fn transparent_art(app: &mut VectorcraftApp) {
    for (x, color) in [(5, "#ff0000"), (20, "#0000ff")] {
        app.run("select.none", json!({})).unwrap();
        app.run("paint.setFill", json!({"color": color})).unwrap();
        app.run("shape.rectangle", json!({"x": x, "y": 5, "width": 30, "height": 30})).unwrap();
    }
    app.run("transparency.set", json!({"opacity": 50})).unwrap();
}

/// Custom… over EPS Options, pressed as the user presses it.
fn open_custom(app: &mut VectorcraftApp) {
    press(app, "Custom…");
    frame(app);
}

#[test]
fn custom_flattener_options_start_from_the_preset_and_cancel_goes_back_unchanged() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "flattenerPreset": "high"})).unwrap();
    frame(&mut app);
    let before = as_json(&fields(&app));
    open_custom(&mut app);
    assert_eq!(kind(&app).as_deref(), Some(eps_options::CUSTOM_KIND));
    let d = fields(&app);
    assert_eq!(
        (d.number("balance"), d.number("lineArtPpi"), d.bool("clipComplexRegions")),
        (Some(100.0), Some(1200.0), Some(true)),
        "High Resolution's"
    );
    assert_ne!(d.bool("preserveAlpha"), Some(true), "EPS has no alpha");
    set(&mut app, "balance", json!(10));
    cancel(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("epsOptions"));
    assert_eq!(as_json(&fields(&app)), before, "Cancel leaves EPS Options as they were");
}

#[test]
fn custom_flattener_options_are_exported_and_remembered_until_a_preset_is_chosen() {
    let (mut app, written) = app(1);
    transparent_art(&mut app);
    app.run("flattener.presets.save", json!({"name": "Mine", "balance": 100})).unwrap();
    let opts = json!({"path": "/out/a.eps", "flattenerPreset": "Mine", "previewFormat": "none", "thumbnails": false});
    app.run("ui.epsOptionsDialog", opts).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    assert_eq!(fields(&app).number("balance"), Some(100.0), "the saved preset's");
    set(&mut app, "balance", json!(0));
    confirm(&mut app);
    frame(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("epsOptions"), "OK goes back to EPS Options");
    let d = fields(&app);
    let custom = flattener(&d);
    assert_eq!((custom["balance"].as_f64(), custom["preserveAlpha"].as_bool()), (Some(0.0), Some(false)));
    assert_eq!(text(&d, "flattenerPreset"), "Medium Resolution", "a built-in preset under options that set everything");
    // The custom options don't need the saved preset they started from.
    app.run("flattener.presets.delete", json!({"name": "Mine"})).unwrap();
    confirm(&mut app);
    assert!(ps(&written.borrow()[0].1).contains("image"), "balance 0 rasterizes the flattened art");
    // The next EPS Options start from them; a preset asked for without options drops them.
    crate::menus::invoke(&mut app, "ui.epsOptionsDialog", json!({}));
    assert_eq!(flattener(&fields(&app))["balance"].as_f64(), Some(0.0));
    cancel(&mut app);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/b.eps", "flattenerPreset": "medium"})).unwrap();
    assert!(!fields(&app).contains("flattener"));
    confirm(&mut app);
    assert!(!ps(&written.borrow()[1].1).contains("image"), "Medium Resolution keeps the squares vector");
}

#[test]
fn a_flattener_preset_set_from_outside_drops_the_custom_options() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps"})).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    set(&mut app, "balance", json!(20));
    confirm(&mut app);
    frame(&mut app);
    assert!(fields(&app).contains("flattener"));
    // `ui.dialog.set {flattenerPreset: "low"}`, as the control channel sets it.
    set(&mut app, "flattenerPreset", json!("low"));
    frame(&mut app);
    let d = fields(&app);
    assert!(!d.contains("flattener"), "the preset replaces the custom options");
    assert_eq!(text(&d, "flattenerPreset"), "Low Resolution");
}

#[test]
fn bad_custom_flattener_options_keep_the_dialog_open() {
    let (mut app, _) = app(1);
    app.run("ui.epsOptionsDialog", json!({"path": "/out/a.eps", "overprints": "discard"})).unwrap();
    frame(&mut app);
    open_custom(&mut app);
    // A resolution out of range is refused as it is set (the framework's `FieldRefusal::Rejects`); the dialog stays.
    let e = try_set(&mut app, "gradientPpi", json!(9000)).unwrap_err();
    assert!(e.contains("gradientPpi"), "{e}");
    frame(&mut app);
    assert_eq!(kind(&app).as_deref(), Some(eps_options::CUSTOM_KIND));
    // Overprints decides whether overprints are kept, so OK keeps the option as it was.
    set(&mut app, "gradientPpi", json!(150));
    // Its row is greyed out, so it refuses the change (pl.F4).
    assert!(try_set(&mut app, "preserveAlpha", json!(true)).is_err());
    confirm(&mut app);
    frame(&mut app);
    let custom = flattener(&fields(&app));
    assert_eq!((custom["preserveOverprints"].as_bool(), custom["preserveAlpha"].as_bool()), (Some(true), Some(false)));
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
    assert!(app.ui.status.contains("Opened with 1 note:") && app.ui.status.contains("frobnicate"), "{}", app.ui.status);
    std::fs::remove_dir_all(dir).ok();
}
