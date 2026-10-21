//! Paragraph panel menu › Justification…: word spacing, letter spacing, glyph scaling, Single Word
//! Justification and Auto Leading in layout.

use super::*;
use kurbo::{Affine, Shape};
use vectorcraft_doc::text::Kashida;
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
