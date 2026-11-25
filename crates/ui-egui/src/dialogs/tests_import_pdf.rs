//! The Import PDF dialog: opening and placing multi-page and password-protected PDFs.

use craft::ui::dialog::{HeldOk, StackDialog};
use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_testkit::ai;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf, pdf_with};

use super::import_pdf::ImportPdfDialog;
use super::*;
use crate::{VectorcraftApp, io};

fn app() -> VectorcraftApp {
    let mut app = crate::testing::windowed(VectorcraftApp::new(Session::new(), Default::default()));
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app
}

fn ctx(app: &VectorcraftApp) -> egui::Context {
    app.ui_ctx.clone().expect("a UI context")
}

/// The id of the open Import PDF dialog.
fn dialog_id(app: &VectorcraftApp) -> Option<craft::ui::dialog::DialogId> {
    app.dialogs()?.with(|host| host.get::<ImportPdfDialog>().map(|d| d.id()))
}

fn is_open(app: &VectorcraftApp) -> bool {
    dialog_id(app).is_some()
}

/// `ui.dialog.set` on the open dialog.
fn try_set(app: &VectorcraftApp, key: &str, v: serde_json::Value) -> Result<(), String> {
    let (ctx, id) = (ctx(app), dialog_id(app).ok_or("no Import PDF dialog")?);
    app.dialogs().ok_or("no dialog host")?.with(|host| host.set_json_field(&ctx, id, key, v))
}

fn set(app: &mut VectorcraftApp, key: &str, v: serde_json::Value) {
    try_set(app, key, v).unwrap();
}

fn field(app: &VectorcraftApp, key: &str) -> serde_json::Value {
    app.dialogs().unwrap().with(|host| host.get::<ImportPdfDialog>().unwrap().json_fields().unwrap()[key].clone())
}

/// OK as the user presses it: the dialog files it, the pump opens or places, and the dialog closes on success.
/// `Err` when the pump refused it.
fn confirm(app: &mut VectorcraftApp) -> Result<(), String> {
    let (ctx, id) = (ctx(app), dialog_id(app).ok_or("no Import PDF dialog")?);
    let dialogs = app.dialogs().ok_or("no dialog host")?;
    dialogs.with(|host| host.confirm_by_id(&ctx, id));
    super::import_pdf::pump(app, &ctx);
    match dialogs.with(|host| host.held_state(id)) {
        Some(HeldOk::Accepted) => {
            dialogs.with(|host| host.confirm_by_id(&ctx, id));
            Ok(())
        }
        Some(HeldOk::Refused(reason)) => Err(format!("{reason:?}")),
        _ => Ok(()),
    }
}

/// Three pages, 100, 200 and 300 pt wide; the second has a 50 × 60 art box.
fn three_pages() -> Vec<u8> {
    let mut second = PdfPage::new(200.0, 200.0, "0 0 1 rg 20 20 40 40 re f");
    second.art = Some([10.0, 10.0, 60.0, 70.0]);
    pdf(&[PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f"), second, PdfPage::new(300.0, 100.0, "1 0 0 rg 0 0 30 30 re f")], None)
}

fn artboard_widths(app: &VectorcraftApp) -> Vec<f64> {
    app.session.active().unwrap().doc.artboards.iter().map(|a| a.rect.width()).collect()
}

#[test]
fn opening_a_multi_page_pdf_asks_for_the_pages() {
    let mut app = app();
    io::open_bytes(&mut app, "three.pdf", &three_pages(), Some("/tmp/three.pdf".into())).unwrap();
    assert_eq!(app.session.documents().len(), 1, "nothing opened yet");
    assert_eq!((field(&app, "mode"), field(&app, "pageCount")), (json!("open"), json!(3)));
    assert_eq!(field(&app, "cropTo"), "crop");
    // Page navigation stays within the file.
    set(&mut app, "page", json!(9));
    assert_eq!(field(&app, "page"), 3);
    // A bad range is refused where it is read: nothing opens and the dialog stays.
    set(&mut app, "allPages", json!(false));
    assert!(try_set(&app, "range", json!("2-7")).is_err());
    assert_eq!(app.session.documents().len(), 1);
    assert!(is_open(&app));
    set(&mut app, "range", json!("2-3"));
    confirm(&mut app).unwrap();
    assert!(!is_open(&app), "the dialog and the file it held are let go");
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(artboard_widths(&app), [200.0, 300.0]);
    assert_eq!(app.views.len(), 2, "views follow the documents");
}

#[test]
fn all_pages_and_a_crop_box() {
    let mut app = app();
    io::open_bytes(&mut app, "three.pdf", &three_pages(), None).unwrap();
    set(&mut app, "cropTo", json!("bounding"));
    confirm(&mut app).unwrap();
    assert_eq!(artboard_widths(&app), [10.0, 40.0, 30.0]);
}

/// Each layer's objects in the active document: placed documents (true) or not.
fn placed_pages(app: &VectorcraftApp) -> Vec<bool> {
    let doc = &app.session.active().unwrap().doc;
    doc.layers.iter().flat_map(|l| l.children().cloned().unwrap_or_default()).map(|n| matches!(n.kind, NodeKind::PlacedDocument(_))).collect()
}

#[test]
fn import_pdf_pages_as_links_opens_each_page_linked() {
    let dir = vectorcraft_testkit::TempDir::new("import-pdf-link-pages");
    let path = dir.join("three.pdf").to_string_lossy().into_owned();
    std::fs::write(&path, three_pages()).unwrap();
    let mut app = app();
    // All pages: checked without a click.
    io::open_bytes(&mut app, &path, &three_pages(), Some(path.clone())).unwrap();
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(artboard_widths(&app), [100.0, 200.0, 300.0]);
    assert_eq!(placed_pages(&app), [true, true, true], "All opens the pages linked");
    // A range: cleared, and checked by hand it opens the range linked.
    io::open_bytes(&mut app, &path, &three_pages(), Some(path.clone())).unwrap();
    set(&mut app, "allPages", json!(false));
    set(&mut app, "range", json!("1, 3"));
    assert_eq!(field(&app, "linksOn"), false, "Range: cleared");
    set(&mut app, "linkPages", json!(true));
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(artboard_widths(&app), [100.0, 300.0]);
    assert_eq!(placed_pages(&app), [true, true]);
    // A file given without its path, or opened with a password, can't link its pages: they open as art.
    io::open_bytes(&mut app, "three.pdf", &three_pages(), None).unwrap();
    set(&mut app, "linkPages", json!(true));
    confirm(&mut app).unwrap();
    assert_eq!(placed_pages(&app), [false, false, false]);
    let two = pdf(&[PdfPage::new(10.0, 10.0, "0 g 0 0 5 5 re f"), PdfPage::new(20.0, 10.0, "0 g 0 0 5 5 re f")], Some("pw"));
    let locked = dir.join("two.pdf").to_string_lossy().into_owned();
    std::fs::write(&locked, &two).unwrap();
    io::open_bytes(&mut app, &locked, &two, Some(locked.clone())).unwrap();
    set(&mut app, "password", json!("pw"));
    confirm(&mut app).unwrap();
    set(&mut app, "linkPages", json!(true));
    confirm(&mut app).unwrap();
    assert_eq!(placed_pages(&app), [false, false]);
}

/// Picking All checks Import PDF pages as links and picking Range clears it. A click on the box
/// stands until the other one is picked; picking the one already picked or editing the range keeps
/// it. `ui.dialog.set` picks the way the buttons do.
#[test]
fn picking_all_or_a_range_sets_the_links_default() {
    let mut app = app();
    io::open_bytes(&mut app, "three.pdf", &three_pages(), Some("/tmp/three.pdf".into())).unwrap();
    let on = |app: &VectorcraftApp| field(app, "linksOn") == json!(true);
    assert!(on(&app), "All: checked");
    set(&mut app, "allPages", json!(false));
    assert!(!on(&app), "Range: cleared");
    set(&mut app, "linkPages", json!(true));
    set(&mut app, "allPages", json!(false));
    set(&mut app, "range", json!("1-2"));
    assert!(on(&app), "checked by hand, picking Range again and editing the range keep it");
    set(&mut app, "allPages", json!(true));
    assert!(on(&app), "All: checked");
    set(&mut app, "linkPages", json!(false));
    set(&mut app, "allPages", json!(true));
    assert!(!on(&app), "cleared by hand, picking All again keeps it");
    set(&mut app, "allPages", json!(false));
    set(&mut app, "allPages", json!(true));
    assert!(on(&app), "All picked after Range: checked again");
    assert_eq!(field(&app, "__linkable"), true);
    assert!(try_set(&app, "__linkable", json!(false)).is_err(), "the facts about the file are not editable");
    // A file given without its path can't link its pages: the box stays clear.
    io::open_bytes(&mut app, "three.pdf", &three_pages(), None).unwrap();
    assert!(!on(&app));
}

#[test]
fn single_page_pdfs_open_directly() {
    let mut app = app();
    io::open_bytes(&mut app, "one.pdf", &pdf(&[PdfPage::new(80.0, 40.0, "")], None), None).unwrap();
    assert!(!is_open(&app));
    assert_eq!(artboard_widths(&app), [80.0]);
}

#[test]
fn a_password_protected_pdf_asks_for_its_password() {
    let mut app = app();
    let bytes = pdf(&[PdfPage::new(120.0, 60.0, "1 0 0 rg 0 0 20 20 re f")], Some("pw"));
    io::open_bytes(&mut app, "locked.pdf", &bytes, None).unwrap();
    assert_eq!(field(&app, "__locked"), true);
    set(&mut app, "password", json!("nope"));
    confirm(&mut app).unwrap();
    assert_eq!(field(&app, "__locked"), true, "a wrong password leaves it locked");
    assert!(is_open(&app));
    assert_eq!(app.session.documents().len(), 1, "nothing opened");
    set(&mut app, "password", json!("pw"));
    // One page: unlocking opens it.
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    assert_eq!(artboard_widths(&app), [120.0]);
    // Several pages: unlocking shows the pages first.
    let two = pdf(&[PdfPage::new(10.0, 10.0, ""), PdfPage::new(20.0, 10.0, "")], Some("pw"));
    io::open_bytes(&mut app, "two.pdf", &two, None).unwrap();
    set(&mut app, "password", json!("pw"));
    confirm(&mut app).unwrap();
    assert_eq!((field(&app, "__locked"), field(&app, "pageCount")), (json!(false), json!(2)));
    confirm(&mut app).unwrap();
    assert_eq!(artboard_widths(&app), [10.0, 20.0]);
}

/// The placed object: its kind (clip group or not) and bounds.
fn placed(app: &VectorcraftApp) -> (bool, vectorcraft_geom::Rect) {
    let st = app.session.active().unwrap();
    let n = st.doc.node(st.selection.objects[0]).unwrap();
    let NodeKind::Group { clip, .. } = n.kind else { panic!("{:?}", n.kind) };
    (clip, n.geometric_bounds().unwrap())
}

fn place(app: &mut VectorcraftApp, bytes: &[u8], extra: serde_json::Value) -> Result<serde_json::Value, String> {
    let mut p = json!({"name": "three.pdf", "dataBase64": vectorcraft_format::base64_encode(bytes)});
    if let (Some(o), serde_json::Value::Object(e)) = (p.as_object_mut(), extra) {
        o.extend(e);
    }
    crate::place::run(app, &p)
}

#[test]
fn placing_a_pdf_page_asks_for_the_page_and_crop_box() {
    let mut app = app();
    let bytes = three_pages();
    place(&mut app, &bytes, json!({"at": [100, 120], "link": false})).unwrap();
    assert_eq!((field(&app, "mode"), field(&app, "cropTo")), (json!("place"), json!("crop")));
    assert_eq!(field(&app, "__place"), json!({"at": [100, 120], "link": false}), "the other place params wait in the dialog");
    set(&mut app, "page", json!(2));
    set(&mut app, "cropTo", json!("art"));
    let clipboard = app.session.clipboard.clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    let (clip, b) = placed(&app);
    assert!(clip, "clipped to the art box");
    assert_eq!((b.width(), b.height()), (50.0, 60.0));
    assert_eq!(b.center(), vectorcraft_geom::Point::new(100.0, 120.0), "where `at` asked");
    assert_eq!(app.session.clipboard, clipboard, "the clipboard is untouched");
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // With a page, no dialog; Bounding Box places the art itself.
    place(&mut app, &bytes, json!({"page": 3, "crop": "bounding"})).unwrap();
    assert!(!is_open(&app));
    let (clip, b) = placed(&app);
    assert!(!clip);
    assert_eq!((b.width(), b.height()), (30.0, 30.0));
    assert!(place(&mut app, &bytes, json!({"page": 4})).is_err());
    // A one-page PDF places right away.
    place(&mut app, &pdf(&[PdfPage::new(80.0, 40.0, "0 g 0 0 10 10 re f")], None), json!({})).unwrap();
    assert!(!is_open(&app));
    assert_eq!(placed(&app).1.width(), 80.0);
}

/// A saved `.ai` with several artboards carries its VectorCraft document: opening it restores that
/// document (artboards, layers) instead of asking which pages to import.
#[test]
fn opening_a_saved_ai_with_several_artboards_restores_it_without_asking() {
    let mut app = app();
    app.run("artboard.new", json!({"width": 200, "height": 100, "name": "Second"})).unwrap();
    app.run("layer.new", json!({"name": "Front"})).unwrap();
    app.run("shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 40})).unwrap();
    let saved = app.run("document.save", json!({"format": "ai"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    assert!(vectorcraft_pdf::info(&bytes, None).unwrap().pages.len() > 1, "a page per artboard");

    io::open_bytes(&mut app, "two.ai", &bytes, Some("/tmp/two.ai".into())).unwrap();
    assert!(!is_open(&app), "no Import PDF dialog");
    assert_eq!(app.session.documents().len(), 2);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.artboards.len(), 2);
    assert_eq!(doc.artboards[1].name, "Second");
    assert!(doc.layers.iter().any(|l| l.name.as_deref() == Some("Front")), "layers come back");
}

/// A PDF without a VectorCraft document still asks, and placing a saved `.ai` still asks for the page.
#[test]
fn placing_a_saved_ai_still_asks_for_the_page() {
    let mut app = app();
    app.run("artboard.new", json!({"width": 200, "height": 100})).unwrap();
    let saved = app.run("document.save", json!({"format": "ai"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(saved["dataBase64"].as_str().unwrap()).unwrap();
    assert!(import_pdf::offer(&mut app, "two.ai", &bytes, None, Some(&json!({}))));
    assert_eq!(field(&app, "mode"), "place");
}

/// Two pages, 100 and 200 pt wide; with `editing_data` the first page has the keys of an editor's
/// copy of the art.
fn two_pages(editing_data: bool, password: Option<&str>) -> Vec<u8> {
    let mut first = PdfPage::new(100.0, 100.0, "0 g 0 0 10 10 re f");
    if editing_data {
        first.entries = "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>".into();
    }
    pdf(&[first, PdfPage::new(200.0, 100.0, "1 0 0 rg 0 0 30 30 re f")], password)
}

/// Two 100 pt pages and, on the first, editing data with two artboards to match and a layer of its
/// own, which draws the square the first page draws.
fn two_pages_with_a_layer_in_the_editing_data(layer: &str) -> Vec<u8> {
    let point = |x: i32, y: i32, n: u8| format!("%_{x} {y} /RealPointRelToROrigin\n%_ (PositionPoint{n}) ,\n");
    let board = |top: i32| format!("%_/Dictionary :\n{}{}%_; ,\n", point(0, top, 1), point(100, top - 100, 2));
    let boards = format!("%_/Document :\n%_/Dictionary :\n%_/Array :\n{}{}%_; (ArtboardArray) ,\n%_; /NotRecorded ,\n%_;\n", board(100), board(-50));
    let art = format!("0 0 0 1 k\n{}f\n", ai::rect(10.0, 10.0, 30.0, 30.0));
    let data = ai::editing_data(100.0, 100.0, &format!("{boards}{}", ai::layer(layer, &art)));
    let mut first = PdfPage::new(100.0, 100.0, "0 0 0 1 k 10 10 20 20 re f");
    first.entries = format!("/PieceInfo << /Illustrator << /Private << /AIPrivateData1 {} 0 R /NumBlock 1 >> >> >>", first_extra(2));
    let stream = format!("<< /Length {} >>\nstream\n{data}\nendstream", data.len());
    pdf_with(&[first, PdfPage::new(100.0, 100.0, "")], &[&stream], None)
}

/// #6135: the editing data of a `.ai` holds every artboard, so opening one asks nothing, opens
/// them all and reads the art from that data: the layer is in nothing else.
#[test]
fn opening_an_ai_with_editing_data_reads_it_without_asking() {
    let mut app = app();
    let bytes = two_pages_with_a_layer_in_the_editing_data("Only in the editing data");
    io::open_bytes(&mut app, "two.ai", &bytes, Some("/tmp/two.ai".into())).unwrap();
    assert!(!is_open(&app), "no Import PDF dialog");
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(artboard_widths(&app), [100.0, 100.0]);
    let layers = &app.session.active().unwrap().doc.layers;
    assert!(layers.iter().any(|l| l.name.as_deref() == Some("Only in the editing data")), "read from the editing data");
    assert!(!import_pdf::offer(&mut app, "two.ait", &bytes, None, None), "a template too");
    assert!(!is_open(&app));
}

/// A `.ai` whose editing data can't be read asks nothing either, and every page opens from the PDF
/// part. Asking at that point is #6134.
#[test]
fn opening_an_ai_with_unreadable_editing_data_opens_every_page_without_asking() {
    let mut app = app();
    io::open_bytes(&mut app, "two.ai", &two_pages(true, None), Some("/tmp/two.ai".into())).unwrap();
    assert!(!is_open(&app), "no Import PDF dialog");
    assert_eq!(artboard_widths(&app), [100.0, 200.0]);
}

/// #6135: the dialog stays for what still has something to ask: a PDF, even one with editing data
/// keys, a `.ai` that is only a PDF, a password, and Place.
#[test]
fn the_import_dialog_stays_where_there_is_something_to_ask() {
    for (name, bytes) in [("two.pdf", two_pages(true, None)), ("two.ai", two_pages(false, None))] {
        let mut app = app();
        assert!(import_pdf::offer(&mut app, name, &bytes, None, None), "{name}");
        assert_eq!((field(&app, "mode"), field(&app, "pageCount")), (json!("open"), json!(2)), "{name}");
    }

    let mut app = app();
    assert!(import_pdf::offer(&mut app, "locked.ai", &two_pages(true, Some("pw")), None, None));
    assert_eq!(field(&app, "__locked"), true);

    let mut app = self::app();
    assert!(import_pdf::offer(&mut app, "two.ai", &two_pages(true, None), None, Some(&json!({}))));
    assert_eq!(field(&app, "mode"), "place");
}

/// `pages` pages, each showing "AB" in a font the file names without embedding it.
fn text_pages(pages: usize) -> Vec<u8> {
    let font = format!("/Font << /F1 {} 0 R >>", first_extra(pages));
    let page = PdfPage { resources: font, ..PdfPage::new(100.0, 100.0, "BT /F1 20 Tf 1 0 0 1 10 40 Tm (AB) Tj ET") };
    pdf_with(&vec![page; pages], &["<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>"], None)
}

/// The type objects of the active document.
fn type_objects(app: &VectorcraftApp) -> usize {
    let mut n = 0;
    app.session.active().unwrap().doc.walk(|node| n += usize::from(matches!(node.kind, NodeKind::Text(_))));
    n
}

fn set_pdf_text(app: &mut VectorcraftApp, value: &str) {
    app.run("prefs.set", json!({"key": "pdfTextAs", "value": value})).unwrap();
}

/// The Import PDF dialog is closed; the fonts the open asked about are let go.
fn opened(app: &mut VectorcraftApp) {
    assert!(!is_open(app), "the Import PDF dialog closed");
    app.ui.pending_fonts.clear();
}

/// The Text choice starts from the PDF Text preference (Editable by default), Outlines opens the
/// pages' text as outlines, and OK keeps the choice for the next PDF. Outlines clears and dims
/// Import PDF pages as links (`linkPages` refuses it).
#[test]
fn the_text_choice_opens_text_as_outlines_and_is_kept() {
    let mut app = app();
    assert_eq!(app.session.prefs.pdf_text_as, "text");
    io::open_bytes(&mut app, "two.pdf", &text_pages(2), None).unwrap();
    assert_eq!(field(&app, "textAs"), "text", "Editable by default");
    confirm(&mut app).unwrap();
    opened(&mut app);
    assert_eq!(type_objects(&app), 2, "one type object per page");
    assert_eq!(app.session.prefs.pdf_text_as, "text");

    io::open_bytes(&mut app, "two.pdf", &text_pages(2), None).unwrap();
    set(&mut app, "textAs", json!("outlines"));
    confirm(&mut app).unwrap();
    opened(&mut app);
    assert_eq!(type_objects(&app), 0, "the text is outlines");
    assert_eq!(app.session.prefs.pdf_text_as, "outlines", "OK keeps the choice");

    // The next PDF starts from Outlines; a file it could link opens unlinked while Outlines is picked.
    io::open_bytes(&mut app, "/tmp/two.pdf", &text_pages(2), Some("/tmp/two.pdf".into())).unwrap();
    assert_eq!(field(&app, "textAs"), "outlines");
    assert_eq!(field(&app, "linksOn"), false, "Outlines: not linked");
    set(&mut app, "textAs", json!("text"));
    assert_eq!(field(&app, "linksOn"), true, "Editable: All links the pages again");
}

/// A PDF that opens without the dialog (one page) takes the PDF Text preference, unless the open
/// gives `textAs`; a `.ai` file opens as it says.
#[test]
fn a_pdf_opened_without_the_dialog_takes_the_pdf_text_preference() {
    let mut app = app();
    let one = text_pages(1);
    io::open_bytes(&mut app, "one.pdf", &one, None).unwrap();
    opened(&mut app);
    assert_eq!(type_objects(&app), 1, "Editable by default");
    set_pdf_text(&mut app, "outlines");
    io::open_bytes(&mut app, "one.pdf", &one, None).unwrap();
    opened(&mut app);
    assert_eq!(type_objects(&app), 0, "Outlines");
    io::open_document(&mut app, "one.pdf", &one, None, &json!({"textAs": "text"})).unwrap();
    opened(&mut app);
    assert_eq!(type_objects(&app), 1, "an open that gives textAs keeps it");
    let p = import_pdf::with_preferred_text_as(&app, "one.ai", &one, &serde_json::Value::Null);
    assert!(p.is_null(), "a .ai file opens as it says: {p}");
    // In the dialog too, a .ai file starts from Editable, and its choice is not kept for PDFs.
    assert!(import_pdf::offer(&mut app, "two.ai", &two_pages(false, None), None, None));
    assert_eq!(field(&app, "textAs"), "text", "a .ai file starts from Editable");
    confirm(&mut app).unwrap();
    opened(&mut app);
    assert_eq!(app.session.prefs.pdf_text_as, "outlines", "the preference stays");
    // An open with linkPages keeps the file's own text, which linked pages show.
    let dir = vectorcraft_testkit::TempDir::new("import-pdf-text-link-pages");
    let path = dir.join("one.pdf").to_string_lossy().into_owned();
    std::fs::write(&path, &one).unwrap();
    io::open_document(&mut app, &path, &one, Some(path.clone()), &json!({"linkPages": true})).unwrap();
    opened(&mut app);
    assert_eq!(placed_pages(&app), [true], "opened linked");
}

/// Placing a page embedded takes the Text choice (and the preference when no dialog asks); a
/// linked place shows the file's own text, so the choice is dimmed and not sent.
#[test]
fn the_text_choice_applies_to_embedded_places_only() {
    let mut app = app();
    // `textAs: null` leaves the param out: the dialog's choice is sent.
    place(&mut app, &text_pages(2), json!({"link": false, "textAs": null})).unwrap();
    assert_eq!(field(&app, "textAs"), "text");
    set(&mut app, "textAs", json!("outlines"));
    confirm(&mut app).unwrap();
    opened(&mut app);
    assert_eq!(type_objects(&app), 0, "placed as outlines");
    assert_eq!(app.session.prefs.pdf_text_as, "outlines");
    // One page asks nothing: placed by the user, it takes the preference.
    let one = json!({"name": "one.pdf", "dataBase64": vectorcraft_format::base64_encode(&text_pages(1))});
    crate::place::run_for_user(&mut app, &one).unwrap();
    assert_eq!(type_objects(&app), 0, "the preference");
    set_pdf_text(&mut app, "text");
    crate::place::run_for_user(&mut app, &one).unwrap();
    assert_eq!(type_objects(&app), 1);

    // Linked: the choice is not sent, and OK leaves the preference as it is.
    set_pdf_text(&mut app, "outlines");
    let dir = vectorcraft_testkit::TempDir::new("import-pdf-text-linked");
    let path = dir.join("two.pdf").to_string_lossy().into_owned();
    std::fs::write(&path, text_pages(2)).unwrap();
    assert!(import_pdf::offer(&mut app, &path, &text_pages(2), Some(path.clone()), Some(&json!({"link": true}))));
    confirm(&mut app).unwrap();
    let st = app.session.active().unwrap();
    let placed = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(matches!(placed.kind, NodeKind::PlacedDocument(_)), "linked: {:?}", placed.kind);
    assert_eq!(app.session.prefs.pdf_text_as, "outlines");

    // A place that gives `textAs: "outlines"` with Link on is embedded as outlines, as without the dialog.
    set_pdf_text(&mut app, "text");
    assert!(import_pdf::offer(&mut app, &path, &text_pages(2), Some(path.clone()), Some(&json!({"link": true, "textAs": "outlines"}))));
    assert_eq!(field(&app, "textAs"), "outlines");
    let before = type_objects(&app);
    confirm(&mut app).unwrap();
    opened(&mut app);
    let st = app.session.active().unwrap();
    let placed = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(!matches!(placed.kind, NodeKind::PlacedDocument(_)), "embedded: {:?}", placed.kind);
    assert_eq!(type_objects(&app), before, "placed as outlines");
}

/// `file.place` run as a command (MCP, scripts) without the dialog takes the PDF Text preference for
/// an embedded PDF with no `textAs`, as a place by the user does; a `textAs` given and a linked
/// place keep their own text.
#[test]
fn file_place_without_the_dialog_takes_the_pdf_text_preference() {
    let mut app = app();
    set_pdf_text(&mut app, "outlines");
    let dir = vectorcraft_testkit::TempDir::new("import-pdf-text-at-once");
    let path = dir.join("two.pdf").to_string_lossy().into_owned();
    std::fs::write(&path, text_pages(2)).unwrap();
    let embedded = json!({"path": path, "link": false, "showImportOptions": false});
    crate::place::run(&mut app, &embedded).unwrap();
    assert!(!is_open(&app), "no dialog");
    assert_eq!(type_objects(&app), 0, "the preference: outlines");

    let one = json!({"name": "one.pdf", "dataBase64": vectorcraft_format::base64_encode(&text_pages(1))});
    crate::place::run(&mut app, &one).unwrap();
    assert_eq!(type_objects(&app), 0, "one page from bytes: outlines");

    let mut given = embedded.clone();
    given["textAs"] = json!("text");
    crate::place::run(&mut app, &given).unwrap();
    assert_eq!(type_objects(&app), 1, "a textAs given is kept");

    crate::place::run(&mut app, &json!({"path": path, "showImportOptions": false})).unwrap();
    let st = app.session.active().unwrap();
    let placed = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(matches!(placed.kind, NodeKind::PlacedDocument(_)), "linked: {:?}", placed.kind);

    // `file.place` reads an empty password as none, so the page stays linked.
    crate::place::run(&mut app, &json!({"path": path, "password": "", "showImportOptions": false})).unwrap();
    let st = app.session.active().unwrap();
    let placed = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(matches!(placed.kind, NodeKind::PlacedDocument(_)), "an empty password: linked: {:?}", placed.kind);

    set_pdf_text(&mut app, "text");
    crate::place::run(&mut app, &embedded).unwrap();
    assert_eq!(type_objects(&app), 2, "Editable: page 1's type object");
}

/// PDF bytes placed without a `name`, or under another extension, are a PDF by their content, as
/// `file.place` reads them, and take the PDF Text preference; under a `.ai` name they are placed as
/// the file says.
#[test]
fn pdf_bytes_without_a_pdf_name_take_the_pdf_text_preference() {
    let mut app = app();
    set_pdf_text(&mut app, "outlines");
    let data = vectorcraft_format::base64_encode(&text_pages(1));
    crate::place::run(&mut app, &json!({"dataBase64": data, "showImportOptions": false})).unwrap();
    assert!(!is_open(&app), "no dialog");
    assert_eq!(type_objects(&app), 0, "unnamed bytes: outlines");

    crate::place::run(&mut app, &json!({"name": "page.bin", "dataBase64": data, "showImportOptions": false})).unwrap();
    assert_eq!(type_objects(&app), 0, "another extension: outlines");

    crate::place::run(&mut app, &json!({"name": "page.ai", "dataBase64": data, "showImportOptions": false})).unwrap();
    assert_eq!(type_objects(&app), 1, "a .ai file: its own text");
}

/// With Show Import Options off, an encrypted PDF placed with an empty `password` asks for its
/// password, as one placed without a `password` does.
#[test]
fn an_empty_password_asks_for_the_password_of_an_encrypted_pdf() {
    let mut app = app();
    let bytes = pdf(&[PdfPage::new(120.0, 60.0, "1 0 0 rg 0 0 20 20 re f")], Some("pw"));
    place(&mut app, &bytes, json!({"password": "", "showImportOptions": false})).unwrap();
    assert_eq!((field(&app, "__locked"), field(&app, "__passwordOnly")), (json!(true), json!(true)));
    set(&mut app, "password", json!("pw"));
    confirm(&mut app).unwrap();
    assert!(!is_open(&app));
    let st = app.session.active().unwrap();
    let b = st.doc.node(st.selection.objects[0]).and_then(|n| n.geometric_bounds()).unwrap();
    assert!((b.width() - 120.0).abs() < 0.01, "placed once unlocked: {}", b.width());
}

/// `file.place` reads an empty or null `password` as none: a place from a path with Link on stays
/// linked with the PDF Text preference at Outlines, and only a password that is given embeds it.
#[test]
fn an_empty_password_keeps_a_place_linked() {
    let mut app = app();
    set_pdf_text(&mut app, "outlines");
    let dir = vectorcraft_testkit::TempDir::new("import-pdf-empty-password");
    let path = dir.join("one.pdf").to_string_lossy().into_owned();
    std::fs::write(&path, text_pages(1)).unwrap();
    for password in [json!(""), json!(null)] {
        let mut p = json!({"path": path, "password": password});
        import_pdf::place_with_preferred_text_as(&app, &mut p);
        assert_eq!(p, json!({"path": path, "password": password}), "no textAs with password {password}");
    }
    let mut p = json!({"path": path, "password": "pw"});
    import_pdf::place_with_preferred_text_as(&app, &mut p);
    assert_eq!(p["textAs"], "outlines", "a password embeds the page");

    crate::place::run_for_user(&mut app, &json!({"path": path, "password": ""})).unwrap();
    opened(&mut app);
    let st = app.session.active().unwrap();
    let placed = st.doc.node(st.selection.objects[0]).unwrap();
    assert!(matches!(placed.kind, NodeKind::PlacedDocument(_)), "linked: {:?}", placed.kind);
}

/// With Show Import Options off, an encrypted PDF placed with an empty `password` asks for its
/// password, as one placed without `password` does; a password that is given goes to `file.place`.
#[test]
fn an_empty_password_still_asks_for_the_password() {
    let mut app = app();
    let bytes = pdf(&[PdfPage::new(120.0, 60.0, "1 0 0 rg 0 0 20 20 re f")], Some("pw"));
    let data = vectorcraft_format::base64_encode(&bytes);
    let close = |app: &VectorcraftApp| {
        let id = dialog_id(app).expect("an Import PDF dialog");
        assert!(app.dialogs().unwrap().with(|host| host.cancel_by_id(&ctx(app), id)));
        assert!(!is_open(app));
    };
    for password in [json!(""), json!(null)] {
        let p = json!({"name": "locked.pdf", "dataBase64": data, "password": password});
        assert!(import_pdf::offer_place_password(&mut app, &p), "asks with password {password}");
        assert_eq!((field(&app, "__locked"), field(&app, "__passwordOnly")), (json!(true), json!(true)));
        close(&app);
    }
    let p = json!({"name": "locked.pdf", "dataBase64": data, "password": "pw"});
    assert!(!import_pdf::offer_place_password(&mut app, &p), "a given password is placed with");
    assert!(!is_open(&app));
}
