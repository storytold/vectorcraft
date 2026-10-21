//! A zero width joiner or non-joiner between characters that the style's font lacks is shaped with
//! those characters, in their fallback face. The Unicode Standard describes both joiners under
//! Cursive Connection and Ligatures
//! (https://www.unicode.org/versions/latest/core-spec/chapter-23/), and Microsoft's OpenType shaping
//! specifications for Devanagari and Bengali include them in a syllable after a halant
//! (https://learn.microsoft.com/en-us/typography/script-development/devanagari,
//! https://learn.microsoft.com/en-us/typography/script-development/bengali).
//! No font is committed: the tests use installed faces and print SKIPPED when none is found, except
//! the check of the half-form test's face filter, which builds its faces in code.
use super::*;
use kurbo::Point;
use std::sync::Arc;
use vectorcraft_doc::CharStyle;

const ZWNJ: char = '\u{200C}';
const ZWJ: char = '\u{200D}';

/// A Latin face that maps both joiners but not `c`, and the face that the fallback gives for `c`,
/// which must pass `suits`. Which face the fallback gives depends on the faces loaded so far, so the
/// tests compare with that face.
fn joiner_test_faces(test: &str, c: char, suits: impl Fn(&FontFace) -> bool) -> Option<(Arc<FontFace>, Arc<FontFace>)> {
    let db = FontDb::global();
    let faces = ["Source Sans 3", "Helvetica", "Arial", "Times New Roman", "Liberation Sans", "DejaVu Sans"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular").filter(|face| face.family == name))
        .filter(|primary| primary.covers(ZWNJ) && primary.covers(ZWJ) && !primary.covers(c))
        .find_map(|primary| db.fallback_for(c, primary.id()).filter(|f| suits(f)).map(|fallback| (primary, fallback)));
    if let Some((primary, fallback)) = &faces {
        let path = fallback.path().map_or_else(|| "bundled font".to_string(), |p| p.display().to_string());
        eprintln!("ASSERTIONS RUN: {test}; {} {} with the fallback {} {} ({path})", primary.family, primary.style, fallback.family, fallback.style);
    } else {
        eprintln!("SKIPPED: {test}; no installed Latin face maps both joiners and falls back to a face for {c} with the lookups it needs");
    }
    faces
}

fn layout_in(face: &FontFace, text: &str, tracking: f64) -> TextLayout {
    layout_in_db(FontDb::global(), face, text, tracking)
}

fn layout_in_db(db: &FontDb, face: &FontFace, text: &str, tracking: f64) -> TextLayout {
    let st = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, tracking, ..CharStyle::default() };
    layout(db, &TextObject::point(Point::ZERO, text, st))
}

/// Ka + halant + ZWJ + Ssa: the half form of Ka before a full Ssa.
fn ka_half_ssa() -> String {
    format!("क्{ZWJ}ष")
}

/// Whether `layout` of [`ka_half_ssa`] in `face` starts with a glyph other than Ka's and the
/// halant's (Ka's half form) and ends with Ssa's.
fn starts_with_a_half_form(face: &FontFace, layout: &TextLayout) -> bool {
    let half = layout.glyphs.first().map(|g| g.gid);
    half.is_some_and(|gid| gid != face.glyph_for('क') && gid != face.glyph_for('\u{094D}'))
        && layout.glyphs.last().map(|g| g.gid) == Some(face.glyph_for('ष'))
}

/// Whether `face` can serve the half-form test: its GSUB script list contains `dev2` or `deva`, and
/// [`ka_half_ssa`] set in `face` alone, in `db`, is drawn in `face` with Ka's half form.
fn forms_a_ka_half_form(db: &FontDb, face: &FontFace) -> bool {
    use skrifa::raw::TableProvider;
    let devanagari = face
        .skrifa()
        .and_then(|f| f.gsub().ok())
        .and_then(|gsub| gsub.script_list().ok())
        .is_some_and(|list| list.script_records().iter().any(|r| matches!(&r.script_tag().to_be_bytes(), b"dev2" | b"deva")));
    let l = layout_in_db(db, face, &ka_half_ssa(), 0.0);
    devanagari && l.glyphs.iter().all(|g| g.font_id == face.id()) && starts_with_a_half_form(face, &l)
}

/// `text` set in `primary` is drawn, spaced and stepped through exactly as `text` set in `fallback`.
fn assert_shaped_as_in_the_fallback(primary: &FontFace, fallback: &FontFace, text: &str) {
    for tracking in [0.0, 200.0] {
        let (mixed, single) = (layout_in(primary, text, tracking), layout_in(fallback, text, tracking));
        // The caret stops where it stops in the fallback face.
        assert_eq!(caret_horizontal(&mixed, 0, true), caret_horizontal(&single, 0, true), "{text:?} with tracking {tracking}");
        // Tracking adds no extra space around the joiner.
        let width = |l: &TextLayout| l.glyphs.iter().map(|g| g.advance).sum::<f64>();
        assert_eq!(width(&mixed), width(&single), "{text:?} with tracking {tracking}");
        let glyphs = |l: &TextLayout| l.glyphs.iter().map(|g| (g.font_id, g.gid, g.byte, g.len, g.origin, g.advance)).collect::<Vec<_>>();
        assert_eq!(glyphs(&mixed), glyphs(&single), "{text:?} with tracking {tracking}");
        assert!(mixed.glyphs.iter().all(|g| g.font_id == fallback.id()), "{text:?}: {:?}", mixed.glyphs);
    }
}

#[test]
fn a_zero_width_non_joiner_in_devanagari_is_shaped_with_the_letters_around_it() {
    let test = "a_zero_width_non_joiner_in_devanagari_is_shaped_with_the_letters_around_it";
    let Some((primary, fallback)) = joiner_test_faces(test, 'क', |_| true) else { return };
    // Ka + halant + ZWNJ + Ssa: Ka keeps its explicit halant instead of forming the KaSsa conjunct.
    let s = format!("क्{ZWNJ}ष");
    assert_shaped_as_in_the_fallback(&primary, &fallback, &s);
}

#[test]
fn a_zero_width_non_joiner_in_bangla_is_shaped_with_the_letters_around_it() {
    let test = "a_zero_width_non_joiner_in_bangla_is_shaped_with_the_letters_around_it";
    let Some((primary, fallback)) = joiner_test_faces(test, 'ক', |_| true) else { return };
    let s = format!("ক্{ZWNJ}ষ");
    assert_shaped_as_in_the_fallback(&primary, &fallback, &s);
}

#[test]
fn a_zero_width_joiner_in_devanagari_asks_for_a_half_form() {
    let test = "a_zero_width_joiner_in_devanagari_asks_for_a_half_form";
    let Some((primary, fallback)) = joiner_test_faces(test, 'क', |f| forms_a_ka_half_form(FontDb::global(), f)) else { return };
    let s = ka_half_ssa();
    assert_shaped_as_in_the_fallback(&primary, &fallback, &s);
    let l = layout_in(&primary, &s, 0.0);
    assert!(starts_with_a_half_form(&fallback, &l), "no half form: {:?}", l.glyphs);
}

#[test]
fn a_devanagari_face_without_a_half_form_is_not_used_for_the_half_form_test() {
    use crate::test_fonts::{HALF_FORM_FAMILY, half_form_font};
    for half in [false, true] {
        let db = FontDb::with_font_dirs(vec![]);
        db.add_font(half_form_font(half).unwrap());
        let face = db.face(HALF_FORM_FAMILY, "Regular").unwrap();
        // Both faces list `dev2`; only the one with a `half` feature forms Ka's half form.
        assert_eq!(forms_a_ka_half_form(&db, &face), half, "half feature {half}");
    }
}
