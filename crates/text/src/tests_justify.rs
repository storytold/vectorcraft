//! Paragraph panel menu › Justification…: word spacing, letter spacing, glyph scaling, Single Word
//! Justification and Auto Leading in layout.

use super::*;
use kurbo::{Affine, Shape};
use vectorcraft_doc::{CharStyle, JustificationOptions, Justify, SingleWordJustify, TextKind};
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
