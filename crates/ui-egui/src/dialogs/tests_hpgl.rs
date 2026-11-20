//! Export As → HP-GL, drawn headlessly: listed with the other formats, then HP-GL Options (version,
//! curve tolerance, origin, pens, path order), remembered for the next export, and one plot per
//! artboard with Use Artboards.

use serde_json::json;
use vectorcraft_engine::cmd::fileio;

use super::tests_export::{app, frame, kind, names, set};
use super::*;

#[test]
fn export_as_hpgl_shows_hpgl_options_then_writes_and_remembers_them() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    assert_eq!(kind(&app), Some("exportAs"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("format"), "hpgl");
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("hpglOptions"), "HP-GL Options follow");
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("path"), d.str("hpglVersion"), d.str("origin")), ("/out/Untitled-1.plt".into(), "hpgl".into(), "bottomLeft".into()));
    assert_eq!((d.f64("curveTolerance", 0.0), d.fields["pens"].is_null(), d.bool("sortPaths"), d.bool("insideFirst")), (0.1, true, false, false));
    frame(&mut app);
    // The defaults write the plot an export with this artboard choice and no HP-GL options writes.
    let fields = &app.ui.dialog.as_ref().unwrap().fields;
    let choice: serde_json::Map<String, serde_json::Value> = fields
        .iter()
        .filter(|(k, _)| ["useArtboards", "range", "artboard", "artboards"].contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let plain = fileio::encode(&app.session.active().unwrap().doc, "hpgl", &serde_json::Value::Object(choice)).unwrap();
    let r = confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(r["path"], "/out/Untitled-1.plt");
    assert_eq!(written.borrow()[0].1, plain);
    assert!(plain.starts_with(b"IN;") && plain.ends_with(b"PU;SP0;"));

    // Every control draws, the pen table with a row per pen; OK writes the options given.
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    frame(&mut app);
    confirm(&mut app).unwrap();
    set(&mut app, "hpglVersion", json!("hpgl2"));
    set(&mut app, "origin", json!("center"));
    set(&mut app, "curveTolerance", json!(0.5));
    set(&mut app, "pens", json!(["#ff0000", "#000000"]));
    set(&mut app, "sortPaths", json!(true));
    set(&mut app, "insideFirst", json!(true));
    frame(&mut app);
    confirm(&mut app).unwrap();
    // HP-GL/2 with the pen table as its palette (the encoded polylines that follow are binary).
    let plot = String::from_utf8_lossy(&written.borrow()[1].1).into_owned();
    assert!(plot.starts_with("IN;NP4;PC1,255,0,0;PC2,0,0,0;"), "{plot}");
    // The next HP-GL Options start from these choices, but not from this export's path.
    crate::menus::invoke(&mut app, "ui.hpglOptionsDialog", json!({}));
    let d = app.ui.dialog.as_ref().unwrap();
    assert_eq!((d.str("hpglVersion"), d.str("origin"), d.f64("curveTolerance", 0.0)), ("hpgl2".into(), "center".into(), 0.5));
    assert_eq!((d.fields["pens"].clone(), d.bool("sortPaths"), d.bool("insideFirst")), (json!(["#ff0000", "#000000"]), true, true));
    assert!(!d.fields.contains_key("path"));
    assert!(app.ui.hpgl_options.get("useArtboards").is_none());
    app.ui.dialog = None;

    // One plot per artboard.
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    set(&mut app, "useArtboards", json!(true));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("hpglOptions"));
    confirm(&mut app).unwrap();
    let n = names(&written);
    assert_eq!(&n[2..], ["/out/Untitled-1-Artboard-1.plt", "/out/Untitled-1-Artboard-2.plt"]);
}

#[test]
fn bad_hpgl_options_keep_the_dialog_open() {
    let (mut app, written) = app(1);
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/a.plt"})).unwrap();
    frame(&mut app);
    for (field, value, want) in [
        ("hpglVersion", json!("hpgl3"), "`hpglVersion`"),
        ("curveTolerance", json!(20), "`curveTolerance`"),
        ("origin", json!("top"), "`origin`"),
        ("pens", json!(["#ff0000", "red"]), "`pens`"),
        ("pens", json!([]), "`pens`"),
        // More pens than a plot takes: drawn without rows, refused by OK.
        ("pens", json!(vec!["#000000"; 300]), "`pens`"),
        ("sortPaths", json!("yes"), "`sortPaths`"),
        // An artboard the document doesn't have.
        ("artboard", json!(3), "artboard"),
        ("useArtboards", json!("yes"), "useArtboards"),
    ] {
        let before = app.ui.dialog.as_ref().unwrap().fields.get(field).cloned();
        set(&mut app, field, value.clone());
        frame(&mut app);
        let e = confirm(&mut app).unwrap_err();
        assert!(e.contains(want), "{field} {value}: {e}");
        assert_eq!(kind(&app), Some("hpglOptions"));
        set(&mut app, field, before.unwrap_or_default());
    }
    assert!(written.borrow().is_empty());
    assert!(app.ui.hpgl_options.is_null(), "nothing is remembered from a refused OK");
    confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/out/a.plt"]);
}

/// Pen Table starts from the pens the art gets, One Pen per Color sets the table aside, and Pen
/// Table shows it again; a document whose art draws nothing starts with one black pen.
#[test]
fn pen_table_starts_from_the_art_s_pens_and_comes_back() {
    let (mut app, _) = app(1);
    let automatic = fileio::hpgl::automatic_pens(&app.session.active().unwrap().doc, &json!({})).unwrap();
    assert_eq!(automatic.len(), 1, "the ellipse's fill is one pen");
    app.run("ui.hpglOptionsDialog", json!({})).unwrap();
    let none = || -> Result<Vec<String>, String> { Ok(Vec::new()) };
    let d = app.ui.dialog.as_mut().unwrap();
    hpgl_options::set_pen_table(d, true, || Ok(automatic.clone())).unwrap();
    assert_eq!(d.fields["pens"], json!(automatic));
    d.fields.insert("pens".into(), json!(["#00ff00", "#0000ff"]));
    hpgl_options::set_pen_table(d, false, none).unwrap();
    assert!(d.fields["pens"].is_null());
    // The table set aside is not an export option.
    assert_eq!(d.fields["__pens"], json!(["#00ff00", "#0000ff"]));
    assert!(form::params(d).get("__pens").is_none());
    // Choosing One Pen per Color again keeps the table set aside.
    hpgl_options::set_pen_table(d, false, none).unwrap();
    hpgl_options::set_pen_table(d, true, || panic!("the table set aside comes back")).unwrap();
    assert_eq!(d.fields["pens"], json!(["#00ff00", "#0000ff"]));
    // Nothing set aside and nothing drawn: one black pen.
    d.fields.insert("pens".into(), serde_json::Value::Null);
    d.fields.remove("__pens");
    hpgl_options::set_pen_table(d, true, none).unwrap();
    assert_eq!(d.fields["pens"], json!(["#000000"]));
    frame(&mut app);
}

/// Pen Table with options the export refuses keeps One Pen per Color and says why, rather than
/// starting from one black pen that the corrected options would keep.
#[test]
fn pen_table_with_refused_options_keeps_one_pen_per_color() {
    let (mut app, _) = app(1);
    app.run("ui.hpglOptionsDialog", json!({"curveTolerance": 20})).unwrap();
    let d = app.ui.dialog.clone().unwrap();
    let e = hpgl_options::automatic_pens(&app, &d).unwrap_err();
    assert!(e.contains("`curveTolerance`"), "{e}");
    let d = app.ui.dialog.as_mut().unwrap();
    assert_eq!(hpgl_options::set_pen_table(d, true, || Err(e.clone())), Err(e));
    assert!(d.fields["pens"].is_null(), "One Pen per Color stays chosen");
    frame(&mut app);
    // With the tolerance corrected, Pen Table starts from the art's pen.
    set(&mut app, "curveTolerance", json!(0.1));
    let d = app.ui.dialog.clone().unwrap();
    let pens = hpgl_options::automatic_pens(&app, &d).unwrap();
    let d = app.ui.dialog.as_mut().unwrap();
    hpgl_options::set_pen_table(d, true, || Ok(pens.clone())).unwrap();
    assert_eq!(d.fields["pens"], json!(pens));
    assert_eq!(pens.len(), 1, "the ellipse's fill is one pen");
}

/// Export Selection to HP-GL checks, lists the pens of and writes the selection alone, and the next
/// whole-document export is not a selection export.
#[test]
fn export_selection_to_hpgl_uses_the_selection_and_is_not_remembered() {
    let (mut app, written) = app(1);
    // A blue outline that is the selection; the ellipse stays out of the file.
    app.run("shape.rectangle", json!({"x": 40, "y": 30, "width": 10, "height": 5})).unwrap();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("paint.setStroke", json!({"color": "#0000ff"})).unwrap();
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| Some("/out/sel.plt".to_string())));
    app.run("document.exportSelection", json!({})).unwrap();
    assert_eq!(kind(&app), Some("hpglOptions"));
    let d = app.ui.dialog.clone().unwrap();
    assert!(d.bool(export_as::SELECTION_FIELD) && !d.fields.contains_key(crate::io::EXPORT_SELECTION));
    assert_eq!(hpgl_options::automatic_pens(&app, &d).unwrap(), ["#0000ff"], "the selection's pen only");
    // An artboard choice the whole document would refuse is not this export's.
    set(&mut app, "artboard", json!(5));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert!(app.ui.dialog.is_none());
    assert_eq!(names(&written), ["/out/sel.plt"]);
    let plot = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
    assert_eq!(plot.matches("PD").count(), 1, "the rectangle alone: {plot}");
    assert!(app.ui.hpgl_options.get(export_as::SELECTION_FIELD).is_none());
    // A selection export asked for by its param is not remembered either.
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/b.plt", "exportSelection": true})).unwrap();
    confirm(&mut app).unwrap();
    assert!(app.ui.hpgl_options.get(crate::io::EXPORT_SELECTION).is_none());
    app.run("select.none", json!({})).unwrap();
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/c.plt"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(names(&written).last().map(String::as_str), Some("/out/c.plt"), "the whole document, with nothing selected");
}

/// A save panel that is canceled keeps HP-GL Options open, with the options remembered before.
#[test]
fn a_canceled_save_panel_keeps_hpgl_options_open() {
    let (mut app, written) = app(1);
    app.ui.hpgl_options = json!({"origin": "center"});
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| None));
    app.run("ui.hpglOptionsDialog", json!({})).unwrap();
    set(&mut app, "origin", json!("bottomLeft"));
    frame(&mut app);
    assert!(confirm(&mut app).is_err());
    assert_eq!(kind(&app), Some("hpglOptions"));
    assert_eq!(app.ui.dialog.as_ref().unwrap().str("origin"), "bottomLeft");
    assert_eq!(app.ui.hpgl_options, json!({"origin": "center"}));
    assert!(written.borrow().is_empty());
    app.services.pick_save = Some(Box::new(|_: &crate::FilePick| Some("/out/d.plt".to_string())));
    confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/out/d.plt"]);
    assert_eq!(app.ui.hpgl_options["origin"], "bottomLeft");
}

/// A save panel shown off the UI thread leaves the options remembered before until it answers:
/// canceled, they stay; answered, the export remembers the dialog's.
#[test]
fn an_off_thread_save_panel_remembers_hpgl_options_when_it_answers() {
    let (mut app, _) = app(1);
    let (shown, written) = crate::tests_picks::off_the_ui_thread(&mut app.services);
    let poll = |app: &mut VectorcraftApp| crate::picks::poll(app, &egui::Context::default());
    app.ui.hpgl_options = json!({"origin": "center"});
    for answer in [&[][..], &["/out/e.plt"][..]] {
        app.run("ui.hpglOptionsDialog", json!({})).unwrap();
        set(&mut app, "origin", json!("bottomLeft"));
        frame(&mut app);
        let _ = confirm(&mut app);
        assert_eq!(shown.requests().len(), 1, "the save panel is showing");
        assert_eq!(app.ui.hpgl_options, json!({"origin": "center"}), "not remembered while the panel is open");
        shown.answer(answer);
        poll(&mut app);
    }
    assert_eq!(*written.borrow(), ["/out/e.plt"], "the canceled panel wrote nothing");
    assert_eq!(app.ui.hpgl_options["origin"], "bottomLeft");
}

/// The pens Pen Table starts from are those of what this export plots: the art's bounds without
/// Use Artboards (art outside the artboard included), the selection with Export Selected Art Only,
/// and only the art on the artboard with Use Artboards.
#[test]
fn pen_table_starts_from_the_pens_of_what_the_export_plots() {
    let (mut app, _) = app(1);
    let on_board = fileio::hpgl::automatic_pens(&app.session.active().unwrap().doc, &json!({})).unwrap();
    assert_eq!(on_board.len(), 1, "the ellipse's fill is one pen");
    // A blue outline outside the artboard (the paint applies to the new rectangle, the selection).
    app.run("shape.rectangle", json!({"x": 500, "y": 500, "width": 30, "height": 30})).unwrap();
    app.run("paint.setFill", json!({"none": true})).unwrap();
    app.run("paint.setStroke", json!({"color": "#0000ff"})).unwrap();
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    set(&mut app, "useArtboards", json!(false));
    frame(&mut app);
    confirm(&mut app).unwrap();
    assert_eq!(kind(&app), Some("hpglOptions"));
    let seed = |app: &VectorcraftApp| hpgl_options::automatic_pens(app, app.ui.dialog.as_ref().unwrap()).unwrap();
    assert_eq!(seed(&app), [on_board[0].as_str(), "#0000ff"], "the art's bounds");
    // The rectangle is the selection.
    set(&mut app, "selectedOnly", json!(true));
    assert_eq!(seed(&app), ["#0000ff"]);
    set(&mut app, "selectedOnly", json!(false));
    set(&mut app, "useArtboards", json!(true));
    assert_eq!(seed(&app), on_board);
}

/// OK after another document has become the active one closes HP-GL Options and writes nothing, so
/// the other document is not written to the path chosen for the first.
#[test]
fn hpgl_options_write_nothing_after_another_document_became_active() {
    let (mut app, written) = app(1);
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/first.plt"})).unwrap();
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    assert_eq!(kind(&app), Some("hpglOptions"));
    assert_eq!(confirm(&mut app).unwrap(), json!({}));
    assert!(app.ui.dialog.is_none());
    assert!(written.borrow().is_empty(), "nothing is written over the first document's path");
    // Opened for the document that is active, OK writes it.
    app.run("window.nextDocument", json!({})).unwrap();
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/first.plt"})).unwrap();
    confirm(&mut app).unwrap();
    assert_eq!(names(&written), ["/out/first.plt"]);
}

#[test]
fn the_hpgl_options_dialog_needs_a_document() {
    let mut app = VectorcraftApp::new(vectorcraft_engine::Session::new(), Default::default());
    assert!(app.run("ui.hpglOptionsDialog", json!({})).is_err());
    assert!(!crate::menus::enabled(&app, "ui.hpglOptionsDialog"));
}
