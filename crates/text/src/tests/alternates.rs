//! A face's alternate glyphs for a character (Glyphs panel › Show › Alternates for Current
//! Selection) and a glyph chosen for one character: read from a `GSUB` table built in code, set by
//! layout, kept by text edits.

use kurbo::Point;
use vectorcraft_doc::{CharStyle, GlyphChoice, TextRun};

use crate::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
use crate::*;

fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(alternates_font(true).expect("the test font builds"));
    db.add_font(alternates_font(false).expect("the test font builds"));
    db
}

fn style(aalt: bool) -> CharStyle {
    CharStyle { font_family: alternates_family(aalt).into(), size: 100.0, ..CharStyle::default() }
}

#[test]
fn alternates_come_in_feature_order_once_each() {
    let db = db();
    let [u1, u2, u3, u4] = unencoded_glyphs().expect("unencoded glyphs");
    let face = db.face(alternates_family(true), "Regular").expect("the test face");
    let g = |c| face.glyph_for(c);
    let listed: Vec<(u32, [u8; 4])> = face.alternates('a').into_iter().map(|a| (a.gid, a.feature)).collect();
    assert_eq!(
        listed,
        [(g('a'), *b"aalt"), (u1, *b"aalt"), (g('b'), *b"aalt"), (u2, *b"ss01"), (u4, *b"ss02"), (u3, *b"swsh"), (g('c'), *b"cv01")],
        "aalt first, then salt (b again, left out), ss01, ss02 (an extension lookup), swsh (u1 again, left out), cv01; smcp is not read"
    );
    assert!(face.alternates('b').is_empty(), "b has no alternates");
    assert!(face.alternates('\u{E000}').is_empty(), "a character the face does not map");
    let plain = db.face(alternates_family(false), "Regular").expect("the test face");
    let listed: Vec<u32> = plain.alternates('a').into_iter().map(|a| a.gid).collect();
    assert_eq!(listed, [g('b'), u2, u4, u3, u1, g('c')], "without aalt the default glyph is not listed");
    let bundled = db.face(FALLBACK_FAMILY, "Regular").expect("the bundled face");
    assert!(bundled.alternates('a').iter().all(|a| a.gid < bundled.glyph_count()));
}

/// The cap on lookups read in all leaves out the lowest ranked features: `salt` is read even when
/// the `cv01` records before it list more lookups than the cap.
#[test]
fn the_lookup_cap_keeps_the_higher_ranked_features() {
    use crate::test_fonts::{CROWDED_ALTERNATES_FAMILY, crowded_alternates_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(crowded_alternates_font().expect("the test font builds"));
    let face = db.face(CROWDED_ALTERNATES_FAMILY, "Regular").expect("the test face");
    let listed: Vec<(u32, [u8; 4])> = face.alternates('a').into_iter().map(|a| (a.gid, a.feature)).collect();
    assert_eq!(listed, [(face.glyph_for('b'), *b"salt"), (face.glyph_for('c'), *b"cv01")]);
}

#[test]
fn a_chosen_glyph_is_drawn_for_its_character_only() {
    let db = db();
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let face = db.face(alternates_family(true), "Regular").expect("the test face");
    let choice = face.choice('a', u1);
    assert_eq!(choice, GlyphChoice { ch: 'a', gid: u1, font: "AlttestSans-Regular".into(), version: "Version 1.000".into() });
    let chosen = CharStyle { glyph_alternate: Some(choice.clone()), ..style(true) };
    let runs = |first: CharStyle| {
        vec![TextRun { text: "aa".into(), style: first, inline: None }, TextRun { text: "ab".into(), style: style(true), inline: None }]
    };
    let set = |runs: Vec<TextRun>| {
        let mut t = TextObject::point(Point::ZERO, "", style(true));
        t.runs = runs;
        layout(&db, &t).glyphs.into_iter().map(|g| (g.gid, g.advance)).collect::<Vec<_>>()
    };
    let a = face.glyph_for('a');
    let k = 100.0 / face.units_per_em();
    let drawn = set(runs(chosen.clone()));
    let gids: Vec<u32> = drawn.iter().map(|g| g.0).collect();
    assert_eq!(gids, [u1, a, a, face.glyph_for('b')], "the run's first character only");
    assert!((drawn[0].1 - face.advance(u1) * k).abs() < 1e-6, "it takes the chosen glyph's advance: {drawn:?}");
    // A choice that does not hold for this text and face draws the default glyph.
    let other = |f: &dyn Fn(&mut GlyphChoice)| {
        let mut c = choice.clone();
        f(&mut c);
        set(runs(CharStyle { glyph_alternate: Some(c), ..style(true) }))[0].0
    };
    assert_eq!(other(&|c| c.ch = 'b'), a, "a choice for another character");
    assert_eq!(other(&|c| c.version = "Version 2.000".into()), a, "another version of the font");
    assert_eq!(other(&|c| c.font = "OtherSans-Regular".into()), a, "another font");
    assert_eq!(other(&|c| c.gid = face.glyph_count()), a, "a glyph id past the face's glyphs");
    let elsewhere = CharStyle { font_family: alternates_family(false).into(), glyph_alternate: Some(choice.clone()), ..style(false) };
    assert_eq!(set(runs(elsewhere))[0].0, a, "the same glyph id in another face");
    let caps = CharStyle { all_caps: true, ..chosen.clone() };
    assert_eq!(set(runs(caps))[0].0, face.glyph_for('A'), "All Caps draws the capital");
}

#[test]
fn adjacent_characters_with_the_same_choice_keep_both_glyphs() {
    let db = db();
    let face = db.face(alternates_family(true), "Regular").expect("the test face");
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let style = CharStyle { glyph_alternate: Some(face.choice('f', u1)), ..style(true) };
    let mut text = TextObject::point(Point::ZERO, "", style.clone());
    text.runs = vec![TextRun { text: "f".into(), style: style.clone(), inline: None }, TextRun { text: "f".into(), style, inline: None }];
    let glyphs = layout(&db, &text).glyphs;
    assert_eq!(glyphs.iter().map(|g| g.gid).collect::<Vec<_>>(), [u1, u1], "each chosen f draws independently");
}

/// A run "fi" whose f has a chosen glyph draws that glyph and the i, not the font's fi ligature.
#[test]
fn a_chosen_character_is_not_taken_into_a_ligature_in_its_run() {
    use crate::test_fonts::{LIGATURE_CARET_FAMILY, ligature_caret_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(ligature_caret_font(None).expect("the test font builds"));
    let face = db.face(LIGATURE_CARET_FAMILY, "Regular").expect("the test face");
    let plain = CharStyle { font_family: LIGATURE_CARET_FAMILY.into(), size: 100.0, ..CharStyle::default() };
    let gids = |st: CharStyle| layout(&db, &TextObject::point(Point::ZERO, "fi", st)).glyphs.into_iter().map(|g| g.gid).collect::<Vec<_>>();
    assert_eq!(gids(plain.clone()).len(), 1, "without a choice the font joins f and i");
    let i = face.glyph_for('i');
    let chosen = CharStyle { glyph_alternate: Some(face.choice('f', i)), ..plain };
    assert_eq!(gids(chosen), [i, i], "the f draws the chosen glyph and the i stays apart");
}

/// A run "f" + combining acute + "i" whose f has a chosen glyph is not split between the f and its
/// mark: the f and the mark stay one cluster, shaped as they are without a choice.
#[test]
fn a_chosen_character_that_starts_a_longer_grapheme_is_not_split_from_it() {
    let db = FontDb::with_font_dirs(vec![]);
    let face = db.face(FALLBACK_FAMILY, "Regular").expect("the bundled face");
    let plain = CharStyle { font_family: FALLBACK_FAMILY.into(), size: 100.0, ..CharStyle::default() };
    let glyphs =
        |st: CharStyle| layout(&db, &TextObject::point(Point::ZERO, "f\u{301}i", st)).glyphs.into_iter().map(|g| (g.gid, g.byte)).collect::<Vec<_>>();
    let shaped = glyphs(plain.clone());
    assert!(shaped.iter().filter(|g| g.1 == 0).count() >= 2, "the mark is shaped in the f's cluster: {shaped:?}");
    let chosen = CharStyle { glyph_alternate: Some(face.choice('f', face.glyph_for('i'))), ..plain };
    assert_eq!(glyphs(chosen), shaped, "the f and its mark are shaped together, as without a choice");
}

/// The font's pair kerning was looked up for the default glyph: a character drawn with another
/// glyph takes no part in it, before or after it.
#[test]
fn a_chosen_glyph_takes_no_kerning_found_for_the_default_glyph() {
    use crate::test_fonts::{PAIR_KERNING_FAMILY, pair_kerning_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(pair_kerning_font().expect("the test font builds"));
    let face = db.face(PAIR_KERNING_FAMILY, "Regular").expect("the test face");
    let st = CharStyle { font_family: PAIR_KERNING_FAMILY.into(), size: 100.0, ..CharStyle::default() };
    // Each glyph's id, advance and how far it is drawn from its pen position, in one run "AV"
    // whose A is drawn with the glyph of `with` (the choice applies to the run's first character).
    let set = |with: char| {
        let style = CharStyle { glyph_alternate: Some(face.choice('A', face.glyph_for(with))), ..st.clone() };
        let t = TextObject::point(Point::ZERO, "AV", style);
        layout(&db, &t).glyphs.into_iter().map(|g| (g.gid, g.advance, g.xf.as_coeffs()[4] - g.origin.x)).collect::<Vec<_>>()
    };
    let k = 100.0 / face.units_per_em();
    let plain = |c: char| face.advance(face.glyph_for(c)) * k;
    let chosen = set('B');
    assert_eq!(chosen.iter().map(|g| g.0).collect::<Vec<_>>(), [face.glyph_for('B'), face.glyph_for('V')]);
    assert!((chosen[0].1 - plain('B')).abs() < 1e-6, "the chosen glyph keeps none of the A-V pair's advance: {chosen:?}");
    assert!(chosen[1].2.abs() < 1e-6, "the V after it is not moved by the A-V pair: {chosen:?}");
    // The default glyph chosen for the A keeps the pair: 6 pt off its advance, the V 10 pt left.
    let same = set('A');
    assert!((same[0].1 - (plain('A') - 6.0)).abs() < 1e-6 && (same[1].2 + 10.0).abs() < 1e-6, "{same:?}");
}

#[test]
fn edits_keep_a_choice_on_its_character() {
    let face = db().face(alternates_family(true), "Regular").expect("the test face");
    let [u1, ..] = unencoded_glyphs().expect("unencoded glyphs");
    let choice = face.choice('a', u1);
    let mut runs = vec![TextRun { text: "xaay".into(), style: style(true), inline: None }];
    edit::style_range(&mut runs, 1, 2, |st| st.glyph_alternate = Some(choice.clone()));
    let texts = |runs: &[TextRun]| runs.iter().map(|r| (r.text.clone(), r.style.glyph_alternate.is_some())).collect::<Vec<_>>();
    assert_eq!(texts(&runs), [("x".into(), false), ("a".into(), true), ("ay".into(), false)]);
    // The second a takes the same choice: a run of its own, not merged with the first.
    edit::style_range(&mut runs, 2, 3, |st| st.glyph_alternate = Some(choice.clone()));
    assert_eq!(texts(&runs), [("x".into(), false), ("a".into(), true), ("a".into(), true), ("y".into(), false)]);
    // Typed text takes the default glyph, also an a typed after a chosen one.
    let caret = edit::replace_range(&mut runs, 2, 2, "a");
    assert_eq!(caret, 3);
    assert_eq!(texts(&runs), [("x".into(), false), ("a".into(), true), ("a".into(), false), ("a".into(), true), ("y".into(), false)]);
    // Styling part of a run keeps the choice on its first character only.
    let mut runs = vec![TextRun { text: "ab".into(), style: CharStyle { glyph_alternate: Some(choice.clone()), ..style(true) }, inline: None }];
    edit::style_range(&mut runs, 1, 2, |st| st.underline = true);
    assert_eq!(texts(&runs), [("a".into(), true), ("b".into(), false)]);
    assert_eq!(texts(&edit::slice_runs(&runs, 0, 2)), [("a".into(), true), ("b".into(), false)]);
    // Deleting the character drops its choice from the run that is left.
    let mut runs = vec![TextRun { text: "ab".into(), style: CharStyle { glyph_alternate: Some(choice.clone()), ..style(true) }, inline: None }];
    edit::replace_range(&mut runs, 0, 1, "");
    assert_eq!(texts(&runs), [("b".into(), false)]);
    let mut runs = vec![TextRun { text: "ab".into(), style: CharStyle { glyph_alternate: Some(choice), ..style(true) }, inline: None }];
    assert_eq!(texts(&edit::slice_runs(&runs, 1, 2)), [("b".into(), false)], "a copy from inside the run");
    edit::normalize(&mut runs);
    assert_eq!(texts(&runs), [("ab".into(), true)]);
}
