//! Arabic marks (harakat) styled apart from their letters ([`crate::harakat`]).
//! Layout cases need an Arabic face: craft-fonts' Noto Sans Arabic when the build has
//! `CRAFT_FONTS_DIR`, else an installed OpenType Arabic face. Without either they skip.

use std::sync::Arc;

use kurbo::{Affine, Point, Rect, Shape};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::text::Kashida;
use vectorcraft_doc::{CharStyle, Justify, TextKind, TextObject};
use vectorcraft_geom::PathData;

use crate::harakat::{harakat_ranges, is_gdef_mark, is_harakat, own_chars, paint_only, paint_only_mark_boundary};
use crate::{FontDb, FontFace, PositionedGlyph, TextLayout, layout};

const FAMILY: &str = "Noto Sans Arabic";
const VOCALIZED: &str = "بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ";
/// Short vocalized word used to probe whether a face's shaped mark glyphs are GDEF class 3.
const GDEF_PROBE: &str = "بَبَب";

/// An OpenType Arabic face covering harakat whose shaped fixture glyphs include GDEF mark
/// (class 3) glyphs. Strict colour/ownership layout tests need that: without GDEF, production
/// correctly keeps the letter's paint and those assertions would fail. Fonts with coverage and
/// GSUB but no mark classes are skipped here; synthetic `own_chars` cases and
/// `installed_fonts_keep_their_glyphs_with_coloured_harakat` still cover the no-GDEF fallback.
fn arabic_face(test: &str) -> Option<Arc<FontFace>> {
    let opentype = |face: &FontFace| {
        use skrifa::raw::TableProvider;
        face.skrifa().is_some_and(|f| f.gsub().is_ok())
    };
    let db = FontDb::global();
    let face = ["Noto Sans Arabic", "Arial", "Geeza Pro", "SF Arabic", "Tahoma", "Times New Roman"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular"))
        .find(|face| {
            if !(face.covers('ب') && face.covers('\u{064E}') && face.covers('\u{0640}') && opentype(face)) {
                return false;
            }
            // Shape the probe: every zero-advance authored glyph must be a GDEF mark, and at least
            // one must appear, so strict colour assertions match production ownership.
            let shaped = layout(db, &text_with_harakat(GDEF_PROBE, &face.family, None));
            let marks: Vec<_> = shaped.glyphs.iter().filter(|g| g.len > 0 && g.advance.abs() < 1e-6).collect();
            !marks.is_empty() && marks.iter().all(|g| is_gdef_mark(face, g.gid))
        });
    match &face {
        Some(face) => eprintln!("ASSERTIONS RUN: {test}; Arabic font {} {} (GDEF marks)", face.family, face.style),
        None => {
            eprintln!("SKIPPED: {test}; no OpenType Arabic font with harakat and GDEF mark classes (set CRAFT_FONTS_DIR or install one)")
        }
    }
    face
}

fn red() -> Paint {
    Paint::solid(Color::rgb8(200, 30, 30))
}

fn style(family: &str) -> CharStyle {
    CharStyle { font_family: family.into(), size: 48.0, ..CharStyle::default() }
}

/// `text` in `family`, its harakat filled with `fill` (`None`: as the letters).
fn text_with_harakat(text: &str, family: &str, fill: Option<Paint>) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style(family));
    if let Some(fill) = fill {
        for r in harakat_ranges(text, 0..text.len()) {
            crate::edit::style_range(&mut t.runs, r.start, r.end, |st| st.fill = fill.clone());
        }
    }
    t
}

/// One glyph's geometry, bit for bit: face, glyph id, cluster, pen position, transform, advance.
type Geometry = (u32, u32, usize, usize, [u64; 2], [u64; 6], u64);

fn geometry(l: &TextLayout) -> Vec<Geometry> {
    l.glyphs
        .iter()
        .map(|g| {
            (g.font_id, g.gid, g.byte, g.len, [g.origin.x.to_bits(), g.origin.y.to_bits()], g.xf.as_coeffs().map(f64::to_bits), g.advance.to_bits())
        })
        .collect()
}

fn painted(t: &TextObject, l: &TextLayout, fill: &Paint) -> usize {
    l.glyphs.iter().filter(|g| run_of(t, g).style.fill == *fill).count()
}

fn run_of<'a>(t: &'a TextObject, g: &PositionedGlyph) -> &'a vectorcraft_doc::TextRun {
    t.runs.get(g.run).unwrap_or_else(|| panic!("glyph {} at byte {} names missing run {}", g.gid, g.byte, g.run))
}

type LetterGlyphs = std::collections::HashSet<(u32, u32)>;

/// Glyphs drawn by the fixture's bare letters, including separately shaped letter dots.
/// Include the face id: glyph ids are only meaningful within their own font.
fn letter_glyphs(text: &str, family: &str) -> LetterGlyphs {
    let bare: String = text.chars().filter(|&c| !is_harakat(c)).collect();
    layout(FontDb::global(), &text_with_harakat(&bare, family, None)).glyphs.iter().map(|g| (g.font_id, g.gid)).collect()
}

fn fixture_mark(g: &PositionedGlyph, letters: &LetterGlyphs) -> bool {
    // Some fonts draw a letter's dots as zero-advance GDEF marks. They keep the letter's paint.
    g.advance.abs() < 1e-6 && !letters.contains(&(g.font_id, g.gid))
}

/// Expected fill from the fixture's source runs, independent of the glyph's assigned `run`:
/// zero-advance glyphs absent from the bare letters take the first harakat's fill in their
/// cluster; letter glyphs (including their dots) take the cluster-start fill.
fn expected_fill(t: &TextObject, text: &str, g: &PositionedGlyph, letters: &LetterGlyphs) -> Paint {
    let end = (g.byte + g.len).min(text.len());
    let span = text.get(g.byte..end).unwrap_or("");
    let letter_fill = style_fill_at(t, g.byte);
    if fixture_mark(g, letters) && span.chars().any(is_harakat) {
        // Mark glyph: first harakat in the cluster span.
        let off = span.char_indices().find(|(_, c)| is_harakat(*c)).map(|(i, _)| g.byte + i).unwrap_or(g.byte);
        return style_fill_at(t, off);
    }
    letter_fill
}

fn style_fill_at(t: &TextObject, byte: usize) -> Paint {
    let mut off = 0;
    for r in &t.runs {
        let end = off + r.text.len();
        if byte < end || (byte == end && byte == off) {
            return r.style.fill.clone();
        }
        off = end;
    }
    t.runs.last().map(|r| r.style.fill.clone()).unwrap_or_else(|| Paint::solid(Color::BLACK))
}

#[test]
fn paint_only_differences_are_told_apart() {
    let a = style(FAMILY);
    let mut b = a.clone();
    b.fill = red();
    b.stroke = red();
    b.stroke_width = 2.0;
    assert!(paint_only(&a, &b) && paint_only(&a, &a));
    let mut c = a.clone();
    c.size = 12.0;
    assert!(!paint_only(&a, &c));
    let mut d = b.clone();
    d.baseline_shift = 3.0;
    assert!(!paint_only(&a, &d), "a raised harakat is not paint only");
    assert!(paint_only_mark_boundary("بِ", "ب".len(), &a, &b));
    assert!(!paint_only_mark_boundary("بب", "ب".len(), &a, &b), "between two letters");
    assert!(!paint_only_mark_boundary("e\u{301}", 1, &a, &b), "a Latin accent keeps the existing rule");
    assert_eq!(harakat_ranges("بِسْمِ", 0.."بِسْمِ".len()), [2..4, 6..8, 10..12]);
    assert_eq!(harakat_ranges("اللَّهِ", 0.."اللَّهِ".len()), [6..10, 12..14], "shadda and fatha together");
}

/// The allocation-free comparison must keep the previous comparison's complete contract,
/// including attributes that affect layout rather than shaping and non-reflexive file values.
#[test]
fn paint_only_keeps_every_nonpaint_attribute_and_partial_equality() {
    use vectorcraft_doc::{CharAlign, CharPosition, Dash, Digits, FontHeightReference, GlyphChoice, LineCap, LineJoin, ParaDirection, StrokeAlign};

    fn previous(a: &CharStyle, b: &CharStyle) -> bool {
        let mut b = b.clone();
        b.fill = a.fill.clone();
        b.stroke = a.stroke.clone();
        b.stroke_width = a.stroke_width;
        b.overprint_fill = a.overprint_fill;
        b.overprint_stroke = a.overprint_stroke;
        b.stroke_cap = a.stroke_cap;
        b.stroke_join = a.stroke_join;
        b.stroke_miter_limit = a.stroke_miter_limit;
        b.stroke_dash = a.stroke_dash.clone();
        b.stroke_align = a.stroke_align;
        b.style_name = a.style_name.clone();
        b.kerning_before = a.kerning_before;
        *a == b
    }

    fn check(a: &CharStyle, b: &CharStyle, want: bool, name: &str) {
        for (left, right) in [(a, b), (b, a)] {
            assert_eq!(paint_only(left, right), previous(left, right), "{name}: old contract");
        }
        assert_eq!(paint_only(a, b), want, "{name}");
    }

    let a = CharStyle {
        fill: Paint::Pattern { pattern: "named pattern".repeat(8), xf: Affine::IDENTITY },
        stroke: Paint::Solid { color: Color::BLACK, swatch: Some("global swatch".repeat(8)), tint: 0.5 },
        stroke_dash: Some(Dash { pattern: vec![2.0, 3.0, 4.0, 5.0], offset: 0.5, align_corners: true }),
        style_name: Some("named character style".repeat(8)),
        ..style(FAMILY)
    };
    macro_rules! field {
        ($field:ident, $value:expr, $want:expr) => {{
            let mut b = a.clone();
            b.$field = $value;
            check(&a, &b, $want, stringify!($field));
        }};
    }
    field!(font_family, "other family".into(), false);
    field!(font_style, "Bold".into(), false);
    field!(font_version, Some("other version".into()), false);
    field!(font_axes, [("wght".into(), 700.0)].into(), false);
    field!(size, 13.0, false);
    field!(leading, Some(60.0), false);
    field!(tracking, 20.0, false);
    field!(kerning, Some(0.0), false);
    field!(kerning_before, 250.0, true);
    field!(optical_kerning, true, false);
    field!(baseline_shift, 1.0, false);
    field!(h_scale, 90.0, false);
    field!(v_scale, 90.0, false);
    field!(rotation, 3.0, false);
    field!(underline, true, false);
    field!(strikethrough, true, false);
    field!(all_caps, true, false);
    field!(features, vec!["-liga".into()], false);
    field!(position, CharPosition::Superscript(Default::default()), false);
    field!(small_caps, Some(80.0), false);
    field!(char_align, CharAlign::EmBoxTop, false);
    field!(char_direction, Some(ParaDirection::RightToLeft), false);
    field!(kashidas, false, false);
    field!(proportional_metrics, true, false);
    field!(tate_chu_yoko, true, false);
    field!(tcy_up_down, 1.0, false);
    field!(tcy_left_right, 1.0, false);
    field!(standard_vertical_roman, true, false);
    field!(digits, Digits::Hindi, false);
    field!(font_height_reference, FontHeightReference::CapHeight, false);
    field!(no_break, true, false);
    field!(language, Some("ar".into()), false);
    field!(glyph_alternate, Some(GlyphChoice { ch: 'a', gid: 1, font: "other".into(), version: String::new() }), false);
    field!(fill, Paint::None, true);
    field!(stroke, Paint::None, true);
    field!(stroke_width, 2.0, true);
    field!(overprint_fill, true, true);
    field!(overprint_stroke, true, true);
    field!(stroke_cap, LineCap::Round, true);
    field!(stroke_join, LineJoin::Round, true);
    field!(stroke_miter_limit, 2.0, true);
    field!(stroke_dash, None, true);
    field!(stroke_align, StrokeAlign::Outside, true);
    field!(style_name, None, true);
    // Only the first style's ignored fields survive the old normalization. A NaN in the
    // second style's paint is ignored, but reversing the arguments must still return false.
    field!(fill, Paint::solid(Color::Gray { k: f32::NAN }), true);
    field!(stroke, Paint::solid(Color::Gray { k: f32::NAN }), true);
    field!(stroke_width, f64::NAN, true);
    field!(stroke_miter_limit, f64::NAN, true);
    field!(stroke_dash, Some(Dash { pattern: vec![f64::NAN], offset: 0.0, align_corners: false }), true);
    field!(tracking, f64::NAN, false);
}

/// A face without usable mark classification keeps cluster-start paint, without changing
/// its glyphs or positioning when a harakat's paint is edited. All variants are in-memory.
#[test]
fn synthetic_harakat_keep_geometry_with_and_without_gdef_marks() {
    use skrifa::raw::TableProvider;
    use write_fonts::FontBuilder;
    use write_fonts::tables::gdef::Gdef;
    use write_fonts::tables::layout::ClassDefFormat1;
    use write_fonts::types::{GlyphId16, Tag};

    let bytes = crate::test_fonts::lam_alef_font(b"rlig").unwrap();
    let font = skrifa::FontRef::new(&bytes).unwrap();
    for variant in ["marks", "no GDEF", "no class definition", "base classes"] {
        let mut builder = FontBuilder::new();
        for r in font.table_directory.table_records() {
            let tag = Tag::new(&r.tag().to_be_bytes());
            if variant == "marks" || tag != Tag::new(b"GDEF") {
                builder.add_raw(tag, font.table_data(r.tag()).unwrap().as_bytes());
            }
        }
        if variant == "no class definition" {
            builder.add_table(&Gdef::new(None, None, None, None)).unwrap();
        } else if variant == "base classes" {
            let classes = ClassDefFormat1::new(GlyphId16::new(0), vec![1; usize::from(font.maxp().unwrap().num_glyphs())]);
            builder.add_table(&Gdef::new(Some(classes.into()), None, None, None)).unwrap();
        }
        let db = FontDb::with_font_dirs(vec![]);
        assert_eq!(db.add_font(builder.build()), 1, "{variant}");
        let family = crate::test_fonts::lam_alef_family(b"rlig");
        let plain = text_with_harakat("بَبَبَ", family, None);
        let coloured = text_with_harakat("بَبَبَ", family, Some(red()));
        let before = layout(&db, &plain);
        let after = layout(&db, &coloured);
        assert!(!before.glyphs.is_empty(), "{variant}");
        assert_eq!(geometry(&before), geometry(&after), "{variant}");
        assert_eq!(coloured.plain_text(), plain.plain_text(), "{variant}");
        if variant == "marks" {
            assert!(painted(&coloured, &after, &red()) > 0, "{variant}: harakat own their paint");
        } else {
            assert!(after.glyphs.iter().all(|g| run_of(&coloured, g).style.fill != red()), "{variant}: cluster-start paint");
        }
    }
}

/// A range is normalised as `edit::style_range` takes one: clamped, each end moved back to the
/// start of its character, put in order. Never a silent empty result for a usable range.
#[test]
fn harakat_ranges_normalise_their_range() {
    let word = "بِسْمِ"; // ب 0..2, kasra 2..4, س 4..6, sukun 6..8, م 8..10, kasra 10..12
    let all = harakat_ranges(word, 0..word.len());
    assert_eq!(harakat_ranges(word, word.len()..0), all, "reversed");
    assert_eq!(harakat_ranges(word, 0..usize::MAX), all, "past the end");
    assert!(harakat_ranges(word, usize::MAX..usize::MAX).is_empty(), "wholly past the end");
    assert_eq!(harakat_ranges(word, 1..word.len()), all, "a start inside ب moves back to it");
    assert_eq!(harakat_ranges(word, 0..7), [std::ops::Range { start: 2, end: 4 }], "an end inside the sukun moves back before it");
    assert_eq!(harakat_ranges(word, 3..9), [2..4, 6..8], "both ends inside characters");
    assert_eq!(harakat_ranges(word, std::ops::Range { start: 9, end: 3 }), [2..4, 6..8], "reversed, inside characters");
    assert!(harakat_ranges(word, 4..4).is_empty(), "empty");
}

/// The character-level pass names each glyph's own character only when it sets the same glyphs
/// the same way, and only a character in that glyph's shaping cluster: repeated glyph ids alone
/// don't do. A later in-cluster owner is kept only for a GDEF mark glyph on a harakat character;
/// everything else stays at the cluster start (conservative).
/// A space before the line's first letter (#5111), set on the letter alone as the Type tool's caret
/// sets it, keeps the letter's harakat coloured apart attached and in their own colour. Right to
/// left the space is past the letter's right side, so every glyph stays where it was and the line
/// is the space longer. Checked with the in-memory Arabic test font (its fatha a `GDEF` mark) in
/// every environment, and with an installed Arabic face when there is one.
#[test]
fn a_space_before_the_first_letter_keeps_its_coloured_harakat() {
    let db = FontDb::with_font_dirs(vec![]);
    assert_eq!(db.add_font(crate::test_fonts::lam_alef_font(b"rlig").unwrap()), 1);
    check_spaced_harakat(&db, crate::test_fonts::lam_alef_family(b"rlig"));
    if let Some(face) = arabic_face("a_space_before_the_first_letter_keeps_its_coloured_harakat") {
        check_spaced_harakat(FontDb::global(), &face.family);
    }
}

fn check_spaced_harakat(db: &FontDb, family: &str) {
    const WORD: &str = "بَبَب"; // ب 0, fatha 2, ب 4, fatha 6, ب 8
    let coloured = text_with_harakat(WORD, family, Some(red()));
    let mut spaced = coloured.clone();
    crate::edit::style_range(&mut spaced.runs, 0, 2, |st| st.kerning_before = 250.0);
    let (a, b) = (layout(db, &coloured), layout(db, &spaced));
    let placed = |l: &TextLayout| l.glyphs.iter().map(|g| (g.font_id, g.gid, g.byte, g.xf.as_coeffs().map(f64::to_bits))).collect::<Vec<_>>();
    assert_eq!(placed(&a), placed(&b), "{family}: the same glyphs where they were");
    assert!(painted(&spaced, &b, &red()) > 0, "{family}: harakat painted apart");
    assert_eq!(painted(&spaced, &b, &red()), painted(&coloured, &a, &red()), "{family}: the harakat keep their colour");
    // 250/1000 of the 48 pt em.
    let length = |l: &TextLayout| l.glyphs.iter().map(|g| g.advance).sum::<f64>();
    assert!((length(&b) - length(&a) - 12.0).abs() < 1e-6, "{family}: {} → {}", length(&a), length(&b));
}

#[test]
fn own_chars_need_the_same_setting_and_each_character_in_its_cluster() {
    // بَبَب right to left: final beh (cluster 8), fatha and medial beh (cluster 4), fatha and
    // initial beh (cluster 0). Bytes: ب0 ف2 ب4 ف6 ب8. Glyph id 20 is the fatha (GDEF mark).
    const TEXT: &str = "بَبَب";
    let marks = |gid: u32| gid == 20;
    let raw = [(10, 8, 500, 0, 0), (20, 4, 0, -30, 400), (11, 4, 300, 0, 0), (20, 0, 0, -40, 420), (12, 0, 450, 0, 0)];
    let by_char = |own: [u32; 5]| raw.iter().zip(own).map(|(g, c)| (g.0, c, g.2, 0, g.3, g.4)).collect::<Vec<_>>();
    let ok = by_char([8, 6, 4, 2, 0]);
    assert_eq!(own_chars(&raw, &ok, 10, TEXT, marks), [8, 6, 4, 2, 0]);
    // The fathas' characters swapped (the same glyph ids): each is outside its glyph's cluster,
    // so both keep their cluster's (their letter's) character, never the other fatha's.
    assert_eq!(own_chars(&raw, &by_char([8, 2, 4, 6, 0]), 10, TEXT, marks), [8, 4, 4, 0, 0]);
    // A character before its glyph's cluster, or past the segment's end: the cluster's own.
    assert_eq!(own_chars(&raw, &by_char([8, 3, 4, 2, 0]), 10, TEXT, marks), [8, 4, 4, 2, 0]);
    assert_eq!(own_chars(&raw, &by_char([9, 6, 4, 2, 0]), 9, TEXT, marks), [8, 6, 4, 2, 0]);
    // A base letter named as its mark keeps the letter (not a GDEF mark).
    assert_eq!(own_chars(&raw, &by_char([8, 6, 6, 2, 0]), 10, TEXT, marks), [8, 6, 4, 2, 0]);
    // Without GDEF mark classification every later owner falls back to the cluster start.
    assert_eq!(own_chars(&raw, &ok, 10, TEXT, |_| false), [8, 4, 4, 0, 0]);
    // Wide mark (advance 500) beside a narrower base (300): mark keeps 2; base keeps 0.
    // Review counterexample: advance must not decide ownership.
    let wide = [(20, 0, 500, 0, 0), (11, 0, 300, 0, 0)];
    let wide_by = [(20, 2, 500, 0, 0, 0), (11, 2, 300, 0, 0, 0)];
    assert_eq!(own_chars(&wide, &wide_by, 4, "بَ", marks), [2, 0], "wide GDEF mark keeps harakat owner");
    // Equal advances: same rule.
    let equal = [(20, 0, 300, 0, 0), (11, 0, 300, 0, 0)];
    let equal_by = [(20, 2, 300, 0, 0, 0), (11, 2, 300, 0, 0, 0)];
    assert_eq!(own_chars(&equal, &equal_by, 4, "بَ", marks), [2, 0], "equal advances");
    // Multi-component glyph (GDEF class 4, not mark) named as a later harakat stays at cluster start.
    let multi = [(40, 0, 100, 0, 0), (11, 0, 300, 0, 0)];
    let multi_by = [(40, 2, 100, 0, 0, 0), (11, 0, 300, 0, 0, 0)];
    assert_eq!(own_chars(&multi, &multi_by, 4, "بَ", marks), [0, 0], "component is not a mark");
    // Single-glyph ligature (one output for letter+mark) named as the mark stays at cluster start:
    // one glyph can represent a ligature containing a base, not a lone mark.
    let liga = [(50, 0, 400, 0, 0)];
    let liga_by = [(50, 2, 400, 0, 0, 0)];
    assert_eq!(own_chars(&liga, &liga_by, 4, "بَ", |_| false), [0], "ligature without GDEF mark class");
    assert_eq!(own_chars(&liga, &liga_by, 4, "بَ", |gid| gid == 50), [2], "only if GDEF names it mark");
    // Sole advancing glyph without GDEF mark proof stays at the cluster start.
    let advancing = [(20, 0, 100, 0, 0)];
    let advancing_by = [(20, 2, 100, 0, 0, 0)];
    assert_eq!(own_chars(&advancing, &advancing_by, 4, "بَ", |_| false), [0]);
    assert_eq!(own_chars(&advancing, &advancing_by, 4, "بَ", marks), [2]);
    // The same ids set otherwise (an advance or an offset changed, another glyph, one short):
    // nothing is borrowed.
    let mut moved = ok.clone();
    moved[1].5 += 1;
    assert!(own_chars(&raw, &moved, 10, TEXT, marks).is_empty(), "a mark moved");
    let mut wider = ok.clone();
    wider[2].2 += 1;
    assert!(own_chars(&raw, &wider, 10, TEXT, marks).is_empty(), "an advance changed");
    let mut other = ok.clone();
    other[0].0 = 13;
    assert!(own_chars(&raw, &other, 10, TEXT, marks).is_empty(), "another glyph");
    assert!(own_chars(&raw, &ok[1..], 10, TEXT, marks).is_empty(), "a glyph short");
}

/// A word whose letters and marks repeat the same glyph ids (بَبَب): each fatha takes its own
/// colour, on its own letter, and every glyph is set as without colour.
#[test]
fn repeated_glyphs_take_their_own_harakat_colour() {
    let Some(face) = arabic_face("repeated_glyphs_take_their_own_harakat_colour") else {
        return;
    };
    const WORD: &str = "بَبَب"; // ب 0, fatha 2, ب 4, fatha 6, ب 8
    let db = FontDb::global();
    let green = Paint::solid(Color::rgb8(30, 140, 70));
    let plain = text_with_harakat(WORD, &face.family, None);
    let mut coloured = plain.clone();
    crate::edit::style_range(&mut coloured.runs, 2, 4, |st| st.fill = green.clone());
    crate::edit::style_range(&mut coloured.runs, 6, 8, |st| st.fill = red());
    let (a, b) = (layout(db, &plain), layout(db, &coloured));
    assert_eq!(geometry(&a), geometry(&b), "joining and mark positions unchanged");
    assert_authored_ownership(&coloured, WORD, &b, "repeated");
    let fill = |g: &PositionedGlyph| run_of(&coloured, g).style.fill.clone();
    let letters = letter_glyphs(WORD, &face.family);
    let marks: Vec<&PositionedGlyph> = b.glyphs.iter().filter(|g| fixture_mark(g, &letters) && !g.outline.is_empty()).collect();
    assert_eq!(marks.len(), 2, "two fathas");
    assert_eq!(marks[0].gid, marks[1].gid, "the same glyph id twice");
    for m in &marks {
        let want = if m.byte == 0 { green.clone() } else { red() };
        assert!(m.byte == 0 || m.byte == 4, "a fatha in its letter's cluster: {}", m.byte);
        assert_eq!(fill(m), want, "the fatha of the letter at {} keeps its own colour", m.byte);
    }
    let letter_fill = plain.runs[0].style.fill.clone();
    assert!(b.glyphs.iter().filter(|g| letters.contains(&(g.font_id, g.gid))).all(|g| fill(g) == letter_fill), "letters keep theirs");
}

/// Coloured harakat: the letters keep their joining and the marks their attachment, glyph for
/// glyph and bit for bit; every mark glyph (and only the marks) is drawn in the harakat's run.
#[test]
fn harakat_coloured_apart_keep_every_glyph_and_take_their_own_colour() {
    let Some(face) = arabic_face("harakat_coloured_apart_keep_every_glyph_and_take_their_own_colour") else {
        return;
    };
    let db = FontDb::global();
    let family = face.family.as_str();
    let plain = text_with_harakat(VOCALIZED, family, None);
    let coloured = text_with_harakat(VOCALIZED, family, Some(red()));
    assert!(coloured.runs.len() > 1);
    let (a, b) = (layout(db, &plain), layout(db, &coloured));
    assert_eq!(geometry(&a), geometry(&b), "joining and mark positions unchanged");
    let letters = letter_glyphs(VOCALIZED, family);
    let marks = b.glyphs.iter().filter(|g| fixture_mark(g, &letters) && !g.outline.is_empty()).count();
    assert!(marks > 0, "vocalized text has mark glyphs");
    assert_eq!(painted(&coloured, &b, &red()), marks);
    assert_authored_ownership(&coloured, VOCALIZED, &b, "coloured");
    let hidden = text_with_harakat(VOCALIZED, family, Some(Paint::None));
    let h = layout(db, &hidden);
    assert_eq!(geometry(&a), geometry(&h));
    assert_eq!(painted(&hidden, &h, &Paint::None), marks);
    assert_authored_ownership(&hidden, VOCALIZED, &h, "hidden");
    let mut sized = text_with_harakat(VOCALIZED, family, None);
    let first = harakat_ranges(VOCALIZED, 0..VOCALIZED.len())[0].clone();
    crate::edit::style_range(&mut sized.runs, first.start, first.end, |st| st.size = 30.0);
    assert_ne!(geometry(&layout(db, &sized)), geometry(&a));
}

/// Justified kashida lines with coloured or hidden harakat keep authored glyph ownership, and
/// inserted tatweels take the base letter's paint—not a trailing mark's.
#[test]
fn coloured_harakat_keep_kashida() {
    let Some(face) = arabic_face("coloured_harakat_keep_kashida") else {
        return;
    };
    let db = FontDb::global();
    let family = face.family.as_str();
    let justified = |t: &mut TextObject, width: f64| {
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, width, 2000.0).to_path(0.1)) };
        t.xf = Affine::IDENTITY;
        t.para.justify = Justify::JustifyAll;
        t.para.kashida = Kashida::Long;
    };
    const PARTLY: &str = "بِسْمِ اللَّهِ الرحمن الرحيم الحمد لله رب العالمين";
    let tat = face.glyph_for('\u{0640}');
    let mut tatweels = 0;
    for width in [200.0, 260.0, 300.0, 420.0, 600.0] {
        let mut plain = text_with_harakat(PARTLY, family, None);
        let mut coloured = text_with_harakat(PARTLY, family, Some(red()));
        let before = layout(db, &coloured);
        assert_authored_ownership(&coloured, PARTLY, &before, &format!("before kashida {width}"));
        justified(&mut plain, width);
        justified(&mut coloured, width);
        let (a, b) = (layout(db, &plain), layout(db, &coloured));
        tatweels += a.glyphs.iter().filter(|g| g.gid == tat && g.len == 0).count();
        assert_eq!(geometry(&a), geometry(&b), "{width}");
        assert_kashida_ownership(&coloured, &before, &b, &red(), &format!("coloured {width}"), false);
    }
    assert!(tatweels > 0, "the text takes kashida");
    let mut stretched = 0;
    for width in [200.0, 260.0, 300.0, 420.0, 600.0] {
        let mut plain = text_with_harakat(FULLY, family, None);
        let mut coloured = text_with_harakat(FULLY, family, Some(red()));
        let mut hidden = text_with_harakat(FULLY, family, Some(Paint::None));
        let before_c = layout(db, &coloured);
        let before_h = layout(db, &hidden);
        assert_authored_ownership(&coloured, FULLY, &before_c, &format!("full before {width}"));
        justified(&mut plain, width);
        justified(&mut coloured, width);
        justified(&mut hidden, width);
        let (a, b, h) = (layout(db, &plain), layout(db, &coloured), layout(db, &hidden));
        assert_eq!(geometry(&a), geometry(&b), "{width}");
        assert_eq!(geometry(&a), geometry(&h), "hidden {width}");
        stretched += stretched_words_with_harakat(FULLY, &b, tat);
        assert_kashida_ownership(&coloured, &before_c, &b, &red(), &format!("full coloured {width}"), false);
        assert_kashida_ownership(&hidden, &before_h, &h, &Paint::None, &format!("full hidden {width}"), false);
    }
    assert!(stretched > 0, "words with coloured harakat take kashida");
}

/// Harakat on every word.
const FULLY: &str = "بِسْمِ اللَّهِ الرَّحْمَنِ الرَّحِيمِ الْحَمْدُ لِلَّهِ رَبِّ الْعَالَمِينَ الرَّحْمَنِ الرَّحِيمِ مَالِكِ يَوْمِ الدِّينِ";

/// The words of `text` with harakat that `l` stretched with a tatweel (glyph `tat`).
fn stretched_words_with_harakat(text: &str, l: &TextLayout, tat: u32) -> usize {
    let mut words: Vec<usize> = l
        .glyphs
        .iter()
        .filter(|g| g.gid == tat && g.len == 0)
        .map(|g| text[..g.byte].rfind(' ').map_or(0, |i| i + 1))
        .filter(|&start| text[start..].split(' ').next().is_some_and(|w| w.chars().any(is_harakat)))
        .collect();
    words.dedup();
    words.len()
}

/// Authored glyphs paint as the independent expected fill from source runs; run indices must exist.
fn assert_authored_ownership(t: &TextObject, text: &str, l: &TextLayout, what: &str) {
    let letters = letter_glyphs(text, &t.runs[0].style.font_family);
    for g in &l.glyphs {
        if g.len == 0 {
            continue;
        }
        let actual = &run_of(t, g).style.fill;
        let want = expected_fill(t, text, g, &letters);
        assert_eq!(actual, &want, "{what}: glyph {} at {} len {} adv {}", g.gid, g.byte, g.len, g.advance);
    }
}

/// Repeated glyphs in one cluster can belong to differently painted runs. Match each authored
/// occurrence in order, without needing an installed font, and still reject a changed owner.
#[test]
fn duplicate_authored_glyphs_keep_occurrence_order() {
    let mut text = TextObject::point(Point::ZERO, "\u{0651}", style(FAMILY));
    text.runs.push(vectorcraft_doc::TextRun::new("\u{0651}", CharStyle { fill: red(), ..style(FAMILY) }));
    let glyph = |run| PositionedGlyph {
        outline: Default::default(),
        run,
        byte: 0,
        origin: Point::ZERO,
        advance: 0.0,
        len: 4,
        angle: 0.0,
        line: 0,
        font_id: 0,
        gid: 7,
        xf: Affine::IDENTITY,
        rtl: true,
        scale: 1.0,
        image: None,
        colour: None,
        bend: None,
        unturned: Affine::IDENTITY,
        pivot: Point::ZERO,
        carets: Vec::new(),
        width: 0.0,
        width_start: 0.0,
    };
    let before = TextLayout { glyphs: vec![glyph(0), glyph(1)], ..TextLayout::default() };
    assert_eq!(assert_kashida_ownership(&text, &before, &before, &red(), "duplicate glyphs", false), 0);

    let mut wrong = before.clone();
    wrong.glyphs[0].run = 1;
    assert!(std::panic::catch_unwind(|| assert_kashida_ownership(&text, &before, &wrong, &red(), "changed owner", false)).is_err());
}

/// Compare authored glyph run indices before/after Insert Kashidas (matched by byte and gid;
/// layout order can differ between point and justified area), and assert every inserted tatweel
/// uses the base letter's paint at the cluster start—not `mark_fill`. When `require_mark_last`,
/// at least one tatweel must follow a same-cluster glyph whose run is the mark paint (so the
/// emitter fix is exercised: reverting `emit_kashida`'s base-run assignment fails).
fn assert_kashida_ownership(
    coloured: &TextObject,
    before: &TextLayout,
    with_kashida: &TextLayout,
    mark_fill: &Paint,
    what: &str,
    require_mark_last: bool,
) -> usize {
    let mut before_runs: std::collections::HashMap<(usize, u32), std::collections::VecDeque<usize>> = std::collections::HashMap::new();
    for g in before.glyphs.iter().filter(|g| g.len > 0) {
        before_runs.entry((g.byte, g.gid)).or_default().push_back(g.run);
    }
    let mut after_n = 0;
    for g in with_kashida.glyphs.iter().filter(|g| g.len > 0) {
        after_n += 1;
        let runs =
            before_runs.get_mut(&(g.byte, g.gid)).unwrap_or_else(|| panic!("{what}: authored glyph {} at {} missing before kashida", g.gid, g.byte));
        // Duplicate glyphs at the same cluster byte retain their occurrence order.
        let want_run = runs.pop_front().unwrap_or_else(|| panic!("{what}: no remaining before-run for {} at {}", g.gid, g.byte));
        assert_eq!(g.run, want_run, "{what}: authored run at {} gid {}", g.byte, g.gid);
        let want_fill = coloured.runs.get(want_run).map(|r| &r.style.fill).unwrap_or_else(|| panic!("{what}: missing run {want_run}"));
        assert_eq!(&run_of(coloured, g).style.fill, want_fill, "{what}: authored fill at {}", g.byte);
    }
    assert_eq!(after_n, before.glyphs.iter().filter(|g| g.len > 0).count(), "{what}: authored glyph count");
    assert!(before_runs.values().all(|v| v.is_empty()), "{what}: unjustified authored glyphs left unmatched");
    let mut mark_last = 0;
    for (i, g) in with_kashida.glyphs.iter().enumerate().filter(|(_, g)| g.len == 0) {
        let letter_fill = style_fill_at(coloured, g.byte);
        let fill = &run_of(coloured, g).style.fill;
        assert_eq!(fill, &letter_fill, "{what}: tatweel at {} must use base-letter paint", g.byte);
        assert_ne!(fill, mark_fill, "{what}: tatweel must not inherit mark paint at {}", g.byte);
        // The glyph that absorbed the kashida (last of the cluster in visual order) is emitted
        // immediately before the tatweel and shares its cluster byte.
        if let Some(prev) = i.checked_sub(1).and_then(|j| with_kashida.glyphs.get(j))
            && prev.byte == g.byte
            && prev.len > 0
            && &run_of(coloured, prev).style.fill == mark_fill
            && letter_fill != *mark_fill
        {
            mark_last += 1;
        }
    }
    if require_mark_last {
        assert!(mark_last > 0, "{what}: need a mark-last insertion (last visual glyph has mark paint ≠ letter) to exercise emit_kashida");
    }
    mark_last
}

/// Installed OpenType Arabic faces (when present): coloured harakat change no glyph, position or
/// advance; letter gids from the bare text must not take harakat paint. Glyphs absent from that
/// bare set may take either the harakat colour or the letter colour (AAT/`morx` ligature fallback
/// keeps the letter)—this test does not prove every mark owns its character. Selected AAT faces
/// below record an explicit letter-fill allowance for non-letter gids; they may also paint no
/// glyph in the harakat colour when GDEF is unavailable.
#[test]
fn installed_fonts_keep_their_glyphs_with_coloured_harakat() {
    const FONTS: &[&str] = &[
        "Geeza Pro",
        "SF Arabic",
        "Al Nile",
        "Baghdad",
        "Damascus",
        "KufiStandardGK",
        "DecoType Naskh",
        "Diwan Thuluth",
        "Farah",
        "Noto Nastaliq Urdu",
        "Arial",
        "Times New Roman",
        "Tahoma",
        "Noto Naskh Arabic",
        "Noto Kufi Arabic",
        "Noto Sans Arabic",
        "Amiri",
        "Aref Ruqaa",
        "IBM Plex Sans Arabic",
    ];
    let db = FontDb::with_font_dirs(crate::system_font_dirs());
    let harakat = VOCALIZED.chars().filter(|c| is_harakat(*c)).count();
    let mut rows = vec![];
    for &family in FONTS {
        if !db.has_family(family) {
            continue;
        }
        let Some(face) = db.face(family, "Regular") else {
            continue;
        };
        let plain = text_with_harakat(VOCALIZED, &face.family, None);
        let coloured = text_with_harakat(VOCALIZED, &face.family, Some(red()));
        let bare: String = VOCALIZED.chars().filter(|c| !is_harakat(*c)).collect();
        let (a, b) = (layout(&db, &plain), layout(&db, &coloured));
        assert_eq!(geometry(&a), geometry(&b), "{family}: coloured harakat changed the glyphs");
        let letter_fill = style_fill_at(&coloured, 0);
        let letter_gids: std::collections::HashSet<u32> =
            layout(&db, &text_with_harakat(&bare, &face.family, None)).glyphs.iter().map(|g| g.gid).collect();
        // AAT faces where a ligature mark may keep the letter's colour (explicit allowance, not a
        // per-glyph ownership proof).
        let aat_letter_ok = matches!(family, "Geeza Pro" | "Al Nile" | "Baghdad" | "Damascus" | "KufiStandardGK" | "DecoType Naskh");
        for g in b.glyphs.iter().filter(|g| g.len > 0) {
            let actual = &run_of(&coloured, g).style.fill;
            if letter_gids.contains(&g.gid) {
                // A glyph the bare letters also draw: must not take harakat paint.
                assert_ne!(actual, &red(), "{family}: letter gid {} at {} took harakat paint", g.gid, g.byte);
            } else if aat_letter_ok {
                assert!(
                    actual == &red() || actual == &letter_fill,
                    "{family}: non-letter gid {} at {} may be harakat or letter paint (AAT), got {actual:?}",
                    g.gid,
                    g.byte
                );
            } else {
                // OpenType with GDEF: prefer harakat paint; letter paint still allowed when the
                // character-level pass falls back (does not prove every mark owns its character).
                assert!(
                    actual == &red() || actual == &letter_fill,
                    "{family}: non-letter at {} should be harakat or letter paint, got {actual:?}",
                    g.byte
                );
            }
        }
        let red_glyphs = painted(&coloured, &b, &red());
        // Without GDEF (or when the character-level pass falls back), every glyph may keep the
        // letter's paint—conservative and allowed. This is not every-mark ownership.
        let all_letter = b.glyphs.iter().filter(|g| g.len > 0).all(|g| run_of(&coloured, g).style.fill == letter_fill);
        assert!(red_glyphs > 0 || all_letter, "{family}: expected some harakat paint or a full letter-paint fallback (aat={aat_letter_ok})");
        rows.push(format!("{family}: {} glyphs identical; {red_glyphs} glyphs in the harakat colour ({harakat} harakat)", b.glyphs.len()));
    }
    if rows.is_empty() {
        eprintln!("SKIPPED: installed_fonts_keep_their_glyphs_with_coloured_harakat; none of the candidate families are installed");
    } else {
        eprintln!("{rows:#?}");
    }
}
