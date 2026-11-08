//! A feature with a value (`salt=2`) on some characters of an Arabic word: the font's alternate
//! for them, with the word shaped once so that every other letter keeps its joining form and the
//! letters still touch. Read from a font built in code (and, when it is installed, the owner's
//! Kufi Murabba PIIIX).

use kurbo::Point;
use vectorcraft_doc::{CharStyle, TextRun};

use crate::test_fonts::{JOINING_ALTERNATES_FAMILY, JOINING_LETTERS, joining_alternates_font, source_sans_gid};
use crate::*;

const BEH: &str = "\u{0628}";
const TEH: &str = "\u{062A}";
const NOON: &str = "\u{0646}";

fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(joining_alternates_font().expect("the test font builds"));
    db
}

fn style(family: &str, features: &[&str]) -> CharStyle {
    CharStyle { font_family: family.into(), size: 100.0, features: features.iter().map(|f| f.to_string()).collect(), ..CharStyle::default() }
}

/// `text` as one text object in `family`, each piece with its own features.
fn set(db: &FontDb, family: &str, pieces: &[(&str, &[&str])]) -> Vec<PositionedGlyph> {
    let mut text = TextObject::point(Point::ZERO, "", style(family, &[]));
    text.runs = pieces.iter().map(|(t, features)| TextRun { text: t.to_string(), style: style(family, features), inline: None }).collect();
    // In reading order of the characters, whichever way the line runs.
    let mut glyphs = layout(db, &text).glyphs;
    glyphs.sort_by_key(|g| g.byte);
    glyphs
}

fn joined(db: &FontDb, pieces: &[(&str, &[&str])]) -> Vec<PositionedGlyph> {
    set(db, JOINING_ALTERNATES_FAMILY, pieces)
}

fn gids(glyphs: &[PositionedGlyph]) -> Vec<u32> {
    glyphs.iter().map(|g| g.gid).collect()
}

fn g(c: char) -> u32 {
    u32::from(source_sans_gid(c).expect("a Source Sans glyph"))
}

/// The gap between each glyph and the next along the line, whichever way the line runs: 0 when the
/// letters touch.
fn gaps(glyphs: &[PositionedGlyph]) -> Vec<f64> {
    // `origin` is the pen position at the glyph's left edge, also in right-to-left text.
    let mut spans: Vec<(f64, f64)> = glyphs.iter().map(|g| (g.origin.x, g.origin.x + g.advance)).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    spans.windows(2).map(|w| w[1].0 - w[0].1).collect()
}

/// Beh, teh, noon: initial, medial and final in one word, teh drawn with `features`.
fn word(db: &FontDb, teh_features: &[&str]) -> Vec<PositionedGlyph> {
    joined(db, &[(BEH, &[]), (TEH, teh_features), (NOON, &[])])
}

/// Without a feature the word is beh initial, teh medial, noon final.
#[test]
fn the_fixture_word_takes_its_joining_forms() {
    let db = db();
    assert_eq!(gids(&joined(&db, &[(&format!("{BEH}{TEH}{NOON}"), &[])])), [g('B'), g('u'), g('M')]);
    assert_eq!(JOINING_LETTERS.len(), 4);
}

/// `salt=N` on the middle letter draws the font's Nth alternate for it, its neighbours keep their
/// initial and final forms (they are not shaped as lone letters), and the letters still touch.
#[test]
fn a_salt_value_on_one_letter_picks_its_alternate_and_keeps_the_word_joined() {
    let db = db();
    let plain = word(&db, &[]);
    assert_eq!(gids(&plain), [g('B'), g('u'), g('M')]);
    let plain_gaps = gaps(&plain);
    assert!(plain_gaps.iter().all(|gap| gap.abs() < 1e-6), "the word's glyphs touch: {plain_gaps:?}");
    for (features, middle) in [(&["salt"][..], 'w'), (&["salt=1"][..], 'w'), (&["salt=2"][..], 'x'), (&["salt=3"][..], 'y')] {
        let glyphs = word(&db, features);
        assert_eq!(gids(&glyphs), [g('B'), g(middle), g('M')], "{features:?}: the alternate between the word's own initial and final forms");
        let gap = gaps(&glyphs);
        assert!(gap.iter().all(|gap| gap.abs() < 1e-6), "{features:?}: the letters touch: {gap:?}");
    }
    // An alternate number the font has no glyph for, and the feature off, draw the default glyph.
    for features in [&["salt=4"][..], &["-salt"][..], &["salt=0"][..], &["salt=2", "-salt"][..], &["ss03"][..]] {
        assert_eq!(gids(&word(&db, features)), [g('B'), g('u'), g('M')], "{features:?}");
    }
    // The last value of a feature wins.
    assert_eq!(gids(&word(&db, &["salt=3", "salt=2"]))[1], g('x'));
    // The font's salt lookup comes before its ss01 lookup, so salt takes the glyph first.
    assert_eq!(gids(&word(&db, &["salt=2", "ss01"]))[1], g('x'));
}

/// A feature on the first or the last letter of the word does not split it either.
#[test]
fn a_feature_on_an_end_letter_does_not_break_the_joining() {
    let db = db();
    for (pieces, expected) in [
        (vec![(BEH, &["salt=2"][..]), (TEH, &[][..]), (NOON, &[][..])], [g('B'), g('u'), g('M')]),
        (vec![(BEH, &[][..]), (TEH, &[][..]), (NOON, &["salt=2"][..])], [g('B'), g('u'), g('M')]),
        (vec![(BEH, &["ss01"][..]), (TEH, &["ss01"][..]), (NOON, &["ss01"][..])], [g('B'), g('w'), g('M')]),
    ] {
        let glyphs = joined(&db, &pieces);
        assert_eq!(gids(&glyphs), expected, "{pieces:?}");
        assert!(gaps(&glyphs).iter().all(|gap| gap.abs() < 1e-6), "{pieces:?}");
    }
}

/// The alternates of a letter depend on the form it takes where it stands: the final teh has two,
/// the medial teh three, so `salt=3` is an alternate of one and nothing for the other.
#[test]
fn the_alternate_a_value_picks_depends_on_the_letters_position() {
    let db = db();
    let medial = |features: &[&str]| gids(&word(&db, features))[1];
    let final_form = |features: &[&str]| gids(&joined(&db, &[(BEH, &[]), (TEH, features)]))[1];
    assert_eq!((medial(&["salt=3"]), final_form(&["salt=3"])), (g('y'), g('U')), "no third alternate for the final form");
    assert_eq!((medial(&["salt=2"]), final_form(&["salt=2"])), (g('x'), g('X')));
    // A letter alone takes the isolated form's alternate.
    assert_eq!(gids(&joined(&db, &[(TEH, &["salt"])])), [g('E')]);
    assert_eq!(gids(&joined(&db, &[(TEH, &[])])), [g('t')]);
}

/// A run with its own feature next to runs of another style in the word shapes with them: the
/// same glyph ids and places as a word typed in one run, when the feature changes nothing.
#[test]
fn runs_that_differ_only_in_features_shape_as_one_word() {
    let db = db();
    let one_run = joined(&db, &[(&format!("{BEH}{TEH}{NOON}"), &[])]);
    let three_runs = joined(&db, &[(BEH, &[]), (TEH, &["salt=9"]), (NOON, &["-liga"])]);
    assert_eq!(gids(&three_runs), gids(&one_run));
    let (a, b): (Vec<f64>, Vec<f64>) = (one_run.iter().map(|g| g.origin.x).collect(), three_runs.iter().map(|g| g.origin.x).collect());
    assert_eq!(a, b, "the same places");
}

/// Where the owner's Kufi Murabba PIIIX is installed.
const KUFI_FILE: &str = "Library/Fonts/KufiMurabbaPIIIX-Regular.ttf";

/// The owner's Heavy weight of it, the one a final yeh was reported against.
const KUFI_HEAVY_FILE: &str = "Library/Fonts/KufiMurabbaPIIIX-Heavy.ttf";

/// The Kufi Murabba PIIIX of the font's author in a font database of its own, `None` when it is not
/// installed (the test then has nothing to read). When the file is there, a face that does not
/// load is a failure, never a skip.
fn kufi() -> Option<(FontDb, std::sync::Arc<FontFace>)> {
    kufi_weight(KUFI_FILE, "Regular")
}

/// [`kufi`] for the installed file `file` (under the home folder), whose face has style `style`.
fn kufi_weight(file: &str, style: &str) -> Option<(FontDb, std::sync::Arc<FontFace>)> {
    let path = std::path::Path::new(&std::env::var_os("HOME")?).join(file);
    if !path.exists() {
        eprintln!("skipped: {} is not installed", path.display());
        return None;
    }
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{} exists but cannot be read: {e}", path.display()));
    let db = FontDb::with_font_dirs(vec![]);
    let added = db.add_font(bytes);
    assert!(added > 0, "{} exists but no face of it loaded", path.display());
    let face = db.face("Kufi Murabba PIIIX", style).expect("a face");
    assert_eq!((face.family.as_str(), face.style.as_str()), ("Kufi Murabba PIIIX", style), "the installed font's face, not a fallback");
    Some((db, face))
}

/// The word al-Khwarizmi: alef, lam, khah, waw, alef, reh, zain, meem, farsi yeh. The lam, khah and
/// waw join (initial, medial, final); the letters around them do not join forward.
const KHWARIZMI: &str = "الخوارزمی";
/// The medial khah, the middle letter of the joined lam-khah-waw.
const KHAH: usize = 2;

/// The word with `features` on the character `at` only, shaped as one piece (the runs differ only
/// in features), in reading order of the characters.
fn khwarizmi(db: &FontDb, family: &str, at: usize, features: &[&str]) -> Vec<PositionedGlyph> {
    let letters: Vec<char> = KHWARIZMI.chars().collect();
    let part = |r: std::ops::Range<usize>| -> String { letters[r].iter().collect() };
    let (before, middle, after) = (part(0..at), part(at..at + 1), part(at + 1..letters.len()));
    set(db, family, &[(&before, &[]), (&middle, features), (&after, &[])])
}

/// The gap between the drawn outlines of the letters that join (the lam, khah and waw), along the
/// line: 0 when the ink of one reaches the ink of the next. Unlike [`gaps`], which only checks that
/// the pen advances by each glyph's advance, this reads the glyphs drawn.
fn ink_gaps(glyphs: &[PositionedGlyph]) -> Vec<f64> {
    use kurbo::Shape;
    let mut spans: Vec<(f64, f64)> = glyphs[KHAH - 1..=KHAH + 1].iter().map(|g| (g.outline.bounding_box().x0, g.outline.bounding_box().x1)).collect();
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    spans.windows(2).map(|w| w[1].0 - w[0].1).collect()
}

/// One line of what the shaper made, for a failure message and `--nocapture`.
fn describe(glyphs: &[PositionedGlyph]) -> String {
    glyphs.iter().map(|g| format!("{}@{:.1}+{:.1}", g.gid, g.origin.x, g.advance)).collect::<Vec<_>>().join(" ")
}

/// The Kufi Murabba PIIIX of the font's author, when it is installed: the medial khah of a word
/// takes each of its `salt` alternates, every other letter keeps the glyph it has without one and
/// the letters still touch.
#[test]
fn kufi_murabba_alternates_keep_the_word_joined() {
    let Some((db, face)) = kufi() else { return };
    let family = face.family.clone();
    let plain = khwarizmi(&db, &family, KHAH, &[]);
    assert_eq!(plain.len(), KHWARIZMI.chars().count(), "one glyph for each letter: {}", describe(&plain));
    assert!(gaps(&plain).iter().all(|gap| gap.abs() < 1e-3), "the word joins without a gap: {:?}", gaps(&plain));
    assert!(ink_gaps(&plain).iter().all(|gap| gap.abs() < 1e-3), "the drawn letters touch: {:?}", ink_gaps(&plain));
    let default = plain[KHAH].gid;
    assert_eq!(face.glyph_name(default).as_deref(), Some("khah.medi"), "the khah in the middle of the word takes its medial form");
    let salt =
        face.substitutions(default).into_iter().find(|s| &s.feature == b"salt" && s.glyphs.len() >= 2).expect("the medial khah has salt alternates");
    for (i, alternate) in salt.glyphs.iter().enumerate() {
        let value = format!("salt={}", i + 1);
        let glyphs = khwarizmi(&db, &family, KHAH, &[value.as_str()]);
        assert_eq!(glyphs.len(), plain.len(), "{value}: {}", describe(&glyphs));
        assert_eq!(glyphs[KHAH].gid, *alternate, "{value}: the alternate");
        assert_ne!(glyphs[KHAH].gid, default, "{value}: not the default glyph");
        assert_eq!(face.glyph_name(*alternate), Some(format!("khah.medi.alt{}", i + 1)), "{value}: an alternate of the medial form");
        for (n, (a, b)) in glyphs.iter().zip(&plain).enumerate().filter(|(n, _)| *n != KHAH) {
            assert_eq!(a.gid, b.gid, "{value}: the letter {n} keeps its form: {}", describe(&glyphs));
        }
        assert!(gaps(&glyphs).iter().all(|gap| gap.abs() < 1e-3), "{value}: no gap: {:?}", gaps(&glyphs));
        assert!(ink_gaps(&glyphs).iter().all(|gap| gap.abs() < 1e-3), "{value}: the drawn letters touch: {:?}", ink_gaps(&glyphs));
    }
}

/// The stylistic sets `ss01` to `ss05` on a letter in the middle of a joined group: the glyph the
/// font's set gives its medial form (the default when it has none), the same forms for the others,
/// no gap.
#[test]
fn kufi_murabba_stylistic_sets_on_a_middle_letter_keep_the_word_joined() {
    let Some((db, face)) = kufi() else { return };
    let family = face.family.clone();
    let plain = khwarizmi(&db, &family, KHAH, &[]);
    let default = plain[KHAH].gid;
    let mut changed = 0;
    for set_number in 1..=5 {
        let tag = format!("ss{set_number:02}");
        let wanted = face.substitutions(default).into_iter().find(|s| s.feature == tag.as_bytes() && !s.glyphs.is_empty()).map(|s| s.glyphs[0]);
        let glyphs = khwarizmi(&db, &family, KHAH, &[tag.as_str()]);
        assert_eq!(glyphs.len(), plain.len(), "{tag}: {}", describe(&glyphs));
        assert_eq!(glyphs[KHAH].gid, wanted.unwrap_or(default), "{tag}: the set's glyph for the medial khah");
        changed += usize::from(wanted.is_some_and(|w| w != default));
        for (n, (a, b)) in glyphs.iter().zip(&plain).enumerate().filter(|(n, _)| *n != KHAH) {
            assert_eq!(a.gid, b.gid, "{tag}: the letter {n} keeps its form: {}", describe(&glyphs));
        }
        assert!(gaps(&glyphs).iter().all(|gap| gap.abs() < 1e-3), "{tag}: no gap: {:?}", gaps(&glyphs));
        assert!(ink_gaps(&glyphs).iter().all(|gap| gap.abs() < 1e-3), "{tag}: the drawn letters touch: {:?}", ink_gaps(&glyphs));
    }
    assert!(changed >= 1, "the font has stylistic sets for the medial khah");
}

/// The `GSUB` reader gives the alternate set of each glyph (position in the set = the feature's
/// value), single substitutions, the feature that reaches a glyph, names and the glyph count.
#[test]
fn the_gsub_reader_lists_alternate_sets_singles_and_the_feature_reaching_a_glyph() {
    use crate::{Substitution, SubstitutionKind};
    let db = db();
    let face = db.face(JOINING_ALTERNATES_FAMILY, "Regular").expect("the test face");
    let salt = Substitution { feature: *b"salt", kind: SubstitutionKind::Alternate, glyphs: vec![g('w'), g('x'), g('y')] };
    let ss01 = Substitution { feature: *b"ss01", kind: SubstitutionKind::Single, glyphs: vec![g('w')] };
    assert_eq!(face.substitutions(g('u')), [salt.clone(), ss01.clone()], "salt (an alternate set) before ss01 (a single)");
    assert_eq!(salt.value_for(g('x')), Some(2));
    assert_eq!(salt.value_for(g('u')), None);
    assert_eq!((ss01.value_for(g('w')), ss01.value_for(g('x'))), (Some(1), None));
    // Each form of a letter has its own alternates.
    let sets = |c| face.substitutions(g(c)).iter().map(|s| (s.feature, s.glyphs.len())).collect::<Vec<_>>();
    assert_eq!(sets('U'), [(*b"salt", 2), (*b"ss01", 1)]);
    assert_eq!(sets('t'), [(*b"salt", 1), (*b"ss01", 1)]);
    assert!(sets('B').is_empty() && sets('M').is_empty(), "the initial and final forms of the others have none");
    assert!(face.substitutions(face.glyph_count()).is_empty(), "past the glyphs");
    // The feature that reaches a glyph no character maps: the listed ones first, then positional forms.
    assert_eq!(face.feature_reaching(g('x')), Some(*b"salt"));
    assert_eq!(face.feature_reaching(g('w')), Some(*b"salt"), "salt ranks before ss01");
    assert_eq!(face.feature_reaching(g('B')), Some(*b"init"));
    assert_eq!(face.feature_reaching(g('m')), Some(*b"medi"));
    assert_eq!(face.feature_reaching(g('A')), Some(*b"fina"));
    assert_eq!(face.feature_reaching(g('b')), None, "the isolated form is reached by no feature");
    // Names and the glyph count.
    let bundled = db.face(FALLBACK_FAMILY, "Regular").expect("the bundled face");
    assert_eq!(face.glyph_count(), bundled.glyph_count());
    assert_eq!(face.glyph_name(g('B')).as_deref(), Some("B"));
    assert_eq!(face.glyph_name(face.glyph_count()), None);
    // `alternates` is the same reading, for a character.
    let listed: Vec<([u8; 4], u32)> = face.alternates('\u{062A}').into_iter().map(|a| (a.feature, a.gid)).collect();
    assert_eq!(listed, [(*b"salt", g('E'))], "the isolated teh");
}

/// The alternates of a letter as it stands in its word: the glyph the shaper makes for it there,
/// its alternates, and the feature value that draws each on that letter alone.
#[test]
fn alternates_in_context_list_the_letters_positional_form_and_the_feature_that_draws_each() {
    let db = db();
    let text = |teh: &[&str]| {
        let mut t = TextObject::point(Point::ZERO, "", style(JOINING_ALTERNATES_FAMILY, &[]));
        t.runs = [(BEH, &[][..]), (TEH, teh), (NOON, &[][..])]
            .into_iter()
            .map(|(s, f)| TextRun { text: s.to_string(), style: style(JOINING_ALTERNATES_FAMILY, f), inline: None })
            .collect();
        t
    };
    let teh = BEH.len();
    let found = alternates_in_context(&db, &text(&[]), teh).expect("the medial teh");
    assert_eq!((found.default, found.current), (g('u'), g('u')));
    let listed: Vec<(u32, Option<String>)> = found.alternates.iter().map(|a| (a.gid, a.apply.clone())).collect();
    assert_eq!(
        listed,
        [(g('w'), Some("salt".to_string())), (g('x'), Some("salt=2".to_string())), (g('y'), Some("salt=3".to_string()))],
        "the medial form's three alternates, not the isolated form's one"
    );
    // Drawn with salt=2 now: the default is still the form without it.
    let now = alternates_in_context(&db, &text(&["salt=2"]), teh).expect("the medial teh");
    assert_eq!((now.default, now.current, now.alternates.len()), (g('u'), g('x'), 3));
    // The first and last letters take their own forms.
    let beh = alternates_in_context(&db, &text(&[]), 0).expect("the initial beh");
    assert_eq!((beh.default, beh.alternates.len()), (g('B'), 0));
    let noon = alternates_in_context(&db, &text(&[]), teh + TEH.len()).expect("the final noon");
    assert_eq!((noon.default, noon.alternates.len()), (g('M'), 0));
    assert_eq!(alternates_in_context(&db, &text(&[]), 1), None, "not a character boundary");
    // alternate_changes: the character keeps only the alternate feature it is given.
    let own = |f: &[&str]| f.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(alternate_changes(&own(&["ss01", "salt=2", "dlig", "-ss02"]), Some("salt=3")), ["-ss01", "salt=3"]);
    assert_eq!(alternate_changes(&own(&["ss01", "salt=2", "dlig"]), None), ["-ss01", "-salt"]);
    assert!(alternate_changes(&own(&["salt=2", "dlig"]), Some("salt=2")).is_empty());
    assert_eq!(alternate_changes(&own(&["salt"]), Some("salt=2")), ["salt=2"]);
    assert!(alternate_changes(&own(&["dlig"]), None).is_empty());
    assert!(is_alternate_feature("-ss03") && is_alternate_feature("salt=2") && !is_alternate_feature("dlig") && !is_alternate_feature("titl"));
}

/// A glyph no feature of the style draws is listed without one, and a feature's glyph is found
/// with the feature that draws it.
#[test]
fn a_variant_no_feature_draws_has_no_feature_to_apply() {
    use crate::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(alternates_font(true).expect("the test font builds"));
    let [u1, u2, u3, u4] = unencoded_glyphs().expect("unencoded glyphs");
    let face = db.face(alternates_family(true), "Regular").expect("the test face");
    let st = CharStyle { font_family: alternates_family(true).into(), size: 100.0, ..CharStyle::default() };
    let found = alternates_in_context(&db, &TextObject::point(Point::ZERO, "xay", st), 1).expect("the a");
    let apply = |gid: u32| found.alternates.iter().find(|a| a.gid == gid).map(|a| a.apply.clone());
    assert_eq!(found.default, face.glyph_for('a'));
    assert_eq!(apply(u1), Some(Some("swsh=2".to_string())), "swsh before aalt");
    assert_eq!(apply(u2), Some(Some("ss01".to_string())));
    assert_eq!(apply(u3), Some(Some("swsh".to_string())));
    assert_eq!(apply(u4), Some(Some("ss02".to_string())));
    assert_eq!(apply(face.glyph_for('c')), Some(None), "cv01 has no style tag");
    assert!(face.unencoded_glyphs().contains(&u1) && !face.unencoded_glyphs().contains(&face.glyph_for('a')));
}

/// The alternates are listed `aalt` first, then `salt`, the stylistic sets, `swsh` and `cv01`, each
/// glyph once under the first feature that lists it, whichever feature draws it in the text.
#[test]
fn alternates_are_listed_aalt_first_under_the_first_feature_that_lists_them() {
    use crate::test_fonts::{alternates_family, alternates_font, unencoded_glyphs};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(alternates_font(true).expect("the test font builds"));
    let [u1, u2, u3, u4] = unencoded_glyphs().expect("unencoded glyphs");
    let face = db.face(alternates_family(true), "Regular").expect("the test face");
    let st = CharStyle { font_family: alternates_family(true).into(), size: 100.0, ..CharStyle::default() };
    let found = alternates_in_context(&db, &TextObject::point(Point::ZERO, "xay", st), 1).expect("the a");
    let listed: Vec<(u32, [u8; 4], Option<String>)> = found.alternates.iter().map(|a| (a.gid, a.feature, a.apply.clone())).collect();
    let (a, b, c) = (face.glyph_for('a'), face.glyph_for('b'), face.glyph_for('c'));
    assert_eq!(
        listed,
        [
            (a, *b"aalt", Some("aalt".to_string())),
            (u1, *b"aalt", Some("swsh=2".to_string())),
            (b, *b"aalt", Some("salt".to_string())),
            (u2, *b"ss01", Some("ss01".to_string())),
            (u4, *b"ss02", Some("ss02".to_string())),
            (u3, *b"swsh", Some("swsh".to_string())),
            (c, *b"cv01", None),
        ],
        "aalt's set first, with the default glyph it lists; b is aalt's though salt draws it"
    );
}

/// A glyph that more than its character makes (the lam-alef ligature) has no alternates of its own
/// there, though the font substitutes it (`salt`).
#[test]
fn a_ligature_has_no_alternates_in_context() {
    use crate::test_fonts::{LAM_ALEF_SALT, lam_alef_family, lam_alef_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(lam_alef_font(b"rlig").expect("the test font builds"));
    let family = lam_alef_family(b"rlig");
    let face = db.face(family, "Regular").expect("the test face");
    let lam_alef = TextObject::point(Point::ZERO, "\u{0644}\u{0627}", style(family, &[]));
    let ligature = g('H');
    assert_eq!(
        face.substitutions(ligature).iter().map(|s| s.glyphs.clone()).collect::<Vec<_>>(),
        [vec![g(LAM_ALEF_SALT)]],
        "the font gives the ligature an alternate"
    );
    let found = alternates_in_context(&db, &lam_alef, 0).expect("the ligature");
    assert_eq!((found.default, found.current), (ligature, ligature));
    assert!(found.alternates.is_empty(), "{:?}", found.alternates);
    // The lam alone is its own character: it has no alternates because the font gives it none.
    let alone = alternates_in_context(&db, &TextObject::point(Point::ZERO, "\u{0644}", style(family, &[])), 0).expect("the lam");
    assert_eq!(alone.default, g('l'));
}

/// A word is shaped as one piece (to 256 characters each side of the selected one), the selected
/// character found in it by its offset: a letter deep in a long word and the letters next to a
/// space, a line feed or a line separator each get the form their place in their own word gives.
#[test]
fn the_probe_finds_the_character_in_its_word_across_a_long_word_and_breaks() {
    let db = db();
    let text = |s: &str| TextObject::point(Point::ZERO, s, style(JOINING_ALTERNATES_FAMILY, &[]));
    let default_of = |t: &TextObject, byte: usize| {
        let found = alternates_in_context(&db, t, byte).expect("a character");
        (found.default, found.alternates.len())
    };
    // 700 letters: beh, 698 teh, noon. Each is found at its own offset, 300 of them past the
    // 256 characters either side that are shaped.
    let long = text(&format!("{BEH}{}{NOON}", TEH.repeat(698)));
    assert_eq!(default_of(&long, 0), (g('B'), 0), "the first letter is initial");
    assert_eq!(default_of(&long, 2 * 300), (g('u'), 3), "a medial teh deep in the word, with its alternates");
    assert_eq!(default_of(&long, 2 * 699), (g('M'), 0), "the last letter is final");
    // Two words: the break leaves each its own initial and final letters. The teh before the break
    // ends its word (final) and the one after starts the next (initial), not medial.
    for brk in [" ", "\n", "\u{2028}", "\u{2029}"] {
        let two = text(&format!("{BEH}{TEH}{brk}{TEH}{NOON}"));
        let after = 2 * 2 + brk.len();
        assert_eq!(default_of(&two, 2), (g('U'), 2), "{brk:?}: final teh before the break, with its two alternates");
        assert_eq!(default_of(&two, after), (g('T'), 0), "{brk:?}: initial teh after it");
        assert_eq!(default_of(&two, after + 2), (g('M'), 0), "{brk:?}: the noon after it is final");
    }
}

/// A word of alternates longer than the candidates tried (512) leaves the features after it
/// untried: `aalt`'s 512 values use them all, so `salt`'s glyph is listed but no feature is applied
/// for it.
#[test]
fn the_candidates_tried_are_capped() {
    use crate::test_fonts::{CAPPED_CANDIDATES_FAMILY, capped_candidates_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(capped_candidates_font().expect("the test font builds"));
    let face = db.face(CAPPED_CANDIDATES_FAMILY, "Regular").expect("the test face");
    let found = alternates_in_context(&db, &TextObject::point(Point::ZERO, "xay", style(CAPPED_CANDIDATES_FAMILY, &[])), 1).expect("the a");
    let listed: Vec<(u32, Option<String>)> = found.alternates.iter().map(|a| (a.gid, a.apply.clone())).collect();
    assert_eq!(listed, [(face.glyph_for('b'), Some("aalt".to_string())), (face.glyph_for('c'), None)]);
}

/// A feature on a one-byte character (Latin) reaches that character: the range of a feature is
/// its source bytes, and harfrust applies a range `a..b` to clusters `a..b - 1` unless the end is
/// inclusive, which would leave a one-byte character out.
#[test]
fn a_feature_on_a_one_byte_character_reaches_it() {
    use crate::test_fonts::{alternates_family, alternates_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(alternates_font(true).expect("the test font builds"));
    let face = db.face(alternates_family(true), "Regular").expect("the test face");
    let st = |features: &[&str]| style(alternates_family(true), features);
    let mut text = TextObject::point(Point::ZERO, "", st(&[]));
    text.runs = vec![
        TextRun { text: "x".into(), style: st(&[]), inline: None },
        TextRun { text: "a".into(), style: st(&["salt"]), inline: None },
        TextRun { text: "y".into(), style: st(&[]), inline: None },
    ];
    let gids: Vec<u32> = layout(&db, &text).glyphs.iter().map(|g| g.gid).collect();
    assert_eq!(gids, [face.glyph_for('x'), face.glyph_for('b'), face.glyph_for('y')], "salt draws b for the a alone");
}

/// The owner's Kufi Murabba PIIIX Heavy, with the final letter of al-Khwarizmi as Persian yeh
/// (U+06CC), Arabic yeh (U+064A) or alef maksura (U+0649): in each the letter takes its own final
/// form, the byte offset of the last character finds its alternates, `salt` to `salt=6` draw them
/// on that letter alone with the rest of the word and its joining untouched, and the glyph an
/// alternate is (`yehfarsi.fina.alt2`) is traced back to its letter and its place in the word.
#[test]
fn kufi_murabba_heavy_final_letters_alternates_belong_to_their_letter() {
    let Some((db, face)) = kufi_weight(KUFI_HEAVY_FILE, "Heavy") else { return };
    let family = face.family.clone();
    for (letter, base) in [('\u{06CC}', "yehfarsi"), ('\u{064A}', "yeh"), ('\u{0649}', "alefmaksura")] {
        let head: String = KHWARIZMI.chars().take(KHWARIZMI.chars().count() - 1).collect();
        let word = format!("{head}{letter}");
        let last = word.len() - letter.len_utf8();
        let letter = letter.to_string();
        let plain = set(&db, &family, &[(&head, &[]), (&letter, &[])]);
        assert_eq!(plain.len(), KHWARIZMI.chars().count(), "{base}: one glyph for each letter: {}", describe(&plain));
        assert_eq!(face.glyph_name(plain[plain.len() - 1].gid), Some(format!("{base}.fina")), "{base}: the letter ends the word in its final form");
        let mut text = TextObject::point(Point::ZERO, "", style(&family, &[]));
        text.runs = vec![TextRun { text: word.clone(), style: style(&family, &[]), inline: None }];
        let found = alternates_in_context(&db, &text, last).expect("the final letter");
        assert_eq!((found.default, found.current), (plain[plain.len() - 1].gid, plain[plain.len() - 1].gid), "{base}");
        assert_eq!(found.alternates.len(), 6, "{base}: the six alternates of the final form");
        for (n, a) in found.alternates.iter().enumerate() {
            let tag = if n == 0 { "salt".to_string() } else { format!("salt={}", n + 1) };
            assert_eq!(a.apply.as_deref(), Some(tag.as_str()), "{base}: the feature that draws alternate {}", n + 1);
            assert_eq!(face.glyph_name(a.gid), Some(format!("{base}.fina.alt{}", n + 1)), "{base}");
            let glyphs = set(&db, &family, &[(&head, &[]), (&letter, &[tag.as_str()])]);
            assert_eq!(glyphs.len(), plain.len(), "{base} {tag}: {}", describe(&glyphs));
            assert_eq!(glyphs[glyphs.len() - 1].gid, a.gid, "{base} {tag}: the alternate");
            assert!(
                glyphs.iter().zip(&plain).take(plain.len() - 1).all(|(x, y)| x.gid == y.gid),
                "{base} {tag}: the rest of the word keeps its forms"
            );
            let origin = face.glyph_origin(a.gid).expect("the alternate comes from a letter");
            assert_eq!(origin.ch.to_string(), letter, "{base}: {} is a form of its own letter", face.glyph_name(a.gid).unwrap_or_default());
            assert_eq!(origin.position(), Some(JoiningPosition::Final), "{base}: and of its end of a word");
        }
    }
    // Glyph 820 of the owner's file is yehfarsi.fina.alt2: Persian yeh's, at the end of a word.
    let g820 = face.glyph_origin(820).expect("glyph 820 comes from a letter");
    assert_eq!((face.glyph_name(820).as_deref(), g820.ch, g820.position()), (Some("yehfarsi.fina.alt2"), '\u{06CC}', Some(JoiningPosition::Final)));
}
