//! PostScript print jobs: DSC page comments, the setup of each sheet, and a file the PostScript
//! reader reads back.

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId};
use vectorcraft_geom::{Affine, Rect, shapes};

use super::*;

/// A 200 × 100 pt document with a red rectangle.
fn doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    let layer = d.layers[0].id;
    let look = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let n = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(20.0, 20.0, 120.0, 60.0)), look);
    d.insert(Some(layer), 0, n).unwrap();
    d
}

/// The document on a letter page, 36 pt in from its top-left corner.
fn page<'a>(d: &'a Document, marks: Option<&'a Node>) -> PrintPage<'a> {
    PrintPage {
        doc: d,
        size: (612.0, 792.0),
        view: Affine::IDENTITY,
        window: None,
        place: Affine::translate((36.0, 36.0)),
        copies: &[],
        area: d.artboards[0].rect,
        marks,
        negative: false,
        ink: None,
    }
}

fn text(out: &EpsOutput) -> String {
    String::from_utf8(out.bytes.clone()).unwrap()
}

#[test]
fn pages_follow_dsc_and_repeat_for_copies() {
    let d = doc();
    let mark =
        Node::path(NodeId(9999), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::None, Paint::registration(), 0.25));
    let pages = [page(&d, None), PrintPage { size: (792.0, 612.0), ..page(&d, Some(&mark)) }];
    let job = PrintJob { title: "Job".into(), created: Some(0), ..PrintJob::default() };
    let out = print(&pages, &[0, 1, 0, 1], &job).unwrap();
    let ps = text(&out);
    assert!(ps.starts_with("%!PS-Adobe-3.0\n") && ps.ends_with("%%EOF\n"));
    let dsc = |key: &str| ps.lines().filter_map(|l| l.strip_prefix(key)).map(str::trim).collect::<Vec<_>>();
    assert_eq!(dsc("%%Pages:"), ["4"]);
    assert_eq!(dsc("%%Page:"), ["1 1", "2 2", "3 3", "4 4"]);
    assert_eq!(dsc("%%PageBoundingBox:"), ["0 0 612 792", "0 0 792 612", "0 0 612 792", "0 0 792 612"]);
    assert_eq!(dsc("%%BoundingBox:"), ["0 0 792 792"]);
    assert_eq!(dsc("%%LanguageLevel:"), ["3"]);
    assert_eq!(ps.matches("showpage").count(), 4);
    assert!(ps.contains("<< /PageSize [792 612] >> setpagedevice"));
    // The marks are drawn in Registration on the pages that have them.
    assert_eq!(ps.matches("/All /DeviceCMYK").count(), 2, "{ps}");
    assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    // The PostScript reader reads the first page back: the rectangle 36 pt in, clipped to the
    // artboard.
    let back = crate::import(&out.bytes).unwrap();
    let art = back.document.layers[0].children().unwrap().clone();
    let clipped = art[0].children().unwrap();
    assert_eq!(clipped[0].geometric_bounds().unwrap(), Rect::new(36.0, 36.0, 236.0, 136.0));
    assert_eq!(clipped[1].geometric_bounds().unwrap(), Rect::new(56.0, 56.0, 156.0, 96.0));
    assert!(back.warnings.iter().any(|w| w.contains("first of the file's 4 pages")), "{:?}", back.warnings);
}

#[test]
fn copies_on_a_sheet_draw_the_art_and_marks_once_each() {
    let d = doc();
    let mark =
        Node::path(NodeId(9999), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::basic(Paint::None, Paint::registration(), 0.25));
    let moves = [kurbo::Vec2::ZERO, kurbo::Vec2::new(218.0, 0.0), kurbo::Vec2::new(0.0, 118.0)];
    let sheet = PrintPage { copies: &moves, ..page(&d, Some(&mark)) };
    let ps = text(&print(&[sheet], &[0], &PrintJob::default()).unwrap());
    // The art is placed three times, the second 218 pt right and the third 118 pt down, and its
    // spot color space and the marks' are defined once.
    assert_eq!(ps.matches("[1 0 0 1 36 36] cm\n").count(), 1, "{ps}");
    assert!(ps.contains("[1 0 0 1 254 36] cm\n") && ps.contains("[1 0 0 1 36 154] cm\n"), "{ps}");
    assert_eq!(ps.matches("/All /DeviceCMYK").count(), 1, "{ps}");
    assert!(ps.contains("q\n[1 0 0 1 218 0] cm\n") && ps.contains("q\n[1 0 0 1 0 118] cm\n"), "{ps}");
    // A mark off the page around the first copy still prints around a later copy that has it on the page.
    let look = Appearance::basic(Paint::None, Paint::registration(), 0.25);
    let off = Node::path(NodeId(9998), shapes::rectangle(Rect::new(-100.0, -100.0, -90.0, -90.0)), look);
    let later = [kurbo::Vec2::ZERO, kurbo::Vec2::new(200.0, 200.0)];
    let ps = text(&print(&[PrintPage { copies: &later, ..page(&d, Some(&off)) }], &[0], &PrintJob::default()).unwrap());
    assert_eq!(ps.matches("/All /DeviceCMYK").count(), 1, "{ps}");
    // One copy unmoved is the page without copies.
    let one = [kurbo::Vec2::ZERO];
    let single = text(&print(&[PrintPage { copies: &one, ..page(&d, Some(&mark)) }], &[0], &PrintJob::default()).unwrap());
    assert_eq!(single, text(&print(&[page(&d, Some(&mark))], &[0], &PrintJob::default()).unwrap()));
}

#[test]
fn separations_screens_flatness_negatives_and_tiles() {
    let d = doc();
    let sheet = PrintPage { negative: true, ink: Some(("Cyan", 85.0, 15.0)), window: Some(Rect::new(0.0, 0.0, 100.0, 100.0)), ..page(&d, None) };
    let job = PrintJob { level: Level::Two, flatness: Some(3.0), ..PrintJob::default() };
    let ps = text(&print(&[sheet], &[0], &job).unwrap());
    assert!(ps.contains("%%PlateColor: Cyan\n"));
    assert!(ps.contains("85 15 {dup mul exch dup mul add 1 exch sub} setscreen"));
    assert!(ps.contains("{1 exch sub} settransfer 1 g 0 0 612 792 rectfill"));
    assert!(ps.contains("3 setflat"));
    assert!(ps.contains("%%LanguageLevel: 2"));
    // The tile clips the page.
    assert!(ps.contains("0 0 m\n100 0 l\n100 100 l\n0 100 l\n"), "{ps}");
    assert!(print(&[], &[], &job).is_err());
    assert!(print(&[PrintPage { size: (f64::NAN, 1.0), ..page(&d, None) }], &[0], &job).is_err());
}
