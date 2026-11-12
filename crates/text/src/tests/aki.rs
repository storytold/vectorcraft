//! Insert Aki Left and Right (`CharStyle::aki_left`, `CharStyle::aki_right`): space before and
//! after a character in reading order, in thousandths of its em, that wrapping counts, alignment
//! keeps at a line's end and justification does not stretch.
use super::{area, db, point, serif, style};
use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{CharStyle, Justify};

/// Where glyph `byte` is drawn along a horizontal line.
fn drawn_x(l: &TextLayout, byte: usize) -> f64 {
    l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.xf.as_coeffs()[4]).unwrap()
}

/// `t` with `f` applied to the style of the character at `byte`.
fn with(t: &TextObject, byte: usize, f: impl Fn(&mut CharStyle)) -> TextObject {
    let mut t = t.clone();
    let end = edit::next_char(&t.plain_text(), byte);
    edit::style_range(&mut t.runs, byte, end, f);
    t
}

/// How far each of `bytes` moved along the line from `a` to `b`, laid out in `fonts`.
fn moves_in(fonts: &FontDb, a: &TextObject, b: &TextObject, bytes: &[usize]) -> Vec<f64> {
    let (la, lb) = (layout(fonts, a), layout(fonts, b));
    bytes.iter().map(|&i| drawn_x(&lb, i) - drawn_x(&la, i)).collect()
}

fn moves(a: &TextObject, b: &TextObject, bytes: &[usize]) -> Vec<f64> {
    moves_in(db(), a, b, bytes)
}

fn near(values: &[f64], expected: &[f64]) -> bool {
    values.len() == expected.len() && values.iter().zip(expected).all(|(v, e)| (v - e).abs() < 1e-6)
}

#[test]
fn aki_left_spaces_before_a_character_and_aki_right_after_it() {
    let plain = point("HOH", style(100.0));
    // 250/1000 em of 100 pt before the O: 25 pt, the O and the H after it move.
    let left = moves(&plain, &with(&plain, 1, |s| s.aki_left = 250.0), &[0, 1, 2]);
    assert!(near(&left, &[0.0, 25.0, 25.0]), "{left:?}");
    // After the O: only the H after it moves.
    let right = moves(&plain, &with(&plain, 1, |s| s.aki_right = 250.0), &[0, 1, 2]);
    assert!(near(&right, &[0.0, 0.0, 25.0]), "{right:?}");
    // Both sides add up, and a value above one em is kept to one em.
    let both = moves(&plain, &with(&plain, 1, |s| (s.aki_left, s.aki_right) = (250.0, 500.0)), &[0, 1, 2]);
    assert!(near(&both, &[0.0, 25.0, 75.0]), "{both:?}");
    let most = moves(&plain, &with(&plain, 1, |s| s.aki_right = 5000.0), &[2]);
    assert!(near(&most, &[100.0]), "{most:?}");
    let not_a_number = moves(&plain, &with(&plain, 1, |s| s.aki_left = f64::NAN), &[1, 2]);
    assert!(near(&not_a_number, &[0.0, 0.0]), "{not_a_number:?}");
    assert_eq!(aki_space("HOH", 1, &with(&plain, 1, |s| s.aki_left = 500.0).runs[1].style, false), (50.0, 0.0));
}

#[test]
fn aki_takes_the_em_of_the_character_as_tracking_does() {
    let narrow = point("HOH", CharStyle { h_scale: 50.0, ..style(100.0) });
    let moved = moves(&narrow, &with(&narrow, 1, |s| s.aki_left = 250.0), &[1, 2]);
    assert!(near(&moved, &[12.5, 12.5]), "{moved:?}");
}

#[test]
fn optical_kerning_spaces_the_pair_as_without_the_aki() {
    let optical = point("AVA", CharStyle { optical_kerning: true, ..style(100.0) });
    let moved = moves(&optical, &with(&optical, 1, |s| s.aki_left = 250.0), &[0, 1, 2]);
    assert!(near(&moved, &[0.0, 25.0, 25.0]), "{moved:?}");
    let moved = moves(&optical, &with(&optical, 0, |s| s.aki_right = 250.0), &[0, 1, 2]);
    assert!(near(&moved, &[0.0, 25.0, 25.0]), "{moved:?}");
}

#[test]
fn a_right_aligned_line_keeps_the_aki_after_its_last_character() {
    let mut right = point("HOH", style(100.0));
    right.para.justify = Justify::Right;
    // Unlike the tracking after a line's last character, the Aki is part of the line.
    let moved = moves(&right, &with(&right, 2, |s| s.aki_right = 250.0), &[0, 1, 2]);
    assert!(near(&moved, &[-25.0, -25.0, -25.0]), "{moved:?}");
    let tracked = moves(&right, &with(&right, 2, |s| s.tracking = 250.0), &[0, 2]);
    assert!(near(&tracked, &[0.0, 0.0]), "{tracked:?}");
    // At the line's start the Aki Left is kept too.
    let mut left = point("HOH", style(100.0));
    left.para.justify = Justify::Left;
    let moved = moves(&left, &with(&left, 0, |s| s.aki_left = 250.0), &[0, 2]);
    assert!(near(&moved, &[25.0, 25.0]), "{moved:?}");
}

#[test]
fn a_line_too_wide_with_its_aki_wraps() {
    let one = layout(db(), &point("HOH HOH", style(50.0)));
    let wide = one.lines[0].x1 - one.lines[0].x0;
    let line_of = |t: &TextObject, byte: usize| layout(db(), t).glyphs.iter().find(|g| g.byte == byte).unwrap().line;
    let fits = area("HOH HOH", style(50.0), Rect::new(0.0, 0.0, wide + 5.0, 400.0), Justify::Left);
    assert_eq!(line_of(&fits, 4), 0, "both words fit");
    // 250/1000 em of 50 pt is 12.5 pt, more than the 5 pt the frame has left.
    assert_eq!(line_of(&with(&fits, 1, |s| s.aki_right = 250.0), 4), 1, "the second word wraps");
    // The Aki after the last letter of a wrapped line stays on that line: the next line starts
    // where it did.
    let narrow = area("HOH HOH", style(50.0), Rect::new(0.0, 0.0, wide * 0.6, 400.0), Justify::Left);
    let spaced = with(&narrow, 2, |s| s.aki_right = 250.0);
    assert_eq!((line_of(&narrow, 4), line_of(&spaced, 4)), (1, 1));
    let moved = moves(&narrow, &spaced, &[0, 4, 5]);
    assert!(near(&moved, &[0.0, 0.0, 0.0]), "{moved:?}");
}

#[test]
fn justification_does_not_stretch_or_narrow_the_aki() {
    // Desired Glyph Scaling 150% widens every glyph, and Word Spacing 200% every word space; the Aki
    // stays 25 pt wide.
    let mut scaled = point("HOH", style(100.0));
    scaled.para.justification.glyph_scaling = [150.0, 150.0, 150.0];
    let moved = moves(&scaled, &with(&scaled, 0, |s| s.aki_right = 250.0), &[0, 1, 2]);
    assert!(near(&moved, &[0.0, 25.0, 25.0]), "{moved:?}");
    let moved = moves(&scaled, &with(&scaled, 1, |s| s.aki_left = 250.0), &[1, 2]);
    assert!(near(&moved, &[25.0, 25.0]), "{moved:?}");
    let mut spaced = point("H H", style(100.0));
    spaced.para.justification.word_spacing = [200.0, 200.0, 200.0];
    let moved = moves(&spaced, &with(&spaced, 1, |s| s.aki_right = 250.0), &[0, 2]);
    assert!(near(&moved, &[0.0, 25.0]), "{moved:?}");
}

#[test]
fn with_whole_point_widths_the_aki_is_whole_points_and_justification_keeps_it() {
    // 250/1000 em of 10 pt is 2.5 pt, set as 3 pt with Fractional Widths off, with or without
    // Desired Glyph Scaling 150%.
    for scaling in [100.0, 150.0] {
        let mut plain = point("HOH", style(10.0));
        plain.fractional_widths = false;
        plain.para.justification.glyph_scaling = [scaling; 3];
        let left = moves(&plain, &with(&plain, 1, |s| s.aki_left = 250.0), &[0, 1, 2]);
        assert!(near(&left, &[0.0, 3.0, 3.0]), "Glyph Scaling {scaling}: {left:?}");
        let right = moves(&plain, &with(&plain, 1, |s| s.aki_right = 250.0), &[0, 1, 2]);
        assert!(near(&right, &[0.0, 0.0, 3.0]), "Glyph Scaling {scaling}: {right:?}");
        // 1.5 pt on each side: each side is set as 2 pt.
        let both = moves(&plain, &with(&plain, 1, |s| (s.aki_left, s.aki_right) = (150.0, 150.0)), &[0, 1, 2]);
        assert!(near(&both, &[0.0, 2.0, 4.0]), "Glyph Scaling {scaling}: {both:?}");
    }
}

#[test]
fn a_ligature_and_a_letter_with_its_mark_take_the_aki_of_their_first_character() {
    let plain = point("fit", serif(100.0));
    let glyphs = |t: &TextObject| layout(db(), t).glyphs.len();
    assert_eq!(glyphs(&plain), 2, "the font joins fi");
    let spaced = with(&plain, 0, |s| (s.aki_left, s.aki_right) = (250.0, 250.0));
    assert_eq!(glyphs(&spaced), 2, "still joined");
    let moved = moves(&plain, &spaced, &[0, 2]);
    assert!(near(&moved, &[25.0, 50.0]), "{moved:?}");
    // On the i alone it is not read: the ligature's first character is the f.
    let on_i = with(&plain, 1, |s| s.aki_right = 250.0);
    assert_eq!(glyphs(&on_i), 2);
    let moved = moves(&plain, &on_i, &[0, 2]);
    assert!(near(&moved, &[0.0, 0.0]), "{moved:?}");
    // A combining acute after e: the letter's Aki spaces the pair, the mark's own is not read.
    let marked = point("e\u{301}a", style(100.0));
    let after = "e\u{301}".len();
    let moved = moves(&marked, &with(&marked, 0, |s| s.aki_right = 250.0), &[0, after]);
    assert!(near(&moved, &[0.0, 25.0]), "{moved:?}");
    let moved = moves(&marked, &with(&marked, 1, |s| s.aki_right = 250.0), &[0, after]);
    assert!(near(&moved, &[0.0, 0.0]), "{moved:?}");
}

#[test]
fn right_to_left_aki_left_is_on_the_right_of_the_character() {
    // Reading order א ב ג is drawn right to left: Aki Left on ב widens the gap between א and ב,
    // Aki Right the gap between ב and ג.
    let fonts = FontDb::global();
    let plain = point("אבג", style(100.0));
    let (a, b, c) = (0, "א".len(), "אב".len());
    let gaps = |t: &TextObject| {
        let l = layout(fonts, t);
        (drawn_x(&l, a) - drawn_x(&l, b), drawn_x(&l, b) - drawn_x(&l, c))
    };
    let base = gaps(&plain);
    let left = gaps(&with(&plain, b, |s| s.aki_left = 250.0));
    assert!((left.0 - base.0 - 25.0).abs() < 1e-6 && (left.1 - base.1).abs() < 1e-6, "{base:?} {left:?}");
    let right = gaps(&with(&plain, b, |s| s.aki_right = 250.0));
    assert!((right.0 - base.0).abs() < 1e-6 && (right.1 - base.1 - 25.0).abs() < 1e-6, "{base:?} {right:?}");
}

#[test]
fn in_vertical_type_aki_left_is_above_the_character_and_aki_right_below() {
    // An upright glyph (§) and letters lying on their side: 250/1000 em of 20 pt is 5 pt down the
    // column.
    for text in ["§§", "ab"] {
        let mut plain = point(text, style(20.0));
        plain.xf = Affine::IDENTITY;
        plain.vertical = true;
        let centers = |t: &TextObject| layout(db(), t).glyphs.iter().map(|g| g.outline.bounding_box().center()).collect::<Vec<_>>();
        let base = centers(&plain);
        let second = edit::next_char(text, 0);
        // [first, second] glyph moved down the column, for Aki on one character.
        for (spaced, down) in [
            (with(&plain, 0, |s| s.aki_left = 250.0), [5.0, 5.0]),
            (with(&plain, 0, |s| s.aki_right = 250.0), [0.0, 5.0]),
            (with(&plain, second, |s| s.aki_left = 250.0), [0.0, 5.0]),
            (with(&plain, second, |s| s.aki_right = 250.0), [0.0, 0.0]),
        ] {
            let moved = centers(&spaced);
            for i in 0..2 {
                assert!((moved[i].y - base[i].y - down[i]).abs() < 1e-6, "{text} {i}: {base:?} {moved:?}");
                assert!((moved[i].x - base[i].x).abs() < 1e-6, "{text} {i}: {base:?} {moved:?}");
            }
        }
    }
}

/// A tate-chu-yoko block takes the Insert Aki of its first character above and below it, and
/// keeps its characters across the column as without it.
#[test]
fn a_tate_chu_yoko_block_is_spaced_by_the_aki_of_its_first_character() {
    let mut plain = point("a12b", style(20.0));
    plain.xf = Affine::IDENTITY;
    plain.vertical = true;
    let centers = |t: &TextObject| layout(db(), t).glyphs.iter().map(|g| g.outline.bounding_box().center()).collect::<Vec<_>>();
    let base = centers(&plain);
    assert_eq!(base.len(), 4);
    // [a, 1, 2, b] moved down the column: 250/1000 em of 20 pt is 5 pt.
    for (spaced, down) in [
        (with(&plain, 1, |s| s.aki_left = 250.0), [0.0, 5.0, 5.0, 5.0]),
        (with(&plain, 1, |s| s.aki_right = 250.0), [0.0, 0.0, 0.0, 5.0]),
        (with(&plain, 1, |s| (s.aki_left, s.aki_right) = (250.0, 500.0)), [0.0, 5.0, 5.0, 15.0]),
        (with(&plain, 2, |s| (s.aki_left, s.aki_right) = (250.0, 500.0)), [0.0; 4]),
    ] {
        let moved = centers(&spaced);
        for i in 0..4 {
            assert!((moved[i].y - base[i].y - down[i]).abs() < 1e-6, "{i}: {base:?} {moved:?}");
            assert!((moved[i].x - base[i].x).abs() < 1e-6, "{i}: {base:?} {moved:?}");
        }
    }
    // With Fractional Widths off, 150/1000 em of 10 pt on each side (1.5 pt) is set as 2 pt above
    // the block and 2 pt below it.
    let mut whole = point("a12b", style(10.0));
    whole.xf = Affine::IDENTITY;
    whole.vertical = true;
    whole.fractional_widths = false;
    let base = centers(&whole);
    let moved = centers(&with(&whole, 1, |s| (s.aki_left, s.aki_right) = (150.0, 150.0)));
    let down = [0.0, 2.0, 2.0, 4.0];
    for i in 0..4 {
        assert!((moved[i].y - base[i].y - down[i]).abs() < 1e-6, "{i}: {base:?} {moved:?}");
        assert!((moved[i].x - base[i].x).abs() < 1e-6, "{i}: {base:?} {moved:?}");
    }
}
