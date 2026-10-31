//! Import options: the pages picked, the box each artboard gets (Crop To), passwords and `info`.

use vectorcraft_doc::{Document, NodeKind};
use vectorcraft_geom::Rect;
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf, pdf_with};

use crate::*;

/// A 600 × 800 page with every box set and a 200 × 100 rectangle at (120, 150) (PDF space).
fn boxed_page() -> PdfPage {
    PdfPage {
        crop: Some([50.0, 50.0, 550.0, 750.0]),
        bleed: Some([40.0, 40.0, 560.0, 760.0]),
        trim: Some([60.0, 60.0, 540.0, 740.0]),
        art: Some([100.0, 100.0, 300.0, 400.0]),
        ..PdfPage::new(600.0, 800.0, "0 0 1 rg 120 150 200 100 re f")
    }
}

fn open(bytes: &[u8], opts: ImportOptions) -> Document {
    import_with_report(bytes, &opts).unwrap().document
}

fn art(d: &Document) -> Rect {
    d.art_bounds().unwrap()
}

fn close(a: Rect, b: Rect) -> bool {
    [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|v| v.abs() < 1e-6)
}

#[test]
fn each_crop_box_sizes_the_artboard_and_places_the_art_in_it() {
    let bytes = pdf(&[boxed_page()], None);
    // (box, artboard size, the rectangle's top-left on the artboard).
    let cases = [
        (CropTo::Media, (600.0, 800.0), (120.0, 550.0)),
        (CropTo::Crop, (500.0, 700.0), (70.0, 500.0)),
        (CropTo::Bleed, (520.0, 720.0), (80.0, 510.0)),
        (CropTo::Trim, (480.0, 680.0), (60.0, 490.0)),
        (CropTo::Art, (200.0, 300.0), (20.0, 150.0)),
        (CropTo::Bounding, (200.0, 100.0), (0.0, 0.0)),
    ];
    for (crop, (w, h), (x, y)) in cases {
        let d = open(&bytes, ImportOptions { crop, ..Default::default() });
        let ab = d.artboards[0].rect;
        assert!((ab.width() - w).abs() < 1e-6 && (ab.height() - h).abs() < 1e-6, "{crop:?}: {ab:?}");
        let r = art(&d);
        assert!(close(r, Rect::new(ab.x0 + x, ab.y0 + y, ab.x0 + x + 200.0, ab.y0 + y + 100.0)), "{crop:?}: art {r:?} on {ab:?}");
    }
    // The default is the crop box, as before.
    assert_eq!(ImportOptions::default().crop, CropTo::Crop);
}

/// 10 mm in points.
const BLEED: f64 = 10.0 * 72.0 / 25.4;

/// A 300 × 200 sheet with a 200 × 100 trim box, the bleed box 10 mm round it, and a red rectangle
/// reaching 20 pt past the trim box's left and bottom edges into the bleed.
fn bled_page() -> PdfPage {
    PdfPage {
        bleed: Some([50.0 - BLEED, 50.0 - BLEED, 250.0 + BLEED, 150.0 + BLEED]),
        trim: Some([50.0, 50.0, 250.0, 150.0]),
        ..PdfPage::new(300.0, 200.0, "1 0 0 rg 30 30 100 60 re f")
    }
}

/// The same within the precision of a box read from a file (32-bit floats).
fn near(a: [f64; 4], b: [f64; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-3)
}

#[test]
fn trim_box_artboards_take_the_bleed_box_round_them_as_the_document_bleed() {
    let bytes = pdf(&[bled_page()], None);
    let trim = ImportOptions { crop: CropTo::Trim, ..Default::default() };
    let d = open(&bytes, trim.clone());
    let ab = d.artboards[0].rect;
    assert!(close(ab, Rect::new(0.0, 0.0, 200.0, 100.0)), "{ab:?}");
    assert!(near(d.setup.bleed, [BLEED; 4]), "{:?}", d.setup.bleed);
    // The rectangle comes in whole, reaching into the bleed (y down: the bottom bleed is below).
    assert!(close(art(&d), Rect::new(-20.0, 60.0, 80.0, 120.0)), "{:?}", art(&d));
    assert!(d.setup.bleed_rect(ab).contains_rect(art(&d)));
    // Each side as the page shows it: PDF space is y up, and a rotated page turns its bleed.
    let uneven = PdfPage { bleed: Some([49.0, 48.0, 253.0, 154.0]), ..bled_page() };
    let turned = PdfPage { rotate: 90, ..uneven.clone() };
    assert!(near(open(&pdf(&[uneven], None), trim.clone()).setup.bleed, [4.0, 2.0, 1.0, 3.0]));
    assert!(near(open(&pdf(&[turned], None), trim).setup.bleed, [1.0, 3.0, 2.0, 4.0]));
    // On the crop box (the sheet here) there is no bleed round the artboard.
    let d = open(&bytes, ImportOptions::default());
    assert_eq!((d.artboards[0].rect.width(), d.setup.bleed), (300.0, [0.0; 4]));
}

#[test]
fn missing_boxes_fall_back_to_the_crop_box_and_boxes_are_clipped_to_the_media_box() {
    let page = PdfPage { crop: Some([10.0, 10.0, 290.0, 390.0]), art: Some([-50.0, 0.0, 100.0, 100.0]), ..PdfPage::new(300.0, 400.0, "") };
    let bytes = pdf(&[page], None);
    for crop in [CropTo::Bleed, CropTo::Trim] {
        let ab = open(&bytes, ImportOptions { crop, ..Default::default() }).artboards[0].rect;
        assert_eq!((ab.width(), ab.height()), (280.0, 380.0), "{crop:?}");
    }
    let ab = open(&bytes, ImportOptions { crop: CropTo::Art, ..Default::default() }).artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (100.0, 100.0));
    // A page without art keeps its crop box for Bounding.
    let ab = open(&bytes, ImportOptions { crop: CropTo::Bounding, ..Default::default() }).artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (280.0, 380.0));
}

#[test]
fn a_rotated_page_rotates_its_boxes() {
    let page = PdfPage { rotate: 90, ..boxed_page() };
    let d = open(&pdf(&[page], None), ImportOptions { crop: CropTo::Art, ..Default::default() });
    let ab = d.artboards[0].rect;
    assert_eq!((ab.width(), ab.height()), (300.0, 200.0));
    // The rectangle turns with the page (clockwise): PDF (x, y) shows at (y − 100, x − 100) on
    // the art box's artboard.
    assert!(close(art(&d), Rect::new(50.0, 20.0, 150.0, 220.0)), "{:?}", art(&d));
}

#[test]
fn pages_pick_which_pages_import_and_in_what_order() {
    let pages: Vec<PdfPage> = (1..=4).map(|i| PdfPage::new(100.0 * i as f64, 100.0, "0 g 0 0 10 10 re f")).collect();
    let bytes = pdf(&pages, None);
    let d = open(&bytes, ImportOptions { pages: Some(vec![1, 2]), ..Default::default() });
    let widths: Vec<f64> = d.artboards.iter().map(|a| a.rect.width()).collect();
    assert_eq!(widths, [200.0, 300.0]);
    let names: Vec<&str> = d.layers.iter().map(|l| l.name.as_deref().unwrap_or_default()).collect();
    assert_eq!(names, ["Page 2", "Page 3"]);
    assert_eq!(d.artboards.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Artboard 1", "Artboard 2"]);
    // Artboards sit side by side with the gap between them.
    assert_eq!(d.artboards[1].rect.x0, d.artboards[0].rect.x1 + 36.0);
    let d = open(&bytes, ImportOptions { pages: Some(vec![3, 0]), max_pages: Some(1), ..Default::default() });
    assert_eq!(d.artboards.len(), 1);
    assert_eq!(d.artboards[0].rect.width(), 400.0);
    assert_eq!(import_with_report(&bytes, &ImportOptions { pages: Some(vec![4]), ..Default::default() }).unwrap_err(), PdfError::BadPage(5, 4));
    assert_eq!(import_with_report(&bytes, &ImportOptions { pages: Some(vec![]), ..Default::default() }).unwrap_err(), PdfError::NoPages);
}

#[test]
fn bounding_boxes_of_several_pages_do_not_overlap() {
    let page = |x: f64| PdfPage::new(300.0, 300.0, &format!("0 g {x} 10 50 50 re f"));
    let d = open(&pdf(&[page(240.0), page(0.0)], None), ImportOptions { crop: CropTo::Bounding, ..Default::default() });
    let (a, b) = (d.artboards[0].rect, d.artboards[1].rect);
    assert_eq!((a.width(), b.width()), (50.0, 50.0));
    assert!(b.x0 >= a.x1 + 36.0, "{a:?} {b:?}");
}

#[test]
fn art_of_neighbouring_artboards_off_a_page_is_left_to_its_own_page() {
    // Page 2 also draws page 1's square, off to its left (outside its box), as files with several
    // artboards are written: only page 1 keeps it, so it doesn't come back twice.
    let pages = [PdfPage::new(100.0, 100.0, "0 g 10 10 20 20 re f"), PdfPage::new(100.0, 100.0, "0 g -90 10 20 20 re f 1 0 0 rg 50 10 20 20 re f")];
    let d = open(&pdf(&pages, None), ImportOptions::default());
    let count = |i: usize| d.layers[i].children().map_or(0, |c| c.len());
    assert_eq!((count(0), count(1)), (1, 1));
    // A single page keeps its art off the page (the pasteboard).
    let d = open(&pdf(&pages[1..], None), ImportOptions::default());
    assert_eq!(d.layers[0].children().map_or(0, |c| c.len()), 2);
}

/// A `.ai` keeps its art past the artboard edge (#646): a note mostly left of artboard 2 (as notes
/// for a design are put beside it) doesn't lie over artboard 1 in the row, where it would be taken
/// for that page's art: page 2 moves right until its art clears page 1's.
#[test]
fn art_of_an_ai_page_reaching_left_does_not_lie_over_the_page_before() {
    let ai = "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>";
    let page = |content: &str| PdfPage { entries: ai.into(), ..PdfPage::new(100.0, 100.0, content) };
    // Page 2's note runs from 70 points left of its box to 10 points into it.
    let pages = [page("0 g 10 10 20 20 re f"), page("1 1 1 rg 1 0 0 RG -70 10 80 60 re B")];
    let d = open(&pdf(&pages, None), ImportOptions::default());
    let (a, b) = (d.artboards[0].rect, d.artboards[1].rect);
    let note = d.layers[1].children().unwrap()[0].visual_bounds().unwrap();
    assert!(note.x0 >= a.x1 + 36.0, "the note clears artboard 1: {note:?} {a:?}");
    assert!((b.x0 - note.x0 - 70.0).abs() < 1.0, "and keeps its place by artboard 2: {note:?} {b:?}");
    assert_eq!((b.width(), b.height()), (100.0, 100.0));
    // A plain PDF clips its art to the page: its pages stay 36 points apart.
    let plain = [PdfPage::new(100.0, 100.0, "0 g 10 10 20 20 re f"), PdfPage::new(100.0, 100.0, "1 0 0 RG -70 10 80 60 re S")];
    let d = open(&pdf(&plain, None), ImportOptions::default());
    assert_eq!(d.artboards[1].rect.x0, d.artboards[0].rect.x1 + 36.0);
}

#[test]
fn an_encrypted_pdf_opens_with_its_password_only() {
    let bytes = pdf(&[PdfPage::new(200.0, 100.0, "1 0 0 rg 10 10 50 50 re f")], Some("secret"));
    assert_eq!(import_with_report(&bytes, &ImportOptions::default()).unwrap_err(), PdfError::NeedsPassword);
    let wrong = ImportOptions { password: Some("guess".into()), ..Default::default() };
    assert_eq!(import_with_report(&bytes, &wrong).unwrap_err(), PdfError::WrongPassword);
    assert_eq!(info(&bytes, None).unwrap_err(), PdfError::NeedsPassword);
    let d = open(&bytes, ImportOptions { password: Some("secret".into()), ..Default::default() });
    let NodeKind::Layer { children, .. } = &d.layers[0].kind else { panic!() };
    assert_eq!(children.len(), 1, "the decrypted page has its rectangle");
    assert!(close(art(&d), Rect::new(10.0, 40.0, 60.0, 90.0)), "{:?}", art(&d));
    // A file that isn't encrypted ignores a password.
    let plain = pdf(&[PdfPage::new(200.0, 100.0, "")], None);
    assert!(import_with_report(&plain, &ImportOptions { password: Some("x".into()), ..Default::default() }).is_ok());
}

#[test]
fn info_lists_pages_with_their_size_rotation_and_boxes() {
    let bytes = pdf(&[boxed_page(), PdfPage { rotate: 270, ..PdfPage::new(100.0, 50.0, "") }], None);
    let i = info(&bytes, None).unwrap();
    assert_eq!(i.pages.len(), 2);
    let p = &i.pages[0];
    assert_eq!((p.width, p.height, p.rotation), (500.0, 700.0, 0));
    let b = |c: CropTo| p.boxes.iter().find(|(k, _)| *k == c).map(|(_, r)| *r).unwrap();
    assert_eq!(b(CropTo::Media), Rect::new(0.0, 0.0, 600.0, 800.0));
    assert_eq!(b(CropTo::Trim), Rect::new(60.0, 60.0, 540.0, 740.0));
    assert!(p.boxes.iter().all(|(k, _)| *k != CropTo::Bounding));
    assert_eq!((i.pages[1].width, i.pages[1].height, i.pages[1].rotation), (50.0, 100.0, 270));
    assert!(matches!(info(b"not a pdf", None), Err(PdfError::Parse(_))));
}

/// The edges of `r`, for [`near`].
fn edges(r: Rect) -> [f64; 4] {
    [r.x0, r.y0, r.x1, r.y1]
}

/// `info` gives a page's boxes in points, as its size: a page whose `UserUnit` is 10 has them at
/// ten times the file's numbers, as the artboard and bleed that Crop To trim makes of them.
/// Whatever the page's `UserUnit` entry, its media box has the size `info` gives.
#[test]
fn info_gives_the_boxes_of_a_user_unit_page_in_points() {
    // A 5 × 7 inch trim box with an eighth of an inch of bleed round it (0.9 units of 10 points).
    let page = |entries: &str| PdfPage {
        trim: Some([0.9, 0.9, 36.9, 51.3]),
        bleed: Some([0.0, 0.0, 37.8, 52.2]),
        entries: entries.into(),
        ..PdfPage::new(37.8, 52.2, "0 g 2 2 10 8 re f")
    };
    // The `UserUnit` given directly, and as an indirect object.
    for bytes in [pdf(&[page("/UserUnit 10")], None), pdf_with(&[page(&format!("/UserUnit {} 0 R", first_extra(1)))], &["10"], None)] {
        let p = &info(&bytes, None).unwrap().pages[0];
        assert!((p.width - 378.0).abs() < 1e-3 && (p.height - 522.0).abs() < 1e-3, "{p:?}");
        let b = |c: CropTo| p.boxes.iter().find(|(k, _)| *k == c).map(|(_, r)| edges(*r)).unwrap();
        assert!(near(b(CropTo::Media), [0.0, 0.0, 378.0, 522.0]) && near(b(CropTo::Trim), [9.0, 9.0, 369.0, 513.0]), "{p:?}");
        let d = open(&bytes, ImportOptions { crop: CropTo::Trim, ..Default::default() });
        assert!(near(edges(d.artboards[0].rect), [0.0, 0.0, 360.0, 504.0]), "{:?}", d.artboards[0].rect);
        assert!(near(d.setup.bleed, [9.0; 4]), "{:?}", d.setup.bleed);
    }
    for unit in
        ["", "/UserUnit 1", "/UserUnit 2.5", "/UserUnit 0.5", "/UserUnit 0", "/UserUnit -10", "/UserUnit /Ten", "/UserUnit (10)", "/UserUnit 1000000"]
    {
        let p = &info(&pdf(&[page(unit)], None), None).unwrap().pages[0];
        let media = p.boxes.iter().find(|(k, _)| *k == CropTo::Media).map(|(_, r)| *r).unwrap();
        assert!((media.width() / p.width - 1.0).abs() < 1e-6 && (media.height() / p.height - 1.0).abs() < 1e-6, "{unit}: {p:?}");
    }
}

#[test]
fn a_page_without_area_gets_the_size_viewers_give_it() {
    let bytes = pdf(&[PdfPage { media: [0.0, 0.0, 0.0, 0.0], ..PdfPage::new(0.0, 0.0, "") }], None);
    let i = info(&bytes, None).unwrap();
    let (w, h) = (i.pages[0].width, i.pages[0].height);
    assert!(w > 0.0 && h > 0.0);
    for crop in [CropTo::Crop, CropTo::Media, CropTo::Art] {
        let ab = open(&bytes, ImportOptions { crop, ..Default::default() }).artboards[0].rect;
        assert_eq!((ab.width(), ab.height()), (w, h), "{crop:?}");
    }
}

#[test]
fn postscript_is_refused_as_such() {
    let ps = b"%!PS\n0 0 moveto 10 10 lineto stroke\nshowpage\n";
    assert_eq!(import(ps).unwrap_err(), PdfError::PostScript);
    assert_eq!(info(ps, None).unwrap_err(), PdfError::PostScript);
    assert!(is_postscript(&[0xC5, 0xD0, 0xD3, 0xC6, 0, 0]) && !is_postscript(b"%PDF-1.7"));
}
