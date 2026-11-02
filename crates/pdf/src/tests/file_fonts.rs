//! Type in a font the PDF carries for a family that isn't installed keeps the file's glyphs and
//! their places until its text is edited ([`vectorcraft_doc::FileFonts`]): drawn as it is in the
//! same PDF whose font is installed, saved and read back so, and in the substitute once edited.

use craft_pdf::write::backend as krilla;
use krilla::geom::Point as KPoint;
use krilla::page::PageSettings;
use krilla::text::{Font, GlyphId, KrillaGlyph, TextDirection};
use kurbo::{Point, Rect, Shape};
use vectorcraft_doc::{Document, FILE_FONT_MIME, TextObject};
use vectorcraft_text::{FontDb, FontMatch};

use crate::tests::live_text::{SOURCE_SERIF, outlines, texts, uninstalled_serif};
use crate::*;

/// Lines of (x, y from the top, size, text): kerned pairs, a long line, two sizes.
const LINES: &[(f32, f32, f32, &str)] =
    &[(20.0, 40.0, 24.0, "AVATAR Toyota"), (20.0, 80.0, 11.0, "Wavy quartz jumps over 12 lazy dogs."), (20.0, 120.0, 30.0, "Ky Wo")];
/// A line set glyph by glyph, each 2 pt further on than its advance (as `Tc` or `TJ` set it).
const SPACED: (f32, f32, f32, &str) = (20.0, 160.0, 14.0, "Spaced out");

/// A page of [`LINES`] and [`SPACED`] in `font` (Source Serif 4 or a renamed copy), written as
/// writers write type: a subset of the font embedded as a CID-keyed TrueType font (Identity-H) with
/// a ToUnicode map.
fn page_pdf(font: Vec<u8>) -> Vec<u8> {
    let mut pdf = krilla::Document::new();
    let mut page = pdf.start_page_with(PageSettings::from_wh(400.0, 200.0).unwrap());
    let mut s = page.surface();
    let kfont = Font::new(font.into(), 0).unwrap();
    for &(x, y, size, text) in LINES {
        s.draw_text(KPoint::from_xy(x, y), kfont.clone(), size, text, false, TextDirection::Auto);
    }
    let face = FontDb::global().face("Source Serif 4", "Regular").unwrap();
    let (x, y, size, text) = SPACED;
    let glyphs: Vec<KrillaGlyph> = text
        .char_indices()
        .map(|(i, c)| {
            let g = face.glyph_for(c);
            let advance = face.advance(g) / face.units_per_em() + 2.0 / f64::from(size);
            KrillaGlyph::new(GlyphId::new(g), advance as f32, 0.0, 0.0, 0.0, i..i + c.len_utf8(), None)
        })
        .collect();
    s.draw_glyphs(KPoint::from_xy(x, y), &glyphs, kfont, text, size, false);
    s.finish();
    page.finish();
    pdf.finish().unwrap()
}

fn import(bytes: &[u8]) -> ImportReport {
    import_with_report(bytes, &ImportOptions::default()).unwrap()
}

/// A drawn line: its text, and its glyphs' origins and ink boxes (document space).
type DrawnLine = (String, Vec<(Point, Option<Rect>)>);

/// Each line `db` lays the type of `d` out in ([`DrawnLine`]), sorted by text.
fn drawn(db: &FontDb, d: &Document) -> Vec<DrawnLine> {
    let mut out = vec![];
    for t in texts(d) {
        let lay = vectorcraft_text::layout(db, &t);
        let plain = t.plain_text();
        for (i, line) in lay.lines.iter().enumerate() {
            let glyphs = lay
                .glyphs
                .iter()
                .filter(|g| g.line == i)
                .map(|g| {
                    let ink = (!g.outline.elements().is_empty()).then(|| (t.xf * g.outline.clone()).bounding_box());
                    (t.xf * g.origin, ink)
                })
                .collect();
            out.push((plain[line.start..line.end].trim().to_string(), glyphs));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The largest distance between the glyphs of `a` and `b`, line by line (their texts the same).
fn farthest(a: &[DrawnLine], b: &[DrawnLine]) -> f64 {
    assert_eq!(a.iter().map(|l| &l.0).collect::<Vec<_>>(), b.iter().map(|l| &l.0).collect::<Vec<_>>(), "the same lines");
    let mut far: f64 = 0.0;
    for ((text, ga), (_, gb)) in a.iter().zip(b) {
        assert_eq!(ga.len(), gb.len(), "{text}");
        for ((pa, ia), (pb, ib)) in ga.iter().zip(gb) {
            far = far.max((*pa - *pb).hypot());
            match (ia, ib) {
                (Some(ia), Some(ib)) => {
                    far = far.max((ia.x0 - ib.x0).abs().max((ia.x1 - ib.x1).abs()).max((ia.y0 - ib.y0).abs()).max((ia.y1 - ib.y1).abs()))
                }
                (None, None) => {}
                _ => panic!("{text}: ink on one side only"),
            }
        }
    }
    far
}

fn text_of<'a>(d: &'a [TextObject], text: &str) -> &'a TextObject {
    d.iter()
        .find(|t| t.plain_text() == text)
        .unwrap_or_else(|| panic!("no `{text}` among {:?}", d.iter().map(TextObject::plain_text).collect::<Vec<_>>()))
}

#[test]
fn type_in_a_font_the_file_carries_keeps_its_glyphs_and_places() {
    let db = FontDb::global();
    let installed = import(&page_pdf(SOURCE_SERIF.to_vec()));
    let carried = import(&page_pdf(uninstalled_serif()));
    let t = texts(&carried.document);
    assert_eq!(t.len(), LINES.len() + 1, "a line stays a line: {:?}", t.iter().map(TextObject::plain_text).collect::<Vec<_>>());
    // Named by the file's font, which stays missing (Find Font and the Missing Fonts dialog list it).
    let st = text_of(&t, "AVATAR Toyota").first_style();
    assert_eq!(st.font_family, "Source Serif 9");
    assert_eq!(db.resolve(&st.font_family, &st.font_style).map(|(_, m)| m), Some(FontMatch::Missing));
    // Drawn as the same file whose font is installed: the file's glyphs, where the file put them.
    let far = farthest(&drawn(db, &carried.document), &drawn(db, &installed.document));
    assert!(far < 0.02, "glyphs {far} pt off the installed font's");
    // Every glyph from the font the file carries, which the document keeps.
    for t in &t {
        let lay = vectorcraft_text::layout(db, t);
        assert!(lay.glyphs.iter().all(|g| db.is_file_font(g.font_id)), "{}", t.plain_text());
        let key = &t.file_fonts.as_ref().unwrap().fonts[0].key;
        assert_eq!(carried.document.images[key].mime, FILE_FONT_MIME);
    }
    assert_eq!(carried.document.images.values().filter(|b| b.mime == FILE_FONT_MIME).count(), 1, "one font for the family and style");
    // Where the file put them: the ink is the file's own glyphs' (read as outlines).
    let as_outlines = import_with_report(&page_pdf(uninstalled_serif()), &ImportOptions { text_as: TextAs::Outlines, ..Default::default() }).unwrap();
    let ink = drawn(db, &carried.document).iter().flat_map(|(_, g)| g.iter().filter_map(|(_, i)| *i)).reduce(|a, b| a.union(b)).unwrap();
    let file = outlines(&as_outlines.document).bounding_box();
    let off = (ink.x0 - file.x0).abs().max((ink.x1 - file.x1).abs()).max((ink.y0 - file.y0).abs()).max((ink.y1 - file.y1).abs());
    assert!(off < 0.05, "{ink:?} vs the file's {file:?}");
    // Spacing measured from the file's widths, as with the font installed (not a stand-in's, #3756).
    let tracking = |d: &Document| text_of(&texts(d), "Spaced out").first_style().tracking;
    assert!(
        (tracking(&carried.document) - tracking(&installed.document)).abs() < 1.0,
        "{} vs {}",
        tracking(&carried.document),
        tracking(&installed.document)
    );
    assert!(carried.warnings.iter().any(|w| w.contains("Source Serif 9") && w.contains("until their text is edited")), "{:?}", carried.warnings);
}

#[test]
fn edited_type_takes_the_substitute_and_unedited_type_keeps_the_files_font() {
    let db = FontDb::global();
    let carried = import(&page_pdf(uninstalled_serif()));
    let t = text_of(&texts(&carried.document), "AVATAR Toyota").clone();
    let from_file = |t: &TextObject| vectorcraft_text::layout(db, t).glyphs.iter().filter(|g| db.is_file_font(g.font_id)).count();
    assert_eq!(from_file(&t), vectorcraft_text::layout(db, &t).glyphs.len(), "every glyph");
    // Moved or resized, the text is the file's: still its font.
    let mut moved = t.clone();
    moved.xf = kurbo::Affine::translate((40.0, 10.0)) * moved.xf;
    for r in &mut moved.runs {
        r.style.size *= 2.0;
    }
    assert!(from_file(&moved) > 0);
    // Edited: the substitute (the run still names the missing font).
    let mut edited = t.clone();
    edited.runs[0].text.insert(0, 'Z');
    assert!(edited.file_fonts_in_use().is_empty());
    assert_eq!(from_file(&edited), 0);
    assert_eq!(edited.first_style().font_family, "Source Serif 9");
    // Undone, the file's again.
    edited.runs[0].text.remove(0);
    assert_eq!(from_file(&edited), from_file(&t));
}

#[test]
fn a_font_that_is_installed_keeps_drawing_its_type() {
    // With the family installed nothing is kept: the type is set in the installed font as before.
    let installed = import(&page_pdf(SOURCE_SERIF.to_vec()));
    assert!(!installed.document.images.values().any(|b| b.mime == FILE_FONT_MIME));
    assert!(texts(&installed.document).iter().all(|t| t.file_fonts.is_none()));
}
