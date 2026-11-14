//! The four composers: the Middle Eastern & South Asian ones shape complex scripts with the font's
//! layout tables, the Standard ones set them glyph by glyph as the font maps their characters.
//! Uses a font built in code ([`conjunct_font`]): Ka K, halant a period and vowel sign I i, in
//! Devanagari and Gujarati, with a `half` ligature of K and the period into k.
use crate::test_fonts::{CONJUNCT_FAMILY, conjunct_font, source_sans_gid};
use crate::*;
use kurbo::Point;
use vectorcraft_doc::{CharStyle, Composer, TextObject};

/// Ka, halant, Ka, vowel sign I in Devanagari, then in Gujarati.
const DEVANAGARI: &str = "क\u{94D}क\u{93F}";
const GUJARATI: &str = "ક\u{ACD}ક\u{ABF}";

fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(conjunct_font().expect("the test font builds"));
    db
}

fn object(text: &str, composer: Composer) -> TextObject {
    let st = CharStyle { font_family: CONJUNCT_FAMILY.into(), font_style: "Regular".into(), size: 20.0, ..CharStyle::default() };
    let mut t = TextObject::point(Point::ZERO, text, st);
    t.para.composer = composer;
    t
}

/// The glyph ids drawn for `text` with `composer`, and each glyph's source cluster length.
fn drawn(db: &FontDb, text: &str, composer: Composer) -> Vec<(u32, usize)> {
    layout(db, &object(text, composer)).glyphs.iter().map(|g| (g.gid, g.len)).collect()
}

fn g(c: char) -> u32 {
    u32::from(source_sans_gid(c).expect("a Source Sans glyph"))
}

/// Under the ME&SA composers the conjunct forms (the vowel sign I drawn before its syllable and Ka's
/// half form, in Devanagari and in Gujarati); under the Standard ones each character
/// has its own glyph, in text order, as the font maps it.
#[test]
fn standard_composers_set_indic_text_glyph_by_glyph() {
    let db = db();
    let ids = |v: Vec<(u32, usize)>| v.into_iter().map(|(gid, _)| gid).collect::<Vec<_>>();
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        assert_eq!(ids(drawn(&db, DEVANAGARI, composer)), [g('i'), g('k'), g('K')], "{composer:?} Devanagari");
        assert_eq!(ids(drawn(&db, GUJARATI, composer)), [g('i'), g('k'), g('K')], "{composer:?} Gujarati");
    }
    for composer in [Composer::StandardSingleLine, Composer::StandardEveryLine] {
        let plain = [g('K'), g('.'), g('K'), g('i')];
        let devanagari = drawn(&db, DEVANAGARI, composer);
        assert_eq!(ids(devanagari.clone()), plain, "{composer:?} Devanagari");
        assert_eq!(ids(drawn(&db, GUJARATI, composer)), plain, "{composer:?} Gujarati");
        assert!(devanagari.iter().all(|&(_, len)| len == 3), "each glyph is one character's own cluster: {devanagari:?}");
    }
}

/// The composer is per paragraph: of two paragraphs of the same text, each is shaped by its own.
#[test]
fn each_paragraph_follows_its_own_composer() {
    let db = db();
    let mut t = object(&format!("{DEVANAGARI}\n{DEVANAGARI}"), Composer::EveryLine);
    let standard = vectorcraft_doc::ParaStyle { composer: Composer::StandardEveryLine, ..t.para.clone() };
    t.set_paragraph_styles(vec![t.para.clone(), standard]);
    let l = layout(&db, &t);
    let second = DEVANAGARI.len() + 1;
    let of = |from: usize, to: usize| l.glyphs.iter().filter(|g| (from..to).contains(&g.byte) && g.len > 0).map(|g| g.gid).collect::<Vec<_>>();
    assert_eq!(of(0, second), [g('i'), g('k'), g('K')], "ME&SA paragraph");
    assert_eq!(of(second, second + DEVANAGARI.len()), [g('K'), g('.'), g('K'), g('i')], "Standard paragraph");
}

/// Layout's composer override ([`LayoutOptions::composer`]) decides the shaping too.
#[test]
fn the_layout_composer_override_decides_the_shaping() {
    let db = db();
    let t = object(DEVANAGARI, Composer::EveryLine);
    let with = |c| layout_with(&db, &t, &LayoutOptions { composer: Some(c), ..Default::default() }).glyphs.len();
    assert_eq!((with(Composer::SingleLine), with(Composer::StandardSingleLine)), (3, 4));
}

/// Standard composers break lines as the ME&SA ones do (`is_single_line`), and old files' composers
/// keep their meaning.
#[test]
fn composers_split_into_breaking_and_shaping() {
    use Composer::*;
    for (c, single, shapes) in
        [(SingleLine, true, true), (EveryLine, false, true), (StandardSingleLine, true, false), (StandardEveryLine, false, false)]
    {
        assert_eq!((c.is_single_line(), c.shapes_complex_scripts()), (single, shapes), "{c:?}");
        assert_eq!(c.with_breaking(single), c, "{c:?}");
        assert_eq!(
            (c.with_breaking(!single).is_single_line(), c.with_breaking(!single).shapes_complex_scripts()),
            (!single, shapes),
            "{c:?} toggled"
        );
    }
    assert_eq!(Composer::default(), EveryLine, "new type and old documents are ME&SA Every-line");
}

/// A joiner or non-joiner has no glyph in most fonts and the shaper hides it; a Standard composer
/// draws it as no width at all, not as a missing-glyph box.
#[test]
fn standard_composers_give_format_characters_no_width() {
    let db = db();
    let text = "क\u{200C}क";
    for composer in [Composer::StandardSingleLine, Composer::StandardEveryLine] {
        let l = layout(&db, &object(text, composer));
        let widths: Vec<f64> = l.glyphs.iter().map(|g| g.advance).collect();
        assert_eq!(widths.len(), 3, "{composer:?}: {widths:?}");
        assert_eq!(widths[1], 0.0, "{composer:?}: the non-joiner takes no room: {widths:?}");
        assert!(widths[0] > 0.0 && widths[2] > 0.0, "{composer:?}: {widths:?}");
    }
}
