//! Windows metafiles another app copied paste as vector art (M4.95, through the system clipboard
//! service of M4.57).

use serde_json::json;
use vectorcraft_doc::NodeKind;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::clipboard::{AI, EMF, EPS, Flavour, PASTE_ORDER, PDF, PNG, TEXT, file_flavour};
use vectorcraft_testkit::ai;

use crate::sysclip::import_command;
use crate::tests_sysclip::{app, blue_png, copy_elsewhere, pasted, run};

#[test]
fn an_emf_from_another_app_pastes_as_vectors_ahead_of_its_bitmap() {
    let (mut app, board) = app();
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    other.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let emf = other.execute("document.export", &json!({"format": "emf"})).unwrap();
    let emf = vectorcraft_format::base64_decode(emf["dataBase64"].as_str().unwrap()).unwrap();
    let flavour = Flavour { mime: EMF, data: emf.clone() };
    assert_eq!(import_command(&flavour, None).0, "clipboard.importEmf");
    copy_elsewhere(&board, vec![(PNG, blue_png()), (EMF, emf)]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty() && p.iter().all(|n| !matches!(n.kind, NodeKind::Image(_))), "{p:?}");
    let b = p[0].geometric_bounds().unwrap();
    assert!((b.width() - 30.0).abs() < 0.05, "{b:?}");
}

/// A 30 pt red square, as an EPS written by hand from the EPSF specification.
const SQUARE_EPS: &str = "%!PS-Adobe-3.0 EPSF-3.0\n%%BoundingBox: 0 0 100 100\n%%EndComments\n1 0 0 setrgbcolor\nnewpath 10 10 moveto 40 10 lineto 40 40 lineto 10 40 lineto closepath fill\nshowpage\n%%EOF\n";

/// An EPS another app copied (an equation editor's, a drawing app's) pastes as editable paths, not
/// as the bitmap or the text beside it, and not as what was copied in the app before (#1499).
#[test]
fn an_eps_from_another_app_pastes_as_vectors() {
    let (mut app, board) = app();
    crate::tests_sysclip::copy_rect(&mut app);
    let eps = SQUARE_EPS.as_bytes().to_vec();
    assert_eq!(import_command(&Flavour { mime: EPS, data: eps.clone() }, None).0, "clipboard.importEps");
    copy_elsewhere(&board, vec![(PNG, blue_png()), (TEXT, b"x = 1".to_vec()), (EPS, eps.clone())]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty(), "something pasted");
    let path = |n: &vectorcraft_doc::Node| matches!(n.kind, NodeKind::Path { .. });
    let mut paths = vec![];
    p.iter().for_each(|n| n.walk(&mut |c| paths.extend(path(c).then(|| c.geometric_bounds().unwrap()))));
    assert_eq!(paths.len(), 1, "{p:?}");
    assert!((paths[0].width() - 30.0).abs() < 0.05 && (paths[0].height() - 30.0).abs() < 0.05, "{paths:?}");
    assert!(p.iter().all(|n| !matches!(n.kind, NodeKind::Image(_) | NodeKind::Text(_))), "{p:?}");
    // A PDF beside it is the better copy: the apps that offer both mean the same picture.
    assert!(PASTE_ORDER.iter().position(|m| *m == PDF) < PASTE_ORDER.iter().position(|m| *m == EPS));
    // Not PostScript: refused, nothing pasted over.
    let pdf = vectorcraft_format::base64_encode(b"%PDF-1.7");
    assert!(app.session.execute("clipboard.importEps", &json!({"dataBase64": pdf})).is_err());
    // An .eps file copied in a file manager is that flavour too (told by its bytes, not its name).
    assert_eq!(file_flavour("equation.eps", &eps, &PASTE_ORDER), Some(EPS));
    assert_eq!(file_flavour("equation.dat", &eps, &PASTE_ORDER), Some(EPS));
    assert_eq!(file_flavour("equation.eps", &eps, &[TEXT]), None);
}

/// A `.ai` copied in a file manager pastes the objects of its editing data, named and in their
/// layers' stacking order, not its page's.
#[test]
fn a_copied_ai_file_pastes_from_its_editing_data() {
    let file = ai::ai(&ai::sample_data(), ai::page_pdf());
    assert_eq!(file_flavour("art.ai", &file, &PASTE_ORDER), Some(AI));
    let (cmd, params) = import_command(&Flavour { mime: AI, data: file.clone() }, None);
    assert_eq!((cmd, &params["editingData"]), ("clipboard.importPdf", &json!(true)));
    let (mut app, _) = app();
    app.paste_from_host(&egui::Context::default(), Flavour { mime: AI, data: file }, egui::Modifiers::COMMAND);
    let names: Vec<String> = pasted(&app).iter().map(|n| n.name.clone().unwrap_or_default()).collect();
    assert_eq!(names, ["Pair", "Ring", "Window", "Shaded", "", "Sub", ""]);
}

/// An ellipse exported as an EMF, the picture of copied cells a Windows spreadsheet offers.
fn ellipse_emf() -> Vec<u8> {
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    other.execute("shape.ellipse", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap();
    let emf = other.execute("document.export", &json!({"format": "emf"})).unwrap();
    vectorcraft_format::base64_decode(emf["dataBase64"].as_str().unwrap()).unwrap()
}

/// The pasted type objects' text, in order.
fn pasted_type(app: &crate::VectorcraftApp) -> Vec<String> {
    let mut texts = vec![];
    pasted(app).iter().for_each(|n| {
        n.walk(&mut |c| {
            if let NodeKind::Text(t) = &c.kind {
                texts.push(t.plain_text());
            }
        })
    });
    texts
}

/// Cells copied from a desktop spreadsheet: tab-separated text beside a picture of the cells (an
/// EMF and a bitmap on Windows, a PDF on macOS). The text wins and pastes as a table (#5942).
#[test]
fn cells_from_a_desktop_spreadsheet_paste_as_a_table_ahead_of_their_picture() {
    let cells = b"Item\tQty\r\nApples\t3\r\nPears\t5\r\n".to_vec();
    let (mut windows, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png()), (EMF, ellipse_emf()), (TEXT, cells.clone())]);
    run(&mut windows, "edit.paste", json!({}));
    assert_eq!(pasted_type(&windows), ["Item", "Qty", "Apples", "3", "Pears", "5"]);
    let (mut app, board) = app();
    let mut other = Session::new();
    other.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let id = other.execute("shape.ellipse", &json!({"x": 0, "y": 0, "width": 30, "height": 30})).unwrap()["id"].clone();
    other.execute("select.set", &json!({"ids": [id]})).unwrap();
    other.execute("edit.copy", &json!({})).unwrap();
    let (pdf, _) = other.clipboard_pdf().unwrap().unwrap();
    copy_elsewhere(&board, vec![(PDF, pdf), (PNG, blue_png()), (TEXT, cells)]);
    run(&mut app, "edit.paste", json!({}));
    assert_eq!(pasted_type(&app), ["Item", "Qty", "Apples", "3", "Pears", "5"]);
}

/// A vector picture with text that is not a table beside it (a drawing app's copy) still pastes
/// the picture.
#[test]
fn a_vector_picture_with_plain_text_beside_it_still_pastes_the_picture() {
    let (mut app, board) = app();
    copy_elsewhere(&board, vec![(PNG, blue_png()), (EMF, ellipse_emf()), (TEXT, b"Ellipse 30 x 30".to_vec())]);
    run(&mut app, "edit.paste", json!({}));
    let p = pasted(&app);
    assert!(!p.is_empty() && pasted_type(&app).is_empty(), "{p:?}");
    // Tab-separated text that reads as SVG markup is not taken over the picture: it would load as
    // SVG, fail, and paste nothing.
    let (mut markup, board) = self::app();
    copy_elsewhere(&board, vec![(PNG, blue_png()), (EMF, ellipse_emf()), (TEXT, b"<svg\tQty\nItem\t2".to_vec())]);
    run(&mut markup, "edit.paste", json!({}));
    let p = pasted(&markup);
    assert!(!p.is_empty() && pasted_type(&markup).is_empty(), "{p:?}");
    // One column of lines, or text indented with tabs, is not a table either.
    for text in ["a\nb\nc", "\tindented\n\tlines"] {
        let f = Flavour { mime: EMF, data: vec![] };
        let read = |_: &[&'static str]| Some(crate::sysclip::ClipboardRead::Now(Flavour { mime: TEXT, data: text.as_bytes().to_vec() }));
        assert!(crate::sysclip::spreadsheet_text(&f, read).is_none(), "{text:?}");
    }
    // Only a vector picture gives way: an SVG keeps its place.
    let read = |_: &[&'static str]| Some(crate::sysclip::ClipboardRead::Now(Flavour { mime: TEXT, data: b"a\tb".to_vec() }));
    assert!(crate::sysclip::spreadsheet_text(&Flavour { mime: vectorcraft_engine::cmd::clipboard::SVG, data: vec![] }, read).is_none());
}
