//! Tsume (`CharStyle::tsume`): the share of the blank space on each side of a character's ink,
//! inside its own width, that horizontal type takes away, the glyph keeping its shape.
use super::{area, db, point, style};
use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{CharStyle, Justify};

/// Where glyph `byte` is drawn along a horizontal line.
fn drawn_x(l: &TextLayout, byte: usize) -> f64 {
    l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.xf.as_coeffs()[4]).unwrap()
}

/// The left and right edges of glyph `byte`'s ink.
fn ink(l: &TextLayout, byte: usize) -> (f64, f64) {
    let b = l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.outline.bounding_box()).unwrap();
    (b.x0, b.x1)
}

/// `t` with `f` applied to the style of the character at `byte`.
fn with(t: &TextObject, byte: usize, f: impl Fn(&mut CharStyle)) -> TextObject {
    let mut t = t.clone();
    let end = edit::next_char(&t.plain_text(), byte);
    edit::style_range(&mut t.runs, byte, end, f);
    t
}

/// How far each of `bytes` moved along the line from `a` to `b`.
fn moves(a: &TextObject, b: &TextObject, bytes: &[usize]) -> Vec<f64> {
    let (la, lb) = (layout(db(), a), layout(db(), b));
    bytes.iter().map(|&i| drawn_x(&lb, i) - drawn_x(&la, i)).collect()
}

fn near(values: &[f64], expected: &[f64]) -> bool {
    values.len() == expected.len() && values.iter().zip(expected).all(|(v, e)| (v - e).abs() < 1e-6)
}

/// The blank space before and after the O of "HOH" in `t`'s layout: between the start of its
/// width and its ink, and between its ink and the next character.
fn o_bearings(t: &TextObject) -> (f64, f64) {
    let l = layout(db(), t);
    let (x0, x1) = ink(&l, 1);
    (x0 - drawn_x(&l, 1), drawn_x(&l, 2) - x1)
}

#[test]
fn tsume_takes_its_share_of_the_space_beside_the_ink() {
    let plain = point("HOH", style(100.0));
    let (before, after) = o_bearings(&plain);
    assert!(before > 1.0 && after > 1.0, "{before} {after}");
    let plain_layout = layout(db(), &plain);
    let width = ink(&plain_layout, 1).1 - ink(&plain_layout, 1).0;
    for tsume in [25.0, 50.0, 100.0] {
        let tight = with(&plain, 1, |s| s.tsume = tsume);
        let share = tsume / 100.0;
        // The O's ink moves back by its share of the space before it; the H after it also moves
        // back by the share of the space after the O.
        let l = layout(db(), &tight);
        let (x0, x1) = ink(&l, 1);
        let moved = (x0 - ink(&plain_layout, 1).0, ink(&l, 2).0 - ink(&plain_layout, 2).0);
        assert!((moved.0 + before * share).abs() < 1e-6, "{tsume}: {moved:?}");
        assert!((moved.1 + (before + after) * share).abs() < 1e-6, "{tsume}: {moved:?}");
        // The glyph keeps its shape, and the H before it stays.
        assert!((x1 - x0 - width).abs() < 1e-6, "{tsume}: {} {width}", x1 - x0);
        assert!((ink(&l, 0).0 - ink(&plain_layout, 0).0).abs() < 1e-9);
    }
    // At 100% the O's ink starts where the H before it ends its width, and the next H starts its
    // width where the O's ink ends.
    let l = layout(db(), &with(&plain, 1, |s| s.tsume = 100.0));
    let cell_start = drawn_x(&plain_layout, 1);
    assert!((ink(&l, 1).0 - cell_start).abs() < 1e-6 && (drawn_x(&l, 2) - ink(&l, 1).1).abs() < 1e-6, "{:?} {cell_start}", ink(&l, 1));
}

#[test]
fn zero_out_of_range_and_spaces_keep_the_layout() {
    let plain = point("H O", style(100.0));
    let base = layout(db(), &plain);
    for (tsume, at) in [(0.0, 2), (f64::NAN, 2), (-50.0, 2), (100.0, 1)] {
        // 0, a value that is not a number, a negative value, and a space at 100%.
        let l = layout(db(), &with(&plain, at, |s| s.tsume = tsume));
        let xs: Vec<f64> = l.glyphs.iter().map(|g| g.xf.as_coeffs()[4]).collect();
        let base_xs: Vec<f64> = base.glyphs.iter().map(|g| g.xf.as_coeffs()[4]).collect();
        assert_eq!(xs, base_xs, "{tsume} at {at}");
    }
    // A value above 100 takes all the space, as 100 does.
    let most = moves(&plain, &with(&plain, 2, |s| s.tsume = 500.0), &[2]);
    let full = moves(&plain, &with(&plain, 2, |s| s.tsume = 100.0), &[2]);
    assert!(near(&most, &full) && full[0] < -1.0, "{most:?} {full:?}");
}

#[test]
fn tracking_and_insert_aki_are_added_after_the_tsume() {
    let plain = point("HOH", style(100.0));
    let tight = moves(&plain, &with(&plain, 1, |s| s.tsume = 50.0), &[0, 1, 2]);
    let spaced = moves(&plain, &with(&plain, 1, |s| (s.tsume, s.aki_left, s.tracking) = (50.0, 250.0, 100.0)), &[0, 1, 2]);
    // 25 pt of Aki before the O, and 10 pt of tracking after it, on top of the Tsume.
    assert!(tight[1] < -1.0 && tight[2] < tight[1], "{tight:?}");
    assert!(near(&spaced, &[0.0, tight[1] + 25.0, tight[2] + 35.0]), "{tight:?} {spaced:?}");
}

#[test]
fn the_font_kerning_is_added_after_the_tsume() {
    // An L, with blank space after its ink, before a letter the font kerns it with: its Tsume is
    // the same as before an H, which the font does not kern it with.
    let lh = point("LH", style(100.0));
    let kerned = ["LT", "LV", "LY", "LW"]
        .map(|s| point(s, style(100.0)))
        .into_iter()
        .find(|t| drawn_x(&layout(db(), t), 1) < drawn_x(&layout(db(), &lh), 1) - 1.0);
    let Some(kerned) = kerned else { panic!("the test font kerns none of the pairs") };
    let before_kerned = moves(&kerned, &with(&kerned, 0, |s| s.tsume = 100.0), &[1]);
    let before_h = moves(&lh, &with(&lh, 0, |s| s.tsume = 100.0), &[1]);
    assert!(near(&before_kerned, &before_h) && before_h[0] < -10.0, "{before_kerned:?} {before_h:?}");
}

#[test]
fn a_letter_with_its_mark_takes_the_tsume_of_the_letter() {
    let plain = point("H\u{301}O", style(100.0));
    let after = "H\u{301}".len();
    let letter = moves(&plain, &with(&plain, 0, |s| s.tsume = 100.0), &[after]);
    let mark = moves(&plain, &with(&plain, 1, |s| s.tsume = 100.0), &[after]);
    assert!(letter[0] < -1.0 && near(&mark, &[0.0]), "{letter:?} {mark:?}");
}

#[test]
fn justification_keeps_the_tsume() {
    let plain = area("HOH HOH", style(20.0), Rect::new(0.0, 0.0, 400.0, 400.0), Justify::JustifyAll);
    let tight = with(&plain, 1, |s| s.tsume = 100.0);
    let (lp, lt) = (layout(db(), &plain), layout(db(), &tight));
    // The O stays nearer the H before it by the space Tsume took before its ink.
    let gap = |l: &TextLayout| ink(l, 1).0 - ink(l, 0).1;
    assert!(gap(&lt) < gap(&lp) - 1.0, "{} {}", gap(&lp), gap(&lt));
}

#[test]
fn vertical_type_keeps_its_spacing() {
    for text in ["§§", "ab"] {
        let mut plain = point(text, style(20.0));
        plain.xf = Affine::IDENTITY;
        plain.vertical = true;
        let centers = |t: &TextObject| layout(db(), t).glyphs.iter().map(|g| g.outline.bounding_box().center()).collect::<Vec<_>>();
        assert_eq!(centers(&with(&plain, 0, |s| s.tsume = 100.0)), centers(&plain), "{text}");
    }
}
