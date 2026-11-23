//! Show Similar Fonts: the numbers a face is compared by ([`FontLikeness`]), a family's likeness
//! from its face nearest an upright Regular, and families ordered by likeness.

use skrifa::raw::TableProvider;

use crate::test_fonts::{
    HEIGHT_CAP, HEIGHT_FAMILY, HEIGHT_X, HEIGHTLESS_FAMILY, NUMBERLESS_FAMILY, VARIABLE_FAMILY, height_font, numberless_font, variable_font,
};
use crate::*;

/// A database of the bundled fonts and the two copies of Source Sans 3 Regular with and without
/// heights in their OS/2 tables.
fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    for heights in [true, false] {
        assert_eq!(db.add_font(height_font(heights).expect("the test font builds")), 1);
    }
    db
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn a_face_is_compared_by_its_tables() {
    let db = db();
    let regular = db.face("Source Sans 3", "Regular").expect("bundled").traits.likeness;
    assert_eq!(regular.weight, 400);
    assert_eq!(regular.italic_angle, Some(0));
    assert!(regular.panose[0] > 1, "a PANOSE family kind: {regular:?}");
    assert!(regular.x_height > 300 && regular.cap_height > regular.x_height, "{regular:?}");
    // The OS/2 average character width, in thousandths of the em.
    let bytes = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/SourceSans3-Regular.ttf")).expect("bundled");
    let font = skrifa::FontRef::new(&bytes).expect("it parses");
    let (avg, em) = (font.os2().expect("an OS/2 table").x_avg_char_width(), font.head().expect("a head table").units_per_em());
    assert_eq!(f32::from(regular.avg_width), (f32::from(avg) / f32::from(em) * 1000.0).round());
    let italic = db.face("Source Sans 3", "Italic").expect("bundled").traits.likeness;
    assert!(italic.italic_angle.is_some_and(|a| a < 0), "leans right: {italic:?}");
    assert!(italic.italic && !regular.italic);
    let bold = db.face("Source Sans 3", "Bold").expect("bundled").traits.likeness;
    assert_eq!(bold.weight, 700);
    // The OS/2 heights, in thousandths of its 1,000-unit em.
    let heights = db.family_traits(HEIGHT_FAMILY).likeness;
    assert_eq!((heights.x_height, heights.cap_height), (HEIGHT_X as u16, HEIGHT_CAP as u16));
    // Without them (a version 1 OS/2 table), the heights are left out.
    let heightless = db.family_traits(HEIGHTLESS_FAMILY).likeness;
    assert_eq!((heightless.x_height, heightless.cap_height), (0, 0), "{heightless:?}");
    assert_eq!(heightless.avg_width, regular.avg_width);
}

/// A variable font's named instance takes its `wght` setting as its weight; the default face its
/// default weight.
#[test]
fn a_named_instance_is_weighed_by_its_axis_setting() {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(variable_font().expect("the test font builds")), 3);
    let regular = db.face(VARIABLE_FAMILY, "Regular").expect("the default face").traits.likeness;
    let semibold = db.face(VARIABLE_FAMILY, "SemiBold").expect("a named instance").traits.likeness;
    let bold = db.face(VARIABLE_FAMILY, "Bold").expect("a named instance").traits.likeness;
    assert_eq!((regular.weight, semibold.weight, bold.weight), (400, 550, 700));
    // The family is compared by its Regular face.
    assert_eq!(db.family_traits(VARIABLE_FAMILY).likeness, regular);
}

/// A face without an italic angle is compared by the other numbers only.
#[test]
fn a_missing_italic_angle_is_left_out() {
    let db = db();
    let italic = db.face("Source Sans 3", "Italic").expect("bundled").traits.likeness;
    assert_eq!(italic.distance(&FontLikeness { italic_angle: None, ..italic }), 0.0);
    assert!(italic.distance(&FontLikeness { italic_angle: Some(0), ..italic }) > 0.0);
}

/// A family is compared by its face nearest an upright Regular, not by the first one loaded.
#[test]
fn a_family_is_compared_by_its_regular_face() {
    let db = db();
    let regular = db.face("Source Sans 3", "Regular").expect("bundled").traits.likeness;
    assert_eq!(db.family_traits("Source Sans 3").likeness, regular);
    assert_eq!(db.family_traits("source sans 3").likeness, regular, "by any case");
}

#[test]
fn likeness_is_a_distance() {
    let db = db();
    let (sans, serif, mono) =
        (db.family_traits("Source Sans 3").likeness, db.family_traits("Source Serif 4").likeness, db.family_traits("JetBrains Mono").likeness);
    assert_eq!(sans.distance(&sans), 0.0);
    assert_eq!(sans.distance(&serif), serif.distance(&sans), "symmetric");
    assert!(sans.distance(&serif) > 0.0 && sans.distance(&mono) > 0.0);
    // A number a face does not give is left out, not counted as 0.
    assert_eq!(sans.distance(&FontLikeness { x_height: 0, cap_height: 0, ..sans }), 0.0);
    // Faces that share no number are further apart than any faces that share one, not alike.
    let unknown = FontLikeness::default();
    assert_eq!(sans.distance(&unknown), f32::INFINITY);
    assert_eq!(unknown.distance(&sans), f32::INFINITY, "symmetric");
    assert_eq!(unknown.distance(&unknown), f32::INFINITY);
    let weight_only = FontLikeness { weight: 400, ..unknown };
    assert_eq!(sans.distance(&weight_only), 0.0, "the weight alone is compared");
    let lighter = FontLikeness { weight: 300, ..sans };
    assert!((sans.distance(&lighter) - 0.5).abs() < 1e-6, "100 weight units apart");
}

/// The family itself first, then nearest first: a copy of Source Sans 3 Regular comes before every
/// other family, and the order never moves away from the family and back.
#[test]
fn families_are_ordered_by_likeness() {
    let db = db();
    let families = names(&["JetBrains Mono", "Source Serif 4", "Inter", HEIGHTLESS_FAMILY, "Source Sans 3"]);
    let order = db.similar_families("Source Sans 3", &families);
    assert_eq!(order.len(), families.len());
    assert_eq!(order[..2], names(&["Source Sans 3", HEIGHTLESS_FAMILY]), "{order:?}");
    let base = db.family_traits("Source Sans 3");
    let far = |f: &str| {
        let t = db.family_traits(f);
        base.likeness.distance(&t.likeness) + if t.class == base.class { 0.0 } else { 2.0 }
    };
    for pair in order[1..].windows(2) {
        assert!(far(&pair[0]) <= far(&pair[1]), "{order:?}");
    }
    // Asked by another case, or for a family not in the list.
    assert_eq!(db.similar_families("SOURCE SANS 3", &families)[0], "Source Sans 3");
    let without = db.similar_families("Source Sans 3", &families[..4]);
    assert_eq!(without[0], HEIGHTLESS_FAMILY);
    assert!(db.similar_families("Source Sans 3", &[]).is_empty());
}

/// A family that gives none of the numbers comes after every family that shares one with the
/// family asked about, instead of first as though it looked the same.
#[test]
fn a_family_without_numbers_comes_after_the_measured_ones() {
    let db = db();
    assert_eq!(db.add_font(numberless_font().expect("the test font builds")), 1);
    assert_eq!(db.family_traits(NUMBERLESS_FAMILY).likeness, FontLikeness::default());
    let families = names(&[NUMBERLESS_FAMILY, "JetBrains Mono", "Source Serif 4", "Inter", HEIGHTLESS_FAMILY, "Source Sans 3"]);
    let order = db.similar_families("Source Sans 3", &families);
    assert_eq!(order[..2], names(&["Source Sans 3", HEIGHTLESS_FAMILY]), "{order:?}");
    assert_eq!(order.last().map(String::as_str), Some(NUMBERLESS_FAMILY), "{order:?}");
    // The measured families keep the order they have without it.
    let measured: Vec<String> = order.iter().filter(|f| *f != NUMBERLESS_FAMILY).cloned().collect();
    assert_eq!(measured, db.similar_families("Source Sans 3", &families[1..]));
    // Asked about itself, it comes first even from the end of the list, and the families it shares
    // no number with keep their order in the list.
    let last = names(&["JetBrains Mono", "Source Serif 4", "Inter", HEIGHTLESS_FAMILY, "Source Sans 3", NUMBERLESS_FAMILY]);
    assert_eq!(db.similar_families(NUMBERLESS_FAMILY, &last), families);
}
