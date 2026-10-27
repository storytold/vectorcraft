//! Inline graphics in text: a symbol's art laid out like a glyph.

use crate::*;
use kurbo::{Affine, Shape};
use vectorcraft_doc::text::INLINE_CHAR;
use vectorcraft_doc::{CharStyle, InlineArt, Justify, TextKind, TextRun};
use vectorcraft_geom::PathData;

use crate::edit;

fn db() -> &'static FontDb {
    static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
    DB.get_or_init(|| FontDb::with_font_dirs(vec![]))
}

fn style(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

/// Art 20 × 10 (2:1), as a symbol's resolved bounds.
fn art(scale: f64) -> InlineArt {
    InlineArt { scale, bounds: Some(Rect::new(-10.0, -5.0, 10.0, 5.0)), ..InlineArt::new("mana") }
}

/// Point type: `before`, the inline graphic, `after`.
fn text(before: &str, a: InlineArt, after: &str, size: f64) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, before, style(size));
    t.runs.push(TextRun::inline(a, style(size)));
    t.runs.push(TextRun::new(after, style(size)));
    t
}

fn inline_glyph(l: &TextLayout) -> &PositionedGlyph {
    l.glyphs.iter().find(|g| g.run == 1).expect("inline glyph")
}

#[test]
fn inline_art_advances_by_its_scaled_width_and_centres_on_the_cap_height() {
    let t = text("A", art(1.0), "B", 20.0);
    let l = layout(db(), &t);
    let g = inline_glyph(&l);
    // 20 pt tall → 40 pt wide (2:1), no outline of its own.
    assert!((g.advance - 40.0).abs() < 1e-6, "{}", g.advance);
    assert!(g.outline.elements().is_empty());
    assert_eq!(g.len, INLINE_CHAR.len_utf8());
    assert_eq!(l.inlines.len(), 1);
    let ig = &l.inlines[0];
    assert_eq!((ig.run, ig.byte, ig.glyph), (1, 1, l.glyphs.iter().position(|g| g.run == 1).unwrap()));
    // Left edge on the pen; vertical centre half the cap height above the baseline.
    let face = db().face("Source Sans 3", "Regular").unwrap();
    let cap = face.cap_height * 20.0 / face.upem;
    let b = ig.bounds;
    assert!((b.x0 - g.origin.x).abs() < 1e-6 && (b.width() - 40.0).abs() < 1e-6 && (b.height() - 20.0).abs() < 1e-6, "{b:?}");
    assert!((b.center().y + cap * 0.5).abs() < 1e-6, "{b:?} cap {cap}");
    // The art's transform maps its bounds there.
    let mapped = ig.xf.transform_rect_bbox(Rect::new(-10.0, -5.0, 10.0, 5.0));
    assert!((mapped.x0 - b.x0).abs() < 1e-6 && (mapped.y1 - b.y1).abs() < 1e-6);
    // `B` follows it; the layout bounds include it.
    let next = l.glyphs.iter().find(|g| g.run == 2).unwrap();
    assert!((next.origin.x - (g.origin.x + 40.0)).abs() < 1e-6);
    assert!(l.bounds.contains(b.origin()) && l.bounds.x1 >= b.x1 - 1e-9);
}

#[test]
fn shift_tracking_and_missing_symbols() {
    let up = InlineArt { baseline_shift: 4.0, ..art(1.0) };
    let a = layout(db(), &text("A", art(1.0), "B", 20.0)).inlines[0].bounds;
    let b = layout(db(), &text("A", up, "B", 20.0)).inlines[0].bounds;
    assert!((a.y0 - b.y0 - 4.0).abs() < 1e-6, "{a:?} {b:?}");
    // Tracking adds to the advance as it does after a glyph.
    let mut t = text("A", art(1.0), "B", 20.0);
    t.runs[1].style.tracking = 100.0;
    assert!((inline_glyph(&layout(db(), &t)).advance - 42.0).abs() < 1e-6);
    // Unresolved (missing symbol): a square of the height, nothing to draw.
    let t = text("A", InlineArt::new("gone"), "B", 20.0);
    let l = layout(db(), &t);
    assert!((inline_glyph(&l).advance - 20.0).abs() < 1e-6);
    assert!(l.inlines.is_empty());
    // Junk numbers stay finite.
    let t = text("A", InlineArt { scale: f64::NAN, baseline_shift: f64::INFINITY, ..art(1.0) }, "B", 20.0);
    let l = layout(db(), &t);
    assert!(l.bounds.is_finite() && l.glyphs.iter().all(|g| g.advance.is_finite()));
}

#[test]
fn tall_inline_art_opens_up_its_line() {
    let plain = layout(db(), &text("Ab", art(1.0), "cd", 20.0));
    let tall = layout(db(), &text("Ab", art(3.0), "cd", 20.0));
    let face = db().face("Source Sans 3", "Regular").unwrap();
    let cap = face.cap_height * 20.0 / face.upem;
    // At scale 1 the art stays inside the font's ascent and descent: the line doesn't change.
    let font_only = layout(db(), &TextObject::point(Point::ZERO, "Abcd", style(20.0)));
    assert!((plain.lines[0].ascent - font_only.lines[0].ascent).abs() < 1e-6);
    assert!((plain.lines[0].descent - font_only.lines[0].descent).abs() < 1e-6);
    // 60 pt tall: half above the cap-height middle, half below.
    assert!((tall.lines[0].ascent - (cap * 0.5 + 30.0)).abs() < 1e-6, "{:?}", tall.lines[0]);
    assert!((tall.lines[0].descent - (30.0 - cap * 0.5)).abs() < 1e-6);
    // A second line moves down by the taller leading (Auto: 120% of the size) only where the
    // art is; the line's band (caret, selection, hit testing) covers the art.
    let mut t = text("Ab", art(3.0), "cd\nef", 20.0);
    t.runs[1].style.leading = None;
    let l = layout(db(), &t);
    assert_eq!(l.lines.len(), 2);
    let (top, bottom) = caret_position(&l, 3);
    assert!(top.y <= l.inlines[0].bounds.y0 + 1e-6 && bottom.y >= l.inlines[0].bounds.y1 - 1e-6);
    let mid = l.inlines[0].bounds.center();
    assert_eq!(l.line_of(hit_byte(&l, mid)), 0);
}

#[test]
fn line_breaking_moves_an_inline_graphic_like_a_word() {
    // The frame is just wider than "tap tap ": the 40 pt graphic doesn't fit after it.
    let w: f64 = layout(db(), &TextObject::point(Point::ZERO, "tap tap ", style(20.0))).glyphs.iter().map(|g| g.advance).sum();
    let frame = Rect::new(0.0, 0.0, w + 10.0, 200.0);
    let area = |t: &mut TextObject| {
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        t.xf = Affine::IDENTITY;
        t.para.justify = Justify::Left;
    };
    let mut t = text("tap tap ", art(1.0), " done", 20.0);
    area(&mut t);
    let l = layout(db(), &t);
    let g = inline_glyph(&l);
    assert_eq!(g.line, 1, "the graphic wraps to the next line");
    // It starts that line, at the frame's left edge, its art with it.
    assert!((g.origin.x - l.lines[1].x0).abs() < 1e-6 && l.lines[1].x0.abs() < 1e-6);
    assert!((l.inlines[0].bounds.x0 - g.origin.x).abs() < 1e-6);
    assert!(l.inlines[0].bounds.center().y > l.lines[0].baseline);
    // No break between the graphic and punctuation stuck to it.
    let mut t2 = text("tap tap ", art(1.0), ".", 20.0);
    area(&mut t2);
    let l2 = layout(db(), &t2);
    assert_eq!(l2.glyphs.iter().find(|g| g.run == 2).unwrap().line, inline_glyph(&l2).line);
}

#[test]
fn inline_art_on_a_path_and_in_vertical_type_moves_with_the_text() {
    let mut t = text("ab", art(1.0), "cd", 20.0);
    let path = kurbo::Line::new((0.0, 0.0), (400.0, 0.0)).to_path(0.1);
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&path), start: 0.0, end: None };
    let l = layout(db(), &t);
    assert_eq!(l.inlines.len(), 1);
    let g = inline_glyph(&l);
    assert!((l.inlines[0].bounds.x0 - g.origin.x).abs() < 1e-6);
    let mut v = text("ab", art(1.0), "cd", 20.0);
    v.vertical = true;
    let l = layout(db(), &v);
    assert_eq!(l.inlines.len(), 1);
    assert!(l.bounds.contains(l.inlines[0].bounds.center()));
}

#[test]
fn snapping_moves_inline_art_with_its_glyph() {
    let mut l = layout(db(), &text("A", art(1.0), "B", 20.0));
    let before = (l.inlines[0].bounds, inline_glyph(&l).origin);
    l.snap_to_pixels(Affine::translate((0.3, 0.3)));
    let after = (l.inlines[0].bounds, inline_glyph(&l).origin);
    let d = after.1 - before.1;
    assert!(((after.0.x0 - before.0.x0) - d.x).abs() < 1e-9 && ((after.0.y0 - before.0.y0) - d.y).abs() < 1e-9);
}

#[test]
fn editing_keeps_inline_runs_whole() {
    let t = text("ab", art(1.0), "cd", 12.0);
    let mut runs = t.runs.clone();
    // Same style on both sides: the plain runs still never merge into the graphic.
    edit::normalize(&mut runs);
    assert_eq!(runs.len(), 3);
    assert!(runs[1].inline.is_some() && runs[1].text == INLINE_CHAR.to_string());
    // Typing next to it makes plain text.
    let caret = edit::replace_range(&mut runs, 5, 5, "X");
    assert_eq!(caret, 6);
    assert_eq!(runs.len(), 3);
    assert_eq!(runs[2].text, "Xcd");
    let caret = edit::replace_range(&mut runs, 2, 2, "Y");
    assert_eq!((caret, runs[0].text.as_str()), (3, "abY"));
    // An offset inside its character lands before it: nothing splits it.
    let plain: String = runs.iter().map(|r| r.text.as_str()).collect();
    let at = plain.find(INLINE_CHAR).unwrap();
    edit::replace_range(&mut runs, at + 1, at + 1, "Z");
    assert!(runs.iter().any(|r| r.inline.is_some() && r.text == INLINE_CHAR.to_string()));
    // Backspace over it (its one character) removes the whole graphic.
    let plain: String = runs.iter().map(|r| r.text.as_str()).collect();
    let at = plain.find(INLINE_CHAR).unwrap();
    let prev = edit::prev_char(&plain, at + INLINE_CHAR.len_utf8());
    assert_eq!(prev, at);
    edit::replace_range(&mut runs, prev, at + INLINE_CHAR.len_utf8(), "");
    assert!(runs.iter().all(|r| r.inline.is_none()));
    assert!(!runs.iter().any(|r| r.text.contains(INLINE_CHAR)));
}

#[test]
fn copying_keeps_the_graphic_and_plain_text_shows_the_replacement_character() {
    let t = text("ab", art(2.0), "cd", 12.0);
    let copy = edit::slice_runs(&t.runs, 1, 6);
    let plain: String = copy.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(plain, format!("b{INLINE_CHAR}c"));
    assert_eq!(copy[1].inline.as_ref().map(|a| a.scale), Some(2.0));
    // Half of its character is nothing (no empty graphic runs).
    let half = edit::slice_runs(&t.runs, 2, 3);
    assert!(half.is_empty(), "{half:?}");
    // Pasting it back keeps it.
    let mut runs = t.runs.clone();
    edit::replace_range_styled(&mut runs, 0, 0, &copy);
    assert_eq!(runs.iter().filter(|r| r.inline.is_some()).count(), 2);
}

#[test]
fn malformed_inline_runs_are_plain_text() {
    let mut runs = vec![TextRun { text: "abc".into(), style: style(12.0), inline: Some(art(1.0)) }];
    edit::normalize(&mut runs);
    assert!(runs[0].inline.is_none());
    let mut t = TextObject::point(Point::ZERO, "", style(12.0));
    t.runs = vec![TextRun { text: format!("{INLINE_CHAR}{INLINE_CHAR}"), style: style(12.0), inline: Some(art(1.0)) }];
    let l = layout(db(), &t);
    assert!(l.inlines.is_empty());
}

/// An inline graphic is an object replacement character (U+FFFC, a bidi neutral): in right-to-left
/// text it takes the direction of the text around it and the line's reordering places it, between
/// two Hebrew words in reading order (right to left); between two English words set right to left
/// it stays between them.
#[test]
fn inline_art_in_a_right_to_left_paragraph_is_placed_by_the_bidi_reordering() {
    use vectorcraft_doc::ParaDirection;
    let db = FontDb::global();
    let order = |before: &str, after: &str, direction: Option<ParaDirection>| {
        let mut t = text(before, art(1.0), after, 20.0);
        (t.para.justify, t.para.direction) = (Justify::Auto, direction);
        let l = layout(db, &t);
        let x = |run: usize| l.glyphs.iter().filter(|g| g.run == run).map(|g| g.origin.x).fold(f64::NAN, f64::min);
        let g = inline_glyph(&l).clone();
        assert_eq!(l.inlines.len(), 1);
        // The art sits at its glyph, inside the line.
        assert!((l.inlines[0].bounds.x0 - g.origin.x).abs() < 1e-6, "{:?} {g:?}", l.inlines[0]);
        (x(0), g.origin.x, x(2), g.rtl, l.lines[0].rtl)
    };
    // Hebrew: the first word on the right, the graphic left of it, the second word further left.
    let (first, inline, second, inline_rtl, line_rtl) = order("שלום ", " עולם", None);
    assert!(line_rtl && inline_rtl);
    assert!(second < inline && inline < first, "{second} < {inline} < {first}");
    // English set right to left: the words keep their order with the graphic between them.
    let (first, inline, second, inline_rtl, line_rtl) = order("Hello", "world", Some(ParaDirection::RightToLeft));
    assert!(line_rtl && !inline_rtl);
    assert!(first < inline && inline < second, "{first} < {inline} < {second}");
    // At the end of a right-to-left paragraph it is the leftmost thing on the line.
    let (first, inline, _, inline_rtl, _) = order("שלום", "", None);
    assert!(inline_rtl && inline < first);
}

/// Character Alignment: an inline graphic's em box is its run's, whatever its scale. It moves with
/// its run's text when that is smaller than the line's largest characters, and a big graphic in
/// small text doesn't make its line's small characters move.
#[test]
fn character_alignment_treats_inline_art_as_a_character_of_its_run_size() {
    use vectorcraft_doc::CharAlign;
    let centre_y = |a: CharAlign, scale: f64, big: f64| {
        let mut t = TextObject::point(Point::ZERO, "M", style(big));
        t.runs.push(TextRun::inline(art(scale), CharStyle { char_align: a, ..style(20.0) }));
        t.runs.push(TextRun::new("x", CharStyle { char_align: a, ..style(20.0) }));
        let l = layout(db(), &t);
        let x = l.glyphs.iter().find(|g| g.run == 2).map(|g| g.origin.y).unwrap();
        (l.inlines[0].bounds.center().y, x)
    };
    let face = db().face("Source Sans 3", "Regular").unwrap();
    let c = face.ideographic_centre();
    // Next to a 40 pt character, the 20 pt run (graphic and letter alike) moves up by the same.
    let (roman, roman_x) = centre_y(CharAlign::RomanBaseline, 1.0, 40.0);
    for (a, k) in [(CharAlign::EmBoxTop, 0.5), (CharAlign::EmBoxCenter, 0.0), (CharAlign::EmBoxBottom, -0.5)] {
        let (y, x) = centre_y(a, 1.0, 40.0);
        let want = (c + k) * 20.0;
        assert!((roman - y - want).abs() < 1e-6, "{a:?}: {} vs {want}", roman - y);
        assert!(((roman_x - x) - (roman - y)).abs() < 1e-6, "{a:?}: the graphic moves with its run's letters");
    }
    // A 60 pt tall graphic in a line of 20 pt text: the line's largest em is still 20 pt, so
    // nothing moves.
    let (roman, roman_x) = centre_y(CharAlign::RomanBaseline, 3.0, 20.0);
    let (top, top_x) = centre_y(CharAlign::EmBoxTop, 3.0, 20.0);
    assert!((roman - top).abs() < 1e-6 && (roman_x - top_x).abs() < 1e-6);
}

/// Top-to-Top leading measures from the em box tops: an inline graphic's is its run's, so a line
/// with a tall graphic is spaced like its text (as Roman leading spaces it by its run's leading).
#[test]
fn top_to_top_leading_spaces_inline_art_by_its_run() {
    use vectorcraft_doc::LeadingModel;
    let lines = |scale: f64| {
        let mut t = text("Ab", art(scale), "cd\nef", 20.0);
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 400.0, 400.0).to_path(0.1)) };
        t.para.leading_model = LeadingModel::EmBoxTop;
        let l = layout(db(), &t);
        assert_eq!(l.lines.len(), 2);
        (l.lines[0].baseline, l.lines[1].baseline)
    };
    let (small0, small1) = lines(1.0);
    let (tall0, tall1) = lines(3.0);
    assert!((small0 - tall0).abs() < 1e-6 && (small1 - tall1).abs() < 1e-6, "{small0} {small1} / {tall0} {tall1}");
}

#[test]
fn inline_art_moves_with_vertically_aligned_lines() {
    use vectorcraft_doc::text::{AreaOptions, VerticalAlign};
    // A rectangle (lines shift) and a triangle (text flows again lower down).
    let rect = Rect::new(0.0, 0.0, 400.0, 400.0).to_path(0.1);
    let mut tri = kurbo::BezPath::new();
    tri.move_to((0.0, 0.0));
    tri.line_to((800.0, 0.0));
    tri.line_to((400.0, 400.0));
    tri.close_path();
    for frame in [rect, tri] {
        for align in [VerticalAlign::Center, VerticalAlign::Bottom] {
            let mut t = text("A", art(1.0), "B", 20.0);
            t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame) };
            t.area = AreaOptions { vertical_align: align, ..AreaOptions::default() };
            let l = layout(db(), &t);
            assert_eq!(l.inlines.len(), 1, "{align:?}");
            let g = inline_glyph(&l);
            let line = &l.lines[g.line];
            assert!(line.baseline > 100.0, "{align:?}: not aligned, baseline {}", line.baseline);
            // Still centred on the cap height above its (moved) baseline.
            let face = db().face("Source Sans 3", "Regular").unwrap();
            let cap = face.cap_height * 20.0 / face.upem;
            let b = l.inlines[0].bounds;
            assert!((b.center().y - (line.baseline - cap * 0.5)).abs() < 1e-6, "{align:?}: {b:?} baseline {}", line.baseline);
            assert!((b.x0 - g.origin.x).abs() < 1e-6, "{align:?}: {b:?}");
        }
    }
}
