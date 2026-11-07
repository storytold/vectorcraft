//! Variable Font settings: type set at axis settings no named instance has is shaped, drawn,
//! measured and embedded there.

use std::collections::BTreeMap;

use kurbo::{Point, Shape};
use skrifa::MetadataProvider;
use skrifa::instance::{LocationRef, Size};
use skrifa::raw::TableProvider;
use vectorcraft_doc::CharStyle;

use crate::test_fonts::{TWO_AXIS_FAMILY as FAMILY, TWO_AXIS_NARROW, VARIABLE_CHARS, VARIABLE_WIDEN, two_axis_font};
use crate::*;

/// A database of the bundled fonts and the two-axis test font.
fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(two_axis_font().expect("the test font builds")), 3, "Regular, Bold and Condensed");
    db
}

fn axes(settings: &[(&str, f64)]) -> BTreeMap<String, f64> {
    settings.iter().map(|(t, v)| (t.to_string(), *v)).collect()
}

fn text(style: &str, settings: &[(&str, f64)]) -> TextObject {
    let st = CharStyle { font_family: FAMILY.into(), font_style: style.into(), size: 100.0, font_axes: axes(settings), ..CharStyle::default() };
    TextObject::point(Point::ZERO, VARIABLE_CHARS, st)
}

/// The advances and the ink width of the laid-out text.
fn widths(db: &FontDb, t: &TextObject) -> (f64, f64) {
    let l = layout(db, t);
    (l.glyphs.iter().map(|g| g.advance).sum(), l.to_bezpath().bounding_box().width())
}

#[test]
fn a_variable_font_lists_its_axes() {
    let db = db();
    let regular = db.face(FAMILY, "Regular").unwrap();
    let list: Vec<(String, f32, f32, f32)> = regular.axes().into_iter().map(|a| (a.name, a.min, a.default, a.max)).collect();
    assert_eq!(list, [("Weight".to_string(), 400.0, 400.0, 700.0), ("Width".to_string(), 75.0, 100.0, 100.0)]);
    assert_eq!(regular.axes()[0].tag, *b"wght");
    // A named instance's settings, and the defaults of the axes it leaves alone.
    let bold = db.face(FAMILY, "Bold").unwrap();
    assert_eq!((bold.axis_value(*b"wght"), bold.axis_value(*b"wdth"), bold.axis_value(*b"opsz")), (Some(700.0), Some(100.0), None));
    assert!(db.face(FALLBACK_FAMILY, "Regular").unwrap().axes().is_empty(), "a static face");
}

/// Font menus' Font Technology › Variable: a family with variation axes is variable, the bundled
/// static families aren't.
#[test]
fn a_family_with_variation_axes_is_variable() {
    let db = db();
    assert!(db.family_traits(FAMILY).variable);
    assert_eq!(db.family_traits(FAMILY).class, FontClass::Sans, "its kind is read as before");
    for family in ["Source Sans 3", "Source Serif 4", "Inter", "JetBrains Mono"] {
        assert!(!db.family_traits(family).variable, "{family} is static");
    }
}

#[test]
fn axis_settings_lay_out_and_draw_between_the_named_instances() {
    let db = db();
    let (regular, bold) = (widths(&db, &text("Regular", &[])), widths(&db, &text("Bold", &[])));
    let between = widths(&db, &text("Regular", &[("wght", 650.0)]));
    // wght 650 is 5/6 of the way to the heaviest weight: the glyphs are 5/6 of the way wider.
    let k = 1.0 + f64::from(VARIABLE_WIDEN) * 250.0 / 300.0;
    assert!((between.0 - regular.0 * k).abs() < 1.0, "advances {regular:?} {between:?} {bold:?}");
    assert!((between.1 - regular.1 * k).abs() < 1.5, "ink {regular:?} {between:?} {bold:?}");
    assert!(regular.0 < between.0 && between.0 < bold.0 && regular.1 < between.1 && between.1 < bold.1);
    // The glyphs come from a face of their own, found again by its id, at those settings.
    let l = layout(&db, &text("Regular", &[("wght", 650.0)]));
    let face = db.face_by_id(l.glyphs[0].font_id).unwrap();
    assert_eq!((face.family.as_str(), face.style.as_str(), face.variations(), face.weight), (FAMILY, "Regular", &[(*b"wght", 650.0)][..], 650.0));
    assert_ne!(face.id(), db.face(FAMILY, "Regular").unwrap().id());
    // The same settings again: the same face.
    assert_eq!(layout(&db, &text("Regular", &[("wght", 650.0)])).glyphs[0].font_id, face.id());
    // Width on top of the Bold instance: Bold's weight stays, the glyphs narrow.
    let narrow = widths(&db, &text("Bold", &[("wdth", 75.0)]));
    let k = (1.0 + f64::from(VARIABLE_WIDEN)) - f64::from(TWO_AXIS_NARROW);
    assert!((narrow.0 - regular.0 * k).abs() < 1.0, "{narrow:?} vs {regular:?}");
    let l = layout(&db, &text("Bold", &[("wdth", 75.0)]));
    assert_eq!(db.face_by_id(l.glyphs[0].font_id).unwrap().variations(), [(*b"wdth", 75.0), (*b"wght", 700.0)]);
}

/// Neighboring runs at other settings are shaped apart, each in its own instance; runs at the same
/// settings that differ only in paint are shaped together and keep the font's kerning.
#[test]
fn other_settings_shape_a_run_apart_and_the_same_settings_keep_the_kerning() {
    let db = db();
    // 40 pt "AVAV", which the font kerns, at `settings`, with the bytes `restyled` restyled by
    // `restyle`: each glyph's source byte, pen position, advance and face id.
    let avav = |settings: &[(&str, f64)], restyled: std::ops::Range<usize>, restyle: &dyn Fn(&mut CharStyle)| {
        let st = CharStyle { font_family: FAMILY.into(), size: 40.0, font_axes: axes(settings), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, "AVAV", st);
        edit::style_range(&mut t.runs, restyled.start, restyled.end, restyle);
        layout(&db, &t).glyphs.iter().map(|g| (g.byte, g.origin.x, g.advance, g.font_id)).collect::<Vec<_>>()
    };
    let same = [("wght", 650.0)];
    let plain = avav(&same, 0..0, &|_| {});
    let unkerned = avav(&same, 0..4, &|s| s.kerning = Some(0.0));
    assert!((plain[0].2 - unkerned[0].2).abs() > 0.1, "the font kerns A and V: {plain:?} {unkerned:?}");
    assert_eq!(avav(&same, 1..2, &|s| s.fill = Default::default()), plain, "a V in another paint at the same settings");
    // The V at another weight: an instance of its own, shaped without the A-V pair.
    let apart = avav(&same, 1..2, &|s| {
        s.font_axes = axes(&[("wght", 500.0)]);
    });
    let face = db.face_by_id(apart[1].3).unwrap();
    assert_eq!(face.variations(), [(*b"wght", 500.0)], "{apart:?}");
    assert_ne!(apart[1].3, plain[1].3);
    assert_eq!((apart[0].2, apart[0].3), (unkerned[0].2, plain[0].3), "the A before it is not kerned with it: {apart:?}");
}

#[test]
fn settings_are_clamped_and_unknown_axes_left_out() {
    let db = db();
    let regular = db.face(FAMILY, "Regular").unwrap();
    let past = db.instance(&regular, &axes(&[("wght", 2000.0), ("opsz", 12.0)]));
    assert_eq!(past.variations(), [(*b"wght", 700.0)]);
    // Drawn as the heaviest weight.
    let bold = db.face(FAMILY, "Bold").unwrap();
    let l = bold.glyph_for('l');
    assert_eq!(past.advance(l), bold.advance(l));
    assert_eq!(db.outline(&past, l).to_svg(), db.outline(&bold, l).to_svg());
    // Settings that change nothing, and a static face: the face itself.
    for same in [axes(&[("wght", 400.0)]), axes(&[("wdth", 100.0), ("opsz", 12.0)]), axes(&[("wght", f64::NAN)])] {
        assert_eq!(db.instance(&regular, &same).id(), regular.id(), "{same:?}");
    }
    assert_eq!(db.instance(&bold, &axes(&[("wght", 700.0)])).id(), bold.id());
    let fixed = db.face(FALLBACK_FAMILY, "Regular").unwrap();
    assert_eq!(db.instance(&fixed, &axes(&[("wght", 650.0)])).id(), fixed.id());
}

#[test]
fn an_instance_is_found_by_its_id_after_the_cache_drops_it() {
    let db = db();
    let regular = db.face(FAMILY, "Regular").unwrap();
    let first = db.instance(&regular, &axes(&[("wght", 401.0)]));
    let (id, l) = (first.id(), first.glyph_for('l'));
    let (advance, outline) = (first.advance(l), db.outline(&first, l).to_svg());
    // More settings than the cache holds.
    for i in 0..fontdb::MAX_LIVE_INSTANCES + 10 {
        db.instance(&regular, &axes(&[("wght", 402.0 + i as f64 * 0.5)]));
    }
    drop(first);
    // A glyph laid out before keeps its face: built again, with the same id and settings.
    let again = db.face_by_id(id).unwrap();
    assert_eq!((again.id(), again.variations(), again.advance(l)), (id, &[(*b"wght", 401.0)][..], advance));
    assert_eq!(db.outline(&again, l).to_svg(), outline);
    assert_eq!(db.instance(&regular, &axes(&[("wght", 401.0)])).id(), id, "the same settings, the same id");
}

#[test]
fn an_instance_embeds_as_a_static_font_at_its_settings() {
    let db = db();
    let regular = db.face(FAMILY, "Regular").unwrap();
    let face = db.instance(&regular, &axes(&[("wght", 650.0), ("wdth", 90.0)]));
    let e = face.embed(&['l']).unwrap();
    let f = skrifa::FontRef::new(&e.data).unwrap();
    assert!(f.fvar().is_err() && f.gvar().is_err(), "no variations left");
    let adv = f.glyph_metrics(Size::unscaled(), LocationRef::default()).advance_width(f.charmap().map('l').unwrap()).unwrap();
    assert!((f64::from(adv) - face.advance(face.glyph_for('l'))).abs() <= 1.0, "{adv}");
    assert!((f64::from(adv) - regular.advance(regular.glyph_for('l'))).abs() > 5.0, "not the default instance's: {adv}");
}
