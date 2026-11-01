//! PDF text in installed fonts imports as live type (one point type object per line).

use craft_pdf::write::backend as krilla;
use krilla::geom::{Point as KPoint, Transform};
use krilla::page::PageSettings;
use krilla::text::{Font, TextDirection};
use kurbo::Shape;
use vectorcraft_doc::{Document, NodeKind, TextObject};

use crate::*;

const SOURCE_SANS: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");
const SOURCE_SERIF: &[u8] = include_bytes!("../../../../assets/fonts/SourceSerif4-Regular.ttf");

/// A 300 × 200 pt page with `lines` of (x, y from the top, size, text) in Source Sans 3, and the
/// same rotated 90° when `rotated`.
pub(crate) fn text_pdf(lines: &[(f32, f32, f32, &str)], rotated: bool) -> Vec<u8> {
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
    let mut s = page.surface();
    let font = Font::new(SOURCE_SANS.into(), 0).unwrap();
    if rotated {
        s.push_transform(&Transform::from_row(0.0, 1.0, -1.0, 0.0, 150.0, 20.0));
    }
    for &(x, y, size, text) in lines {
        s.draw_text(KPoint::from_xy(x, y), font.clone(), size, text, false, TextDirection::Auto);
    }
    if rotated {
        s.pop();
    }
    s.finish();
    page.finish();
    pdf.finish().unwrap()
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

fn ink_width(t: &TextObject) -> f64 {
    vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).glyphs.iter().map(|g| g.advance).sum()
}

#[test]
fn lines_of_text_in_installed_fonts_come_in_as_point_type() {
    let bytes = text_pdf(&[(20.0, 50.0, 24.0, "Gagaku Concert"), (20.0, 90.0, 12.0, "Hall A")], false);
    let r = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    let t = texts(&r.document);
    assert_eq!(t.len(), 2, "{t:?}");
    let first = t.iter().find(|t| t.plain_text() == "Gagaku Concert").unwrap();
    let st = first.first_style();
    assert_eq!((st.font_family.as_str(), st.font_style.as_str()), ("Source Sans 3", "Regular"));
    assert!((st.size - 24.0).abs() < 0.01);
    let origin = first.xf * kurbo::Point::ZERO;
    assert!((origin.x - 20.0).abs() < 0.01 && (origin.y - 50.0).abs() < 0.01, "on the PDF's baseline: {origin:?}");
    assert!(st.tracking.abs() <= 1.0, "set as VectorCraft sets it: {}", st.tracking);
    assert!(t.iter().any(|t| t.plain_text() == "Hall A" && (t.first_style().size - 12.0).abs() < 0.01));
    assert!(!r.warnings.iter().any(|w| w.contains("outlines")), "{:?}", r.warnings);
}

#[test]
fn rotated_lines_keep_their_angle_and_outline_text_option_outlines_everything() {
    let bytes = text_pdf(&[(0.0, 0.0, 18.0, "Sideways")], true);
    let d = import_with_report(&bytes, &ImportOptions::default()).unwrap().document;
    let t = texts(&d);
    assert_eq!(t.len(), 1);
    let dir = t[0].xf * kurbo::Point::new(1.0, 0.0) - t[0].xf * kurbo::Point::ZERO;
    assert!(dir.x.abs() < 1e-6 && (dir.y - 1.0).abs() < 1e-6, "the baseline runs down the page: {dir:?}");
    let outlined = import_with_report(&bytes, &ImportOptions { text_as: TextAs::Outlines, ..Default::default() }).unwrap();
    assert!(texts(&outlined.document).is_empty());
    assert!(outlined.warnings.iter().any(|w| w.contains("outlines")));
}

#[test]
fn letter_spacing_in_the_pdf_becomes_tracking() {
    // Source Sans 3 at 20 pt, every letter 2 pt further apart than its advance.
    let mut x = 20.0;
    let mut lines = vec![];
    let word = "Wide";
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    let mut pieces = vec![];
    for c in word.chars() {
        pieces.push((x, c.to_string()));
        x += font.advance(font.glyph_for(c)) as f32 / font.units_per_em() as f32 * 20.0 + 2.0;
    }
    for (x, c) in &pieces {
        lines.push((*x, 60.0, 20.0, c.as_str()));
    }
    let d = import_with_report(&text_pdf(&lines, false), &ImportOptions::default()).unwrap().document;
    let t = texts(&d);
    assert_eq!(t.len(), 1, "{t:?}");
    assert_eq!(t[0].plain_text(), "Wide");
    assert!((t[0].first_style().tracking - 100.0).abs() <= 1.0, "2 pt at 20 pt = 100/1000 em: {}", t[0].first_style().tracking);
    let expected = (x - 2.0 - 20.0) as f64 + 2.0;
    assert!((ink_width(&t[0]) - expected).abs() < 0.3, "{} vs {expected}", ink_width(&t[0]));
}

#[test]
fn common_tracking_does_not_turn_gaps_into_spaces() {
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    for size in [12.0, 13.0, 14.0, 15.0] {
        for tracking in [150.0, 180.0, 200.0, 220.0, 250.0] {
            let mut x = 20.0;
            let mut pieces = vec![];
            for (word_index, word) in ["OPEN", "DAILY", "TYPE"].iter().enumerate() {
                for character in word.chars() {
                    pieces.push((x, character.to_string()));
                    x += font.advance(font.glyph_for(character)) as f32 / font.units_per_em() as f32 * size + size * tracking / 1000.0;
                }
                if word_index < 2 {
                    x += font.advance(font.glyph_for(' ')) as f32 / font.units_per_em() as f32 * size + size * tracking / 1000.0;
                }
            }
            let lines: Vec<_> = pieces.iter().map(|(x, text)| (*x, 60.0, size, text.as_str())).collect();
            let d = import_with_report(&text_pdf(&lines, false), &ImportOptions::default()).unwrap().document;
            let t = texts(&d);
            assert_eq!(t.len(), 1, "size {size}, tracking {tracking}: {t:?}");
            assert_eq!(t[0].plain_text(), "OPEN DAILY TYPE", "size {size}, tracking {tracking}");
            assert!((t[0].first_style().tracking - f64::from(tracking)).abs() <= 10.0, "size {size}, tracking {tracking}: {:?}", t[0].runs);
        }
    }
}

/// Words of one letter each, placed one by one with ordinary word spaces between them, are words:
/// even gaps alone don't read as letter spacing.
#[test]
fn evenly_spaced_one_letter_words_keep_their_spaces() {
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    let size = 12.0_f32;
    for gap in [0.25_f32, 0.3, 0.35] {
        let mut x = 20.0;
        let mut lines = vec![];
        for text in ["A", "B", "C", "D"] {
            lines.push((x, 60.0, size, text));
            x += font.advance(font.glyph_for(text.chars().next().unwrap())) as f32 / font.units_per_em() as f32 * size + gap * size;
        }
        let d = import_with_report(&text_pdf(&lines, false), &ImportOptions::default()).unwrap().document;
        let t = texts(&d);
        assert_eq!(t.len(), 1, "gap {gap} em: {t:?}");
        assert_eq!(t[0].plain_text(), "A B C D", "gap {gap} em");
    }
}

/// Glyphs set more than half an em apart, each on its own, are words or columns: every gap stays a
/// space, however evenly they are spaced.
#[test]
fn evenly_wide_gaps_stay_spaces() {
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    let size = 12.0_f32;
    let one = font.advance(font.glyph_for('1')) as f32 / font.units_per_em() as f32 * size;
    for gap in [0.55_f32, 0.75, 1.0] {
        let lines: Vec<_> =
            ["1", "2", "3", "4"].iter().enumerate().map(|(i, text)| (20.0 + i as f32 * (one + gap * size), 60.0, size, *text)).collect();
        let d = import_with_report(&text_pdf(&lines, false), &ImportOptions::default()).unwrap().document;
        let t = texts(&d);
        assert_eq!(t.len(), 1, "gap {gap} em: {t:?}");
        assert_eq!(t[0].plain_text(), "1 2 3 4", "gap {gap} em");
    }
}

/// A word set beside a much larger one, apart by a gap and no space glyph (as a browser prints a
/// margin between two sizes), reads as two words: the gap is measured against the smaller size
/// (#3623). Set close, the two sizes stay one word.
#[test]
fn a_gap_between_two_sizes_is_measured_against_the_smaller() {
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    let advance = |c: char, size: f32| font.advance(font.glyph_for(c)) as f32 / font.units_per_em() as f32 * size;
    // 3.4 pt: 0.17 em of the 20 pt "4", 0.43 em of the 8 pt "hours".
    for (gap, want) in [(3.4_f32, "4 hours"), (0.4, "4hours")] {
        let lines = [(20.0, 60.0, 20.0, "4"), (20.0 + advance('4', 20.0) + gap, 60.0, 8.0, "hours")];
        let d = import_with_report(&text_pdf(&lines, false), &ImportOptions::default()).unwrap().document;
        let t = texts(&d);
        assert_eq!(t.len(), 1, "gap {gap} pt: {t:?}");
        assert_eq!(t[0].plain_text(), want, "gap {gap} pt");
    }
    // The small size first.
    let lines = [(20.0, 60.0, 8.0, "ca"), (20.0 + advance('c', 8.0) + advance('a', 8.0) + 3.4, 60.0, 20.0, "4")];
    let t = texts(&import_with_report(&text_pdf(&lines, false), &ImportOptions::default()).unwrap().document);
    assert_eq!(t.iter().map(TextObject::plain_text).collect::<Vec<_>>(), ["ca 4"]);
}

/// Embedded font data is untrusted: anything that isn't a CID-keyed CFF is no table, never a panic.
#[test]
fn cid_tables_ignore_fonts_that_are_not_cid_keyed_cff() {
    assert!(crate::import::CidText::of(&[]).is_none());
    assert!(crate::import::CidText::of(b"not a font at all").is_none());
    let mut x: u32 = 0x9E37_79B9;
    for len in [4usize, 64, 1024, 65_536] {
        let junk: Vec<u8> = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x as u8
            })
            .collect();
        assert!(crate::import::CidText::of(&junk).is_none(), "{len} bytes");
        // A CFF header in front of the junk.
        let mut cff = vec![1, 0, 4, 4];
        cff.extend_from_slice(&junk);
        let _ = crate::import::CidText::of(&cff);
    }
    let sans = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).unwrap();
    assert!(crate::import::CidText::of(&sans).is_none(), "a TrueType font has no CID charset");
}

#[test]
fn kangxi_radicals_from_cid_tables_become_ideographs() {
    assert_eq!(crate::import::unify_radical('\u{2FD3}'), '龍');
    assert_eq!(crate::import::unify_radical('\u{2F00}'), '一');
    assert_eq!(crate::import::unify_radical('笙'), '笙');
}

#[test]
fn a_glyph_whose_unicode_names_another_letter_stays_outlined() {
    // The glyph of `l` labelled `W` in the PDF's ToUnicode map: opening it as live `W` would change
    // what the page shows.
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    let l = font.glyph_for('l');
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
    let mut s = page.surface();
    let kfont = Font::new(SOURCE_SANS.into(), 0).unwrap();
    let glyph = krilla::text::KrillaGlyph {
        glyph_id: krilla::text::GlyphId::new(l),
        text_range: 0..1,
        x_advance: 0.25,
        x_offset: 0.0,
        y_offset: 0.0,
        y_advance: 0.0,
        location: None,
    };
    s.draw_glyphs(KPoint::from_xy(20.0, 50.0), &[glyph], kfont, "W", 24.0, false);
    s.finish();
    page.finish();
    let r = import_with_report(&pdf.finish().unwrap(), &ImportOptions::default()).unwrap();
    assert!(texts(&r.document).is_empty(), "{:?}", texts(&r.document));
    assert!(r.warnings.iter().any(|w| w.contains("differ from the installed font")), "{:?}", r.warnings);
}

/// Invisible text (rendering mode 3) shows no glyphs that could differ from the installed font:
/// the glyph of `l` labelled `W` comes in as type showing `W`, and nothing is said to be kept as
/// outlines (#2675).
#[test]
fn invisible_text_is_type_whatever_its_glyphs_look_like() {
    let font = vectorcraft_text::FontDb::global().find_postscript("SourceSans3-Regular").unwrap();
    let l = font.glyph_for('l');
    let mut pdf = krilla::Document::new_with(krilla::SerializeSettings { compress_content_streams: false, ..Default::default() });
    let mut page = pdf.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
    let mut s = page.surface();
    let kfont = Font::new(SOURCE_SANS.into(), 0).unwrap();
    let glyph = krilla::text::KrillaGlyph::new(krilla::text::GlyphId::new(l), 0.25, 0.0, 0.0, 0.0, 0..1, None);
    s.draw_glyphs(KPoint::from_xy(20.0, 50.0), &[glyph], kfont, "W", 24.0, false);
    s.finish();
    page.finish();
    let mut bytes = pdf.finish().unwrap();
    // Filled (mode 0) made invisible: the same length, so the cross-reference table still holds.
    let at = bytes.windows(4).position(|w| w == b"0 Tr").expect("a rendering mode");
    bytes[at] = b'3';
    let r = import_with_report(&bytes, &ImportOptions::default()).unwrap();
    let t = texts(&r.document);
    assert_eq!(t.iter().map(TextObject::plain_text).collect::<Vec<_>>(), ["W"]);
    assert!(t[0].runs.iter().all(|r| r.style.fill.is_none() && !r.style.has_stroke()), "{:?}", t[0].runs);
    assert!(!r.warnings.iter().any(|w| is_outlines_note(w) || w == INVISIBLE_TEXT_NOTE), "{:?}", r.warnings);
}

/// `font` with each of `names` (`from`, `to`, the same length) replaced, in ASCII and UTF-16.
fn renamed(font: &[u8], names: &[(&str, &str)]) -> Vec<u8> {
    let mut bytes = font.to_vec();
    let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
    for &(from, to) in names {
        assert_eq!(from.len(), to.len());
        for (from, to) in [(from.as_bytes().to_vec(), to.as_bytes().to_vec()), (utf16(from), utf16(to))] {
            let mut i = 0;
            while let Some(at) = bytes[i..].windows(from.len()).position(|w| w == from.as_slice()) {
                bytes[i + at..i + at + from.len()].copy_from_slice(&to);
                i += at + from.len();
            }
        }
    }
    bytes
}

/// Source Sans 3 renamed `SourceSans9-Regular` (a font no machine has installed).
fn uninstalled_font() -> Vec<u8> {
    renamed(SOURCE_SANS, &[("SourceSans3-Regular", "SourceSans9-Regular")])
}

/// Source Serif 4 renamed `SourceSerif9-Regular`, family Source Serif 9: a serif font that isn't
/// installed (the fallback font, Source Sans 3, is a sans).
fn uninstalled_serif() -> Vec<u8> {
    renamed(SOURCE_SERIF, &[("SourceSerif4", "SourceSerif9"), ("Source Serif 4", "Source Serif 9")])
}

/// A page with `text` in `font` (24 pt, at 20, 50).
fn one_line_pdf(font: Vec<u8>, text: &str) -> Vec<u8> {
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
    let mut s = page.surface();
    s.draw_text(KPoint::from_xy(20.0, 50.0), Font::new(font.into(), 0).unwrap(), 24.0, text, false, TextDirection::Auto);
    s.finish();
    page.finish();
    pdf.finish().unwrap()
}

/// The outlines text imported as (`textAs: outlines`), as one path.
fn outlines(d: &Document) -> kurbo::BezPath {
    let mut all = kurbo::BezPath::new();
    d.walk(|n| {
        if let NodeKind::Path { path, .. } = &n.kind {
            all.extend(path.to_bezpath().iter());
        }
    });
    all
}

#[test]
fn outlined_text_in_a_missing_font_keeps_the_embedded_glyphs() {
    let outlined = |bytes: &[u8]| {
        let r = import_with_report(bytes, &ImportOptions { text_as: TextAs::Outlines, ..Default::default() }).unwrap();
        assert!(texts(&r.document).is_empty());
        outlines(&r.document)
    };
    let missing = outlined(&one_line_pdf(uninstalled_serif(), "Serif Hamburg"));
    let installed = outlined(&one_line_pdf(SOURCE_SERIF.to_vec(), "Serif Hamburg"));
    let fallback = outlined(&one_line_pdf(SOURCE_SANS.to_vec(), "Serif Hamburg"));
    let (m, i, f) = (missing.bounding_box(), installed.bounding_box(), fallback.bounding_box());
    assert!(m.width() > 50.0, "{m:?}");
    let near = |a: kurbo::Rect, b: kurbo::Rect| (a.x0 - b.x0).abs() + (a.x1 - b.x1).abs() + (a.y0 - b.y0).abs() + (a.y1 - b.y1).abs() < 0.01;
    assert!(near(m, i), "the file's own serif glyphs: {m:?} vs {i:?}");
    assert!(!near(m, f), "not the fallback font's: {m:?} vs {f:?}");
}

#[test]
fn text_in_a_missing_font_stays_live_type() {
    for stroke_only in [false, true] {
        let mut pdf = krilla::Document::new();
        let mut page = pdf.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
        let mut s = page.surface();
        let font = Font::new(uninstalled_font().into(), 0).unwrap();
        if stroke_only {
            s.set_fill(None);
            s.set_stroke(Some(krilla::paint::Stroke::default()));
        }
        s.draw_text(KPoint::from_xy(20.0, 50.0), font, 24.0, "Gagaku", false, TextDirection::Auto);
        s.finish();
        page.finish();
        let r = import_with_report(&pdf.finish().unwrap(), &ImportOptions::default()).unwrap();
        assert_eq!(r.document.layers.len(), 1, "no extra layers");
        let t = texts(&r.document);
        assert_eq!(t.len(), 1, "stroke only {stroke_only}");
        assert_eq!(t[0].plain_text(), "Gagaku");
        // Named by the family its PostScript name reads as, so it picks the font up once installed.
        let family = t[0].first_style().font_family.clone();
        assert_eq!(family, "Source Sans 9");
        assert!(r.warnings.iter().any(|w| w.contains(family.as_str()) && w.contains("fallback font")), "{:?}", r.warnings);
    }
}

#[test]
fn a_single_upright_glyph_comes_in_as_vertical_type_and_takes_no_horizontal_glyphs() {
    use crate::import_text::{Look, Placement, TextLine, Upright};
    use vectorcraft_geom::{Point, Vec2};
    let look = Look {
        font: 1,
        family: "Source Sans 3".into(),
        style: "Regular".into(),
        version: None,
        size: 20.0,
        h_scale: 100.0,
        fill: Some(vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 0.0))),
        stroke: None,
        overprint_fill: false,
        overprint_stroke: false,
    };
    let at = |x: f64, y: f64| Placement { origin: Point::new(x, y), dir: Vec2::new(1.0, 0.0), size: 20.0, h_scale: 100.0, slant: 0.0 };
    let mut line = TextLine::new(at(100.0, 100.0), 1.0);
    assert!(line.push_upright(&look, at(100.0, 100.0), 1.0, Upright { top: Point::new(110.0, 82.0) }, "§"));
    assert!(!line.push(&look, at(130.0, 100.0), 1.0, 10.0, "a"), "a horizontal glyph starts another line");
    let (t, ..) = line.finish().unwrap();
    assert!(t.vertical, "one upright glyph is vertical type");
    assert_eq!(t.plain_text(), "§");
}

#[test]
fn letter_spaced_vertical_type_keeps_its_characters_together() {
    use crate::import_text::{Look, Placement, TextLine, Upright};
    use vectorcraft_geom::{Point, Vec2};
    let look = Look {
        font: 1,
        family: "Source Sans 3".into(),
        style: "Regular".into(),
        version: None,
        size: 20.0,
        h_scale: 100.0,
        fill: Some(vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(0.0, 0.0, 0.0))),
        stroke: None,
        overprint_fill: false,
        overprint_stroke: false,
    };
    // Glyphs set down a column 1.5 em apart (tracking 500): no spaces come in between them.
    let at = |y: f64| Placement { origin: Point::new(100.0, y), dir: Vec2::new(1.0, 0.0), size: 20.0, h_scale: 100.0, slant: 0.0 };
    let mut line = TextLine::new(at(100.0), 1.0);
    for (i, c) in ["§", "§", "§"].iter().enumerate() {
        let y = 100.0 + 30.0 * i as f64;
        assert!(line.push_upright(&look, at(y), 1.0, Upright { top: Point::new(110.0, y - 18.0) }, c));
    }
    let (t, ..) = line.finish().unwrap();
    assert!(t.vertical);
    assert_eq!(t.plain_text(), "§§§", "letter spacing isn't a space");
}

#[test]
fn a_tagged_span_inside_a_line_leaves_the_line_whole() {
    use krilla::tagging::{ContentTag, SpanTag};
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::from_wh(300.0, 200.0).unwrap());
    let mut s = page.surface();
    let font = Font::new(SOURCE_SANS.into(), 0).unwrap();
    // "Ga" "ga" "ku" with the middle part in its own marked content (a span, as apps write for
    // actual text), all on one baseline.
    s.draw_text(KPoint::from_xy(20.0, 50.0), font.clone(), 24.0, "Ga", false, TextDirection::Auto);
    let x = 20.0 + ink_width_of(&font, 24.0, "Ga");
    s.start_tagged(ContentTag::Span(SpanTag::empty()));
    s.draw_text(KPoint::from_xy(x, 50.0), font.clone(), 24.0, "ga", false, TextDirection::Auto);
    s.end_tagged();
    let x = x + ink_width_of(&font, 24.0, "ga");
    s.draw_text(KPoint::from_xy(x, 50.0), font, 24.0, "ku", false, TextDirection::Auto);
    s.finish();
    page.finish();
    let d = import(&pdf.finish().unwrap()).unwrap();
    let t: Vec<String> = texts(&d).iter().map(TextObject::plain_text).collect();
    assert_eq!(t, ["Gagaku"]);
}

/// Advance of `text` in `font` at `size` (from the bundled Source Sans 3, as laid out).
fn ink_width_of(_font: &Font, size: f32, text: &str) -> f32 {
    let style = vectorcraft_doc::CharStyle { font_family: "Source Sans 3".into(), size: f64::from(size), ..Default::default() };
    ink_width(&TextObject::point(vectorcraft_geom::Point::ZERO, text, style)) as f32
}

/// Hebrew drawn by a PDF (in visual order, as VectorCraft's own export draws it) comes back as type
/// in logical order that shows as drawn. Needs an installed font with Hebrew (skipped without one).
#[test]
fn hebrew_comes_back_in_logical_order() {
    let db = vectorcraft_text::FontDb::global();
    let Some(face) =
        ["Arial", "Noto Sans Hebrew", "DejaVu Sans", "Liberation Sans"].into_iter().filter_map(|f| db.face(f, "Regular")).find(|f| f.covers('ש'))
    else {
        return;
    };
    for text in ["שלום עולם", "שלום 123", "Hello שלום"] {
        let style = vectorcraft_doc::CharStyle { font_family: face.family.clone(), size: 24.0, ..Default::default() };
        let mut d = Document::new(300.0, 200.0);
        let l = d.layers[0].id;
        let n = vectorcraft_doc::Node::new(
            d.alloc_id(),
            NodeKind::Text(Box::new(TextObject::point(vectorcraft_geom::Point::new(20.0, 60.0), text, style))),
        );
        d.insert(Some(l), 0, n).unwrap();
        let settings: PdfSettings = serde_json::from_value(serde_json::json!({"advanced": {"outlineText": false}})).unwrap();
        let bytes = export_with_report(&d, &PdfOptions { settings, ..Default::default() }).unwrap().bytes;
        let r = import_with_report(&bytes, &ImportOptions::default()).unwrap();
        let t = texts(&r.document);
        assert_eq!(t.iter().map(TextObject::plain_text).collect::<Vec<_>>(), [text], "{:?}", r.warnings);
        let shown = |t: &TextObject| -> String {
            let plain = t.plain_text();
            vectorcraft_text::layout(db, t).glyphs.iter().filter_map(|g| plain.get(g.byte..)?.chars().next()).collect()
        };
        let original = TextObject::point(vectorcraft_geom::Point::ZERO, text, vectorcraft_doc::CharStyle::default());
        assert_eq!(shown(&t[0]), shown(&original), "{text}");
    }
}
