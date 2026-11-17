//! Tsume (`CharStyle::tsume`): the share of the blank space on each side of a character's ink,
//! inside its own width (in vertical type, down an upright character's cell), that is taken away,
//! the glyph keeping its shape.
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
fn optical_kerning_keeps_the_tsume() {
    // Optical kerning spaces each pair as without the Tsume, and the Tsume stays on top: the
    // characters move back as far with Optical kerning as without it.
    let plain = point("HOH", style(100.0));
    let optical = point("HOH", CharStyle { optical_kerning: true, ..style(100.0) });
    let xs = |t: &TextObject| {
        let l = layout(db(), t);
        [0, 1, 2].map(|i| drawn_x(&l, i))
    };
    // The test font's Optical spacing differs from its own for these pairs.
    assert!(!near(&xs(&optical), &xs(&plain)), "{:?} {:?}", xs(&optical), xs(&plain));
    for at in [0, 1, 2] {
        let without = moves(&plain, &with(&plain, at, |s| s.tsume = 100.0), &[0, 1, 2]);
        let with_optical = moves(&optical, &with(&optical, at, |s| s.tsume = 100.0), &[0, 1, 2]);
        assert!(without.iter().any(|m| *m < -1.0) && near(&with_optical, &without), "{at}: {without:?} {with_optical:?}");
    }
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

fn vertical(text: &str, st: CharStyle) -> TextObject {
    let mut t = point(text, st);
    t.xf = Affine::IDENTITY;
    t.vertical = true;
    t
}

/// The box of what glyph `byte` paints: its outline, its bitmap or its color art.
fn painted(l: &TextLayout, byte: usize) -> Rect {
    let g = l.glyphs.iter().find(|g| g.byte == byte).unwrap();
    let image = g.placed_image().map(|(image, xf)| xf.transform_rect_bbox(image.rect()));
    let colour = g.colour.as_ref().and_then(|c| c.geometric_bounds()).map(|b| g.xf.transform_rect_bbox(b));
    image.or(colour).unwrap_or_else(|| g.outline.bounding_box())
}

/// Where glyph `byte`'s cell starts down a vertical column.
fn cell_top(l: &TextLayout, byte: usize) -> f64 {
    l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.origin.y).unwrap()
}

#[test]
fn upright_glyphs_take_their_share_of_the_space_above_and_below_their_ink() {
    use crate::test_fonts::{VERTICAL_FAMILY, vertical_font};
    // The bundled font without vertical metrics (a cell of one em), and with them: § 1.4 em down
    // the column from its vertical origin, given by VORG or by its top side bearing.
    for vorg in [None, Some(false), Some(true)] {
        let fonts = FontDb::with_font_dirs(vec![]);
        let mut st = style(20.0);
        if let Some(vorg) = vorg {
            fonts.add_font(vertical_font(vorg).unwrap());
            st.font_family = VERTICAL_FAMILY.into();
        }
        let plain = vertical("§§§", st.clone());
        let lp = layout(&fonts, &plain);
        let (before, after) = (painted(&lp, 2).y0 - cell_top(&lp, 2), cell_top(&lp, 4) - painted(&lp, 2).y1);
        assert!(before > 1.0 && after > 1.0, "{vorg:?}: {before} {after}");
        for tsume in [25.0, 100.0] {
            let share = tsume / 100.0;
            let lt = layout(&fonts, &with(&plain, 2, |s| s.tsume = tsume));
            // The middle § moves up by its share of the space above its ink, and the § after it by
            // the share of the space above and below it; the first stays, and the glyph keeps its
            // shape.
            let moved = (painted(&lt, 2).y0 - painted(&lp, 2).y0, painted(&lt, 4).y0 - painted(&lp, 4).y0);
            assert!((moved.0 + before * share).abs() < 1e-6, "{vorg:?} {tsume}: {moved:?}");
            assert!((moved.1 + (before + after) * share).abs() < 1e-6, "{vorg:?} {tsume}: {moved:?}");
            assert!((painted(&lt, 2).height() - painted(&lp, 2).height()).abs() < 1e-6);
            assert!((painted(&lt, 2).center().x - painted(&lp, 2).center().x).abs() < 1e-6, "it stays on the column's center line");
            assert!((painted(&lt, 0).y0 - painted(&lp, 0).y0).abs() < 1e-9);
        }
        // At 100% the ink runs edge to edge down the column.
        let lt = layout(&fonts, &with(&plain, 2, |s| s.tsume = 100.0));
        assert!((painted(&lt, 2).y0 - cell_top(&lp, 2)).abs() < 1e-6 && (cell_top(&lt, 4) - painted(&lt, 2).y1).abs() < 1e-6, "{vorg:?}");
        // A space keeps its length down the column.
        let spaced = vertical("§ §", st);
        let tops = |t: &TextObject| layout(&fonts, t).glyphs.iter().map(|g| g.origin.y).collect::<Vec<_>>();
        assert_eq!(tops(&with(&spaced, 2, |s| s.tsume = 100.0)), tops(&spaced), "{vorg:?}");
    }
}

#[test]
fn letters_on_their_side_take_the_tsume_of_horizontal_type() {
    // In vertical type a letter lies on its side: its space before and after its ink runs down
    // the column, as far as it runs along a horizontal line.
    let (across, down) = (point("HOH", style(20.0)), vertical("HOH", style(20.0)));
    let moves_along = |t: &TextObject, vertical: bool| {
        let (lp, lt) = (layout(db(), t), layout(db(), &with(t, 1, |s| s.tsume = 100.0)));
        [1, 2].map(|i| if vertical { painted(&lt, i).y0 - painted(&lp, i).y0 } else { painted(&lt, i).x0 - painted(&lp, i).x0 })
    };
    let (h, v) = (moves_along(&across, false), moves_along(&down, true));
    assert!(h[1] < -1.0 && near(&v, &h), "{h:?} {v:?}");
}

#[test]
fn tsume_inside_a_tate_chu_yoko_block_keeps_its_cell() {
    // Two digits are set across the column as one block one em long; Tsume closes them up across
    // it, and the letter after the block keeps its place.
    let plain = vertical("H12H", style(20.0));
    let tight = with(&with(&plain, 1, |s| s.tsume = 100.0), 2, |s| s.tsume = 100.0);
    let (lp, lt) = (layout(db(), &plain), layout(db(), &tight));
    let side_by_side = (painted(&lp, 1).center().y - painted(&lp, 2).center().y).abs();
    assert!(side_by_side < 1.0, "the digits stand side by side across the column: {side_by_side}");
    let apart = |l: &TextLayout| (painted(l, 2).center().x - painted(l, 1).center().x).abs();
    assert!(apart(&lt) < apart(&lp) - 1.0, "{} {}", apart(&lp), apart(&lt));
    assert!((painted(&lt, 3).y0 - painted(&lp, 3).y0).abs() < 1e-9);
    assert!((painted(&lt, 1).width() - painted(&lp, 1).width()).abs() < 1e-6, "the digits keep their size");
}

#[test]
fn upright_glyphs_on_a_vertical_path_take_the_tsume_of_point_type() {
    use vectorcraft_doc::TextKind;
    use vectorcraft_geom::PathData;
    let point_type = vertical("§§§", style(20.0));
    let mut on_path = point_type.clone();
    on_path.kind =
        TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (0.0, 3000.0)).to_path(0.1)), start: 0.0, end: None };
    let moved = |t: &TextObject| {
        let (lp, lt) = (layout(db(), t), layout(db(), &with(t, 2, |s| s.tsume = 100.0)));
        [2, 4].map(|i| painted(&lt, i).y0 - painted(&lp, i).y0)
    };
    let (p, path) = (moved(&point_type), moved(&on_path));
    assert!(p[1] < -1.0 && near(&path, &p), "{p:?} {path:?}");
}

#[test]
fn bitmap_and_color_glyphs_measure_what_they_paint() {
    use crate::test_fonts::{COLOUR_CHAR, colour_bitmap_family, colour_bitmap_font, colr_family, colr_font};
    // Each font keeps the base outline of its color glyph; the image or art drawn in its place is
    // what Tsume measures.
    let fonts = [
        (colour_bitmap_font(true), colour_bitmap_family(true)),
        (colour_bitmap_font(false), colour_bitmap_family(false)),
        (colr_font(false), colr_family(false)),
        (colr_font(true), colr_family(true)),
    ];
    for (font, family) in fonts {
        let fonts = FontDb::with_font_dirs(vec![]);
        fonts.add_font(font.expect("the test font builds"));
        let plain = point(&format!("{COLOUR_CHAR}H"), CharStyle { font_family: family.into(), ..style(100.0) });
        let lp = layout(&fonts, &plain);
        let g = &lp.glyphs[0];
        assert!(g.image.is_some() || g.colour.is_some(), "{family}");
        let (start, end) = (g.origin.x, lp.glyphs[1].origin.x);
        let p = painted(&lp, 0);
        let width = end - start;
        let (left, right) = ((p.x0 - start).clamp(0.0, width), (end - p.x1).clamp(0.0, width));
        let lt = layout(&fonts, &with(&plain, 0, |s| s.tsume = 100.0));
        let moved = (painted(&lt, 0).x0 - p.x0, lt.glyphs[1].origin.x - end);
        assert!((moved.0 + left).abs() < 1e-6 && (moved.1 + left + right).abs() < 1e-6, "{family}: {moved:?} {left} {right}");
        assert!((painted(&lt, 0).width() - p.width()).abs() < 1e-6, "{family}");
    }
}
