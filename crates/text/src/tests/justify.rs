//! Paragraph panel menu › Justification…: word spacing, letter spacing, glyph scaling, Single Word
//! Justification and Auto Leading in layout.

use crate::*;
use kurbo::{Affine, Shape};
use vectorcraft_doc::text::Kashida;
use vectorcraft_doc::{CharStyle, JustificationOptions, Justify, SingleWordJustify, TabAlign, TabStop, TextKind, TextRun};
use vectorcraft_geom::PathData;

fn db() -> &'static FontDb {
    FontDb::global()
}

fn style(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

fn area(text: &str, width: f64, justify: Justify, j: JustificationOptions) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style(20.0));
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, width, 1000.0).to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    t.para.justify = justify;
    t.para.justification = j;
    t
}

fn point(text: &str, j: JustificationOptions) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style(20.0));
    t.xf = Affine::IDENTITY;
    t.para.justification = j;
    t
}

/// The glyph of the character at byte `b`.
fn at(l: &TextLayout, b: usize) -> &PositionedGlyph {
    l.glyphs.iter().find(|g| g.byte == b).unwrap()
}

fn width(g: &PositionedGlyph) -> f64 {
    g.outline.bounding_box().width()
}

/// Desired values apply to unjustified type: word spaces are that share of the font's space,
/// letters are followed by that share of a space, and glyphs are drawn that wide.
#[test]
fn desired_spacing_and_scaling_apply_to_every_line() {
    let plain = layout(db(), &point("ab cd", JustificationOptions::default()));
    let space = at(&plain, 2).advance;
    let a = at(&plain, 0).advance;
    assert!(space > 0.0 && a > 0.0);

    let words = layout(db(), &point("ab cd", JustificationOptions { word_spacing: [0.0, 200.0, 300.0], ..JustificationOptions::default() }));
    assert!((at(&words, 2).advance - 2.0 * space).abs() < 1e-6, "a word space at 200%");
    assert!((at(&words, 0).advance - a).abs() < 1e-6, "letters unchanged");

    let letters = layout(db(), &point("ab cd", JustificationOptions { letter_spacing: [0.0, 50.0, 50.0], ..JustificationOptions::default() }));
    assert!((at(&letters, 0).advance - (a + 0.5 * space)).abs() < 1e-6, "half a space after a");
    assert!((at(&letters, 1).advance - at(&plain, 1).advance).abs() < 1e-6, "no letter spacing before a word space");
    assert!((at(&letters, 4).advance - at(&plain, 4).advance).abs() < 1e-6, "nor after the last letter");

    let scaled = layout(db(), &point("ab cd", JustificationOptions { glyph_scaling: [50.0, 150.0, 200.0], ..JustificationOptions::default() }));
    assert!((at(&scaled, 0).advance - 1.5 * a).abs() < 1e-6, "advance at 150%");
    assert!((width(at(&scaled, 0)) - 1.5 * width(at(&plain, 0))).abs() < 1e-6, "outline at 150%");
    assert!((at(&scaled, 2).advance - space).abs() < 1e-6, "word spaces aren't scaled");
}

/// The line's text from a layout, one string per line.
fn lines(l: &TextLayout, text: &str) -> Vec<String> {
    l.lines.iter().map(|li| text.get(li.start..li.end).unwrap_or_default().trim_end().to_string()).collect()
}

/// A justified line fills its measure with word spacing first, then letter spacing, then glyph
/// scaling, each within its range; past them the word spaces take the rest.
#[test]
fn justified_lines_use_word_then_letter_spacing_then_glyph_scaling() {
    let text = "ab cd ef gh";
    let natural = layout(db(), &point("ab cd", JustificationOptions::default()));
    let space = at(&natural, 2).advance;
    let full = natural.lines[0].x1 - natural.lines[0].x0;
    // The four letters' width.
    let letters_w = full - space;
    // "ab cd" on the first line, `room` points short of the measure.
    let line = |room: f64, j: JustificationOptions| {
        let w = full + room;
        let l = layout(db(), &area(text, w, Justify::JustifyLeft, j));
        assert_eq!(lines(&l, text).first().map(String::as_str), Some("ab cd"), "{room}");
        l
    };
    let fixed = JustificationOptions { word_spacing: [100.0, 100.0, 100.0], ..JustificationOptions::default() };

    // Word spacing within its range: only the space grows.
    let l = line(space * 0.2, JustificationOptions { word_spacing: [80.0, 100.0, 150.0], letter_spacing: [0.0, 0.0, 10.0], ..fixed.clone() });
    assert!((at(&l, 2).advance - space * 1.2).abs() < 1e-6);
    assert!((at(&l, 0).advance - at(&natural, 0).advance).abs() < 1e-6);

    // No word spacing range: the letter gaps (a–b and c–d) share the room.
    let l = line(4.0, JustificationOptions { letter_spacing: [0.0, 0.0, 50.0], ..fixed.clone() });
    assert!((at(&l, 2).advance - space).abs() < 1e-6, "the space keeps its width");
    assert!((at(&l, 0).advance - (at(&natural, 0).advance + 2.0)).abs() < 1e-6);
    assert!((at(&l, 1).advance - at(&natural, 1).advance).abs() < 1e-6, "no gap before the space");

    // Only glyph scaling: the four letters widen, their outlines with them.
    let l = line(letters_w * 0.05, JustificationOptions { glyph_scaling: [90.0, 100.0, 110.0], ..fixed.clone() });
    assert!((at(&l, 0).advance - at(&natural, 0).advance * 1.05).abs() < 1e-6, "half of the range");
    assert!((width(at(&l, 0)) - width(at(&natural, 0)) * 1.05).abs() < 1e-6);
    let end = at(&l, 4).origin.x + at(&l, 4).advance;
    assert!((end - (full + letters_w * 0.05)).abs() < 1e-6, "the line reaches the edge: {end}");

    // Every range used up: the word space takes the rest.
    let l = line(10.0, JustificationOptions { letter_spacing: [0.0, 0.0, 10.0], ..fixed });
    let letters = 2.0 * 0.1 * space;
    assert!((at(&l, 2).advance - (space + 10.0 - letters)).abs() < 1e-6);
}

/// Full Justify spreads a right-to-left word between its letters as they are drawn: the word runs
/// from the left edge to the right edge, with the same gap between each two letters.
#[test]
fn a_justified_right_to_left_word_is_spread_between_its_drawn_letters() {
    let text = "שלום";
    let natural = layout(db(), &point(text, JustificationOptions::default()));
    let l = layout(db(), &area(text, 300.0, Justify::JustifyAll, JustificationOptions::default()));
    // The letters from left to right: ם, ו, ל, ש.
    let mut drawn: Vec<&PositionedGlyph> = l.glyphs.iter().collect();
    drawn.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    assert!(drawn.len() == 4 && drawn.iter().all(|g| g.rtl), "{drawn:?}");
    let gaps: Vec<f64> = drawn.windows(2).map(|p| p[1].origin.x - p[0].origin.x - at(&natural, p[0].byte).advance).collect();
    let last = drawn[3];
    assert!(drawn[0].origin.x.abs() < 1e-6, "{:?}", drawn[0].origin);
    assert!((last.origin.x + at(&natural, last.byte).advance - 300.0).abs() < 1e-6, "{:?}", last.origin);
    assert!(gaps.iter().all(|&g| g > 1.0 && (g - gaps[0]).abs() < 1e-6), "{gaps:?}");
}

/// A word with a combining mark that has no precomposed form, `x` + U+0301: the base and the mark
/// are two glyphs of one shaped cluster at byte 1, between `a` (byte 0) and `z` (byte 4).
const MARKED: &str = "ax\u{301}z";

/// Where each glyph of the cluster at byte `b` sits from the cluster's first glyph.
fn cluster_offsets(l: &TextLayout, b: usize) -> Vec<Vec2> {
    let glyphs: Vec<&PositionedGlyph> = l.glyphs.iter().filter(|g| g.byte == b).collect();
    glyphs.iter().map(|g| g.origin - glyphs[0].origin).collect()
}

fn assert_same_offsets(a: &[Vec2], b: &[Vec2], what: &str) {
    assert!(a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (*p - *q).hypot() < 1e-6), "{what}: {a:?} vs {b:?}");
}

/// The precondition of the cluster tests: `MARKED` shapes the base and its mark as two glyphs
/// with one byte.
fn marked_cluster(l: &TextLayout) -> Vec<Vec2> {
    let offsets = cluster_offsets(l, 1);
    assert_eq!(offsets.len(), 2, "x and its mark are one cluster: {:?}", l.glyphs);
    offsets
}

/// The space Full Justify put between two clusters, from `a`'s first glyph to `b`'s, beyond the
/// distance the unspaced `natural` layout has between them.
fn added_gap(l: &TextLayout, natural: &TextLayout, a: usize, b: usize) -> f64 {
    let d = |l: &TextLayout| (at(l, b).origin - at(l, a).origin).hypot();
    d(l) - d(natural)
}

/// Full Justify spreads a single word between its clusters, never between a letter and its
/// combining mark: the mark keeps its place on its letter, the two gaps are equal, and the word
/// still reaches the measure.
#[test]
fn single_word_full_justify_keeps_combining_mark_with_base() {
    let natural = layout(db(), &point(MARKED, JustificationOptions::default()));
    let unspaced = marked_cluster(&natural);
    let l = layout(db(), &area(MARKED, 300.0, Justify::JustifyAll, JustificationOptions::default()));
    assert_same_offsets(&cluster_offsets(&l, 1), &unspaced, "the mark stays on its letter");
    let (first, second) = (added_gap(&l, &natural, 0, 1), added_gap(&l, &natural, 1, 4));
    assert!(first > 1.0 && (first - second).abs() < 1e-6, "{first} {second}");
    let z = at(&l, 4);
    assert!((z.origin.x + at(&natural, 4).advance - 300.0).abs() < 1e-6, "{:?}", z.origin);
}

/// Desired Letter Spacing on a ragged line, and the letter spacing range of a justified one, go
/// after each cluster, not between a letter and its mark.
#[test]
fn desired_and_fitted_letter_spacing_skip_shaped_cluster() {
    let natural = layout(db(), &point(MARKED, JustificationOptions::default()));
    let unspaced = marked_cluster(&natural);

    let desired = layout(db(), &point(MARKED, JustificationOptions { letter_spacing: [0.0, 50.0, 50.0], ..JustificationOptions::default() }));
    assert_same_offsets(&cluster_offsets(&desired, 1), &unspaced, "Desired Letter Spacing");
    let gap = at(&desired, 0).advance - at(&natural, 0).advance;
    assert!(gap > 1.0, "{gap}");
    assert!((added_gap(&desired, &natural, 1, 4) - gap).abs() < 1e-6, "one gap after the cluster");

    // 4 pt of room, less than the two gaps' Maximum (half a space each): letter spacing alone.
    let w = natural.lines[0].x1 - natural.lines[0].x0 + 4.0;
    let range = JustificationOptions { letter_spacing: [0.0, 0.0, 50.0], ..JustificationOptions::default() };
    let fitted = layout(db(), &area(MARKED, w, Justify::JustifyAll, range));
    assert_same_offsets(&cluster_offsets(&fitted, 1), &unspaced, "fitted letter spacing");
    let (first, second) = (added_gap(&fitted, &natural, 0, 1), added_gap(&fitted, &natural, 1, 4));
    assert!((first - 2.0).abs() < 1e-6 && (second - 2.0).abs() < 1e-6, "{first} {second}");
}

/// Right to left (a bidi override), Full Justify puts its gaps between the clusters as they are
/// drawn and leaves the mark on its letter.
#[test]
fn rtl_single_word_cluster_gap_follows_drawn_order() {
    // RLO, then MARKED, then PDF: a at byte 3, the cluster at byte 4, z at byte 7.
    let text = "\u{202E}ax\u{301}z\u{202C}";
    let natural = layout(db(), &point(text, JustificationOptions::default()));
    let unspaced = cluster_offsets(&natural, 4);
    assert_eq!(unspaced.len(), 2, "x and its mark are one cluster: {:?}", natural.glyphs);
    assert!(at(&natural, 4).rtl, "the override sets the word right to left");
    let l = layout(db(), &area(text, 300.0, Justify::JustifyAll, JustificationOptions::default()));
    assert_same_offsets(&cluster_offsets(&l, 4), &unspaced, "the mark stays on its letter");
    // Drawn from left to right: z, the cluster, a.
    assert!(at(&l, 7).origin.x < at(&l, 4).origin.x && at(&l, 4).origin.x < at(&l, 3).origin.x);
    let (first, second) = (added_gap(&l, &natural, 7, 4), added_gap(&l, &natural, 4, 3));
    assert!(first > 1.0 && (first - second).abs() < 1e-6, "{first} {second}");
    // The zero-width override marks take no gap: the word runs from edge to edge.
    assert!(at(&l, 7).origin.x.abs() < 1e-6, "{:?}", at(&l, 7).origin);
    assert!((at(&l, 3).origin.x + at(&natural, 3).advance - 300.0).abs() < 1e-6, "{:?}", at(&l, 3).origin);
}

/// Desired Letter Spacing goes between the letters of a word wrapped in bidi override marks,
/// not after the zero-width marks or before the closing one.
#[test]
fn desired_letter_spacing_skips_bidi_format_controls() {
    let text = "\u{202E}ax\u{301}z\u{202C}";
    let natural = layout(db(), &point(text, JustificationOptions::default()));
    let spaced = layout(db(), &point(text, JustificationOptions { letter_spacing: [0.0, 50.0, 50.0], ..JustificationOptions::default() }));
    let gap = at(&spaced, 3).advance - at(&natural, 3).advance;
    assert!(gap > 1.0, "{gap}");
    assert!((at(&spaced, 7).origin.x - at(&natural, 7).origin.x).abs() < 1e-6, "{:?}", at(&spaced, 7).origin);
    let extent = |l: &TextLayout| l.lines[0].x1 - l.lines[0].x0;
    assert!((extent(&spaced) - extent(&natural) - 2.0 * gap).abs() < 1e-6, "{} {} {gap}", extent(&spaced), extent(&natural));
}

/// A zero-width joiner at the start and end of a word, and a word joiner and a soft hyphen inside
/// it, take no letter spacing and no Full Justify gap: the space goes between a, b and c only.
#[test]
fn letter_spacing_and_full_justify_skip_joiners_and_soft_hyphens() {
    // ZWJ (bytes 0..3), a (3), word joiner (4..7), b (7), soft hyphen (8..10), c (10), ZWJ (11..14).
    let text = "\u{200D}a\u{2060}b\u{AD}c\u{200D}";
    let natural = layout(db(), &point(text, JustificationOptions::default()));

    let spaced = layout(db(), &point(text, JustificationOptions { letter_spacing: [0.0, 50.0, 50.0], ..JustificationOptions::default() }));
    let gap = at(&spaced, 3).advance - at(&natural, 3).advance;
    assert!(gap > 1.0, "{gap}");
    assert!((at(&spaced, 3).origin.x - at(&natural, 3).origin.x).abs() < 1e-6, "{:?}", at(&spaced, 3).origin);
    assert!((added_gap(&spaced, &natural, 7, 10) - gap).abs() < 1e-6, "one gap between b and c");
    let extent = |l: &TextLayout| l.lines[0].x1 - l.lines[0].x0;
    assert!((extent(&spaced) - extent(&natural) - 2.0 * gap).abs() < 1e-6, "{} {} {gap}", extent(&spaced), extent(&natural));

    let l = layout(db(), &area(text, 300.0, Justify::JustifyAll, JustificationOptions::default()));
    let (first, second) = (added_gap(&l, &natural, 3, 7), added_gap(&l, &natural, 7, 10));
    assert!(first > 1.0 && (first - second).abs() < 1e-6, "{first} {second}");
    assert!(at(&l, 3).origin.x.abs() < 1e-6, "{:?}", at(&l, 3).origin);
    assert!((at(&l, 10).origin.x + at(&natural, 10).advance - 300.0).abs() < 1e-6, "{:?}", at(&l, 10).origin);
}

/// Every assigned default-ignorable character drawn with no width is a format control, the
/// zero-width space included; word spaces, the Hangul fillers and ordinary letters are not.
#[test]
fn format_controls_are_the_zero_width_default_ignorables() {
    use crate::justify::is_format_control;
    let controls = "\u{AD}\u{34F}\u{61C}\u{180E}\u{200B}\u{200C}\u{200D}\u{200E}\u{202E}\u{2060}\u{2065}\u{2066}\u{206F}";
    let more = "\u{FE0F}\u{FEFF}\u{1BCA0}\u{1D173}\u{E0001}\u{E007F}\u{E0100}";
    for ch in controls.chars().chain(more.chars()) {
        assert!(is_format_control(ch), "U+{:04X}", ch as u32);
    }
    for ch in "a \u{2009}\u{3000}\u{2028}\u{3164}\u{FFA0}\u{301}".chars() {
        assert!(!is_format_control(ch), "U+{:04X}", ch as u32);
    }
}

/// A zero-width space is a line break opportunity but not a word separator: a justified line
/// gives it no share of its room, Full Justify spreads a word with one between its visible letters,
/// and Desired Letter Spacing goes between the letters on either side of it.
#[test]
fn zero_width_space_takes_no_word_space_room() {
    // a (0), b (1), zero-width space (2..5), c (5), d (6), space (7), e (8), f (9).
    let natural = layout(db(), &point("ab\u{200B}cd ef", JustificationOptions::default()));
    let l = layout(db(), &area("ab\u{200B}cd ef", 300.0, Justify::JustifyAll, JustificationOptions::default()));
    assert!(added_gap(&l, &natural, 1, 5).abs() < 1e-6, "b and c stay together: {}", added_gap(&l, &natural, 1, 5));
    assert!((at(&l, 9).origin.x + at(&natural, 9).advance - 300.0).abs() < 1e-6, "the word space takes the room: {:?}", at(&l, 9).origin);

    // No word space: the zero-width space does not stand in for one.
    let text = "ab\u{200B}cd";
    let natural = layout(db(), &point(text, JustificationOptions::default()));
    let l = layout(db(), &area(text, 300.0, Justify::JustifyAll, JustificationOptions::default()));
    let gaps = [added_gap(&l, &natural, 0, 1), added_gap(&l, &natural, 1, 5), added_gap(&l, &natural, 5, 6)];
    assert!(gaps[0] > 1.0 && gaps.iter().all(|g| (g - gaps[0]).abs() < 1e-6), "{gaps:?}");
    assert!((at(&l, 6).origin.x + at(&natural, 6).advance - 300.0).abs() < 1e-6, "{:?}", at(&l, 6).origin);

    let spaced = layout(db(), &point(text, JustificationOptions { letter_spacing: [0.0, 50.0, 50.0], ..JustificationOptions::default() }));
    let gap = at(&spaced, 0).advance - at(&natural, 0).advance;
    assert!(gap > 1.0 && (added_gap(&spaced, &natural, 1, 5) - gap).abs() < 1e-6, "{gap} {}", added_gap(&spaced, &natural, 1, 5));

    // It is still a place to break the line.
    let w = natural.lines[0].x1 - natural.lines[0].x0 - 5.0;
    let wrapped = layout(db(), &area(text, w, Justify::Left, JustificationOptions::default()));
    assert_eq!(wrapped.lines.len(), 2);
    assert_eq!(wrapped.lines[0].end, 5, "breaks after the zero-width space");
}

/// Desired Letter Spacing puts the gap between b and c on the zero-width space, so a line that
/// breaks there ends at b's own advance and wraps where the unspaced text does.
#[test]
fn desired_letter_spacing_ends_a_line_broken_at_a_zero_width_space_without_a_gap() {
    // a (0), b (1), zero-width space (2..5), c (5), d (6).
    let text = "ab\u{200B}cd";
    let desired = JustificationOptions { letter_spacing: [0.0, 50.0, 50.0], ..JustificationOptions::default() };
    let natural = layout(db(), &point(text, JustificationOptions::default()));
    let spaced = layout(db(), &point(text, desired.clone()));
    let gap = at(&spaced, 0).advance - at(&natural, 0).advance;
    assert!(gap > 1.0, "{gap}");
    // Room for a, its gap and b, and half a gap more: not for a second gap after b.
    let w = at(&natural, 0).advance + gap + at(&natural, 1).advance + 0.5 * gap;
    let unspaced = layout(db(), &area(text, w, Justify::Left, JustificationOptions::default()));
    assert_eq!(unspaced.lines.first().map(|l| l.end), Some(5), "the unspaced text breaks after the zero-width space");
    let wrapped = layout(db(), &area(text, w, Justify::Left, desired));
    assert_eq!(wrapped.lines.len(), 2, "{:?}", wrapped.lines);
    assert_eq!(wrapped.lines[0].end, 5, "breaks after the zero-width space");
    assert!((at(&wrapped, 1).advance - at(&natural, 1).advance).abs() < 1e-6, "b keeps its own advance: {}", at(&wrapped, 1).advance);
    let b = at(&wrapped, 1);
    let end = b.origin.x + b.advance;
    assert!((end - (at(&natural, 0).advance + gap + at(&natural, 1).advance)).abs() < 1e-6, "the line ends at b: {end}");
}

/// A single word under Full Justify that breaks at a soft hyphen spreads its first line between its
/// letters and the hyphen drawn after the last one, all with the same gap.
#[test]
fn single_word_full_justify_spreads_up_to_the_drawn_hyphen() {
    // a (0), b (1), c (2), soft hyphen (3..5), then d at byte 5.
    let text = "abc\u{AD}defghij";
    let hyphenated = layout(db(), &point("abc-", JustificationOptions::default()));
    let w = hyphenated.lines[0].x1 - hyphenated.lines[0].x0 + 30.0;
    // The same first line, set ragged: the distances justification adds to.
    let natural = layout(db(), &area(text, w, Justify::Left, JustificationOptions::default()));
    let l = layout(db(), &area(text, w, Justify::JustifyLeft, JustificationOptions::default()));
    for layout in [&natural, &l] {
        assert!(layout.lines.len() >= 2 && layout.lines[0].end == 5, "the first line ends at the soft hyphen: {:?}", layout.lines);
    }
    let hyphen = |l: &TextLayout| l.glyphs[l.lines[0].glyph_end - 1].clone();
    let ab = added_gap(&l, &natural, 0, 1);
    let bc = added_gap(&l, &natural, 1, 2);
    let c_hyphen = (hyphen(&l).origin.x - at(&l, 2).origin.x) - (hyphen(&natural).origin.x - at(&natural, 2).origin.x);
    assert!(ab > 1.0 && (ab - bc).abs() < 1e-6 && (ab - c_hyphen).abs() < 1e-6, "{ab} {bc} {c_hyphen}");
    let end = hyphen(&l).origin.x + hyphen(&l).advance;
    assert!((end - w).abs() < 1e-6, "the hyphen reaches the edge: {end} {w}");
}

/// In vertical type, Full Justify spreads a word down the column between its clusters and leaves
/// the mark on its letter.
#[test]
fn vertical_single_word_cluster_gap_follows_drawn_order() {
    let vertical = |t: &mut TextObject| t.vertical = true;
    let mut plain = point(MARKED, JustificationOptions::default());
    vertical(&mut plain);
    let natural = layout(db(), &plain);
    let unspaced = marked_cluster(&natural);
    let mut t = area(MARKED, 100.0, Justify::JustifyAll, JustificationOptions::default());
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 100.0, 300.0).to_path(0.1)) };
    vertical(&mut t);
    let l = layout(db(), &t);
    assert_same_offsets(&cluster_offsets(&l, 1), &unspaced, "the mark stays on its letter");
    assert!(at(&l, 0).origin.y < at(&l, 1).origin.y && at(&l, 1).origin.y < at(&l, 4).origin.y, "down the column");
    let (first, second) = (added_gap(&l, &natural, 0, 1), added_gap(&l, &natural, 1, 4));
    assert!(first > 1.0 && (first - second).abs() < 1e-6, "{first} {second}");
}

/// A Devanagari syllable whose vowel sign is drawn before its consonant (कि) is one cluster of two
/// glyphs: Full Justify keeps them together. Skipped without an installed Devanagari face.
#[test]
fn single_word_full_justify_keeps_conjunct_cluster() {
    let Some(face) = super::devanagari::devanagari_test_face("single_word_full_justify_keeps_conjunct_cluster") else { return };
    // कि (bytes 0..6) and क (byte 6).
    let text = "\u{915}\u{93F}\u{915}";
    let styled = |mut t: TextObject| {
        for run in &mut t.runs {
            run.style.font_family = face.family.clone();
            run.style.font_style = face.style.clone();
        }
        t
    };
    let natural = layout(db(), &styled(point(text, JustificationOptions::default())));
    let unspaced = cluster_offsets(&natural, 0);
    assert!(unspaced.len() >= 2, "कि is one cluster of several glyphs: {:?}", natural.glyphs);
    let l = layout(db(), &styled(area(text, 300.0, Justify::JustifyAll, JustificationOptions::default())));
    assert_same_offsets(&cluster_offsets(&l, 0), &unspaced, "the syllable stays together");
    assert!(added_gap(&l, &natural, 0, 6) > 1.0);
}

/// Single Word Justification: a justified line holding one word is spread between its letters
/// (Full Justify, the default) or aligned as chosen.
#[test]
fn single_word_lines_follow_single_word_justification() {
    let text = "Typography internationalization";
    let word = layout(db(), &point("Typography", JustificationOptions::default()));
    // "Typography" alone on the first line, 30 pt short of the measure.
    let w = word.lines[0].x1 - word.lines[0].x0 + 30.0;
    let first = |sw: SingleWordJustify| {
        let l = layout(db(), &area(text, w, Justify::JustifyLeft, JustificationOptions { single_word: sw, ..JustificationOptions::default() }));
        assert_eq!(lines(&l, text).first().map(String::as_str), Some("Typography"));
        let li = &l.lines[0];
        let g: Vec<_> = l.glyphs[li.glyph_start..li.glyph_end].iter().filter(|g| g.byte < "Typography".len()).collect();
        (g.first().unwrap().origin.x, g.last().map(|g| g.origin.x + g.advance).unwrap())
    };
    let (x0, x1) = first(SingleWordJustify::FullJustify);
    assert!(x0.abs() < 1e-6 && (x1 - w).abs() < 1e-3, "spread across: {x0}..{x1} of {w}");
    let (x0, x1) = first(SingleWordJustify::AlignLeft);
    assert!(x0.abs() < 1e-6 && (x1 - (w - 30.0)).abs() < 1e-3, "{x0}..{x1}");
    let (r0, r1) = first(SingleWordJustify::AlignRight);
    assert!((r1 - w).abs() < 1e-3 && (r0 - 30.0).abs() < 1e-3, "{r0}..{r1}");
    let (c0, c1) = first(SingleWordJustify::AlignCenter);
    assert!((c0 - 15.0).abs() < 1e-3 && (c1 - (w - 15.0)).abs() < 1e-3, "centred: {c0}..{c1}");
}

/// Word spacing at 100% with no range: the line's room goes on its word spaces alone.
fn no_ranges() -> JustificationOptions {
    JustificationOptions { word_spacing: [100.0, 100.0, 100.0], ..JustificationOptions::default() }
}

/// `text` in a 300 pt wide frame with left tab stops at `stops` (a leader of `leader` on each).
fn tabbed(text: &str, stops: &[f64], leader: &str, justify: Justify, j: JustificationOptions) -> TextLayout {
    let mut t = area(text, 300.0, justify, j);
    t.para.tabs = stops.iter().map(|&position| TabStop { position, align: TabAlign::Left, leader: leader.to_string(), align_on: '.' }).collect();
    layout(db(), &t)
}

/// Where the glyph of the character at byte `b` ends.
fn end(l: &TextLayout, b: usize) -> f64 {
    at(l, b).origin.x + at(l, b).advance
}

/// A justified line with a tab: the text before its last tab keeps its spacing and the tab its
/// stop, and the word spaces after the tab take the line's room, so the line ends on the far edge.
#[test]
fn a_justified_line_with_a_tab_spreads_the_text_after_it_to_the_edge() {
    // A (0), A (1), space (2), B (3), B (4), tab (5), C (6), C (7), space (8), D (9), D (10); a
    // soft return ends the line, so Justify Left justifies it.
    let text = "AA BB\tCC DD\u{2028}x";
    for j in [no_ranges(), JustificationOptions::default()] {
        let natural = tabbed(text, &[150.0], "", Justify::Left, j.clone());
        let l = tabbed(text, &[150.0], "", Justify::JustifyLeft, j);
        for b in 0..5 {
            assert!((at(&l, b).origin.x - at(&natural, b).origin.x).abs() < 1e-6, "byte {b} stays where it was");
            assert!((at(&l, b).advance - at(&natural, b).advance).abs() < 1e-6, "byte {b} keeps its advance");
        }
        assert!((end(&l, 5) - 150.0).abs() < 1e-6, "the tab keeps its stop: {}", end(&l, 5));
        assert!((end(&l, 10) - 300.0).abs() < 1e-6, "the last word ends on the edge: {}", end(&l, 10));
        assert!((l.lines[0].x1 - 300.0).abs() < 1e-6, "the line's record ends there too: {}", l.lines[0].x1);
        assert!(at(&l, 8).advance > at(&natural, 8).advance + 1.0, "the word space after the tab takes the room");
    }
}

/// A leader stays inside its tab's gap on a justified line.
#[test]
fn a_justified_tab_keeps_its_leader_inside_its_gap() {
    let l = tabbed("AA BB\tCC DD", &[150.0], ".", Justify::JustifyAll, no_ranges());
    let tab = at(&l, 5);
    let dots = tab.outline.bounding_box();
    assert!(!tab.outline.elements().is_empty(), "the leader is drawn");
    assert!(dots.x0 >= tab.origin.x - 1e-6 && dots.x1 <= 150.0 + 1e-6, "{dots:?} inside {}..150", tab.origin.x);
    assert!((end(&l, 10) - 300.0).abs() < 1e-6, "{}", end(&l, 10));
}

/// Two words with a tab between them and no word space: the word after the tab is spread
/// between its letters to the edge, whatever Single Word Justification says, since a tab
/// separates two words.
#[test]
fn a_justified_word_tab_word_line_reaches_the_edge() {
    // A (0), A (1), tab (2), B (3), B (4).
    let text = "AA\tBB\u{2028}x";
    for single_word in [SingleWordJustify::FullJustify, SingleWordJustify::AlignLeft, SingleWordJustify::AlignRight] {
        let j = JustificationOptions { single_word, ..JustificationOptions::default() };
        let natural = tabbed(text, &[150.0], "", Justify::Left, j.clone());
        let l = tabbed(text, &[150.0], "", Justify::JustifyLeft, j);
        assert!(at(&l, 0).origin.x.abs() < 1e-6, "{single_word:?}: starts at the left edge: {}", at(&l, 0).origin.x);
        assert!((at(&l, 0).advance - at(&natural, 0).advance).abs() < 1e-6, "{single_word:?}: the word before the tab keeps its width");
        assert!((end(&l, 2) - 150.0).abs() < 1e-6, "{single_word:?}: the tab keeps its stop: {}", end(&l, 2));
        assert!((end(&l, 4) - 300.0).abs() < 1e-6, "{single_word:?}: ends on the edge: {}", end(&l, 4));
    }
}

/// The last line of a paragraph: Justify All spreads the text after its tab to the edge, and
/// Justify Left leaves it aligned left.
#[test]
fn the_last_line_with_a_tab_fills_the_measure_only_with_justify_all() {
    let text = "AA BB\tCC DD";
    let all = tabbed(text, &[150.0], "", Justify::JustifyAll, no_ranges());
    assert!((end(&all, 5) - 150.0).abs() < 1e-6 && (end(&all, 10) - 300.0).abs() < 1e-6, "{} {}", end(&all, 5), end(&all, 10));
    let left = tabbed(text, &[150.0], "", Justify::JustifyLeft, no_ranges());
    let natural = tabbed(text, &[150.0], "", Justify::Left, no_ranges());
    assert!((end(&left, 10) - end(&natural, 10)).abs() < 1e-6 && end(&left, 10) < 250.0, "{}", end(&left, 10));
}

/// A line with two tabs: both keep their stops, the word space between them keeps its width, and
/// the text after the last tab takes the room.
#[test]
fn a_justified_line_takes_its_room_after_its_last_tab() {
    // A (0), tab (1), B (2), space (3), C (4), tab (5), D (6), space (7), E (8).
    let text = "A\tB C\tD E";
    let natural = tabbed(text, &[80.0, 160.0], "", Justify::Left, no_ranges());
    let l = tabbed(text, &[80.0, 160.0], "", Justify::JustifyAll, no_ranges());
    assert!((end(&l, 1) - 80.0).abs() < 1e-6 && (end(&l, 5) - 160.0).abs() < 1e-6, "{} {}", end(&l, 1), end(&l, 5));
    assert!((at(&l, 3).advance - at(&natural, 3).advance).abs() < 1e-6, "the space between the tabs keeps its width");
    assert!((end(&l, 8) - 300.0).abs() < 1e-6, "{}", end(&l, 8));
}

/// Vertical area type: the tab keeps its stop down the column, and the text after it reaches the
/// column's end.
#[test]
fn a_justified_vertical_line_with_a_tab_reaches_the_end_of_its_column() {
    let mut t = area("AA BB\tCC DD", 100.0, Justify::JustifyAll, no_ranges());
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 100.0, 300.0).to_path(0.1)) };
    t.vertical = true;
    t.para.tabs = vec![TabStop { position: 150.0, align: TabAlign::Left, leader: String::new(), align_on: '.' }];
    let l = layout(db(), &t);
    let down = |b: usize| at(&l, b).origin.y + at(&l, b).advance;
    assert!(at(&l, 0).origin.y.abs() < 1e-6, "{}", at(&l, 0).origin.y);
    assert!((down(5) - 150.0).abs() < 1e-6, "the tab keeps its stop: {}", down(5));
    assert!((down(10) - 300.0).abs() < 1e-6, "the last word ends at the column's end: {}", down(10));
}

/// Right-to-left area type: the glyphs drawn up to the last tab drawn keep their advances, the tab
/// its stop, and the text drawn after it reaches the right edge.
#[test]
fn a_justified_right_to_left_line_with_a_tab_spans_its_measure() {
    // א (0), ב (2), space (4), ג (5), ד (7), tab (9), ה (10), ו (12), space (14), ז (15), ח (17).
    let text = "אב גד\tהו זח";
    let natural = tabbed(text, &[150.0], "", Justify::Right, no_ranges());
    let l = tabbed(text, &[150.0], "", Justify::JustifyAll, no_ranges());
    let mut drawn: Vec<&PositionedGlyph> = l.glyphs.iter().collect();
    drawn.sort_by(|a, b| a.origin.x.total_cmp(&b.origin.x));
    assert!(drawn[0].origin.x.abs() < 1e-6, "starts at the left edge: {:?}", drawn[0].origin);
    let tab = drawn.iter().position(|g| g.byte == 9).unwrap();
    for g in &drawn[..tab] {
        assert!((g.advance - at(&natural, g.byte).advance).abs() < 1e-6, "byte {} keeps its advance", g.byte);
    }
    assert!((end(&l, 9) - 150.0).abs() < 1e-6, "the tab keeps its stop: {}", end(&l, 9));
    let last = drawn.last().unwrap();
    assert!((last.origin.x + last.advance - 300.0).abs() < 1e-6, "ends on the right edge: {}", last.origin.x + last.advance);
    assert!((l.lines[0].x1 - 300.0).abs() < 1e-6, "{}", l.lines[0].x1);
}

/// Point type has no measure to fill: a justified line with a tab keeps its natural width.
#[test]
fn justified_point_type_with_a_tab_keeps_its_width() {
    let mut t = point("AA BB\tCC DD", no_ranges());
    t.para.justify = Justify::JustifyAll;
    t.para.tabs = vec![TabStop { position: 150.0, align: TabAlign::Left, leader: String::new(), align_on: '.' }];
    let l = layout(db(), &t);
    t.para.justify = Justify::Left;
    let natural = layout(db(), &t);
    assert!((end(&l, 10) - end(&natural, 10)).abs() < 1e-6, "{} {}", end(&l, 10), end(&natural, 10));
}

/// An OpenType (GSUB) Arabic face with a tatweel: craft-fonts' Noto Sans Arabic when the build has
/// craft-fonts (`CRAFT_FONTS_DIR`), else an installed one. None skips the test.
fn arabic_face(test: &str) -> Option<std::sync::Arc<FontFace>> {
    let opentype = |face: &FontFace| {
        use skrifa::raw::TableProvider;
        face.skrifa().is_some_and(|f| f.gsub().is_ok())
    };
    let face = ["Noto Sans Arabic", "Arial", "Geeza Pro"]
        .into_iter()
        .filter_map(|name| db().face(name, "Regular"))
        .find(|face| face.covers('ب') && face.covers('د') && face.covers('\u{0640}') && opentype(face));
    match &face {
        Some(face) => eprintln!("ASSERTIONS RUN: {test}; Arabic font {} {}", face.family, face.style),
        None => eprintln!("SKIPPED: {test}; no font covers Arabic and the tatweel"),
    }
    face
}

/// Arabic type at 20 pt in `face`, in a frame 300 pt wide, aligned as `justify` with `kashida`.
fn arabic_area(face: &FontFace, text: &str, justify: Justify, kashida: Kashida) -> TextObject {
    let st = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 20.0, ..CharStyle::default() };
    let mut t = TextObject::point(Point::ZERO, text, st);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 300.0, 1000.0).to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    t.para.justify = justify;
    t.para.kashida = kashida;
    t
}

/// The kashida glyphs of a layout: tatweels that stand for no character.
fn kashidas<'a>(l: &'a TextLayout, face: &FontFace) -> Vec<&'a PositionedGlyph> {
    let tatweel = face.glyph_for('\u{0640}');
    l.glyphs.iter().filter(|g| g.len == 0 && g.gid == tatweel).collect()
}

/// Insert Kashidas: a justified Arabic line lengthens each word with one kashida between its last
/// two joined letters, up to a quarter, half or whole em (Stylistic: whole tatweels up to an em),
/// before it widens the word space. The line keeps its width, the kashida fills the gap between
/// the letters it joins, and None leaves the line as it was.
#[test]
fn justified_arabic_lines_take_their_room_on_kashidas() {
    let Some(face) = arabic_face("justified_arabic_lines_take_their_room_on_kashidas") else { return };
    // Two words whose last letters join (teh and beh, meem and dal) and one whose letters (waw, reh,
    // dal) join none after them.
    let text = "كتب ورد محمد";
    let spaces: Vec<usize> = text.match_indices(' ').map(|(b, _)| b).collect();
    let off = layout(db(), &arabic_area(&face, text, Justify::JustifyAll, Kashida::None));
    assert!(off.glyphs.iter().all(|g| g.len > 0), "no kashidas without Insert Kashidas");
    let measure = |l: &TextLayout| l.lines[0].x1 - l.lines[0].x0;
    assert!((measure(&off) - 300.0).abs() < 1e-6, "the line fills the frame: {}", measure(&off));
    let space = at(&off, spaces[0]).advance;
    let tatweel = face.advance(face.glyph_for('\u{0640}')) * 20.0 / face.units_per_em();
    for (kind, longest) in [(Kashida::Short, 5.0), (Kashida::Medium, 10.0), (Kashida::Long, 20.0), (Kashida::Stylistic, 20.0)] {
        let on = layout(db(), &arabic_area(&face, text, Justify::JustifyAll, kind));
        let k = kashidas(&on, &face);
        let mut bytes: Vec<usize> = k.iter().map(|g| g.byte).collect();
        bytes.sort_unstable();
        bytes.dedup();
        assert_eq!(bytes, ["كتب".len() - 'ب'.len_utf8(), text.len() - 'د'.len_utf8()], "{kind:?}: before the last letter of each word");
        let each = if kind == Kashida::Stylistic { (longest / tatweel).floor() * tatweel } else { longest };
        let mut total = 0.0;
        for &b in &bytes {
            let (start, end, count) = kashida_span(&on, &face, b);
            assert!((end - start - each).abs() < 1e-6, "{kind:?}: {start}..{end}");
            total += end - start;
            if kind == Kashida::Stylistic {
                assert!((count as f64 * tatweel - each).abs() < 1e-6, "{kind:?}: whole tatweels");
            }
            // The letter after the kashida in the text advances over it to the letter before it,
            // so the caret before that letter and a selection of it reach the letter before it.
            let after = on.glyphs.iter().filter(|o| o.byte == b && o.len > 0);
            let len = after.clone().map(|o| o.len).max().unwrap();
            let ink = after.clone().map(|o| o.origin.x).fold(f64::MAX, f64::min);
            assert!(ink < start && (after.map(|o| o.origin.x + o.advance).fold(f64::MIN, f64::max) - end).abs() < 1e-6, "{kind:?}");
            let caret = caret_position(&on, b).0.x;
            assert!((caret - end).abs() < 1e-6, "{kind:?}: the caret before the letter: {caret} {end}");
            let x_max = selection_quads(&on, b, b + len).iter().flatten().map(|p| p.x).fold(f64::MIN, f64::max);
            assert!((x_max - end).abs() < 1e-6, "{kind:?}: the selection takes the kashida in: {x_max} {end}");
        }
        assert!((measure(&on) - measure(&off)).abs() < 1e-6, "{kind:?}: same width");
        for &b in &spaces {
            assert!((at(&on, b).advance - (space - total / 2.0)).abs() < 1e-6, "{kind:?}: the spaces give up the kashidas' length");
        }
    }
    // A click on a letter and its kashida goes to the nearer edge of the two taken together: dal is
    // narrower than a Long kashida.
    let on = layout(db(), &arabic_area(&face, text, Justify::JustifyAll, Kashida::Long));
    let dal = text.len() - 'د'.len_utf8();
    let (start, end, _) = kashida_span(&on, &face, dal);
    let ink = on.glyphs.iter().filter(|o| o.byte == dal && o.len > 0).map(|o| o.origin.x).fold(f64::MAX, f64::min);
    let middle = (ink + end) / 2.0;
    assert!(start < middle, "the kashida is longer than the letter: {ink} {start} {end}");
    assert_eq!(byte_in_line(&on, 0, (start + middle) / 2.0), text.len(), "left of the middle: the letter's left edge");
    assert_eq!(byte_in_line(&on, 0, (middle + end) / 2.0), dal, "right of the middle: its right edge");
    // A letter with a vowel mark is a cluster of two glyphs: the caret before it and a selection of
    // it still reach the letter before the kashida.
    let marked = "كتبَ ورد";
    let on = layout(db(), &arabic_area(&face, marked, Justify::JustifyAll, Kashida::Medium));
    let beh = "كت".len();
    let (_, end, _) = kashida_span(&on, &face, beh);
    assert!(on.glyphs.iter().filter(|o| o.byte == beh && o.len > 0).count() >= 2, "beh and fatha");
    assert!((caret_position(&on, beh).0.x - end).abs() < 1e-6, "{} {end}", caret_position(&on, beh).0.x);
    let x_max = selection_quads(&on, beh, "كتبَ".len()).iter().flatten().map(|p| p.x).fold(f64::MIN, f64::max);
    assert!((x_max - end).abs() < 1e-6, "{x_max} {end}");
    // Punctuation ends a word as a space does.
    let text = "كتب/محمد";
    let on = layout(db(), &arabic_area(&face, text, Justify::JustifyAll, Kashida::Medium));
    let mut bytes: Vec<usize> = kashidas(&on, &face).iter().map(|g| g.byte).collect();
    bytes.sort_unstable();
    assert_eq!(bytes, ["كتب".len() - 'ب'.len_utf8(), text.len() - 'د'.len_utf8()]);
    // At 50% horizontal scale an em is half as wide; the line still fills the frame.
    for kind in [Kashida::Long, Kashida::Stylistic] {
        let text = "كتب ورد محمد";
        let mut t = arabic_area(&face, text, Justify::JustifyAll, kind);
        for r in &mut t.runs {
            r.style.h_scale = 50.0;
        }
        let half = layout(db(), &t);
        let each = if kind == Kashida::Stylistic { (10.0 / (tatweel * 0.5)).floor() * tatweel * 0.5 } else { 10.0 };
        for b in ["كتب".len() - 'ب'.len_utf8(), text.len() - 'د'.len_utf8()] {
            let (start, end, _) = kashida_span(&half, &face, b);
            assert!((end - start - each).abs() < 1e-6, "{kind:?} at 50%: {start}..{end}");
        }
        assert!((measure(&half) - 300.0).abs() < 1e-6, "{kind:?} at 50%: {}", measure(&half));
    }
}

/// A character with Kashidas off prevents insertion at either of its joined edges. Other
/// eligible pairs in the word remain available; old text defaults to allowing them.
#[test]
fn character_kashidas_filter_joined_pairs() {
    let Some(face) = arabic_face("character_kashidas_filter_joined_pairs") else { return };
    let text = "كتب ورد محمد";
    let mut t = arabic_area(&face, text, Justify::JustifyAll, Kashida::Medium);
    let default_bytes = kashidas(&layout(db(), &t), &face).into_iter().map(|g| g.byte).collect::<Vec<_>>();
    assert!(default_bytes.contains(&("كت".len())));
    let mut off = t.runs[0].style.clone();
    off.kashidas = false;
    let on = t.runs[0].style.clone();
    t.runs = vec![TextRun::new("كت", on.clone()), TextRun::new("ب", off.clone()), TextRun::new(" ورد محمد", on.clone())];
    let one_off = layout(db(), &t);
    let bytes = kashidas(&one_off, &face).into_iter().map(|g| g.byte).collect::<Vec<_>>();
    assert!(!bytes.contains(&("كت".len())), "the disabled beh has no kashida before it");
    assert!(bytes.contains(&("ك".len())), "the earlier eligible pair remains");
    t.runs = vec![TextRun::new("ك", on.clone()), TextRun::new("تب", off), TextRun::new(" ورد محمد", on)];
    let both_off = layout(db(), &t);
    let bytes = kashidas(&both_off, &face).into_iter().map(|g| g.byte).collect::<Vec<_>>();
    assert!(!bytes.contains(&("ك".len())) && !bytes.contains(&("كت".len())));
    assert!(bytes.contains(&(text.len() - 'د'.len_utf8())), "another word still takes its kashida");
}

/// Where the kashida before the cluster at byte `b` starts, where the letter before it in the text
/// starts (the kashida's end), and how many tatweels draw it.
fn kashida_span(l: &TextLayout, face: &FontFace, b: usize) -> (f64, f64, usize) {
    let k: Vec<_> = kashidas(l, face).into_iter().filter(|g| g.byte == b).collect();
    let start = k.iter().map(|g| g.origin.x).fold(f64::MAX, f64::min);
    let end = l.glyphs.iter().filter(|o| o.byte + o.len == b && o.len > 0).map(|o| o.origin.x).fold(f64::MAX, f64::min);
    (start, end, k.len())
}

/// Insert Kashidas changes only justified horizontal lines of Arabic: right-aligned and Auto
/// Arabic paragraphs, a justified Latin one and justified vertical Arabic type are set as with
/// None.
#[test]
fn kashidas_leave_unjustified_and_latin_lines_as_they_were() {
    let same = |a: &TextLayout, b: &TextLayout| {
        assert_eq!(a.glyphs.len(), b.glyphs.len());
        for (x, y) in a.glyphs.iter().zip(&b.glyphs) {
            assert_eq!((x.gid, x.byte, x.len, x.origin, x.advance), (y.gid, y.byte, y.len, y.origin, y.advance));
        }
    };
    let latin = |k: Kashida| {
        let mut t = area("ab cd", 300.0, Justify::JustifyAll, JustificationOptions::default());
        t.para.kashida = k;
        layout(db(), &t)
    };
    same(&latin(Kashida::Long), &latin(Kashida::None));
    let Some(face) = arabic_face("kashidas_leave_unjustified_and_latin_lines_as_they_were") else { return };
    let arabic = |j: Justify, k: Kashida| layout(db(), &arabic_area(&face, "كتب محمد", j, k));
    same(&arabic(Justify::Right, Kashida::Long), &arabic(Justify::Right, Kashida::None));
    same(&arabic(Justify::Auto, Kashida::Medium), &arabic(Justify::Auto, Kashida::None));
    assert!(!kashidas(&arabic(Justify::JustifyAll, Kashida::Medium), &face).is_empty());
    // Vertical type is set as with None too.
    let vertical = |k: Kashida| {
        let mut t = arabic_area(&face, "كتب محمد", Justify::JustifyAll, k);
        t.vertical = true;
        layout(db(), &t)
    };
    same(&vertical(Kashida::Medium), &vertical(Kashida::None));
}

/// Auto Leading: characters whose leading is Auto are set at the paragraph's share of their size;
/// an explicit leading is kept.
#[test]
fn auto_leading_sets_the_leading_of_auto_characters() {
    let text = "one\ntwo";
    let gap = |t: &TextObject| {
        let l = layout(db(), t);
        l.lines[1].baseline - l.lines[0].baseline
    };
    let t = area(text, 300.0, Justify::Left, JustificationOptions::default());
    assert!((gap(&t) - 24.0).abs() < 1e-6, "120% of 20 pt");
    let mut t = area(text, 300.0, Justify::Left, JustificationOptions { auto_leading: 200.0, ..JustificationOptions::default() });
    assert!((gap(&t) - 40.0).abs() < 1e-6, "200% of 20 pt");
    t.runs[0].style.leading = Some(30.0);
    assert!((gap(&t) - 30.0).abs() < 1e-6, "explicit leading kept");
}
