//! The space before a character that starts a line by itself (`CharStyle::kerning_before`, #5111):
//! the first character of the text, of a paragraph or after a forced line break moves along its line
//! by the value, the rest of the line follows, and the value does nothing anywhere else.
use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{CharStyle, Justify};

fn db() -> &'static FontDb {
    FontDb::global()
}

fn style(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

/// Where glyph `byte` is drawn along a horizontal line.
fn drawn_x(l: &TextLayout, byte: usize) -> f64 {
    l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.xf.as_coeffs()[4]).unwrap()
}

/// `t` with `kerning_before` set on the character at `byte`.
fn spaced(t: &TextObject, byte: usize, value: f64) -> TextObject {
    let mut t = t.clone();
    let end = edit::next_char(&t.plain_text(), byte);
    edit::style_range(&mut t.runs, byte, end, |st| st.kerning_before = value);
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

#[test]
fn the_first_character_moves_along_its_line_and_the_rest_follow() {
    let plain = TextObject::point(Point::ZERO, "HOH", style(100.0));
    // 250/1000 em of 100 pt: 25 pt, away from the line's start; a negative value pulls it back.
    let away = moves(&plain, &spaced(&plain, 0, 250.0), &[0, 1, 2]);
    assert!(near(&away, &[25.0, 25.0, 25.0]), "{away:?}");
    let back = moves(&plain, &spaced(&plain, 0, -100.0), &[0, 1, 2]);
    assert!(near(&back, &[-10.0, -10.0, -10.0]), "{back:?}");
    // The line is that much longer, so a centred line moves its first character half as far.
    let mut centred = plain.clone();
    centred.para.justify = Justify::Center;
    let half = moves(&centred, &spaced(&centred, 0, 250.0), &[0, 2]);
    assert!(near(&half, &[12.5, 12.5]), "{half:?}");
}

#[test]
fn it_does_nothing_on_a_character_inside_a_line() {
    let plain = TextObject::point(Point::ZERO, "HOH", style(100.0));
    let inside = moves(&plain, &spaced(&plain, 1, 250.0), &[0, 1, 2]);
    assert!(near(&inside, &[0.0, 0.0, 0.0]), "{inside:?}");
    // Nor on the first character of a line that wrapped in area type: the line break moves.
    let area = super::area("HOH HOH", style(50.0), Rect::new(0.0, 0.0, 180.0, 400.0), Justify::Left);
    let wrapped = layout(db(), &area);
    let second_line = wrapped.glyphs.iter().find(|g| g.byte == 4).unwrap().line;
    assert_eq!(second_line, 1, "the second HOH wraps");
    let moved = moves(&area, &spaced(&area, 4, 250.0), &[0, 4, 5]);
    assert!(near(&moved, &[0.0, 0.0, 0.0]), "{moved:?}");
}

#[test]
fn a_paragraph_and_a_forced_line_break_start_lines_by_themselves() {
    let paragraphs = TextObject::point(Point::ZERO, "HO\nHO", style(100.0));
    let moved = moves(&paragraphs, &spaced(&paragraphs, 3, 250.0), &[0, 1, 3, 4]);
    assert!(near(&moved, &[0.0, 0.0, 25.0, 25.0]), "{moved:?}");
    let forced = TextObject::point(Point::ZERO, "HO\u{2028}HO", style(100.0));
    let after = "HO\u{2028}".len();
    let moved = moves(&forced, &spaced(&forced, after, 250.0), &[0, after, after + 1]);
    assert!(near(&moved, &[0.0, 25.0, 25.0]), "{moved:?}");
    // On a line break it stays: an empty line has nothing to move.
    let empty = TextObject::point(Point::ZERO, "HO\n\nHO", style(100.0));
    let moved = moves(&empty, &spaced(&empty, 3, 250.0), &[0, 4]);
    assert!(near(&moved, &[0.0, 0.0]), "{moved:?}");
}

#[test]
fn it_takes_the_em_of_the_character_as_manual_kerning_does() {
    // Horizontal Scale 50%: half the space, as kerning after a character takes it.
    let narrow = TextObject::point(Point::ZERO, "HOH", CharStyle { h_scale: 50.0, ..style(100.0) });
    let moved = moves(&narrow, &spaced(&narrow, 0, 250.0), &[0, 2]);
    assert!(near(&moved, &[12.5, 12.5]), "{moved:?}");
}

#[test]
fn right_to_left_the_space_is_at_the_right_of_the_first_character() {
    // The first character in reading order is drawn rightmost; the space before it is on its right,
    // so the two keep their distance, and the line is 25 pt longer.
    let plain = TextObject::point(Point::ZERO, "אב", style(100.0));
    let kerned = spaced(&plain, 0, 250.0);
    let (a, b) = (layout(db(), &plain), layout(db(), &kerned));
    assert!(drawn_x(&a, 0) > drawn_x(&a, 2), "right to left");
    let gap = |l: &TextLayout| drawn_x(l, 0) - drawn_x(l, 2);
    assert!((gap(&b) - gap(&a)).abs() < 1e-6, "{} {}", gap(&a), gap(&b));
    let length = |l: &TextLayout| l.lines[0].x1 - l.lines[0].x0;
    assert!((length(&b) - length(&a) - 25.0).abs() < 1e-6, "{} {}", length(&a), length(&b));
}

#[test]
fn in_vertical_type_the_first_character_moves_down_its_column() {
    // An upright glyph (§) and letters lying on their side both move 5 pt (250/1000 em of 20 pt)
    // down the column, and stay in it.
    for text in ["§§", "ab"] {
        let mut plain = TextObject::point(Point::ZERO, text, style(20.0));
        plain.xf = Affine::IDENTITY;
        plain.vertical = true;
        let kerned = spaced(&plain, 0, 250.0);
        let (a, b) = (layout(db(), &plain), layout(db(), &kerned));
        for i in 0..2 {
            let (before, after) = (a.glyphs[i].outline.bounding_box(), b.glyphs[i].outline.bounding_box());
            assert!((after.center().y - before.center().y - 5.0).abs() < 1e-6, "{text} {i}: {before:?} {after:?}");
            assert!((after.center().x - before.center().x).abs() < 1e-6, "{text} {i}: {before:?} {after:?}");
        }
    }
}

#[test]
fn a_space_before_the_first_letter_of_a_ligature_keeps_it_whole() {
    // Set on the "f" alone, as the caret before it sets it: Source Serif 4's "fi" ligature stays one
    // glyph, and moves.
    let serif = CharStyle { font_family: "Source Serif 4".into(), ..style(100.0) };
    let plain = TextObject::point(Point::ZERO, "fit", serif);
    let glyphs = |t: &TextObject| layout(db(), t).glyphs.len();
    assert_eq!(glyphs(&plain), 2, "the font joins fi");
    let kerned = spaced(&plain, 0, 250.0);
    assert_eq!(glyphs(&kerned), 2, "still joined");
    let moved = moves(&plain, &kerned, &[0, 2]);
    assert!(near(&moved, &[25.0, 25.0]), "{moved:?}");
    assert!((line_start_space("fit", 0, &kerned.runs[0].style, false) - 25.0).abs() < 1e-9);
}
