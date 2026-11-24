//! Export As → HP-GL, drawn headlessly: listed with the other formats, then HP-GL Options (version,
//! curve tolerance, origin, pens, path order), remembered for the next export, and one plot per
//! artboard with Use Artboards.

use craft::ui::patterns::form_dialogs::{DialogFields, FieldValue};
use serde_json::json;
use vectorcraft_engine::cmd::fileio;

// PLAN: assumed `fields(&app)` (values of the top hosted dialog) and `press(&mut app, name)` (the control channel's
// click on a control by name, export-w23) in export-w9's helpers.
use super::tests_export::{answer_saves, app, cancel, confirm, frame, kind, names, on_save, set, try_confirm, try_set};
use super::tests_print::press_control;
use super::tests_raster_formats::fields;
use super::*;
use crate::VectorcraftApp;

/// The pen table a dialog holds: null for One Pen per Color.
fn pens(d: &DialogFields) -> serde_json::Value {
    d.get("pens").and_then(|v| serde_json::to_value(v).ok()).unwrap_or_default()
}

fn text(d: &DialogFields, key: &str) -> String {
    d.text(key).unwrap_or_default().to_string()
}

#[test]
fn export_as_hpgl_shows_hpgl_options_then_writes_and_remembers_them() {
    let (mut app, written) = app(2);
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("exportAs"));
    assert_eq!(text(&fields(&app), "format"), "hpgl");
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("hpglOptions"), "HP-GL Options follow");
    let d = fields(&app);
    assert_eq!((text(&d, "path"), text(&d, "hpglVersion"), text(&d, "origin")), ("/out/Untitled-1.plt".into(), "hpgl".into(), "bottomLeft".into()));
    assert_eq!(
        (d.number("curveTolerance"), pens(&d).is_null(), d.bool("sortPaths"), d.bool("insideFirst")),
        (Some(0.1), true, Some(false), Some(false))
    );
    frame(&mut app);
    // The defaults write the plot an export with this artboard choice and no HP-GL options writes.
    let choice: serde_json::Map<String, serde_json::Value> = form::json_of(&[], &fields(&app))
        .into_iter()
        .filter(|(k, _)| ["useArtboards", "range", "artboard", "artboards"].contains(&k.as_str()))
        .collect();
    let plain = fileio::encode(&app.session.active().unwrap().doc, "hpgl", &serde_json::Value::Object(choice)).unwrap();
    confirm(&mut app);
    assert!(kind(&app).is_none());
    assert_eq!(names(&written), ["/out/Untitled-1.plt"]);
    assert_eq!(written.borrow()[0].1, plain);
    assert!(plain.starts_with(b"IN;") && plain.ends_with(b"PU;SP0;"));

    // Every control draws, the pen table with a row per pen; OK writes the options given.
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    frame(&mut app);
    confirm(&mut app);
    set(&mut app, "hpglVersion", json!("hpgl2"));
    set(&mut app, "origin", json!("center"));
    set(&mut app, "curveTolerance", json!(0.5));
    set(&mut app, "pens", json!(["#ff0000", "#000000"]));
    set(&mut app, "sortPaths", json!(true));
    set(&mut app, "insideFirst", json!(true));
    frame(&mut app);
    confirm(&mut app);
    // HP-GL/2 with the pen table as its palette (the encoded polylines that follow are binary).
    let plot = String::from_utf8_lossy(&written.borrow()[1].1).into_owned();
    assert!(plot.starts_with("IN;NP4;PC1,255,0,0;PC2,0,0,0;"), "{plot}");
    // The next HP-GL Options start from these choices, but not from this export's path.
    crate::menus::invoke(&mut app, "ui.hpglOptionsDialog", json!({}));
    let d = fields(&app);
    assert_eq!((text(&d, "hpglVersion"), text(&d, "origin"), d.number("curveTolerance")), ("hpgl2".into(), "center".into(), Some(0.5)));
    assert_eq!((pens(&d), d.bool("sortPaths"), d.bool("insideFirst")), (json!(["#ff0000", "#000000"]), Some(true), Some(true)));
    assert!(!d.contains("path"));
    assert!(app.ui.hpgl_options.get("useArtboards").is_none());
    cancel(&mut app);

    // One plot per artboard.
    app.run("file.exportAs", json!({"format": "hpgl"})).unwrap();
    set(&mut app, "useArtboards", json!(true));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("hpglOptions"));
    confirm(&mut app);
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
        let before = fields(&app).get(field).and_then(|v| serde_json::to_value(v).ok());
        // A value the row can't hold is refused as it is set, the others by OK.
        let e = match try_set(&mut app, field, value.clone()) {
            Err(e) => e,
            Ok(()) => {
                frame(&mut app);
                try_confirm(&mut app).unwrap_err()
            }
        };
        assert!(e.contains(want), "{field} {value}: {e}");
        assert_eq!(kind(&app).as_deref(), Some("hpglOptions"));
        // A field the dialog doesn't have was never set: there is nothing to put back.
        if let Some(before) = before {
            set(&mut app, field, before);
        }
    }
    assert!(written.borrow().is_empty());
    assert!(app.ui.hpgl_options.is_null(), "nothing is remembered from a refused OK");
    confirm(&mut app);
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
    let mut d = fields(&app);
    hpgl_options::set_pen_table(&mut d, true, || Ok(automatic.clone())).unwrap();
    assert_eq!(pens(&d), json!(automatic));
    d.set("pens", FieldValue::Json(json!(["#00ff00", "#0000ff"])));
    hpgl_options::set_pen_table(&mut d, false, none).unwrap();
    assert!(pens(&d).is_null());
    // The table set aside is not an export option.
    assert_eq!(serde_json::to_value(d.get("__pens")).unwrap(), json!(["#00ff00", "#0000ff"]));
    assert!(form::json_of(&[], &d).get("__pens").is_none());
    // Choosing One Pen per Color again keeps the table set aside.
    hpgl_options::set_pen_table(&mut d, false, none).unwrap();
    hpgl_options::set_pen_table(&mut d, true, || panic!("the table set aside comes back")).unwrap();
    assert_eq!(pens(&d), json!(["#00ff00", "#0000ff"]));
    // Nothing set aside and nothing drawn: one black pen.
    d.set("pens", FieldValue::Json(serde_json::Value::Null));
    d.remove("__pens");
    hpgl_options::set_pen_table(&mut d, true, none).unwrap();
    assert_eq!(pens(&d), json!(["#000000"]));
    // The Pen Table button asks the host, which starts the table from the art's pens.
    frame(&mut app);
    press_control(&mut app, json!({"name": "Pen Table", "role": "RadioButton"}));
    frame(&mut app);
    assert_eq!(pens(&fields(&app)), json!(automatic));
}

/// A press of Add adds one pen and a press of One Pen per Color sets the table aside, though the rows' body may run
/// twice in the pass that reads the press.
#[test]
fn pen_rows_apply_each_press_once() {
    let (mut app, _) = app(1);
    app.run("ui.hpglOptionsDialog", json!({})).unwrap();
    set(&mut app, "pens", json!(["#ff0000", "#000000"]));
    frame(&mut app);
    press_control(&mut app, json!({"name": "Add", "role": "Button"}));
    frame(&mut app);
    assert_eq!(pens(&fields(&app)), json!(["#ff0000", "#000000", "#000000"]));
    press_control(&mut app, json!({"name": "One Pen per Color", "role": "RadioButton"}));
    frame(&mut app);
    let d = fields(&app);
    assert!(pens(&d).is_null());
    assert_eq!(serde_json::to_value(d.get("__pens")).unwrap(), json!(["#ff0000", "#000000", "#000000"]));
}

/// Pen Table with options the export refuses keeps One Pen per Color and says why, rather than
/// starting from one black pen that the corrected options would keep.
#[test]
fn pen_table_with_refused_options_keeps_one_pen_per_color() {
    let (mut app, _) = app(1);
    app.run("ui.hpglOptionsDialog", json!({"curveTolerance": 20})).unwrap();
    let mut d = fields(&app);
    let e = hpgl_options::automatic_pens(&app, &d, false).unwrap_err();
    assert!(e.contains("`curveTolerance`"), "{e}");
    assert_eq!(hpgl_options::set_pen_table(&mut d, true, || Err(e.clone())), Err(e));
    assert!(pens(&d).is_null(), "One Pen per Color stays chosen");
    frame(&mut app);
    // With the tolerance corrected, Pen Table starts from the art's pen.
    set(&mut app, "curveTolerance", json!(0.1));
    let mut d = fields(&app);
    let found = hpgl_options::automatic_pens(&app, &d, false).unwrap();
    hpgl_options::set_pen_table(&mut d, true, || Ok(found.clone())).unwrap();
    assert_eq!(pens(&d), json!(found));
    assert_eq!(found.len(), 1, "the ellipse's fill is one pen");
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
    on_save(|_| Some("/out/sel.plt".to_string()));
    app.run("document.exportSelection", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("hpglOptions"));
    let d = fields(&app);
    assert!(!d.contains(crate::io::EXPORT_SELECTION));
    assert_eq!(hpgl_options::automatic_pens(&app, &d, true).unwrap(), ["#0000ff"], "the selection's pen only");
    // An artboard choice (one the whole document would refuse) is not this export's: the dialog has none to set.
    assert!(try_set(&mut app, "artboard", json!(5)).is_err());
    frame(&mut app);
    confirm(&mut app);
    assert!(kind(&app).is_none());
    assert_eq!(names(&written), ["/out/sel.plt"]);
    let plot = String::from_utf8_lossy(&written.borrow()[0].1).into_owned();
    assert_eq!(plot.matches("PD").count(), 1, "the rectangle alone: {plot}");
    assert!(app.ui.hpgl_options.get(crate::io::EXPORT_SELECTION).is_none());
    // A selection export asked for by its param is not remembered either.
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/b.plt", "exportSelection": true})).unwrap();
    confirm(&mut app);
    assert!(app.ui.hpgl_options.get(crate::io::EXPORT_SELECTION).is_none());
    app.run("select.none", json!({})).unwrap();
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/c.plt"})).unwrap();
    confirm(&mut app);
    assert_eq!(names(&written).last().map(String::as_str), Some("/out/c.plt"), "the whole document, with nothing selected");
}

/// A save panel that is canceled opens HP-GL Options again, with the options remembered before.
#[test]
fn a_canceled_save_panel_keeps_hpgl_options_open() {
    let (mut app, written) = app(1);
    app.ui.hpgl_options = json!({"origin": "center"});
    on_save(|_| None);
    app.run("ui.hpglOptionsDialog", json!({})).unwrap();
    set(&mut app, "origin", json!("bottomLeft"));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("hpglOptions"));
    assert_eq!(text(&fields(&app), "origin"), "bottomLeft");
    assert_eq!(app.ui.hpgl_options, json!({"origin": "center"}));
    assert!(written.borrow().is_empty());
    on_save(|_| Some("/out/d.plt".to_string()));
    confirm(&mut app);
    assert_eq!(names(&written), ["/out/d.plt"]);
    assert_eq!(app.ui.hpgl_options["origin"], "bottomLeft");
}

/// A save panel that answers later leaves the options remembered before until it answers:
/// canceled, they stay; answered, the export remembers the dialog's.
#[test]
fn an_off_thread_save_panel_remembers_hpgl_options_when_it_answers() {
    let (mut app, _) = app(1);
    // A panel this test answers itself, while the frames run.
    let shown = crate::tests_picks::scripted(&mut app);
    let ctx = app.ui_ctx.clone().expect("a test app has a window");
    app.ui.hpgl_options = json!({"origin": "center"});
    for (asked, answer) in [None, Some("/out/e.plt")].into_iter().enumerate() {
        app.run("ui.hpglOptionsDialog", json!({})).unwrap();
        set(&mut app, "origin", json!("bottomLeft"));
        frame(&mut app);
        confirm(&mut app);
        assert_eq!(shown.requests().len(), asked + 1, "the save panel is showing");
        assert_eq!(app.ui.hpgl_options, json!({"origin": "center"}), "not remembered while the panel is open");
        match answer {
            Some(path) => shown.answer_saved(path, None),
            None => shown.cancel(),
        }
        crate::tests_picks::poll(&mut app, &ctx);
    }
    assert_eq!(shown.written(), ["/out/e.plt"], "the canceled panel wrote nothing");
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
    confirm(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("hpglOptions"));
    let seed = |app: &VectorcraftApp| hpgl_options::automatic_pens(app, &fields(app), false).unwrap();
    assert_eq!(seed(&app), [on_board[0].as_str(), "#0000ff"], "the art's bounds");
    // The rectangle is the selection.
    // Export Selected Art Only comes with Export Selection's fields: the dialog has no row for it.
    let mut selected = fields(&app);
    selected.set("selectedOnly", true);
    assert_eq!(hpgl_options::automatic_pens(&app, &selected, false).unwrap(), ["#0000ff"]);
    // Use Artboards comes with Export As's fields too: the dialog has no row for it.
    let mut boards = fields(&app);
    boards.set("useArtboards", true);
    assert_eq!(hpgl_options::automatic_pens(&app, &boards, false).unwrap(), on_board);
}

/// OK after another document has become the active one closes HP-GL Options and writes nothing, so
/// the other document is not written to the path chosen for the first.
#[test]
fn hpgl_options_write_nothing_after_another_document_became_active() {
    let (mut app, written) = app(1);
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/first.plt"})).unwrap();
    app.run("file.new", json!({"width": 60, "height": 40})).unwrap();
    assert_eq!(kind(&app).as_deref(), Some("hpglOptions"));
    confirm(&mut app);
    assert!(kind(&app).is_none());
    assert!(written.borrow().is_empty(), "nothing is written over the first document's path");
    // Opened for the document that is active, OK writes it.
    app.run("window.nextDocument", json!({})).unwrap();
    app.run("ui.hpglOptionsDialog", json!({"path": "/out/first.plt"})).unwrap();
    confirm(&mut app);
    assert_eq!(names(&written), ["/out/first.plt"]);
}
