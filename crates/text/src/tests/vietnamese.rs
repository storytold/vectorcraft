//! Vietnamese letters with two diacritics (Ể, Ứ, Ự, Ẩ…) draw both marks on their base letter
//! (#4491), whichever form the text comes in: precomposed (NFC), decomposed (NFD), or as
//! Vietnamese keyboards type it (a precomposed vowel and a combining tone mark), in a font with
//! the precomposed letters, one that builds them from its marks (GPOS mark-to-base and
//! mark-to-mark), and one that has to borrow the marks from a fallback font.

use kurbo::{Point, Rect, Shape};
use skrifa::MetadataProvider;
use unicode_segmentation::UnicodeSegmentation;
use vectorcraft_doc::{CharStyle, TextObject};
use write_fonts::FontBuilder;
use write_fonts::types::Tag;

use crate::test_fonts::name_table;
use crate::{FontDb, TextLayout, layout};

/// Letters as (precomposed, decomposed, keyboard) text: the words "HỌC ĐỂ ỨNG DỤNG" and more
/// letters with two marks, capital and small.
const LETTERS: [[&str; 3]; 20] = [
    ["H", "H", "H"],
    ["\u{1ECC}", "O\u{323}", "O\u{323}"],
    ["C", "C", "C"],
    ["\u{110}", "\u{110}", "\u{110}"],
    ["\u{1EC2}", "E\u{302}\u{309}", "\u{CA}\u{309}"],
    ["\u{1EE8}", "U\u{31B}\u{301}", "\u{1AF}\u{301}"],
    ["N", "N", "N"],
    ["G", "G", "G"],
    ["D", "D", "D"],
    ["\u{1EE4}", "U\u{323}", "U\u{323}"],
    ["\u{1EF0}", "U\u{31B}\u{323}", "\u{1AF}\u{323}"],
    ["\u{1EA8}", "A\u{302}\u{309}", "\u{C2}\u{309}"],
    ["\u{1EBE}", "E\u{302}\u{301}", "\u{CA}\u{301}"],
    ["\u{1ED8}", "O\u{302}\u{323}", "\u{D4}\u{323}"],
    ["\u{1EDC}", "O\u{31B}\u{300}", "\u{1A0}\u{300}"],
    ["\u{1EC3}", "e\u{302}\u{309}", "\u{EA}\u{309}"],
    ["\u{1EE9}", "u\u{31B}\u{301}", "\u{1B0}\u{301}"],
    ["\u{1EF1}", "u\u{31B}\u{323}", "\u{1B0}\u{323}"],
    ["\u{1EA9}", "a\u{302}\u{309}", "\u{E2}\u{309}"],
    ["\u{1EAB}", "a\u{306}\u{303}", "\u{103}\u{303}"],
];

const SIZE: f64 = 100.0;
const SOURCE_SANS: &[u8] = include_bytes!("../../../../assets/fonts/SourceSans3-Regular.ttf");

fn text(form: usize) -> String {
    LETTERS.iter().filter_map(|l| l.get(form).copied()).collect::<Vec<_>>().join(" ")
}

/// Each letter's ink, relative to the pen position of its first glyph.
fn letter_inks(db: &FontDb, family: &str, text: &str) -> Vec<Rect> {
    let st = CharStyle { font_family: family.into(), font_style: "Regular".into(), size: SIZE, ..CharStyle::default() };
    let l: TextLayout = layout(db, &TextObject::point(Point::ZERO, text, st));
    text.grapheme_indices(true)
        .filter(|(_, g)| *g != " ")
        .map(|(start, g)| {
            let glyphs: Vec<_> = l.glyphs.iter().filter(|gl| (start..start + g.len()).contains(&gl.byte)).collect();
            assert!(!glyphs.is_empty(), "{g:?} has glyphs");
            let pen = glyphs[0].origin.x;
            let ink = glyphs.iter().map(|gl| gl.outline.bounding_box()).reduce(|a, b| a.union(b)).unwrap();
            assert!(ink.area() > 0.0, "{g:?} draws");
            assert!(glyphs.iter().all(|gl| gl.gid != 0), "{g:?} has no missing glyph");
            ink - kurbo::Vec2::new(pen, 0.0)
        })
        .collect()
}

/// Every letter of `form` (drawn in `family` of `db`) has the ink of the precomposed letter in
/// Source Sans 3: its marks are on the letter, not beside it.
fn assert_attached(db: &FontDb, family: &str, form: usize) {
    let want = letter_inks(FontDb::global(), "Source Sans 3", &text(0));
    let got = letter_inks(db, family, &text(form));
    assert_eq!(want.len(), got.len());
    for ((w, g), letter) in want.iter().zip(&got).zip(LETTERS) {
        let off = [w.x0 - g.x0, w.x1 - g.x1, w.y0 - g.y0, w.y1 - g.y1].into_iter().fold(0.0_f64, |m, d| m.max(d.abs()));
        assert!(off < 0.08 * SIZE, "{family}, form {form}: {:?} drawn {g:?}, precomposed {w:?}", letter[form]);
    }
}

#[test]
fn stacked_marks_sit_on_their_letter_in_every_form() {
    for form in 0..3 {
        assert_attached(FontDb::global(), "Source Sans 3", form);
    }
}

/// Source Sans 3 renamed `family`, with only the characters `keep` takes in its cmap.
fn source_sans_with(family: &str, keep: impl Fn(u32) -> bool) -> Vec<u8> {
    let font = skrifa::FontRef::new(SOURCE_SANS).unwrap();
    let map: Vec<(u32, u16)> = font.charmap().mappings().filter(|(c, _)| keep(*c)).map(|(c, g)| (c, u16::try_from(g.to_u32()).unwrap())).collect();
    let mut builder = FontBuilder::new();
    builder.add_raw(Tag::new(b"cmap"), crate::embed::cmap(&map).unwrap());
    builder.add_raw(Tag::new(b"name"), name_table(&[(1, family), (2, "Regular"), (4, family), (6, "Viettest-Regular")]).unwrap());
    for r in font.table_directory.table_records() {
        let tag = Tag::new(&r.tag().to_be_bytes());
        if !builder.contains(tag) {
            builder.add_raw(tag, font.table_data(r.tag()).unwrap().as_bytes());
        }
    }
    builder.build()
}

/// A font without the precomposed letters builds them from its base letters and marks.
#[test]
fn stacked_marks_are_positioned_mark_to_mark() {
    const FAMILY: &str = "Viettest Marks";
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(source_sans_with(FAMILY, |c| c < 0x1E00 && !(0xC0..0x250).contains(&c))), 1);
    for form in 0..3 {
        assert_attached(&db, FAMILY, form);
    }
}

/// A font with the base letters but none of the marks takes the whole letter, marks included,
/// from a font that has them.
#[test]
fn letters_whose_marks_a_font_lacks_fall_back_whole() {
    const FAMILY: &str = "Viettest Bare";
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(source_sans_with(FAMILY, |c| c < 0x80)), 1);
    for form in 0..3 {
        assert_attached(&db, FAMILY, form);
    }
}
