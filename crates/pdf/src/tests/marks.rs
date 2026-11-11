//! Marks and Bleeds: page boxes, the art kept in the bleed, non-printing layers and printer's marks.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint, Swatch};
use vectorcraft_doc::marks::PrinterMarks;
use vectorcraft_doc::{Appearance, Document, LayerColor, Node, NodeKind};
use vectorcraft_geom::{Rect, shapes};

use crate::*;

/// A 200 × 100 document with a red square reaching 5 pt left of its artboard.
fn doc() -> Document {
    let mut d = Document::new(200.0, 100.0);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(Rect::new(-5.0, 10.0, 40.0, 50.0)), red)).unwrap();
    d
}

fn settings(v: serde_json::Value) -> PdfSettings {
    serde_json::from_value(v).unwrap()
}

/// The PDF of `d` with settings `v`; marks and bleed raise no warnings.
fn pdf(d: &Document, v: serde_json::Value) -> Vec<u8> {
    let opts = PdfOptions { settings: settings(v), created: Some(1_791_200_000), ..PdfOptions::uncompressed() };
    let r = export_with_report(d, &opts).unwrap();
    assert!(r.warnings.iter().all(|w| !w.contains("bleed") && !w.contains("marks")), "{:?}", r.warnings);
    r.bytes
}

/// Page 1's box `which` (PDF space: y up).
fn page_box(bytes: &[u8], which: CropTo) -> Rect {
    info(bytes, None).unwrap().pages[0].boxes.iter().find(|(c, _)| *c == which).unwrap().1
}

/// The bounds of every path of `d` painted with a paint `pick` accepts.
fn painted(d: &Document, pick: impl Fn(&Paint) -> bool) -> Vec<Rect> {
    let mut out = vec![];
    d.walk(|n| {
        let paints = n.appearance.fill().map(|f| &f.paint).into_iter().chain(n.appearance.stroke().map(|s| &s.paint));
        if paints.into_iter().any(&pick)
            && let Some(b) = n.path_data().and_then(|p| p.bounds())
        {
            out.push(b);
        }
    });
    out
}

fn rgb(p: &Paint, rgb: [u8; 3]) -> bool {
    p.color().is_some_and(|c| c.to_rgba8(1.0)[..3] == rgb)
}

#[test]
fn bleed_grows_the_media_box_around_the_trim_box() {
    let d = doc();
    // No bleed and no marks: the page is the artboard.
    let plain = pdf(&d, json!({}));
    assert_eq!(page_box(&plain, CropTo::Media), Rect::new(0.0, 0.0, 200.0, 100.0));
    assert_eq!(page_box(&plain, CropTo::Trim), Rect::new(0.0, 0.0, 200.0, 100.0));
    // A 9 pt bleed, custom or the document's.
    let mut nine = d.clone();
    nine.setup.bleed = [9.0; 4];
    for v in [json!({"bleed": {"top": 9, "bottom": 9, "left": 9, "right": 9}}), json!({"bleed": {"useDocument": true, "top": 1}})] {
        let bytes = pdf(&nine, v.clone());
        let media = page_box(&bytes, CropTo::Media);
        assert_eq!((media.width(), media.height()), (218.0, 118.0), "{v}: MediaBox = artboard + 2 × 9 pt");
        assert_eq!(page_box(&bytes, CropTo::Trim), Rect::new(9.0, 9.0, 209.0, 109.0), "{v}: TrimBox = artboard");
        assert_eq!(page_box(&bytes, CropTo::Bleed), media, "{v}: BleedBox = artboard + bleed");
    }
    // A document bleed read from a file is kept in range.
    nine.setup.bleed = [f64::NAN, -3.0, 1e9, 2.0];
    let b = pdf(&nine, json!({"bleed": {"useDocument": true}}));
    assert_eq!(page_box(&b, CropTo::Media), Rect::new(0.0, 0.0, 274.0, 100.0));
    // Uneven bleed (PDF space is y up: the bottom bleed is below the trim box).
    let bytes = pdf(&d, json!({"bleed": {"top": 4, "bottom": 2, "left": 3, "right": 1}}));
    assert_eq!(page_box(&bytes, CropTo::Media), Rect::new(0.0, 0.0, 204.0, 106.0));
    assert_eq!(page_box(&bytes, CropTo::Trim), Rect::new(3.0, 2.0, 203.0, 102.0));
}

#[test]
fn a_pdfx_file_leaves_out_the_document_bleed_side_by_side() {
    let x4 = |bleed: serde_json::Value| settings(json!({"standard": "pdfX4", "bleed": bleed}));
    let mut d = doc();
    d.setup.bleed = [6.0, 0.0, 0.0, 0.0];
    assert!(x4(json!({})).leaves_out_bleed(&d));
    assert!(x4(json!({"bottom": 1})).leaves_out_bleed(&d), "the top is left out");
    assert!(!x4(json!({"top": 1})).leaves_out_bleed(&d), "the sides without a document bleed don't count");
    assert!(!x4(json!({"useDocument": true})).leaves_out_bleed(&d));
    assert!(!settings(json!({})).leaves_out_bleed(&d), "not a PDF/X file");
    // A document bleed read from a file counts as it is kept in range.
    d.setup.bleed = [f64::INFINITY, f64::NAN, -3.0, 0.0];
    assert!(!x4(json!({})).leaves_out_bleed(&d));
    assert!(!x4(json!({"useDocument": true})).leaves_out_bleed(&d));
}

/// Each page's `UserUnit` and media box as the file holds them (in its units).
fn units_and_media(bytes: &[u8]) -> Vec<(f32, Rect)> {
    let file = crate::pages::open(bytes, None).unwrap();
    file.pages().iter().map(|p| (p.user_unit(), Rect::new(p.media_box().x0, p.media_box().y0, p.media_box().x1, p.media_box().y1))).collect()
}

/// A page with a side longer than ISO 32000-1's 14,400 units is written in larger units (its
/// `UserUnit`), so that its sides are 14,400 units at most, and reads back at its size in points.
/// Pages up to 14,400 pt, files before PDF 1.6 and PDF/X files keep their sizes in points.
#[test]
fn a_page_past_14400_pt_is_written_with_a_user_unit_and_reads_back_at_its_size() {
    let mut d = doc();
    d.artboards[0].rect = Rect::new(0.0, 0.0, 20_000.0, 10_000.0);
    let mut small = d.artboards[0].clone();
    small.rect = Rect::new(0.0, 11_000.0, 14_400.0, 11_300.0);
    d.artboards.push(small.clone());
    small.rect = Rect::new(0.0, 12_000.0, 20_000.0, 12_001.0);
    d.artboards.push(small);
    let near = |a: Rect, b: Rect| [a.x0 - b.x0, a.y0 - b.y0, a.x1 - b.x1, a.y1 - b.y1].iter().all(|d| d.abs() < 0.01);
    let bytes = pdf(&d, json!({}));
    let pages = units_and_media(&bytes);
    // 20,000 / 14,400 = 1.3889, rounded up to 1.389: the page is 14,398.85 × 7,199.42 units.
    assert_eq!(pages[0].0, 1.389, "{pages:?}");
    assert!(pages[0].1.width() <= 14_400.0 && near(pages[0].1.scale_from_origin(1.389), Rect::new(0.0, 0.0, 20_000.0, 10_000.0)), "{pages:?}");
    assert_eq!(pages[1], (1.0, Rect::new(0.0, 0.0, 14_400.0, 300.0)), "a page of 14,400 pt keeps its points");
    assert!(pages[2].0 == 1.389 && (pages[2].1.height() * 1.389 - 1.0).abs() < 1e-3, "a side of 1 pt stays 1 pt: {pages:?}");
    assert_eq!(String::from_utf8_lossy(&bytes).matches("/UserUnit").count(), 2);
    // Read back in points: the page, its trim box and the square reaching 5 pt left of the page.
    let page = &info(&bytes, None).unwrap().pages[0];
    assert!((page.width - 20_000.0).abs() < 0.01 && (page.height - 10_000.0).abs() < 0.01, "{page:?}");
    assert!(near(page_box(&bytes, CropTo::Trim), Rect::new(0.0, 0.0, 20_000.0, 10_000.0)));
    let back = import(&bytes).unwrap();
    assert!(near(back.artboards[0].rect, Rect::new(0.0, 0.0, 20_000.0, 10_000.0)), "{:?}", back.artboards[0].rect);
    let red = painted(&back, |p| rgb(p, [255, 0, 0]));
    assert!(red.len() == 1 && near(red[0], Rect::new(-5.0, 10.0, 40.0, 50.0)), "{red:?}");
    // PDF/X-4 files have the page in points, past 14,400.
    for v in [json!({"standard": "pdfX4", "compatibility": "1.5"}), json!({"standard": "pdfX4", "compatibility": "1.6"})] {
        let pages = units_and_media(&pdf(&d, v.clone()));
        assert_eq!(pages[0], (1.0, Rect::new(0.0, 0.0, 20_000.0, 10_000.0)), "{v}");
    }
}

/// 20 m in points.
const TWENTY_M: f64 = 20.0 / 0.0254 * 72.0;

/// [`doc`] with its artboard `w` × `h` points large.
fn sized(w: f64, h: f64) -> Document {
    let mut d = doc();
    d.artboards[0].rect = Rect::new(0.0, 0.0, w, h);
    d
}

/// The export of `d` with `set`.
fn report(d: &Document, set: PdfSettings) -> Result<ExportReport, PdfError> {
    export_with_report(d, &PdfOptions { settings: set, created: Some(1_791_200_000), ..PdfOptions::uncompressed() })
}

/// The numbers that follow `key` in the file's bytes (`/MediaBox [0 0 w h]`, `/UserUnit u`), read
/// as text, without the PDF reader: none when the file has no `key`.
fn raw_numbers(bytes: &[u8], key: &str, count: usize) -> Vec<f64> {
    let text = String::from_utf8_lossy(bytes);
    let Some(at) = text.find(key) else { return vec![] };
    let words = text[at + key.len()..].split(|c: char| c.is_whitespace() || matches!(c, '[' | ']' | '/' | '>')).filter(|w| !w.is_empty());
    words.take(count).map(|w| w.parse().unwrap_or(f64::NAN)).collect()
}

/// The page's width and height in meters as the file's bytes give them: its media box times its
/// `UserUnit` (1 without one).
fn raw_meters(bytes: &[u8]) -> (f64, f64) {
    let media = raw_numbers(bytes, "/MediaBox", 4);
    let unit = raw_numbers(bytes, "/UserUnit", 1).first().copied().unwrap_or(1.0);
    let m = |v: f64| v * unit / 72.0 * 0.0254;
    (m(media[2] - media[0]), m(media[3] - media[1]))
}

/// The warnings about pages longer than 14,400 pt.
fn long_page_warnings(r: &ExportReport) -> Vec<&String> {
    r.warnings.iter().filter(|w| w.contains("page limit of 14,400 pt")).collect()
}

/// A 20 m page at PDF 1.7 (the default) is written with a `UserUnit` of 3.938, reads back at 20 m
/// from the file's bytes, through the PDF reader and through the import, and the export says how
/// it is written.
#[test]
fn a_20_m_page_keeps_its_size_with_a_user_unit_and_the_export_says_so() {
    let d = sized(TWENTY_M, TWENTY_M / 20.0);
    let r = report(&d, PdfSettings::default()).unwrap();
    assert!(r.bytes.starts_with(b"%PDF-1.7"));
    assert_eq!(
        long_page_warnings(&r),
        [
            "the page of artboard 1 is 56693 × 2835 pt (20.00 × 1.00 m), longer than PDF's page limit of 14,400 pt (200 in): it is written with a UserUnit of 3.938, so apps that ignore UserUnit show it that many times smaller"
        ],
        "{:?}",
        r.warnings
    );
    assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [3.938]);
    let (w, h) = raw_meters(&r.bytes);
    assert!((w - 20.0).abs() < 0.001 && (h - 1.0).abs() < 0.001, "{w} × {h} m");
    let pages = units_and_media(&r.bytes);
    assert_eq!(pages[0].0, 3.938);
    assert!(pages[0].1.width() <= 14_400.0, "{pages:?}");
    let page = &info(&r.bytes, None).unwrap().pages[0];
    assert!((page.width - TWENTY_M).abs() < 0.01, "{page:?}");
    let back = import(&r.bytes).unwrap();
    assert!((back.artboards[0].rect.width() - TWENTY_M).abs() < 0.01, "{:?}", back.artboards[0].rect);
}

/// Every built-in preset and every PDF version keeps a 20 m page at its size, or refuses it with
/// the reason: PDF 1.3 and 1.4 files (PDF/X-1a and PDF/X-3 among them) have no `UserUnit` and
/// hold no real number past 32,767, PDF 1.5 is written as PDF 1.6 to have one, and PDF/X-4 pages
/// at PDF 1.5 or 1.6 stay in points with a warning.
#[test]
fn every_preset_and_version_keeps_a_20_m_page_or_says_why_not() {
    let d = sized(TWENTY_M, TWENTY_M / 20.0);
    let refused = |e: PdfError, label: &str| {
        assert_eq!(
            e.to_string(),
            format!(
                "invalid PDF setting: {label} files hold real numbers up to 32,767, and the page of artboard 1, 56693 pt long, needs larger ones: choose PDF 1.6 or later without a PDF/X standard, or a smaller artboard"
            )
        );
    };
    for preset in crate::builtin_presets() {
        let name = preset.name.as_str();
        match report(&d, preset.settings.clone()) {
            Err(e) if matches!(name, "PDF/X-1a:2001" | "PDF/X-3:2002") => refused(e, name),
            Err(e) => panic!("{name}: {e}"),
            Ok(r) if name == "PDF/X-4:2010" => {
                assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), Vec::<f64>::new(), "{name}");
                assert!((raw_meters(&r.bytes).0 - 20.0).abs() < 0.001, "{name}");
                let w = long_page_warnings(&r);
                let points = "PDF/X-4:2010 pages are written without a UserUnit, in points, and apps that keep to the limit cut the page to 200 in";
                assert!(w.len() == 1 && w[0].ends_with(points), "{w:?}");
            }
            Ok(r) => {
                assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [3.938], "{name}");
                assert!((raw_meters(&r.bytes).0 - 20.0).abs() < 0.001, "{name}");
                assert_eq!(long_page_warnings(&r).len(), 1, "{name}: {:?}", r.warnings);
            }
        }
    }
    for c in Compatibility::ALL.iter().copied() {
        let set = PdfSettings { compatibility: c, ..PdfSettings::default() };
        match (c, report(&d, set)) {
            (Compatibility::Pdf13 | Compatibility::Pdf14, Err(e)) => refused(e, c.label()),
            (Compatibility::Pdf13 | Compatibility::Pdf14, Ok(_)) => panic!("{} is refused", c.label()),
            (_, Err(e)) => panic!("{}: {e}", c.label()),
            (_, Ok(r)) => {
                let header = if c == Compatibility::Pdf15 { "1.6" } else { c.id() };
                assert!(r.bytes.starts_with(format!("%PDF-{header}").as_bytes()), "{}", c.label());
                assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [3.938], "{}", c.label());
                assert!((raw_meters(&r.bytes).0 - 20.0).abs() < 0.001, "{}", c.label());
                let raised = r.warnings.iter().any(|w| w.starts_with("PDF 1.5 has no UserUnit") && w.ends_with("the file is written as PDF 1.6"));
                assert_eq!(raised, c == Compatibility::Pdf15, "{}: {:?}", c.label(), r.warnings);
            }
        }
    }
    // PDF/X-4 files at PDF 1.4 have no `UserUnit` either; PDF/A-2b files at PDF 1.4 are written as
    // PDF 1.6 to have one.
    let at_14 = |standard| report(&d, PdfSettings { standard, compatibility: Compatibility::Pdf14, ..PdfSettings::default() });
    refused(at_14(Standard::PdfX4).err().unwrap(), "PDF 1.4");
    let r = at_14(Standard::PdfA2b).unwrap();
    assert!(r.bytes.starts_with(b"%PDF-1.6"));
    assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [3.938]);
}

/// PDF/A-2 limits page boundaries to 14,400 units (ISO 19005-2, 6.1.13), and PDF 1.4 has no
/// `UserUnit`: a PDF/A-2b file at PDF 1.4 with a page longer than 14,400 pt is written as PDF 1.6,
/// with a `UserUnit` that keeps the page within 14,400 units and a warning, with
/// `long_pages_in_points` on or off. Pages past 32,767 pt that aren't whole numbers of points are
/// written too. A file whose pages are all 14,400 pt or shorter stays PDF 1.4.
#[test]
fn pdfa2b_at_pdf_1_4_writes_a_long_page_as_pdf_1_6_within_14400_units() {
    for in_points in [false, true] {
        let set = PdfSettings {
            standard: Standard::PdfA2b,
            compatibility: Compatibility::Pdf14,
            long_pages_in_points: in_points,
            ..PdfSettings::default()
        };
        for side in [17_000.0, 32_767.0, 32_767.5, 40_000.0, TWENTY_M] {
            let r = report(&sized(side, 300.0), set.clone()).unwrap_or_else(|e| panic!("{side} {in_points}: {e}"));
            assert!(r.bytes.starts_with(b"%PDF-1.6"), "{side} {in_points}");
            let unit = raw_numbers(&r.bytes, "/UserUnit", 1);
            assert_eq!(unit.len(), 1, "{side} {in_points}");
            let media = raw_numbers(&r.bytes, "/MediaBox", 4);
            assert!(media[2] - media[0] <= 14_400.0 && media[3] - media[1] <= 14_400.0, "{side} {in_points}: {media:?}");
            assert!(((media[2] - media[0]) * unit[0] - side).abs() < side * 0.001, "{side} {in_points}: {media:?} {unit:?}");
            let raised = "PDF 1.4 has no UserUnit, which a page longer than 14,400 pt (200 in) needs to stay within PDF/A-2's page limit of 14,400 units: the file is written as PDF 1.6";
            assert!(r.warnings.iter().any(|w| w == raised), "{side} {in_points}: {:?}", r.warnings);
        }
        let r = report(&sized(14_400.0, 300.0), set.clone()).unwrap();
        assert!(r.bytes.starts_with(b"%PDF-1.4"), "{in_points}");
        assert!(!r.warnings.iter().any(|w| w.contains("has no UserUnit")), "{in_points}: {:?}", r.warnings);
    }
}

/// A page from 14,400 to 32,767 pt long is written in points, with the warning, in PDF 1.3 and 1.4
/// files, PDF/X-1a, PDF/X-3 and PDF/X-4 files among them, as before the warning. Past
/// 32,767 pt a page of whole points is written too, as the writer puts whole numbers as integers;
/// a side that isn't a whole number, or art that needs a real number past 32,767, gets the error.
#[test]
fn pdf_1_4_writes_a_page_up_to_32767_pt_in_points() {
    let presets = crate::builtin_presets();
    let preset = |name: &str| presets.iter().find(|p| p.name == name).unwrap().settings.clone();
    let sets = [
        ("PDF 1.3", PdfSettings { compatibility: Compatibility::Pdf13, ..PdfSettings::default() }),
        ("PDF 1.4", PdfSettings { compatibility: Compatibility::Pdf14, ..PdfSettings::default() }),
        ("PDF/X-1a:2001", preset("PDF/X-1a:2001")),
        ("PDF/X-3:2002", preset("PDF/X-3:2002")),
        ("PDF/X-4:2010", PdfSettings { standard: Standard::PdfX4, compatibility: Compatibility::Pdf14, ..PdfSettings::default() }),
    ];
    for (label, set) in &sets {
        for side in [17_000.0, 32_767.0, 40_000.0] {
            let r = report(&sized(side, 300.0), set.clone()).unwrap_or_else(|e| panic!("{label} at {side}: {e}"));
            assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), Vec::<f64>::new(), "{label}");
            assert_eq!(raw_numbers(&r.bytes, "/MediaBox", 4), [0.0, 0.0, side, 300.0], "{label}");
            let w = long_page_warnings(&r);
            let points = format!("{label} pages are written without a UserUnit, in points, and apps that keep to the limit cut the page to 200 in");
            assert!(w.len() == 1 && w[0].ends_with(&points), "{label}: {w:?}");
        }
        // For PDF/X-4 files the error names the PDF version.
        let label = if *label == "PDF/X-4:2010" { "PDF 1.4" } else { label };
        let refused = |e: PdfError, side: &str| {
            let refusal = format!("{label} files hold real numbers up to 32,767, and the page of artboard 1, {side} pt long, needs larger ones");
            assert!(e.to_string().contains(&refusal), "{e}");
        };
        refused(report(&sized(32_767.5, 300.0), set.clone()).err().unwrap(), "32768");
        // Art at a coordinate that isn't a whole number past 32,767.
        let mut d = sized(40_000.0, 300.0);
        let layer = d.default_layer().unwrap();
        let id = d.alloc_id();
        let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
        d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(Rect::new(35_000.5, 10.0, 35_100.25, 50.0)), red)).unwrap();
        refused(report(&d, set.clone()).err().unwrap(), "40000");
    }
}

/// With several pages past 32,767 pt in a PDF 1.4 file, the error names the page whose art needs a
/// real number past 32,767, not the first long page.
#[test]
fn the_error_names_the_long_page_whose_art_needs_a_larger_real() {
    let mut d = sized(40_000.0, 300.0);
    let mut second = d.artboards[0].clone();
    second.rect = Rect::new(0.0, 10_000.0, 40_000.0, 10_300.0);
    d.artboards.push(second);
    let layer = d.default_layer().unwrap();
    let id = d.alloc_id();
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    d.insert(Some(layer), 0, Node::path(id, shapes::rectangle(Rect::new(35_000.5, 10_010.0, 35_100.25, 10_050.0)), red)).unwrap();
    let set = PdfSettings { compatibility: Compatibility::Pdf14, ..PdfSettings::default() };
    let e = report(&d, set).err().unwrap();
    assert!(e.to_string().contains("the page of artboard 2, 40000 pt long, needs larger ones"), "{e}");
}

/// A long page whose short side would be under 3 units at the `UserUnit` its long side needs
/// (ISO 32000-1, Annex C.2; PDF/A-2b's 6.1.13) is written with a smaller unit that keeps the
/// short side at 3 units, and a warning; 100 m × 4 mm is one. A short side under 3 pt is under 3
/// units at any unit, and keeps the unit its long side needs.
#[test]
fn a_page_too_narrow_for_its_user_unit_keeps_3_units_on_its_short_side() {
    const MM: f64 = 72.0 / 25.4;
    for set in [PdfSettings::default(), PdfSettings { standard: Standard::PdfA2b, ..PdfSettings::default() }] {
        let r = report(&sized(TWENTY_M * 5.0, 4.0 * MM), set).unwrap();
        assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [3.779]);
        let media = raw_numbers(&r.bytes, "/MediaBox", 4);
        assert!(media[3] - media[1] >= 3.0, "{media:?}");
        assert!((media[2] - media[0] - TWENTY_M * 5.0 / 3.779).abs() < 0.01, "{media:?}");
        assert_eq!(
            r.warnings.iter().filter(|w| w.contains("too long and narrow")).collect::<Vec<_>>(),
            [
                "the page of artboard 1 is 283465 × 12 pt (100.00 × 0.00 m), too long and narrow for PDF's page sides of 3 to 14,400 units: it is written with a UserUnit of 3.779, which keeps the short side at 3 units or more, and apps that keep to the limit cut the long side to 14,400 units"
            ]
        );
        assert!(long_page_warnings(&r).is_empty(), "{:?}", r.warnings);
    }
    // A short side of 3 units or more at the long side's unit keeps that unit.
    let r = report(&sized(TWENTY_M * 5.0, 60.0), PdfSettings::default()).unwrap();
    assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [19.686]);
    let media = raw_numbers(&r.bytes, "/MediaBox", 4);
    assert!(media[3] - media[1] >= 3.0, "{media:?}");
    let r = report(&sized(TWENTY_M * 5.0, 2.0), PdfSettings::default()).unwrap();
    assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [19.686]);
}

/// A PDF 1.5 file written as PDF 1.6 for a long page keeps PDF 1.5's encryption, 128-bit RC4, as
/// the Save PDF dialog shows it.
#[test]
fn a_pdf_1_5_file_written_as_1_6_keeps_rc4_encryption() {
    let mut set = PdfSettings { compatibility: Compatibility::Pdf15, ..PdfSettings::default() };
    set.security.open_password = "secret".into();
    let has = |bytes: &[u8], s: &[u8]| bytes.windows(s.len()).any(|w| w == s);
    let r = report(&sized(TWENTY_M, 300.0), set).unwrap();
    assert!(r.bytes.starts_with(b"%PDF-1.6"));
    assert!(has(&r.bytes, b"/CFM/V2") && !has(&r.bytes, b"AESV2"), "the file is encrypted with RC4");
}

/// The warning gives a page's size rounded up, as the error does: a side of 14,400.4 pt is 14401.
#[test]
fn the_long_page_warning_rounds_the_size_up() {
    let r = report(&sized(14_400.4, 300.0), PdfSettings::default()).unwrap();
    assert!(long_page_warnings(&r)[0].contains("is 14401 × 300 pt"), "{:?}", r.warnings);
}

/// A page of exactly 14,400 pt is written in points with no warning, at any PDF version; one
/// point longer gets a `UserUnit` of 1.001 and the warning, and stays in points at PDF 1.4.
#[test]
fn a_page_of_14400_pt_stays_in_points_and_one_point_more_does_not() {
    let at = |side: f64, c: Compatibility| report(&sized(side, 300.0), PdfSettings { compatibility: c, ..PdfSettings::default() });
    for c in [Compatibility::Pdf14, Compatibility::Pdf15, Compatibility::Pdf17] {
        let r = at(14_400.0, c).unwrap();
        assert!(long_page_warnings(&r).is_empty() && !r.warnings.iter().any(|w| w.contains("UserUnit")), "{}: {:?}", c.label(), r.warnings);
        assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), Vec::<f64>::new(), "{}", c.label());
        assert_eq!(raw_numbers(&r.bytes, "/MediaBox", 4), [0.0, 0.0, 14_400.0, 300.0], "{}", c.label());
    }
    let r = at(14_401.0, Compatibility::Pdf17).unwrap();
    assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), [1.001]);
    assert_eq!(long_page_warnings(&r).len(), 1, "{:?}", r.warnings);
    assert!(long_page_warnings(&r)[0].contains("14401 × 300 pt (5.08 × 0.11 m)"), "{:?}", r.warnings);
    let r = at(14_401.0, Compatibility::Pdf14).unwrap();
    assert_eq!(raw_numbers(&r.bytes, "/MediaBox", 4), [0.0, 0.0, 14_401.0, 300.0]);
    assert_eq!(long_page_warnings(&r).len(), 1, "{:?}", r.warnings);
}

/// A page of 14,400 pt or less is written as before in every built-in preset and every PDF
/// version: the header names the version asked for, the media box is in points, and there is no
/// `UserUnit` and no page-size warning. A PDF 1.5 file with a password keeps 128-bit RC4.
#[test]
fn a_page_within_14400_pt_is_written_as_before_at_every_version() {
    let d = sized(14_400.0, 14_400.0);
    let mut sets: Vec<PdfSettings> = crate::builtin_presets().into_iter().map(|p| p.settings).collect();
    for c in [Compatibility::Pdf13, Compatibility::Pdf14, Compatibility::Pdf15, Compatibility::Pdf16, Compatibility::Pdf17, Compatibility::Pdf20] {
        sets.push(PdfSettings { compatibility: c, ..PdfSettings::default() });
    }
    for set in sets {
        let label = format!("{} {}", set.standard.label(), set.compatibility.label());
        let r = report(&d, set.clone()).unwrap_or_else(|e| panic!("{label}: {e}"));
        // PDF/X-1a and PDF/X-3 files are always PDF 1.3.
        let pdf13 = set.compatibility == Compatibility::Pdf13 || matches!(set.standard, Standard::PdfX1a | Standard::PdfX3);
        let header = if pdf13 { "%PDF-1.3".to_string() } else { format!("%{}", set.compatibility.label().replace(' ', "-")) };
        assert!(r.bytes.starts_with(header.as_bytes()), "{label}: {:?}", String::from_utf8_lossy(&r.bytes[..8]));
        assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), Vec::<f64>::new(), "{label}");
        assert_eq!(raw_numbers(&r.bytes, "/MediaBox", 4), [0.0, 0.0, 14_400.0, 14_400.0], "{label}");
        assert!(!r.warnings.iter().any(|w| w.contains("14,400") || w.contains("UserUnit")), "{label}: {:?}", r.warnings);
    }
    let mut set = PdfSettings { compatibility: Compatibility::Pdf15, ..PdfSettings::default() };
    set.security.open_password = "secret".into();
    let r = report(&d, set).unwrap();
    let has = |s: &[u8]| r.bytes.windows(s.len()).any(|w| w == s);
    assert!(r.bytes.starts_with(b"%PDF-1.5") && has(b"/CFM/V2") && !has(b"AESV2"));
}

/// 10 m in points.
const TEN_M: f64 = 10.0 / 0.0254 * 72.0;

/// With `long_pages_in_points`, a 10 m page at PDF 1.7 is written at its size in points, with no
/// `UserUnit`, the warning says so, and the page reads back at 10 m. Without it the page keeps
/// its `UserUnit`.
#[test]
fn long_pages_in_points_writes_a_10_m_page_in_points() {
    let d = sized(TEN_M, TEN_M / 10.0);
    let r = report(&d, PdfSettings { long_pages_in_points: true, ..PdfSettings::default() }).unwrap();
    assert!(r.bytes.starts_with(b"%PDF-1.7"));
    assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), Vec::<f64>::new());
    let media = raw_numbers(&r.bytes, "/MediaBox", 4);
    assert!((media[2] - media[0] - 28_346.46).abs() < 0.01 && (media[3] - media[1] - 2_834.65).abs() < 0.01, "{media:?}");
    assert_eq!(
        long_page_warnings(&r),
        [
            "the page of artboard 1 is 28347 × 2835 pt (10.00 × 1.00 m), longer than PDF's page limit of 14,400 pt (200 in): PDF 1.7 pages are written without a UserUnit, in points, and apps that keep to the limit cut the page to 200 in"
        ],
        "{:?}",
        r.warnings
    );
    let page = &info(&r.bytes, None).unwrap().pages[0];
    assert!((page.width - TEN_M).abs() < 0.01 && (page.height - TEN_M / 10.0).abs() < 0.01, "{page:?}");
    let back = import(&r.bytes).unwrap();
    let rect = back.artboards[0].rect;
    assert!((rect.width() - TEN_M).abs() < 0.01 && (rect.height() - TEN_M / 10.0).abs() < 0.01, "{rect:?}");
    // Off (the default), the page has a `UserUnit` of 28,346.46 / 14,400, rounded up.
    let off = report(&d, PdfSettings::default()).unwrap();
    assert_eq!(raw_numbers(&off.bytes, "/UserUnit", 1), [1.969]);
    assert!(
        long_page_warnings(&off)[0].ends_with("it is written with a UserUnit of 1.969, so apps that ignore UserUnit show it that many times smaller")
    );
    assert!(!settings(json!({})).long_pages_in_points, "off by default");
}

/// With `long_pages_in_points`, every PDF version writes a 10 m page in points: PDF 1.5 stays
/// PDF 1.5, and versions that write it in points anyway (PDF 1.3 and 1.4, PDF/X-4) give the same
/// file as without it.
#[test]
fn long_pages_in_points_keeps_every_version_and_changes_nothing_where_pages_are_in_points_already() {
    let d = sized(TEN_M, TEN_M / 10.0);
    let with = |set: &PdfSettings| PdfSettings { long_pages_in_points: true, ..set.clone() };
    for c in Compatibility::ALL.iter().copied() {
        let set = PdfSettings { compatibility: c, ..PdfSettings::default() };
        let r = report(&d, with(&set)).unwrap();
        assert!(r.bytes.starts_with(format!("%PDF-{}", c.id()).as_bytes()), "{}", c.label());
        assert_eq!(raw_numbers(&r.bytes, "/UserUnit", 1), Vec::<f64>::new(), "{}", c.label());
        assert!((raw_meters(&r.bytes).0 - 10.0).abs() < 0.001, "{}", c.label());
        assert!(!r.warnings.iter().any(|w| w.starts_with("PDF 1.5 has no UserUnit")), "{}: {:?}", c.label(), r.warnings);
        assert_eq!(long_page_warnings(&r).len(), 1, "{}: {:?}", c.label(), r.warnings);
        if matches!(c, Compatibility::Pdf13 | Compatibility::Pdf14) {
            assert_eq!(r.bytes, report(&d, set).unwrap().bytes, "{}", c.label());
        }
    }
    let x4 = crate::builtin_presets().into_iter().find(|p| p.name == "PDF/X-4:2010").unwrap().settings;
    assert_eq!(report(&d, with(&x4)).unwrap().bytes, report(&d, x4).unwrap().bytes);
}

/// A page of 14,400 pt or less gives the same file with `long_pages_in_points` on or off, at every
/// PDF version.
#[test]
fn long_pages_in_points_leaves_a_page_within_14400_pt_as_it_is() {
    let d = sized(14_400.0, 300.0);
    for c in Compatibility::ALL.iter().copied() {
        let off = PdfSettings { compatibility: c, ..PdfSettings::default() };
        let on = PdfSettings { long_pages_in_points: true, ..off.clone() };
        let (off, on) = (report(&d, off).unwrap(), report(&d, on).unwrap());
        assert_eq!(on.bytes, off.bytes, "{}", c.label());
        assert_eq!(on.warnings, off.warnings, "{}", c.label());
    }
}

#[test]
fn art_in_the_bleed_is_kept_and_clipped_to_the_bleed_box() {
    let d = doc();
    let back = import(&pdf(&d, json!({"bleed": {"left": 9, "top": 9, "bottom": 9, "right": 9}}))).unwrap();
    let red = painted(&back, |p| rgb(p, [255, 0, 0]));
    assert_eq!(red.len(), 1);
    assert!((red[0].x0 - 4.0).abs() < 0.01, "the square reaches 5 pt into the 9 pt bleed: {red:?}");
    // With marks the sheet is larger than the bleed box: the art is clipped to it.
    let bytes = pdf(&d, json!({"bleed": {"left": 3}, "marks": {"trim": true}}));
    let (trim, bleed) = (page_box(&bytes, CropTo::Trim), page_box(&bytes, CropTo::Bleed));
    assert!((trim.x0 - bleed.x0 - 3.0).abs() < 1e-3 && bleed.x0 > 0.0, "{trim:?} {bleed:?}");
    let back = import(&bytes).unwrap();
    let mut clips = vec![];
    back.walk(|n| {
        if let NodeKind::Group { children, clip: true } = &n.kind {
            clips.extend(children.first().and_then(|c| c.path_data()).and_then(|p| p.bounds()));
        }
    });
    let media = page_box(&bytes, CropTo::Media);
    let bleed_doc = Rect::new(bleed.x0, media.y1 - bleed.y1, bleed.x1, media.y1 - bleed.y0);
    assert!(clips.iter().any(|c| (c.x0 - bleed_doc.x0).abs() < 0.01 && (c.x1 - bleed_doc.x1).abs() < 0.01), "{clips:?} vs {bleed_doc:?}");
}

#[test]
fn non_printing_layers_are_left_out_unless_asked_for() {
    let mut d = doc();
    let mut layer = Node::layer(d.alloc_id(), "Notes", LayerColor::Preset(1));
    let blue = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::None, 0.0);
    let note = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(100.0, 20.0, 150.0, 60.0)), blue);
    if let NodeKind::Layer { children, printable, .. } = &mut layer.kind {
        *printable = false;
        children.push(Arc::new(note));
    }
    d.layers.push(Arc::new(layer));
    let blues = |v| painted(&import(&pdf(&d, v)).unwrap(), |p| rgb(p, [0, 0, 255])).len();
    assert_eq!(blues(json!({})), 0, "a non-printing layer is absent");
    assert_eq!(blues(json!({"includeNonPrinting": true})), 1);
    assert_eq!(blues(json!({"createLayers": true})), 1, "PDF layers keep every layer");
}

#[test]
fn marks_are_drawn_in_registration_on_every_plate_inside_the_media_box() {
    // Wide enough for the spot ink's colour bar patch after the process ones.
    let mut d = doc();
    let w = 300.0;
    d.artboards[0].rect = Rect::new(0.0, 0.0, w, 100.0);
    d.swatches.push(Swatch { name: "Gold".into(), paint: Paint::solid(Color::cmyk(0.0, 0.2, 0.8, 0.1)), global: true, spot: true });
    let all = json!({"marks": {"trim": true, "registration": true, "colorBars": true, "pageInfo": true}});
    let bytes = pdf(&d, all.clone());
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("/Separation/All"), "Registration is the /All separation");
    assert!(text.contains("/Separation/Gold"), "the spot ink gets a colour bar patch");
    // MediaBox fits the marks: [top, bottom, left, right] around the trim box.
    let [top, bottom, left, right] = settings(all).marks.printer_marks().reach([0.0; 4]);
    let (media, trim) = (page_box(&bytes, CropTo::Media), page_box(&bytes, CropTo::Trim));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-3;
    assert!(near(media.width(), w + left + right) && near(media.height(), 100.0 + top + bottom), "{media:?}");
    assert!(near(trim.x0, left) && near(trim.y0, bottom), "{trim:?}");
    // Read back (y down): every Registration mark lies outside the trim box and inside the sheet.
    let back = import(&bytes).unwrap();
    let sheet = Rect::new(0.0, 0.0, media.width(), media.height()).inflate(0.5, 0.5);
    let trim_doc = Rect::new(left, top, left + w, top + 100.0).inflate(-0.5, -0.5);
    let marks = painted(&back, Paint::is_registration);
    // 8 trim marks, 4 targets (2 rings and 2 lines each) and the page information's glyphs.
    assert!(marks.len() > 8 + 16, "{}", marks.len());
    for b in &marks {
        assert!(sheet.contains_rect(*b) && b.intersect(trim_doc).area() <= 0.0, "{b:?}");
    }
    let gold = painted(&back, |p| matches!(p, Paint::Solid { swatch: Some(s), .. } if s == "Gold"));
    assert_eq!(gold.len(), 1);
    assert!(gold[0].y1 < trim_doc.y0, "colour bars sit above the trim box: {gold:?}");
}

#[test]
fn page_information_names_the_file_artboard_and_date() {
    let d = doc();
    assert_eq!(crate::marks::page_info(&d, "Poster", 0, Some(1_791_200_000)), "Poster  ·  Artboard 1 (1 of 1)  ·  2026-10-05 11:33 UTC");
    assert_eq!(crate::marks::page_info(&d, " ", 0, None), "Untitled  ·  Artboard 1 (1 of 1)");
    // Its glyphs are filled in Registration under the band of bottom marks.
    let bytes = pdf(&d, json!({"marks": {"pageInfo": true}}));
    let [top, ..] = settings(json!({"marks": {"pageInfo": true}})).marks.printer_marks().reach([0.0; 4]);
    let back = import(&bytes).unwrap();
    let glyphs = painted(&back, Paint::is_registration);
    assert!(!glyphs.is_empty());
    let band_end = top + 100.0 + 6.0 + PrinterMarks::LENGTH;
    assert!(glyphs.iter().all(|g| g.y0 > band_end && g.x0 >= top - 0.5), "{glyphs:?}");
}

#[test]
fn page_information_prints_the_time_of_the_export_not_the_creation_date() {
    let d = doc();
    // The outlines of the page information of a PDF of `d` created at `created`.
    let info = |created: i64| {
        let opts = PdfOptions { settings: settings(json!({"marks": {"pageInfo": true}})), created: Some(created), ..PdfOptions::uncompressed() };
        let back = import(&export_with_report(&d, &opts).unwrap().bytes).unwrap();
        let mut out = vec![];
        back.walk(|n| {
            if n.appearance.fill().is_some_and(|f| f.paint.is_registration())
                && let Some(p) = n.path_data()
            {
                out.push(p.clone());
            }
        });
        out
    };
    let now = || vectorcraft_doc::metadata::output_time().unwrap();
    // Both print the minute they are written in: try again when the minute turns between them.
    for _ in 0..3 {
        let minute = now() / 60;
        let (old, new) = (info(1_577_836_800), info(now()));
        if now() / 60 == minute {
            assert!(!old.is_empty());
            assert!(old == new, "a file created in 2020 prints the same date as one created now");
            return;
        }
    }
    panic!("the minute turned during each of three tries");
}
