//! Export Selection (`document.exportSelection` without params): the picked format's options
//! dialog follows the save panel, as in Export As, and writes the selection alone.

use std::cell::Cell;
use std::rc::Rc;

use serde_json::json;
use vectorcraft_plugins::wat::{self as tpl, Echo};

use super::tests_export::{Written, answer_saves, app_saving, confirm, fields, frame, kind, names, on_save, rgba, set};

/// The last file written, as text.
fn last_text(written: &Written) -> String {
    String::from_utf8_lossy(&written.borrow().last().unwrap().1).into_owned()
}

/// The width and height in a PNG's header.
fn png_size(png: &[u8]) -> (u32, u32) {
    let n = |at: usize| u32::from_be_bytes([png[at], png[at + 1], png[at + 2], png[at + 3]]);
    (n(16), n(20))
}

/// Pick `path` in the next save panels.
fn picking(path: &str) {
    let picked = path.to_string();
    on_save(move |_| Some(picked.clone()));
}

/// How many paths a native document's JSON (or the input an exporter plug-in is given) holds.
fn paths_in(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(vectorcraft_format::json_part(bytes)).matches("\"anchors\"").count()
}

/// Every format with an options dialog shows it after the save panel, writes nothing before its OK, and OK
/// writes the selection alone, cropped to the selected 10×5 pt rectangle: the PDF's page, the PNG at 72 ppi,
/// the SVG's view box, the EPS bounding box and the DXF's extents.
#[test]
fn export_selection_continues_in_each_formats_options() {
    let (mut app, written) = app_saving(2, |_| String::new());
    app.run("shape.rectangle", json!({"x": 40, "y": 30, "width": 10, "height": 5})).unwrap();
    for (ext, options) in [("pdf", "pdf-export"), ("svg", "svgOptions"), ("png", "pngOptions"), ("eps", "epsOptions"), ("dxf", "dxfOptions")] {
        let path = format!("/out/sel.{ext}");
        let before = names(&written).len();
        picking(&path);
        app.run("document.exportSelection", json!({})).unwrap();
        answer_saves(&mut app);
        assert_eq!(kind(&app).as_deref(), Some(options), "{ext}");
        assert_eq!(names(&written).len(), before, "{ext}: nothing is written before OK");
        frame(&mut app);
        confirm(&mut app);
        assert!(kind(&app).is_none(), "{ext}");
        assert_eq!(names(&written).last(), Some(&path), "{ext}");
        let text = last_text(&written);
        match ext {
            "pdf" => {
                let pdf = vectorcraft_pdf::import(&written.borrow().last().unwrap().1).unwrap();
                assert_eq!(pdf.artboards.len(), 1, "{ext}");
                let page = pdf.artboards[0].rect;
                assert_eq!((page.width().round(), page.height().round()), (10.0, 5.0), "the selection's crop, not the art's bounds");
            }
            "png" => assert_eq!(png_size(&written.borrow().last().unwrap().1), (10, 5), "10×5 pt at 72 ppi"),
            "svg" => assert!(text.contains(r#"viewBox="0 0 10 5""#), "{text}"),
            "eps" => assert!(text.contains("%%BoundingBox: 0 0 10 5"), "{text}"),
            _ => {
                let at = text.find("$EXTMAX").unwrap_or_else(|| panic!("no extents: {text}"));
                let max: Vec<f64> = text[at..].lines().skip(1).collect::<Vec<_>>().chunks(2).take(2).map(|c| c[1].trim().parse().unwrap()).collect();
                // In millimeters, DXF Options' default unit: 10×5 pt.
                assert!((max[0] - 10.0 * 25.4 / 72.0).abs() < 1e-3 && (max[1] - 5.0 * 25.4 / 72.0).abs() < 1e-3, "the selection's extents: {max:?}");
            }
        }
    }
    assert_eq!(names(&written).len(), 5, "one file each");
}

/// Text Export Options opens with Selection Only on, and OK writes the selected type alone.
#[test]
fn export_selection_to_text_writes_the_selected_type() {
    let (mut app, written) = app_saving(1, |_| "/out/sel.txt".into());
    app.run("text.create", json!({"x": 10, "y": 10, "text": "Left out"})).unwrap();
    app.run("text.create", json!({"x": 10, "y": 30, "text": "Selected"})).unwrap();
    app.run("document.exportSelection", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("txtOptions"));
    assert_eq!(fields(&app)["selectionOnly"], json!(true));
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(names(&written), ["/out/sel.txt"]);
    // Line endings default to the platform's.
    assert_eq!(last_text(&written), if cfg!(windows) { "Selected\r\n" } else { "Selected\n" });
}

/// SVG Options opened by Export Selection hides Use Artboards, and its OK leaves the Use
/// Artboards setting that the next Export As SVG starts from as it was.
#[test]
fn export_selection_to_svg_keeps_the_remembered_use_artboards() {
    let (mut app, _) = app_saving(2, |_| "/out/sel.svg".into());
    app.run("file.export.svg", json!({})).unwrap();
    set(&mut app, "useArtboards", json!(true));
    confirm(&mut app);
    assert_eq!(app.ui.svg_options["useArtboards"], true);
    app.run("shape.rectangle", json!({"x": 40, "y": 30, "width": 10, "height": 5})).unwrap();
    app.run("document.exportSelection", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("svgOptions"));
    confirm(&mut app);
    assert_eq!(app.ui.svg_options["useArtboards"], true, "{}", app.ui.svg_options);
}

/// A format without an options dialog (a native document) and an exporter plug-in, with or
/// without parameters, are given the selection alone: fewer paths than the whole document.
#[test]
fn export_selection_without_a_dialog_and_to_plug_ins_writes_the_selection() {
    let (mut app, written) = app_saving(1, |_| String::new());
    app.run("shape.rectangle", json!({"x": 40, "y": 30, "width": 10, "height": 5})).unwrap();
    app.run("file.exportAs", json!({"path": "/out/all.vectorcraft"})).unwrap();
    let all = paths_in(&written.borrow().last().unwrap().1);
    picking("/out/sel.vectorcraft");
    app.run("document.exportSelection", json!({})).unwrap();
    answer_saves(&mut app);
    assert!(kind(&app).is_none(), "written at once");
    assert_eq!(names(&written).last().map(String::as_str), Some("/out/sel.vectorcraft"));
    let selected = paths_in(&written.borrow().last().unwrap().1);
    assert!(selected >= 1 && selected < all, "{selected} of {all} paths");

    for (id, ext, params) in
        [("test.ui.selwhole", "tsw", "{}"), ("test.ui.selparams", "tsp", r#"{"density":{"type":"number","min":0,"max":10,"default":4}}"#)]
    {
        let wasm = wat::parse_str(tpl::echo_exporter(&tpl::exporter(id, &format!(r#"["{ext}"]"#), params), Echo::Input)).unwrap();
        app.run("plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap();
        picking(&format!("/out/sel.{ext}"));
        app.run("document.exportSelection", json!({})).unwrap();
        answer_saves(&mut app);
        if params != "{}" {
            assert!(kind(&app).is_some(), "{id} asks for its parameters");
            frame(&mut app);
            confirm(&mut app);
        }
        assert!(kind(&app).is_none(), "{id}");
        assert_eq!(names(&written).last(), Some(&format!("/out/sel.{ext}")));
        let given = paths_in(&written.borrow().last().unwrap().1);
        app.run("plugin.remove", json!({"id": id})).unwrap();
        assert_eq!(given, selected, "{id} is given the selection");
    }
}

/// The question before replacing the file the document was opened from runs the export again as
/// an Export Selection: the PNG written after OK is the selection's.
#[test]
fn export_selection_after_the_question_before_losing_writes_the_selection() {
    let (mut app, written) = app_saving(1, |_| "/out/sel.png".into());
    app.run("shape.rectangle", json!({"x": 40, "y": 30, "width": 10, "height": 5})).unwrap();
    let st = app.session.doc_mut().unwrap();
    st.imported_from = Some("/out/sel.png".into());
    st.import_losses = vec!["hidden type".into()];
    app.run("document.exportSelection", json!({})).unwrap();
    answer_saves(&mut app);
    assert_eq!(kind(&app).as_deref(), Some("pngOptions"));
    frame(&mut app);
    confirm(&mut app);
    assert!(kind(&app).is_some(), "the question is open");
    assert!(written.borrow().is_empty(), "nothing is written before OK");
    confirm(&mut app);
    assert_eq!(names(&written), ["/out/sel.png"]);
    assert_eq!(png_size(&written.borrow()[0].1), (10, 5), "the selection's crop");
}

/// `file.exportAs {path, exportSelection: true}` writes the selection without a dialog; a value
/// that is not true or false is refused.
#[test]
fn export_as_with_export_selection_writes_the_selection() {
    let (mut app, written) = app_saving(1, |_| String::new());
    app.run("shape.rectangle", json!({"x": 40, "y": 30, "width": 10, "height": 5})).unwrap();
    app.run("file.exportAs", json!({"path": "/out/sel.png", "exportSelection": true, "useArtboards": true})).unwrap();
    assert_eq!(png_size(&written.borrow()[0].1), (10, 5));
    let e = app.run("file.exportAs", json!({"path": "/out/sel.png", "exportSelection": "yes"})).unwrap_err();
    assert!(e.contains("exportSelection must be true or false"), "{e}");
    assert_eq!(names(&written).len(), 1);
}

/// Appearance of Black › Printing / Exporting set to rich black draws a CMYK black as RGB black in
/// a PNG from Export As and from Export Selection, as `document.export` draws it.
#[test]
fn rich_black_reaches_files_from_the_export_dialogs() {
    let (mut app, written) = app_saving(1, |_| "/out/sel.png".into());
    app.run("file.new", json!({"width": 40, "height": 30, "colorMode": "cmyk"})).unwrap();
    app.run("paint.setStroke", json!({"none": true})).unwrap();
    app.run("paint.setFill", json!({"color": {"c": 0, "m": 0, "y": 0, "k": 1}})).unwrap();
    app.run("shape.rectangle", json!({"x": 0, "y": 0, "width": 40, "height": 30})).unwrap();
    let px = |w: &Written| rgba(&w.borrow().last().unwrap().1).get_pixel(10, 10).0;
    app.run("file.exportAs", json!({"path": "/out/all.png"})).unwrap();
    let accurate = px(&written);
    assert!(accurate[0] > 10, "accurate: dark grey, {accurate:?}");
    app.session.prefs.black_output = "rich".into();
    app.run("file.exportAs", json!({"path": "/out/all.png"})).unwrap();
    assert_eq!(px(&written), [0, 0, 0, 255], "Export As");
    app.run("document.exportSelection", json!({})).unwrap();
    frame(&mut app);
    confirm(&mut app);
    assert_eq!(names(&written).last().map(String::as_str), Some("/out/sel.png"));
    assert_eq!(px(&written), [0, 0, 0, 255], "Export Selection");
}

/// With nothing selected, Export Selection stops before the save panel.
#[test]
fn nothing_selected_stops_before_the_save_panel() {
    let (mut app, written) = app_saving(1, |_| "/out/sel.pdf".into());
    app.run("select.none", json!({})).unwrap();
    let asked = Rc::new(Cell::new(false));
    let a = asked.clone();
    on_save(move |_| {
        a.set(true);
        Some("/out/sel.pdf".into())
    });
    let e = app.run("document.exportSelection", json!({})).unwrap_err();
    assert!(e.contains("select something to export"), "{e}");
    assert!(!asked.get() && kind(&app).is_none() && written.borrow().is_empty());
}
