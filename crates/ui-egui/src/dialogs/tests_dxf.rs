//! Export As → DXF Options, and DWG listed as unavailable, drawn headlessly.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use serde_json::json;

use super::tests_export::{app, cancel, confirm, frame, kind, names, set, try_confirm, try_set};
use super::tests_raster_formats::fields;
use crate::VectorcraftApp;

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// A value as the old tests compared it.
fn show(d: &DialogFields, key: &str) -> String {
    match d.get(key) {
        Some(FieldValue::Number(n)) => format!("{n:?}"),
        Some(FieldValue::Int(n)) => n.to_string(),
        Some(FieldValue::Bool(b)) => b.to_string(),
        // Text, also as automation sets a companion's: a JSON string.
        _ => d.text(key).unwrap_or_default().to_string(),
    }
}

/// The preset's fields as one line.
fn line(d: &DialogFields) -> String {
    ["dxfPreset", "version", "unit", "scale", "colors", "preserve", "alterPaths", "outlineText", "includeImages", "includeHatches"]
        .map(|k| show(d, k))
        .join(" ")
}

#[test]
fn export_as_dxf_shows_dxf_options_then_writes_and_remembers_them() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "dxf"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    frame(&mut app);
    set(&mut app, "useArtboards", json!(true));
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("dxfOptions"), "DXF Options follow");
    let d = fields(&app);
    assert_eq!((show(&d, "path"), show(&d, "version"), show(&d, "preserve")), ("/out/Untitled-1.dxf".into(), "2018".into(), "appearance".into()));
    frame(&mut app);
    // Every control draws: R12 with true colour shows its note, Maximize Editability frees Outline Text.
    set(&mut app, "version", json!("R12"));
    set(&mut app, "preserve", json!("editability"));
    set(&mut app, "unit", json!("Inches"));
    frame(&mut app);
    set(&mut app, "colors", json!("256"));
    confirm(&mut app);
    assert!(kind(&app).is_none());
    assert_eq!(names(&written), ["/out/Untitled-1.dxf"]);
    let dxf = text(&written.borrow()[0].1);
    assert!(dxf.contains("$ACADVER\n  1\nAC1009"), "{dxf}");
    // The next DXF Options start from these choices, but not from this export's path.
    crate::menus::invoke(&mut app, "ui.dxfOptionsDialog", json!({}));
    let d = fields(&app);
    assert_eq!((show(&d, "version"), show(&d, "unit"), show(&d, "colors")), ("R12".into(), "Inches".into(), "256".into()));
    assert!(!d.contains("path"));
    assert!(app.ui.dxf_options.get("useArtboards").is_none());
}

#[test]
fn the_cnc_cam_preset_sets_the_fields_and_a_change_shows_custom() {
    let (mut app, written) = app(1);
    app.run("ui.dxfOptionsDialog", json!({"path": "/out/a.dxf"})).unwrap();
    frame(&mut app);
    assert_eq!(show(&fields(&app), "dxfPreset"), "custom");
    set(&mut app, "dxfPreset", json!("cnc"));
    frame(&mut app);
    assert_eq!(line(&fields(&app)), "cnc 2000 Millimeters 1.0 256 editability false true false false");
    // Any change to a field the preset sets shows Custom, and keeps the other fields.
    set(&mut app, "colors", json!("true"));
    frame(&mut app);
    assert_eq!(line(&fields(&app)), "custom 2000 Millimeters 1.0 true editability false true false false");
    set(&mut app, "colors", json!("256"));
    frame(&mut app);
    assert_eq!(show(&fields(&app), "dxfPreset"), "cnc", "matching every field again");
    confirm(&mut app);
    let dxf = text(&written.borrow()[0].1);
    let lines: Vec<&str> = dxf.lines().collect();
    let header = |var: &str| lines.iter().position(|l| *l == var).and_then(|i| lines.get(i + 2)).map(|v| v.trim());
    assert_eq!((header("$ACADVER"), header("$INSUNITS")), (Some("AC1015"), Some("4")));
    // The next DXF Options start from the preset; Custom puts back the defaults.
    crate::menus::invoke(&mut app, "ui.dxfOptionsDialog", json!({}));
    assert_eq!(show(&fields(&app), "dxfPreset"), "cnc");
    set(&mut app, "dxfPreset", json!("custom"));
    frame(&mut app);
    assert_eq!(line(&fields(&app)), "custom 2018 Millimeters 1.0 true appearance false false true true");
    // A preset the command gives sets the fields it does not give.
    cancel(&mut app);
    app.run("ui.dxfOptionsDialog", json!({"dxfPreset": "cnc", "unit": "Inches"})).unwrap();
    assert_eq!(line(&fields(&app)), "custom 2000 Inches 1.0 256 editability false true false false");
    // A preset set and confirmed without drawing (the control channel) sets its fields too.
    set(&mut app, "dxfPreset", json!("cnc"));
    confirm(&mut app);
    assert_eq!((app.ui.dxf_options["dxfPreset"].as_str(), app.ui.dxf_options["unit"].as_str()), (Some("cnc"), Some("Millimeters")));
    // A field set after the preset, with no frame between, keeps its value and shows Custom.
    app.run("ui.dxfOptionsDialog", json!({"dxfPreset": "custom"})).unwrap();
    set(&mut app, "dxfPreset", json!("cnc"));
    set(&mut app, "unit", json!("Inches"));
    frame(&mut app);
    let d = fields(&app);
    assert_eq!((show(&d, "unit"), show(&d, "dxfPreset"), show(&d, "version")), ("Inches".into(), "custom".into(), "2000".into()));
    // Setting the preset again, once Custom is shown, sets its fields again.
    set(&mut app, "dxfPreset", json!("cnc"));
    frame(&mut app);
    assert_eq!(show(&fields(&app), "unit"), "Millimeters");
    set(&mut app, "unit", json!("Inches"));
    confirm(&mut app);
    let remembered = ["dxfPreset", "version", "unit"].map(|k| app.ui.dxf_options[k].as_str());
    assert_eq!(remembered, [Some("custom"), Some("2000"), Some("Inches")]);
}

#[test]
fn include_placed_images_and_fill_hatches_follow_the_preset_and_a_change_shows_custom() {
    let (mut app, written) = app(1);
    let shown = |app: &VectorcraftApp| {
        let d = fields(app);
        (show(&d, "dxfPreset"), d.bool("includeImages") == Some(true), d.bool("includeHatches") == Some(true))
    };
    app.run("ui.dxfOptionsDialog", json!({"path": "/out/a.dxf"})).unwrap();
    frame(&mut app);
    assert_eq!(shown(&app), ("custom".into(), true, true));
    confirm(&mut app);
    assert!(text(&written.borrow()[0].1).contains("\nHATCH\n"), "Custom keeps the fill's hatch");
    app.run("ui.dxfOptionsDialog", json!({"path": "/out/b.dxf"})).unwrap();
    set(&mut app, "dxfPreset", json!("cnc"));
    frame(&mut app);
    assert_eq!(shown(&app), ("cnc".into(), false, false));
    // Either checkbox changed shows Custom; both back as the preset sets them show CNC / CAM.
    set(&mut app, "includeHatches", json!(true));
    frame(&mut app);
    assert_eq!(shown(&app), ("custom".into(), false, true));
    set(&mut app, "includeHatches", json!(false));
    set(&mut app, "includeImages", json!(true));
    frame(&mut app);
    assert_eq!(shown(&app), ("custom".into(), true, false));
    set(&mut app, "includeImages", json!(false));
    frame(&mut app);
    assert_eq!(shown(&app).0, "cnc");
    confirm(&mut app);
    assert!(!text(&written.borrow()[1].1).contains("\nHATCH\n"), "CNC / CAM writes the fill's outline alone");
    let remembered = ["includeImages", "includeHatches"].map(|k| app.ui.dxf_options[k].as_bool());
    assert_eq!(remembered, [Some(false), Some(false)]);
    // The next DXF Options start from them, and Custom turns both back on.
    crate::menus::invoke(&mut app, "ui.dxfOptionsDialog", json!({}));
    assert_eq!(shown(&app), ("cnc".into(), false, false));
    set(&mut app, "dxfPreset", json!("custom"));
    frame(&mut app);
    assert_eq!(shown(&app), ("custom".into(), true, true));
}

#[test]
fn bad_dxf_options_keep_the_dialog_open() {
    let (mut app, written) = app(1);
    app.run("ui.dxfOptionsDialog", json!({"path": "/out/a.dxf"})).unwrap();
    set(&mut app, "scale", json!(-1));
    assert!(try_confirm(&mut app).is_err());
    assert_eq!(kind(&app).as_deref(), Some("dxfOptions"));
    set(&mut app, "scale", json!(1));
    let e = try_set(&mut app, "dxfPreset", json!("laser")).unwrap_err();
    assert!(e.contains("dxfPreset"), "{e}");
    set(&mut app, "dxfPreset", json!("custom"));
    set(&mut app, "scale", json!(10));
    // Export Selected Art Only with nothing selected.
    app.run("select.none", json!({})).unwrap();
    set(&mut app, "selectedOnly", json!(true));
    frame(&mut app);
    let e = try_confirm(&mut app).unwrap_err();
    assert!(e.contains("select something"), "{e}");
    assert!(written.borrow().is_empty());
    set(&mut app, "selectedOnly", json!(false));
    confirm(&mut app);
    assert_eq!(names(&written), ["/out/a.dxf"]);
}

#[test]
fn dwg_is_listed_but_points_to_dxf() {
    let (mut app, written) = app(1);
    app.run("file.exportAs", json!({"format": "dwg"})).unwrap();
    assert_eq!(show(&fields(&app), "format"), "dwg");
    frame(&mut app);
    let e = try_confirm(&mut app).unwrap_err();
    assert!(e.contains("DWG") && e.contains("DXF"), "{e}");
    assert_eq!(kind(&app).as_deref(), Some("exportAs"), "the dialog stays open to pick DXF");
    set(&mut app, "format", json!("dxf"));
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("dxfOptions"));
    assert!(written.borrow().is_empty());
}
