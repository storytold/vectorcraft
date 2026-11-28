//! The shaping plans a face keeps: text shaped with a kept plan is laid out as text shaped with a
//! plan of its own, and text in one style makes one plan however many objects hold it.

use harfrust::{Feature, ShapeOptions, Tag, UnicodeBuffer};
use kurbo::Point;
use vectorcraft_doc::{CharStyle, TextObject, TextRun};

use crate::test_fonts::{VARIABLE_FAMILY, variable_font};
use crate::*;

/// A database of the bundled fonts and the test variable font, whose faces hold no plans yet.
fn db() -> FontDb {
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(variable_font().expect("the test font builds"));
    db
}

fn style() -> CharStyle {
    CharStyle { size: 100.0, ..CharStyle::default() }
}

fn point(text: &str, style: CharStyle) -> TextObject {
    TextObject::point(Point::ZERO, text, style)
}

/// What a layout draws and where: every glyph (its font's id, which differs between databases,
/// aside), the bounds and the baselines.
fn drawn(l: &TextLayout) -> Vec<String> {
    let glyph = |g: &PositionedGlyph| {
        format!(
            "{} {} {} {} {:?} {:?} {:?} {:?} {:?} {:?} {:?}",
            g.gid,
            g.byte,
            g.len,
            g.line,
            g.xf,
            g.origin,
            g.advance,
            g.width,
            g.width_start,
            g.angle,
            g.outline.elements()
        )
    };
    let mut all: Vec<String> = l.glyphs.iter().map(glyph).collect();
    all.push(format!("{:?} {:?}", l.bounds, l.baselines()));
    all
}

/// Text objects whose shaping differs in script, direction, language, features and face.
fn cases() -> Vec<(&'static str, TextObject)> {
    let mut mixed = point("AVAV", style());
    let manual = CharStyle { kerning: Some(0.0), ..style() };
    mixed.runs =
        vec![TextRun { text: "AV".into(), style: style(), inline: None }, TextRun { text: "AV".into(), style: manual.clone(), inline: None }];
    let mut vertical = point("AB\u{65e5}\u{672c}", style());
    vertical.vertical = true;
    let variable = |name: &str| CharStyle { font_family: VARIABLE_FAMILY.into(), font_style: name.into(), ..style() };
    vec![
        ("Latin", point("AVATAR office", style())),
        ("other Latin in the same style", point("To Wa fi", style())),
        ("digits and punctuation, no script", point("123 ... 456", style())),
        ("Turkish", point("fi office", CharStyle { language: Some("tr".into()), ..style() })),
        ("kerning off", point("AVAV", manual)),
        ("kerning off in the second run", mixed),
        ("optical kerning", point("AVAV", CharStyle { optical_kerning: true, ..style() })),
        ("small capitals", point("Small Caps", CharStyle { small_caps: Some(70.0), ..style() })),
        ("Arabic", point("\u{645}\u{631}\u{62d}\u{628}\u{627}", style())),
        ("Hebrew", point("\u{5e9}\u{5dc}\u{5d5}\u{5dd}", style())),
        ("vertical", vertical),
        ("variable, Regular", point("AVA", variable("Regular"))),
        ("variable, Bold", point("AVA", variable("Bold"))),
    ]
}

#[test]
fn text_shaped_with_a_kept_plan_is_laid_out_as_with_a_plan_of_its_own() {
    let shared = db();
    // Each object shapes with plans the others left, in both orders.
    for (_, t) in cases().iter().chain(cases().iter().rev()) {
        layout(&shared, t);
    }
    for (name, t) in cases() {
        assert_eq!(drawn(&layout(&shared, &t)), drawn(&layout(&db(), &t)), "{name}");
    }
    // The cases tell plans apart: with the font's kerning off, the pair is spaced differently.
    let kerned = layout(&shared, &point("AV", style()));
    let unkerned = layout(&shared, &point("AV", CharStyle { kerning: Some(0.0), ..style() }));
    assert_ne!(kerned.glyphs[0].advance, unkerned.glyphs[0].advance);
}

#[test]
fn objects_in_one_style_share_one_plan() {
    let db = db();
    let face = db.style_face(&style()).unwrap();
    assert_eq!(face.plans_kept(), 0);
    for i in 0..100 {
        layout(&db, &point(&format!("Ab{i}"), style()));
    }
    assert_eq!(face.plans_kept(), 1, "one plan for 100 objects");
    layout(&db, &point("Ab", CharStyle { language: Some("tr".into()), ..style() }));
    assert_eq!(face.plans_kept(), 2, "another language has another plan");
}

#[test]
fn a_face_keeps_a_limited_number_of_plans() {
    let db = db();
    let face = db.style_face(&style()).unwrap();
    let languages: Vec<String> = ('a'..='z').flat_map(|a| ['a', 'b'].map(|b| format!("{a}{b}"))).collect();
    let text = |language: &str| point("Ab fi", CharStyle { language: Some(language.into()), ..style() });
    for language in &languages {
        layout(&db, &text(language));
    }
    assert!((1..languages.len()).contains(&face.plans_kept()), "{} plans kept", face.plans_kept());
    // One the face no longer holds is made again, and one it holds is used again.
    for language in [&languages[0], &languages[languages.len() - 1]] {
        assert_eq!(drawn(&layout(&db, &text(language))), drawn(&layout(&self::db(), &text(language))), "{language}");
    }
}

/// A kept plan applies the ranges of the features the text is shaped with, not those of the text
/// it was made for: a plan tells only whether a feature covers all of the text.
#[test]
fn a_kept_plan_takes_the_ranges_of_the_features_it_shapes_with() {
    let db = db();
    let face = db.style_face(&style()).unwrap();
    let hb = face.hb().unwrap();
    let shaper = face.shaper.shaper(&hb).instance(face.instance.as_ref()).build();
    let buffer = || {
        let mut buf = UnicodeBuffer::new();
        buf.push_str("AVAVAV");
        buf.guess_segment_properties();
        buf
    };
    let positions = |gb: harfrust::GlyphBuffer| gb.glyph_positions().iter().map(|p| (p.x_advance, p.x_offset)).collect::<Vec<_>>();
    let kern_off = |from: usize| [Feature::new(Tag::new(b"kern"), 0, from..6)];
    let mut seen = vec![];
    for from in [1, 3] {
        let features = kern_off(from);
        let buf = buffer();
        let plan = face.shape_plan(&shaper, buf.direction(), Some(buf.script()), None, &features);
        let kept = positions(shaper.shape(buf, ShapeOptions::new().features(&features).plan(Some(&plan))));
        let own = positions(shaper.shape(buffer(), ShapeOptions::new().features(&features)));
        assert_eq!(kept, own, "kerning off from {from}");
        seen.push(kept);
    }
    assert_eq!(face.plans_kept(), 1, "the two ranges share a plan");
    assert_ne!(seen[0], seen[1], "the two ranges space the text differently");
}
