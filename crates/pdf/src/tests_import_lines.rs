//! What a file's lines of type were (#508): the layer art ending a group's marked content belongs
//! to, layers ordered as the pages mark them, a stroke written as stroked glyph outlines, spaces
//! read from wide gaps, and lines wrapped in a frame rebuilt as area type.

use kurbo::{BezPath, PathEl, Point, Shape};
use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{AppearanceItem, CharStyle, Document, Justify, Node, NodeKind, TextKind, TextObject};
use vectorcraft_geom::{Affine, Rect, shapes};
use vectorcraft_testkit::pdf::{PdfPage, first_extra, pdf_with_catalog};

use crate::*;

const HELVETICA: &str = "/Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica >> >>";

/// Groups "A" and "B" (A painted first) for a file of `pages` pages, and the catalog entries.
fn groups(pages: usize) -> ([String; 2], String, String) {
    let (a, b) = (first_extra(pages), first_extra(pages) + 1);
    let objs = ["<< /Type /OCG /Name (A) >>".to_string(), "<< /Type /OCG /Name (B) >>".to_string()];
    let catalog = format!("/OCProperties << /OCGs [{a} 0 R {b} 0 R] /D << /Order [{b} 0 R {a} 0 R] >> >> ");
    let resources = format!("/Properties << /MC0 {a} 0 R /MC1 {b} 0 R >> {HELVETICA}");
    (objs, catalog, resources)
}

/// A file of 100 × 100 pt pages drawing `contents`, with groups A (`/MC0`) and B (`/MC1`).
fn layered(contents: &[&str]) -> Document {
    let (objs, catalog, resources) = groups(contents.len());
    let pages: Vec<PdfPage> = contents.iter().map(|c| PdfPage { resources: resources.clone(), ..PdfPage::new(100.0, 100.0, c) }).collect();
    import(&pdf_with_catalog(&pages, &[&objs[0], &objs[1]], &catalog, None)).unwrap()
}

/// The layers' names, bottom first, and the kinds of their art.
fn layers(d: &Document) -> Vec<(String, Vec<&'static str>)> {
    let kind = |n: &std::sync::Arc<Node>| match n.kind {
        NodeKind::Text(_) => "text",
        NodeKind::Group { .. } => "group",
        _ => "path",
    };
    d.layers.iter().map(|l| (l.name.clone().unwrap_or_default(), l.children().map_or(vec![], |c| c.iter().map(kind).collect()))).collect()
}

fn texts(d: &Document) -> Vec<TextObject> {
    let mut v = vec![];
    d.walk(|n| {
        if let NodeKind::Text(t) = &n.kind {
            v.push((**t).clone());
        }
    });
    v
}

fn named(layers: &[(&str, &[&'static str])]) -> Vec<(String, Vec<&'static str>)> {
    layers.iter().map(|(n, k)| (n.to_string(), k.to_vec())).collect()
}

#[test]
fn type_ending_a_layers_marked_content_stays_in_its_layer() {
    // The line is still being gathered when the group's sequence ends.
    let d = layered(&["/OC /MC0 BDC 0 0 1 rg 0 0 10 10 re f BT /F1 12 Tf 10 50 Td (Lead) Tj ET EMC /OC /MC1 BDC EMC"]);
    assert_eq!(layers(&d), named(&[("A", &["path", "text"]), ("B", &[])]));
}

#[test]
fn a_clip_ending_in_a_later_group_stays_with_its_art() {
    // The clip opens in A, its art is drawn in A, and it closes after B's (empty) sequence.
    let d = layered(&["/OC /MC0 BDC q 0 0 50 50 re W n 1 0 0 rg 40 40 20 20 re f EMC /OC /MC1 BDC EMC Q"]);
    assert_eq!(layers(&d), named(&[("A", &["group"]), ("B", &[])]));
}

#[test]
fn layers_stack_as_the_pages_mark_them_when_one_first_paints_on_a_later_page() {
    // A, the bottom layer, is empty on page 1 and has art on page 2 only.
    let d = layered(&["/OC /MC0 BDC EMC /OC /MC1 BDC 1 0 0 rg 0 0 10 10 re f EMC", "/OC /MC0 BDC 0 0 1 rg 0 0 10 10 re f EMC /OC /MC1 BDC EMC"]);
    assert_eq!(layers(&d), named(&[("A", &["path"]), ("B", &["path"])]));
    // A group marked with no art on any page is an empty layer in its place.
    let d = layered(&["/OC /MC0 BDC EMC /OC /MC1 BDC 1 0 0 rg 0 0 10 10 re f EMC"]);
    assert_eq!(layers(&d), named(&[("A", &[]), ("B", &["path"])]));
}

/// A 100 × 100 pt page drawing `content` in Helvetica.
fn page(content: &str) -> Vec<u8> {
    vectorcraft_testkit::pdf::pdf(&[PdfPage { resources: HELVETICA.into(), ..PdfPage::new(100.0, 100.0, content) }], None)
}

/// The outline of glyph `c` drawn at 24 pt from (x, 50), in PDF operators (y up), stroked.
fn outline(c: char, x: f64) -> String {
    let opts = ImportOptions { text_as: TextAs::Outlines, ..Default::default() };
    let d = import_with_report(&page(&format!("BT /F1 24 Tf {x} 50 Td ({c}) Tj ET")), &opts).unwrap().document;
    let mut p: Option<BezPath> = None;
    d.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            p = Some(path.to_bezpath());
        }
    });
    let pt = |q: Point| format!("{:.3} {:.3}", q.x, 100.0 - q.y);
    let ops: String = p
        .unwrap()
        .elements()
        .iter()
        .map(|e| match e {
            PathEl::MoveTo(a) => format!("{} m ", pt(*a)),
            PathEl::LineTo(a) => format!("{} l ", pt(*a)),
            PathEl::QuadTo(a, b) => format!("{} {} {} c ", pt(*a), pt(*b), pt(*b)),
            PathEl::CurveTo(a, b, c) => format!("{} {} {} c ", pt(*a), pt(*b), pt(*c)),
            PathEl::ClosePath => "h ".into(),
        })
        .collect();
    ops + "S "
}

#[test]
fn a_stroke_written_as_stroked_glyph_outlines_is_the_types_stroke() {
    // "Hi": the H is 722 units wide in Helvetica.
    let strokes = format!("0 0 1 RG 0.5 w {}{}", outline('H', 10.0), outline('i', 10.0 + 0.722 * 24.0));
    let text = "BT 1 0 0 rg /F1 24 Tf 10 50 Td (Hi) Tj ET ";
    for (content, below) in [(format!("{text}{strokes}"), false), (format!("{strokes}{text}"), true)] {
        let d = import(&page(&content)).unwrap();
        let art = d.layers[0].children().unwrap();
        assert_eq!(art.len(), 1, "one object: {art:?}");
        let NodeKind::Text(t) = &art[0].kind else { panic!("{:?}", art[0].kind) };
        assert_eq!(t.plain_text(), "Hi");
        assert_eq!(t.first_style().fill.color(), Some(Color::rgb(1.0, 0.0, 0.0)));
        let ap = &art[0].appearance;
        let [AppearanceItem::Stroke(st)] = ap.items.as_slice() else { panic!("{ap:?}") };
        assert_eq!(st.paint.color(), Some(Color::rgb(0.0, 0.0, 1.0)));
        assert!((st.width - 0.5).abs() < 1e-6);
        assert_eq!(ap.contents_at(), usize::from(below), "a stroke drawn first paints below the characters");
    }
    // A stroke that isn't on the glyphs stays a path.
    let d = import(&page(&format!("{text}0 0 1 RG 0.5 w {}", outline('H', 40.0)))).unwrap();
    assert_eq!(d.layers[0].children().unwrap().len(), 2);
}

/// A `.ai` page moved right in the row, so its art past the artboard clears the page before it
/// (#1528), still has the stroke drawn on its type's glyphs read as the type's stroke.
#[test]
fn a_stroke_on_the_type_of_a_moved_ai_page_is_still_the_types_stroke() {
    let ai = "/PieceInfo << /Illustrator << /Private << /AIPrivateData1 7 /NumBlock 1 >> >> >>";
    let content = format!(
        "0 g -70 10 80 20 re f BT 1 0 0 rg /F1 24 Tf 10 50 Td (Hi) Tj ET 0 0 1 RG 0.5 w {}{}",
        outline('H', 10.0),
        outline('i', 10.0 + 0.722 * 24.0)
    );
    let pages = [("0 g 10 10 20 20 re f", String::new()), (content.as_str(), HELVETICA.to_string())].map(|(c, resources)| PdfPage {
        entries: ai.into(),
        resources,
        ..PdfPage::new(100.0, 100.0, c)
    });
    let d = import(&vectorcraft_testkit::pdf::pdf(&pages, None)).unwrap();
    assert!(d.artboards[1].rect.x0 > d.artboards[0].rect.x1 + 36.0 + 1.0, "page 2 moved: {:?}", d.artboards);
    let art = d.layers[1].children().unwrap();
    assert_eq!(art.len(), 2, "the note and the type: {art:?}");
    assert!(matches!(art[1].appearance.items.as_slice(), [AppearanceItem::Stroke(_)]), "{:?}", art[1].appearance);
}

#[test]
fn glyphs_each_followed_by_their_stroked_outline_stay_one_object() {
    // Type on a path is written glyph by glyph: each glyph, then its outline stroked in the same
    // colour (a shadow under the type).
    let glyph = |c: char, x: f64, stroke: &str| format!("BT 0 0 1 rg /F1 24 Tf {x} 50 Td ({c}) Tj ET {stroke} 0.5 w {}", outline(c, x));
    let i_at = 10.0 + 0.722 * 24.0;
    let d = import(&page(&(glyph('H', 10.0, "0 0 1 RG") + &glyph('i', i_at, "0 0 1 RG")))).unwrap();
    let art = d.layers[0].children().unwrap();
    assert_eq!(art.len(), 1, "one object: {art:?}");
    let NodeKind::Text(t) = &art[0].kind else { panic!("{:?}", art[0].kind) };
    assert_eq!(t.plain_text(), "Hi");
    let [AppearanceItem::Stroke(st)] = art[0].appearance.items.as_slice() else { panic!("{:?}", art[0].appearance) };
    assert_eq!(st.paint.color(), Some(Color::rgb(0.0, 0.0, 1.0)));
    // A stroke of another colour drawn over each glyph in turn would also cover the next one's
    // neighbour as one object's stroke: the glyphs stay as the file draws them.
    let d = import(&page(&(glyph('H', 10.0, "1 1 1 RG") + &glyph('i', i_at, "1 1 1 RG")))).unwrap();
    assert_eq!(texts(&d).len(), 2);
}

/// A 300 × 200 pt page with `lines` of (x, y from the top, size, text) in Source Sans 3.
fn source_sans(lines: &[(f32, f32, f32, &str)]) -> Vec<u8> {
    crate::tests_live_text::text_pdf(lines, false)
}

#[test]
fn spaces_read_from_wide_gaps_keep_the_glyphs_after_them_in_place() {
    // A barcode's digits in three groups, set apart by more than a space.
    let (x1, x2) = (29.0, 69.0);
    let bytes = source_sans(&[(20.0, 50.0, 10.0, "3"), (x1, 50.0, 10.0, "533121"), (x2, 50.0, 10.0, "010629")]);
    let t = texts(&import(&bytes).unwrap());
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].plain_text(), "3 533121 010629");
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]);
    let at = |byte: usize| (t[0].xf * laid.glyphs.iter().find(|g| g.byte == byte).unwrap().origin).x;
    assert!((at(2) - f64::from(x1)).abs() < 0.05, "the second group starts where the file has it: {} vs {x1}", at(2));
    assert!((at(9) - f64::from(x2)).abs() < 0.05, "the third group starts where the file has it: {} vs {x2}", at(9));
}

#[test]
fn words_set_ems_apart_keep_their_place() {
    // Gaps of about 1.75 and 2.6 em (#2384): each space's own tracking is more than an em.
    let (x1, x2) = (48.0, 84.0);
    let bytes = source_sans(&[(20.0, 50.0, 10.0, "ab"), (x1, 50.0, 10.0, "cd"), (x2, 50.0, 10.0, "ef")]);
    let t = texts(&import(&bytes).unwrap());
    assert_eq!(t.len(), 1);
    assert_eq!(t[0].plain_text(), "ab cd ef");
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]);
    let at = |byte: usize| (t[0].xf * laid.glyphs.iter().find(|g| g.byte == byte).unwrap().origin).x;
    assert!((at(3) - f64::from(x1)).abs() < 0.05, "the second word starts where the file has it: {} vs {x1}", at(3));
    assert!((at(6) - f64::from(x2)).abs() < 0.05, "the third word starts where the file has it: {} vs {x2}", at(6));
}

#[test]
fn words_set_ems_apart_beside_the_spaces_the_file_draws_keep_their_place() {
    // The same words, the file drawing each space (2 pt wide) after a word, before the next one,
    // or alone with a gap on each side.
    let (x1, x2) = (48.0, 84.0);
    let drawings: [&[(f32, f32, f32, &str)]; 3] = [
        &[(20.0, 50.0, 10.0, "ab "), (x1, 50.0, 10.0, "cd "), (x2, 50.0, 10.0, "ef")],
        &[(20.0, 50.0, 10.0, "ab"), (x1 - 2.0, 50.0, 10.0, " cd"), (x2 - 2.0, 50.0, 10.0, " ef")],
        &[(20.0, 50.0, 10.0, "ab"), (36.0, 50.0, 10.0, " "), (x1, 50.0, 10.0, "cd"), (70.0, 50.0, 10.0, " "), (x2, 50.0, 10.0, "ef")],
    ];
    for (k, lines) in drawings.iter().enumerate() {
        let t = texts(&import(&source_sans(lines)).unwrap());
        assert_eq!(t.len(), 1, "drawing {k}");
        assert_eq!(t[0].plain_text(), "ab cd ef", "drawing {k}");
        let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]);
        let at = |byte: usize| (t[0].xf * laid.glyphs.iter().find(|g| g.byte == byte).unwrap().origin).x;
        assert!((at(3) - f64::from(x1)).abs() < 0.05, "drawing {k}: the second word starts where the file has it: {} vs {x1}", at(3));
        assert!((at(6) - f64::from(x2)).abs() < 0.05, "drawing {k}: the third word starts where the file has it: {} vs {x2}", at(6));
    }
}

/// A document of area type `text` (Source Sans 3, 10 pt) `width` wide at (20, 30).
fn area_doc(text: &str, justify: Justify, width: f64, space_before: f64) -> Document {
    let style = CharStyle { size: 10.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut t = TextObject::point(Point::ORIGIN, text, style);
    t.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, width, 200.0)) };
    t.xf = Affine::translate((20.0, 30.0));
    t.para.justify = justify;
    t.para.space_before = space_before;
    let mut d = Document::new(300.0, 300.0);
    let l = d.layers[0].id;
    let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// `d` exported (real text) and imported again.
fn round_trip(d: &Document) -> Document {
    let settings: PdfSettings = serde_json::from_value(json!({"compression": {"compressText": false}, "advanced": {"outlineText": false}})).unwrap();
    let bytes = export_with_report(d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
    import(&bytes).unwrap()
}

const PROSE: &str = "Stop losing your sinkers on the cast: the clip holds the lead until a snag pulls it free, \
                     so the fish swims on without the weight.\nIt works on every running rig and leaves the line clear.";

#[test]
fn lines_wrapped_in_a_frame_come_back_as_area_type() {
    for (justify, space) in [(Justify::Left, 0.0), (Justify::JustifyLeft, 4.0)] {
        let d = round_trip(&area_doc(PROSE, justify, 120.0, space));
        let t = texts(&d);
        assert_eq!(t.len(), 1, "{justify:?}: one object, not a line each: {:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
        assert_eq!(t[0].plain_text(), PROSE);
        assert_eq!(t[0].para.justify, justify);
        assert!((t[0].para.space_before - space).abs() < 0.01, "{}", t[0].para.space_before);
        let TextKind::Area { frame } = &t[0].kind else { panic!("{:?}", t[0].kind) };
        let w = frame.to_bezpath().bounding_box().width();
        assert!(w <= 120.5 && w > 100.0, "{w}");
        let origin = t[0].xf * Point::ZERO;
        assert!((origin.x - 20.0).abs() < 0.05, "{origin:?}");
    }
}

#[test]
fn justified_lines_come_back_as_area_type_only_with_their_word_gaps_alike() {
    // "aa bb cc", "aa bb cc", "aa" with the file's spaces drawn, the wrapped lines 44 pt wide: "cc"
    // ends at 64, and "bb" starts where both gaps are 4.87 pt wider than a space, or elsewhere.
    let lines = |bb: f32| {
        [
            (20.0, 50.0, 10.0, "aa "),
            (bb, 50.0, 10.0, "bb "),
            (54.88, 50.0, 10.0, "cc "),
            (20.0, 62.0, 10.0, "aa "),
            (bb, 62.0, 10.0, "bb "),
            (54.88, 62.0, 10.0, "cc "),
            (20.0, 74.0, 10.0, "aa"),
        ]
    };
    let t = texts(&import(&source_sans(&lines(36.95))).unwrap());
    assert_eq!(t.len(), 1, "one object: {:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    assert_eq!(t[0].plain_text(), "aa bb cc aa bb cc aa");
    assert_eq!(t[0].para.justify, Justify::JustifyLeft);
    // Gaps 1 and 8.74 pt wider than a space were set by hand: the lines keep their words in place.
    let t = texts(&import(&source_sans(&lines(33.08))).unwrap());
    assert_eq!(t.len(), 3, "{:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    let laid = vectorcraft_text::layout(vectorcraft_text::FontDb::global(), &t[0]);
    let bb = (t[0].xf * laid.glyphs.iter().find(|g| g.byte == 3).unwrap().origin).x;
    assert!((bb - 33.08).abs() < 0.05, "the second word starts where the file has it: {bb} vs 33.08");
}

#[test]
fn lines_that_were_not_wrapped_stay_point_type() {
    // A list: each line its own, none ending in a space.
    let bytes = source_sans(&[(20.0, 50.0, 10.0, "Hooks"), (20.0, 62.0, 10.0, "Swivels"), (20.0, 74.0, 10.0, "Leads")]);
    assert_eq!(texts(&import(&bytes).unwrap()).len(), 3);
    // Wrapped lines whose breaks a frame doesn't give again (a line far short of the others).
    let bytes = source_sans(&[(20.0, 50.0, 10.0, "One two three four five six "), (20.0, 62.0, 10.0, "a "), (20.0, 74.0, 10.0, "seven")]);
    assert_eq!(texts(&import(&bytes).unwrap()).len(), 3);
}
