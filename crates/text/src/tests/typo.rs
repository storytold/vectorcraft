//! Typography tests: rich-text editing, caret navigation, selection, area flow, composer,
//! hyphenation, OpenType features, area type options.

use crate::*;
use kurbo::{Affine, Circle, Shape};
use vectorcraft_doc::{CharStyle, Justify, TextKind, TextRun};
use vectorcraft_geom::PathData;

fn db() -> &'static FontDb {
    FontDb::global()
}

fn style(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

fn run(text: &str, size: f64) -> TextRun {
    TextRun { text: text.into(), style: style(size), inline: None }
}

fn area_path(text: &str, st: CharStyle, frame: &BezPath, justify: Justify) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, st);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(frame) };
    t.xf = Affine::IDENTITY;
    t.para.justify = justify;
    t
}

fn area(text: &str, st: CharStyle, frame: Rect, justify: Justify) -> TextObject {
    area_path(text, st, &frame.to_path(0.1), justify)
}

const COPY: &str = "Typography is the craft of arranging type to make written language legible, readable and appealing when displayed. \
The arrangement of type involves selecting typefaces, point sizes, line lengths, line spacing and letter spacing, and adjusting the space \
between pairs of letters. Designers balance hyphenation against justification to produce even texture across the paragraph.";

// ---------- run editing ----------

#[test]
fn range_style_splits_runs() {
    let mut runs = vec![run("Hello brave world", 12.0)];
    edit::style_range(&mut runs, 6, 11, |s| s.size = 24.0);
    assert_eq!(runs.len(), 3);
    assert_eq!((runs[0].text.as_str(), runs[1].text.as_str(), runs[2].text.as_str()), ("Hello ", "brave", " world"));
    assert_eq!((runs[0].style.size, runs[1].style.size, runs[2].style.size), (12.0, 24.0, 12.0));
    // Restyling back merges the runs again.
    edit::style_range(&mut runs, 6, 11, |s| s.size = 12.0);
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "Hello brave world");
}

#[test]
fn range_style_across_run_boundaries() {
    let mut runs = vec![run("aaa", 10.0), run("bbb", 20.0), run("ccc", 30.0)];
    edit::style_range(&mut runs, 2, 7, |s| s.tracking = 50.0);
    let sizes: Vec<(String, f64, f64)> = runs.iter().map(|r| (r.text.clone(), r.style.size, r.style.tracking)).collect();
    assert_eq!(
        sizes,
        vec![("aa".into(), 10.0, 0.0), ("a".into(), 10.0, 50.0), ("bbb".into(), 20.0, 50.0), ("c".into(), 30.0, 50.0), ("cc".into(), 30.0, 0.0)]
    );
    // Reversed and out-of-range offsets are clamped.
    edit::style_range(&mut runs, 100, 0, |s| s.tracking = 0.0);
    assert!(runs.iter().all(|r| r.style.tracking == 0.0));
    assert_eq!(runs.len(), 3);
}

#[test]
fn replace_range_takes_replaced_style_and_normalizes() {
    let mut runs = vec![run("Hello ", 12.0), run("brave", 24.0), run(" world", 12.0)];
    // Typing over the big word keeps its size.
    let caret = edit::replace_range(&mut runs, 6, 11, "bold");
    assert_eq!(caret, 10);
    assert_eq!(runs[1].text, "bold");
    assert_eq!(runs[1].style.size, 24.0);
    // Inserting at a run end continues the preceding run.
    let caret = edit::replace_range(&mut runs, 10, 10, "er");
    assert_eq!((caret, runs[1].text.as_str()), (12, "bolder"));
    // Deleting a whole run merges its neighbours.
    edit::replace_range(&mut runs, 6, 12, "");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "Hello  world");
    // Deleting everything keeps one empty run with the style.
    edit::replace_range(&mut runs, 0, 100, "");
    assert_eq!(runs.len(), 1);
    assert!(runs[0].text.is_empty());
    assert_eq!(runs[0].style.size, 12.0);
}

/// #2119: deleting all of the text with no runs to insert (Backspace over a full selection, an IME
/// clearing its marked text) keeps the first replaced character's style, not the default one.
#[test]
fn replace_range_styled_with_nothing_keeps_the_style_of_emptied_text() {
    let mut runs = vec![run("Chair", 36.0), run(" 123", 48.0)];
    let caret = edit::replace_range_styled(&mut runs, 0, 9, &[]);
    assert_eq!(caret, 0);
    assert_eq!(runs.len(), 1);
    assert!(runs[0].text.is_empty());
    assert_eq!(runs[0].style.size, 36.0);
}

#[test]
fn replace_respects_char_boundaries_and_slices() {
    let mut runs = vec![run("héllo", 12.0)];
    // Offset 2 is inside 'é' (2 bytes): clamped back to the boundary.
    edit::replace_range(&mut runs, 2, 3, "E");
    assert_eq!(runs[0].text, "hEllo");
    let mut runs = vec![run("ab", 10.0), run("cd", 20.0)];
    let s = edit::slice_runs(&runs, 1, 3);
    assert_eq!(s.len(), 2);
    assert_eq!((s[0].text.as_str(), s[1].text.as_str()), ("b", "c"));
    let caret = edit::replace_range_styled(&mut runs, 4, 4, &s);
    assert_eq!(caret, 6);
    assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<String>(), "abcdbc");
}

#[test]
fn word_and_paragraph_navigation() {
    let s = "Hello, big world\nNext line";
    assert_eq!(edit::next_word(s, 0), 5);
    assert_eq!(edit::next_word(s, 5), 10);
    assert_eq!(edit::prev_word(s, 10), 7);
    assert_eq!(edit::prev_word(s, 7), 0);
    assert_eq!(edit::word_at(s, 8), 7..10);
    assert_eq!(edit::word_at(s, 10), 7..10, "at a word end");
    assert_eq!(edit::word_at(s, 5), 0..5, "word end wins over punctuation");
    assert_eq!(edit::word_at("x ,y", 2), 2..3, "punctuation alone");
    assert_eq!(edit::word_at(s, 6), 6..7, "spaces");
    assert_eq!(edit::paragraph_at(s, 3), 0..16);
    assert_eq!(edit::paragraph_at(s, 20), 17..26);
    assert_eq!(edit::next_char(s, 0), 1);
    assert_eq!(edit::prev_char("é", 2), 0);
    assert_eq!(edit::next_word(s, s.len()), s.len());
}

// ---------- caret navigation / selection ----------

#[test]
fn caret_up_down_keeps_column() {
    let t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 200.0, 600.0), Justify::Left);
    let l = layout(db(), &t);
    assert!(l.lines.len() > 5);
    let b = l.lines[2].start + 6;
    let (top, _) = caret_position(&l, b);
    let down = caret_vertical(&l, b, 1, top.x);
    assert_eq!(l.line_of(down), 3);
    let (t2, _) = caret_position(&l, down);
    assert!((t2.x - top.x).abs() < 12.0, "{} vs {}", t2.x, top.x);
    let up = caret_vertical(&l, down, -1, top.x);
    assert_eq!(up, b);
    assert_eq!(caret_vertical(&l, b, -10, top.x), 0);
    assert_eq!(caret_vertical(&l, b, 100, top.x), COPY.len());
}

#[test]
fn home_end_on_wrapped_lines() {
    let t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 200.0, 600.0), Justify::Left);
    let l = layout(db(), &t);
    let b = l.lines[1].start + 3;
    assert_eq!(line_home(&l, b), l.lines[1].start);
    let e = line_end_of(&l, b);
    // Before the trailing space: the caret stays on line 1.
    assert_eq!(l.line_of(e), 1);
    assert_eq!(&COPY[e..e + 1], " ");
}

#[test]
fn selection_quads_cover_lines() {
    let t = TextObject::point(Point::ZERO, "One two\nThree four", style(20.0));
    let l = layout(db(), &t);
    let q = selection_quads(&l, 4, 7);
    assert_eq!(q.len(), 1);
    let (a, _) = caret_position(&l, 4);
    let (b, _) = caret_position(&l, 7);
    assert!((q[0][0].x - a.x).abs() < 1e-6 && (q[0][1].x - b.x).abs() < 1e-6);
    // Across the paragraph break: two quads.
    let q = selection_quads(&l, 4, 12);
    assert_eq!(q.len(), 2);
    assert!(q[1][0].y > q[0][0].y);
    assert!(selection_quads(&l, 3, 3).is_empty());
}

#[test]
fn hit_testing_picks_the_right_column() {
    let text: String = COPY.repeat(3);
    let opts = LayoutOptions { columns: 2, gutter: 20.0, ..Default::default() };
    let t = area(&text, style(12.0), Rect::new(0.0, 0.0, 420.0, 200.0), Justify::Left);
    let l = layout_with(db(), &t, &opts);
    assert_eq!(l.frames.len(), 2);
    let right: Vec<&LineInfo> = l.lines.iter().filter(|li| li.x0 >= 220.0 - 1e-6).collect();
    assert!(!right.is_empty(), "text flowed into the second column");
    let r0 = right[0];
    let b = hit_byte(&l, Point::new(r0.x0 + 1.0, r0.baseline));
    assert!(b >= r0.start && b <= r0.end, "{b} not in {}..{}", r0.start, r0.end);
    for li in &l.lines {
        assert!((li.x0 >= -1e-6 && li.x1 <= 200.0 + 1e-6) || (li.x0 >= 220.0 - 1e-6 && li.x1 <= 420.0 + 1e-6), "{li:?}");
    }
}

// ---------- area flow ----------

#[test]
fn area_flow_inside_circle_keeps_glyphs_inside() {
    let circle = Circle::new((150.0, 150.0), 120.0).to_path(0.1);
    let t = area_path(COPY, style(11.0), &circle, Justify::Left);
    let l = layout(db(), &t);
    assert!(l.lines.len() > 8);
    let inside = |p: Point| (p - Point::new(150.0, 150.0)).hypot() <= 120.0 + 0.5;
    for g in &l.glyphs {
        let li = &l.lines[g.line];
        for p in [
            Point::new(g.origin.x, li.baseline - li.ascent),
            Point::new(g.origin.x + g.advance, li.baseline - li.ascent),
            Point::new(g.origin.x, li.baseline + li.descent),
            Point::new(g.origin.x + g.advance, li.baseline + li.descent),
        ] {
            if g.outline.elements().is_empty() {
                continue;
            }
            assert!(inside(p), "glyph box corner {p:?} outside the circle (line {})", g.line);
        }
    }
    // Lines near the middle are wider than lines near the top.
    let w = |i: usize| l.lines[i].avail.1 - l.lines[i].avail.0;
    assert!(w(l.lines.len() / 2) > w(0));
}

/// In a frame that is narrow at the top, a word that doesn't fit the top lines moves down to the
/// first line wide enough for it (the lines above stay empty), with either composer. Only a word
/// wider than every line is broken.
#[test]
fn area_flow_moves_a_word_down_to_a_line_wide_enough_for_it() {
    let triangle = |base: f64, height: f64| {
        let mut p = BezPath::new();
        p.move_to((base / 2.0, 0.0));
        p.line_to((base, height));
        p.line_to((0.0, height));
        p.close_path();
        p
    };
    let text = "Wonderful extraordinary typesetting";
    for composer in [Composer::EveryLine, Composer::SingleLine] {
        let mut t = area_path(text, style(12.0), &triangle(200.0, 200.0), Justify::Left);
        t.para.composer = composer;
        let l = layout(db(), &t);
        assert!(!l.overflow, "{composer:?}");
        let lines: Vec<&str> = l.lines.iter().map(|li| text[li.start..li.end].trim_end()).collect();
        // The top lines are wide enough for "Wo" but not for "Wonderful": it starts lower.
        assert_eq!(lines.first(), Some(&"Wonderful"), "{composer:?}: {lines:?}");
        for li in &lines {
            assert!(li.split(' ').all(|w| text.split(' ').any(|word| word == w)), "{composer:?}: {lines:?}");
        }
    }
    // No line of this triangle is as wide as the word: it is still broken, and every letter is set.
    let word = "Supercalifragilisticexpialidocious";
    let l = layout(db(), &area_path(word, style(12.0), &triangle(60.0, 400.0), Justify::Left));
    assert!(!l.overflow && l.lines.len() > 1, "{:?}", line_places(&l));
}

/// Lays `t` out as [`layout`] does, on a thread of its own, and fails the test when that panics or
/// takes more than 20 s.
fn layout_soon(t: &TextObject) -> TextLayout {
    let (tx, rx) = std::sync::mpsc::channel();
    let (t, text) = (t.clone(), t.plain_text());
    std::thread::spawn(move || tx.send(layout(db(), &t)).ok());
    rx.recv_timeout(std::time::Duration::from_secs(20)).unwrap_or_else(|e| panic!("laying out {text:?}: {e}"))
}

/// Each line's baseline, left and right edges, and byte range.
fn line_places(l: &TextLayout) -> Vec<(f64, f64, f64, usize, usize)> {
    l.lines.iter().map(|li| (li.baseline, li.x0, li.x1, li.start, li.end)).collect()
}

/// A frame or text wrap shape with a coordinate beyond a billion points either way, or one that
/// isn't finite, is left out: the frame holds no text, and the wrap shape is ignored.
#[test]
fn frames_and_wrap_shapes_beyond_a_billion_points_are_left_out() {
    // A box from x0 to x1 and from 0 to `height` whose top edge has a control point at x = `far`,
    // on the edge's own line.
    let boxed = |x0: f64, x1: f64, height: f64, far: f64| {
        let mut p = BezPath::new();
        p.move_to((x0, 0.0));
        p.curve_to((x0, 0.0), (far, 0.0), (x1, 0.0));
        p.line_to((x1, height));
        p.line_to((x0, height));
        p.close_path();
        p
    };
    let rectangle = layout(db(), &area("Hi there", style(12.0), Rect::new(0.0, 0.0, 100.0, 40.0), Justify::Left));
    let within = layout_soon(&area_path("Hi there", style(12.0), &boxed(0.0, 100.0, 40.0, 1e9), Justify::Left));
    assert_eq!(line_places(&within), line_places(&rectangle));
    let mut beyond = [8e115, 2e9, f64::INFINITY].map(|far| boxed(0.0, 100.0, 40.0, far)).to_vec();
    beyond.push(Rect::new(0.0, 0.0, 100.0, f64::NAN).to_path(0.1));
    for frame in &beyond {
        let l = layout_soon(&area_path("Hi there", style(12.0), frame, Justify::Left));
        assert!(l.lines.is_empty() && l.glyphs.is_empty() && l.overflow, "{frame:?}");
    }
    // Text wrapped around the right half of a 300 by 200 frame.
    let mut t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 300.0, 200.0), Justify::Left);
    let unwrapped = layout(db(), &t);
    let mut wrapped = |path: BezPath| {
        t.wrap = vec![vectorcraft_doc::WrapShape { path: PathData::from_bezpath(&path), wrap: Default::default() }];
        layout_soon(&t)
    };
    let half = wrapped(Rect::new(150.0, 0.0, 300.0, 200.0).to_path(0.1));
    assert_ne!(line_places(&half), line_places(&unwrapped));
    assert_eq!(line_places(&wrapped(boxed(150.0, 300.0, 200.0, 1e9))), line_places(&half));
    for far in [8e115, 2e9, f64::INFINITY] {
        assert_eq!(line_places(&wrapped(boxed(150.0, 300.0, 200.0, far))), line_places(&unwrapped), "{far}");
    }
    assert_eq!(line_places(&wrapped(Rect::new(150.0, 0.0, 300.0, f64::NAN).to_path(0.1))), line_places(&unwrapped));
}

/// Layout steps a line down a frame (or a row or column of one) at most 100,000 times to find room
/// for it: in a frame taller than 100,000 leads, a step is a 100,000th of the frame's height.
#[test]
fn tall_frames_are_searched_for_room_in_at_most_100000_steps() {
    // A triangle from y = -1e9 to 1e9, `base` wide at the bottom.
    let triangle = |base: f64| {
        let mut p = BezPath::new();
        p.move_to((0.0, -1e9));
        p.line_to((base / 2.0, 1e9));
        p.line_to((-base / 2.0, 1e9));
        p.close_path();
        p
    };
    // No line fits in this triangle: it is at most 0.5 pt wide, and a line needs a span wider than
    // 1 pt or its ascent, whichever is more. A lead of 0.5 pt type counts as 1 pt.
    let none = layout_soon(&area_path("Hi", style(0.5), &triangle(0.5), Justify::Left));
    assert!(none.lines.is_empty() && none.overflow);
    // This one is wide enough for 12 pt type from about y = -7.6e8 down.
    let some = layout_soon(&area_path("Hi", style(12.0), &triangle(100.0), Justify::Left));
    assert!(!some.overflow && some.lines.len() == 1);
    // At the height of the line's top, the line's left and right edges lie inside the triangle.
    let line = &some.lines[0];
    let half_width = (line.baseline - line.ascent + 1e9) / 2e9 * 50.0;
    assert!(line.x0 >= -half_width - 1e-6 && line.x1 <= half_width + 1e-6, "{line:?}");
}

/// A paragraph whose Space Before puts it 1e300 pt above a triangle frame that comes to a point at
/// its top overflows. At that height a step of one lead leaves the baseline where it is, and
/// layout takes at most 100,000 such steps.
#[test]
fn a_paragraph_spaced_far_above_its_frame_overflows() {
    let mut triangle = BezPath::new();
    triangle.move_to((0.0, 0.0));
    triangle.line_to((500.0, 1000.0));
    triangle.line_to((-500.0, 1000.0));
    triangle.close_path();
    let mut t = area_path("Hi\nthere", style(12.0), &triangle, Justify::Left);
    let far = vectorcraft_doc::ParaStyle { space_before: -1e300, ..Default::default() };
    t.set_paragraph_styles(vec![t.para.clone(), far]);
    let l = layout_soon(&t);
    assert!(l.overflow && l.lines.len() == 1, "{:?}", line_places(&l));
}

/// Type on a path whose path has a coordinate beyond a billion points either way, or one that isn't
/// finite, holds no text.
#[test]
fn type_on_a_path_beyond_a_billion_points_holds_no_text() {
    // An S-shaped curve from (0, 0) to (3s, 0) with control points (s, s) and (2s, -s).
    let on = |s: f64| {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.curve_to((s, s), (2.0 * s, -s), (3.0 * s, 0.0));
        let mut t = TextObject::point(Point::ZERO, "On a path", style(8.0));
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&p), start: 0.0, end: None };
        t
    };
    let within = layout_soon(&on(1e9 / 3.0));
    assert!(!within.overflow && within.glyphs.len() == 9);
    for s in [1e115, 1e9, f64::INFINITY] {
        let l = layout_soon(&on(s));
        assert!(l.glyphs.is_empty() && l.overflow, "{s}");
    }
}

#[test]
fn justify_all_line_widths_equal() {
    let t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 240.0, 800.0), Justify::JustifyAll);
    let l = layout(db(), &t);
    assert!(l.lines.len() > 4);
    for li in &l.lines {
        assert!((li.x1 - li.x0 - 240.0).abs() < 1e-6, "{li:?}");
    }
}

#[test]
fn composer_lines_never_exceed_frame() {
    for (hy, composer) in [(false, Composer::EveryLine), (true, Composer::EveryLine), (false, Composer::SingleLine), (true, Composer::SingleLine)] {
        let mut t = area(&COPY.repeat(2), style(12.0), Rect::new(0.0, 0.0, 180.0, 2000.0), Justify::JustifyLeft);
        t.para.hyphenate = hy;
        let l = layout_with(db(), &t, &LayoutOptions { composer: Some(composer), ..Default::default() });
        assert!(l.lines.len() > 10);
        for (i, li) in l.lines.iter().enumerate() {
            assert!(li.x1 - li.x0 <= 180.0 + 1e-6, "{composer:?} hy={hy} line {i} too wide: {li:?}");
            assert!(li.x0 >= -1e-6);
            if i + 1 < l.lines.len() {
                assert!((li.x1 - li.x0 - 180.0).abs() < 1e-6, "{composer:?} hy={hy} justified line {i}: {li:?}");
            }
        }
        let clusters: std::collections::BTreeSet<(usize, usize)> = l.glyphs.iter().filter(|g| g.len > 0).map(|g| (g.byte, g.len)).collect();
        assert_eq!(clusters.iter().map(|c| c.1).sum::<usize>(), COPY.len() * 2, "every character placed");
    }
}

/// Every-line composition spreads the spacing more evenly than greedy breaking.
#[test]
fn every_line_composer_is_more_even() {
    let t = area(&COPY.repeat(2), style(12.0), Rect::new(0.0, 0.0, 190.0, 2000.0), Justify::JustifyLeft);
    let spread = |c: Composer| {
        let l = layout_with(db(), &t, &LayoutOptions { composer: Some(c), ..Default::default() });
        // Worst word-space stretch (space glyph advance) over all justified lines.
        let mut worst: f64 = 0.0;
        for li in &l.lines[..l.lines.len() - 1] {
            for g in &l.glyphs[li.glyph_start..li.glyph_end] {
                if COPY.as_bytes().get(g.byte % COPY.len()) == Some(&b' ') {
                    worst = worst.max(g.advance);
                }
            }
        }
        worst
    };
    let (kp, greedy) = (spread(Composer::EveryLine), spread(Composer::SingleLine));
    assert!(kp <= greedy + 1e-6, "every-line worst space {kp} vs greedy {greedy}");
}

/// Burasagari (new type's Standard) leaves Latin paragraphs to the every-line composer: only a
/// paragraph with a Japanese comma or full stop is composed line by line.
#[test]
fn burasagari_keeps_the_every_line_composer_for_latin_text() {
    // A measure where the two composers break the copy differently.
    let mut t = area(&COPY.repeat(2), style(12.0), Rect::new(0.0, 0.0, 300.0, 4000.0), Justify::JustifyLeft);
    let ends = |t: &TextObject, c: Composer| {
        let l = layout_with(db(), t, &LayoutOptions { composer: Some(c), ..Default::default() });
        l.lines.iter().map(|li| li.glyph_end).collect::<Vec<_>>()
    };
    let every_line = ends(&t, Composer::EveryLine);
    assert_ne!(every_line, ends(&t, Composer::SingleLine));
    t.para.burasagari = vectorcraft_doc::Burasagari::Standard;
    assert_eq!(ends(&t, Composer::EveryLine), every_line);
}

/// Width of `s` set on one line.
fn natural_width(s: &str, size: f64) -> f64 {
    let l = layout(db(), &TextObject::point(Point::ZERO, s, style(size)));
    l.lines[0].x1 - l.lines[0].x0
}

/// The text of each line.
fn line_texts(t: &TextObject, l: &TextLayout) -> Vec<String> {
    let text = t.plain_text();
    l.lines.iter().map(|li| text[li.start..li.end].trim_end().to_string()).collect()
}

/// Ragged text: Every-line breaks before a word that would fit, to avoid leaving a lone short
/// word on the last line (single-line keeps filling the first line). Auto alignment is ragged
/// too, in either paragraph direction: set right to left, the lines break the same and stay
/// flush right.
#[test]
fn every_line_composer_balances_ragged_lines() {
    use vectorcraft_doc::ParaDirection;
    const S: &str = "Destroy all creatures with toughness X or less.";
    let w = natural_width("Destroy all creatures with toughness X or", 12.0) + 2.0;
    assert!(w < natural_width(S, 12.0), "the sentence needs two lines");
    let cases = [
        (Justify::Left, None),
        (Justify::Center, None),
        (Justify::Right, None),
        (Justify::Auto, None),
        (Justify::Auto, Some(ParaDirection::RightToLeft)),
        (Justify::Left, Some(ParaDirection::RightToLeft)),
    ];
    for (justify, direction) in cases {
        let mut t = area(S, style(12.0), Rect::new(0.0, 0.0, w, 400.0), justify);
        t.para.direction = direction;
        t.para.composer = Composer::SingleLine;
        let greedy = line_texts(&t, &layout(db(), &t));
        assert_eq!(greedy, ["Destroy all creatures with toughness X or", "less."], "{justify:?} {direction:?}");
        t.para.composer = Composer::EveryLine;
        let every = line_texts(&t, &layout(db(), &t));
        assert_eq!(every, ["Destroy all creatures with toughness X", "or less."], "{justify:?} {direction:?}");
        // Burasagari (new type's Standard) leaves Latin text to the every-line composer.
        t.para.burasagari = vectorcraft_doc::Burasagari::Standard;
        assert_eq!(line_texts(&t, &layout(db(), &t)), every, "{justify:?} {direction:?} with burasagari");
        if (justify, direction) == (Justify::Auto, Some(ParaDirection::RightToLeft)) {
            let l = layout(db(), &t);
            assert!(l.lines.iter().all(|li| li.rtl && (li.x1 - w).abs() < 1e-6), "right to left Auto is flush right: {:?}", l.lines);
        }
        // The options override wins over the object's setting.
        let forced = layout_with(db(), &t, &LayoutOptions { composer: Some(Composer::SingleLine), ..Default::default() });
        assert_eq!(line_texts(&t, &forced), greedy);
    }
}

/// Every-line evens the rag of a long ragged paragraph: the spread of the line ends is no worse
/// than greedy breaking, every line fits, and nothing is lost.
#[test]
fn every_line_composer_evens_the_rag() {
    for (width, hy) in [(150.0, false), (190.0, false), (240.0, true), (310.0, false)] {
        let mut t = area(&COPY.repeat(2), style(12.0), Rect::new(0.0, 0.0, width, 4000.0), Justify::Left);
        t.para.hyphenate = hy;
        let rag = |c: Composer| {
            let mut t = t.clone();
            t.para.composer = c;
            let l = layout(db(), &t);
            assert!(!l.overflow);
            for li in &l.lines {
                assert!(li.x1 - li.x0 <= width + 1e-6, "{c:?} {width}: line too wide {li:?}");
            }
            let clusters: std::collections::BTreeSet<(usize, usize)> = l.glyphs.iter().filter(|g| g.len > 0).map(|g| (g.byte, g.len)).collect();
            assert_eq!(clusters.iter().map(|c| c.1).sum::<usize>(), COPY.len() * 2, "every character placed");
            // Leftover space of all lines but the last: its sum of squares (how ragged) and the
            // sum of squared differences between neighbours (how jagged).
            let slack: Vec<f64> = l.lines[..l.lines.len() - 1].iter().map(|li| width - (li.x1 - li.x0)).collect();
            let jag: f64 = slack.windows(2).map(|p| (p[0] - p[1]).powi(2)).sum();
            (slack.iter().map(|s| s * s).sum::<f64>(), jag, l.lines.len())
        };
        let ((kp, kp_jag, kp_lines), (greedy, greedy_jag, greedy_lines)) = (rag(Composer::EveryLine), rag(Composer::SingleLine));
        assert!(kp <= greedy + 1e-6, "{width}: every-line rag {kp} vs greedy {greedy}");
        assert!(kp_jag <= greedy_jag + 1e-6, "{width}: every-line jag {kp_jag} vs greedy {greedy_jag}");
        assert!(kp_lines <= greedy_lines + 1, "{width}: {kp_lines} lines vs {greedy_lines}");
    }
}

/// Single-line breaking is the greedy breaker: each line takes as many words as fit.
#[test]
fn single_line_composer_is_greedy() {
    let width = 190.0;
    let mut t = area(COPY, style(12.0), Rect::new(0.0, 0.0, width, 2000.0), Justify::Left);
    t.para.composer = Composer::SingleLine;
    let l = layout(db(), &t);
    let lines = line_texts(&t, &l);
    for pair in lines.windows(2) {
        let next_word = pair[1].split(' ').next().unwrap();
        assert!(natural_width(&format!("{} {next_word}", pair[0]), 12.0) > width - 1e-6, "{pair:?}");
    }
}

#[test]
fn hyphenation_rules() {
    assert_eq!(hyphen::hyphenate_word("typography"), "typo-gra-phy");
    assert_eq!(hyphen::hyphenate_word("happen"), "hap-pen");
    assert_eq!(hyphen::hyphenate_word("hyphenation"), "hyphe-na-tion");
    assert!(hyphen::hyphen_points("short").is_empty());
    assert!(hyphen::hyphen_points("NASAJPL").is_empty(), "all caps stay whole");
    assert!(hyphen::hyphen_points("abc123def").is_empty());
    for w in ["justification", "arrangement", "paragraph", "legible"] {
        let h = hyphen::hyphenate_word(w);
        for part in h.split('-') {
            assert!(part.chars().count() >= 2, "{h}");
        }
        let pts = hyphen::hyphen_points(w);
        assert!(pts.iter().all(|&p| p >= hyphen::MIN_BEFORE && p <= w.chars().count() - hyphen::MIN_AFTER), "{w}: {pts:?}");
    }
}

#[test]
fn hyphenated_layout_adds_hyphens() {
    let text = "Extraordinarily comprehensive typographical considerations notwithstanding everything.";
    let mut t = area(text, style(14.0), Rect::new(0.0, 0.0, 120.0, 600.0), Justify::Left);
    let plain = layout(db(), &t);
    t.para.hyphenate = true;
    let hy = layout(db(), &t);
    let hyphens = |l: &TextLayout| l.glyphs.iter().filter(|g| g.len == 0).count();
    assert_eq!(hyphens(&plain), 0);
    assert!(hyphens(&hy) > 0, "some words hyphenated");
    for li in &hy.lines {
        assert!(li.x1 - li.x0 <= 120.0 + 1e-6, "{li:?}");
    }
    // Hyphenation fills lines better: no more lines than without it.
    assert!(hy.lines.len() <= plain.lines.len());
    // The hyphen glyph is the last glyph of its line and has an outline.
    for g in hy.glyphs.iter().filter(|g| g.len == 0) {
        let li = &hy.lines[g.line];
        assert_eq!(li.glyph_end - 1, hy.glyphs.iter().position(|x| std::ptr::eq(x, g)).unwrap());
        assert!(!g.outline.elements().is_empty());
    }
}

/// Preferences › Hyphenation › Exceptions (#394): listing a word stops the layout from breaking it.
#[test]
fn hyphenation_exceptions_keep_listed_words_whole_in_layout() {
    // Same copy as `hyphenated_layout_adds_hyphens` (known to produce soft hyphens at 120pt width).
    let text = "Extraordinarily comprehensive typographical considerations notwithstanding everything.";
    let mut t = area(text, style(14.0), Rect::new(0.0, 0.0, 120.0, 600.0), Justify::Left);
    t.para.hyphenate = true;
    let with_breaks = layout(db(), &t);
    let hyphens = |l: &TextLayout| l.glyphs.iter().filter(|g| g.len == 0).count();
    assert!(hyphens(&with_breaks) > 0, "pattern produces soft hyphens");
    hyphen::with_hyphenation_exceptions_for_test(
        "extraordinarily, comprehensive, typographical, considerations, notwithstanding, everything",
        || {
            let blocked = layout(db(), &t);
            assert_eq!(hyphens(&blocked), 0, "every long word listed: no soft hyphens");
            assert!(blocked.lines.len() >= with_breaks.lines.len());
        },
    );
}

/// The width of the frame [`the_language`] makes: "The lan-" and "language" fit, and "The
/// language" does not.
fn the_language_width() -> f64 {
    natural_width("The lan-", 12.0).max(natural_width("Language", 12.0)) + 1.0
}

/// "The language" (or "The Language") in a frame that holds "The lan-" and not the whole of it,
/// hyphenated.
fn the_language(word: &str, composer: Composer) -> TextObject {
    let text = format!("The {word}");
    let w = the_language_width();
    assert!(w < natural_width(&text, 12.0));
    let mut t = area(&text, style(12.0), Rect::new(0.0, 0.0, w, 400.0), Justify::Left);
    t.para.hyphenate = true;
    t.para.composer = composer;
    t
}

/// Paragraph panel menu › Hyphenation… › Words Longer Than: an 8-letter word that breaks with
/// the default (longer than 5 letters) stays whole when only words longer than 10 letters may
/// break.
#[test]
fn hyphenation_words_longer_than_keeps_shorter_words_whole() {
    let mut t = the_language("language", Composer::SingleLine);
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The lan", "guage"]);
    t.para.hyphenation.words_longer_than = 10;
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The", "language"]);
    let mut t = the_language("language", Composer::EveryLine);
    t.para.hyphenation.words_longer_than = 10;
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The", "language"]);
}

/// Hyphenation… › After First and Before Last: the fewest letters on each side of the hyphen.
#[test]
fn hyphenation_after_first_and_before_last_limit_the_break() {
    let mut t = the_language("language", Composer::SingleLine);
    t.para.hyphenation.after_first = 4;
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The", "language"]);
    t.para.hyphenation.after_first = 3;
    t.para.hyphenation.before_last = 6;
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The", "language"]);
}

/// Hyphenation… › Hyphenate Capitalized Words: when it is off, a word that starts with a capital stays whole.
#[test]
fn hyphenation_leaves_capitalized_words_whole_when_off() {
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let mut t = the_language("Language", composer);
        if composer == Composer::SingleLine {
            assert_eq!(line_texts(&t, &layout(db(), &t)), ["The Lan", "guage"]);
        }
        t.para.hyphenation.capitalized = false;
        assert_eq!(line_texts(&t, &layout(db(), &t)), ["The", "Language"], "{composer:?}");
    }
}

/// Hyphenation… › Hyphenation Zone: with the Single-line Composer, a line whose text before the
/// next word already ends within the zone breaks before that word; the Every-line Composer
/// ignores the zone.
#[test]
fn hyphenation_zone_breaks_before_the_word_with_the_single_line_composer() {
    let mut t = the_language("language", Composer::SingleLine);
    let gap = the_language_width() - natural_width("The", 12.0);
    t.para.hyphenation.zone = gap - 1.0;
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The lan", "guage"]);
    t.para.hyphenation.zone = gap + 1.0;
    assert_eq!(line_texts(&t, &layout(db(), &t)), ["The", "language"]);
    let mut every = the_language("language", Composer::EveryLine);
    let without = line_texts(&every, &layout(db(), &every));
    every.para.hyphenation.zone = gap + 1.0;
    assert_eq!(line_texts(&every, &layout(db(), &every)), without);
}

/// The default Hyphenation options keep the line breaks of hyphenated text: the copy twice over
/// at three widths, ragged and justified, with either composer. The constant is an FNV-1a hash of
/// each line's end and whether it ends with a hyphen. It changed once since: justified lines
/// follow the default Justification options (word spacing up to 133%, where the composer used
/// to stretch spaces to 150%), and the every-line composer no longer gives up on loose lines.
#[test]
fn default_hyphenation_options_keep_the_line_breaks() {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| {
        for b in v.to_le_bytes() {
            hash = (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    };
    for width in [150.0, 190.0, 240.0] {
        for justify in [Justify::Left, Justify::JustifyLeft] {
            for composer in [Composer::SingleLine, Composer::EveryLine] {
                let mut t = area(&COPY.repeat(2), style(12.0), Rect::new(0.0, 0.0, width, 4000.0), justify);
                t.para.hyphenate = true;
                t.para.composer = composer;
                let l = layout(db(), &t);
                for li in &l.lines {
                    let hyphenated = li.glyph_end > li.glyph_start && l.glyphs.get(li.glyph_end - 1).is_some_and(|g| g.len == 0);
                    mix(li.end as u64 * 2 + u64::from(hyphenated));
                }
            }
        }
    }
    assert_eq!(hash, 4_188_642_633_634_327_633);
}

/// The longest run of consecutive lines of `l` that end with a hyphen.
fn hyphen_run(l: &TextLayout) -> usize {
    let mut run = 0;
    let mut longest = 0;
    for li in &l.lines {
        let hyphenated = li.glyph_end > li.glyph_start && l.glyphs.get(li.glyph_end - 1).is_some_and(|g| g.len == 0);
        run = if hyphenated { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

/// Hyphenation… › Hyphen Limit: at most that many lines in a row end with a hyphen, with either
/// composer; 0 sets no limit.
#[test]
fn hyphen_limit_caps_consecutive_hyphenated_lines() {
    let text = "Extraordinarily comprehensive typographical considerations notwithstanding everything. ".repeat(3);
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        for justify in [Justify::Left, Justify::JustifyLeft] {
            let mut t = area(text.trim_end(), style(14.0), Rect::new(0.0, 0.0, 120.0, 2000.0), justify);
            t.para.hyphenate = true;
            t.para.composer = composer;
            let free = layout(db(), &t);
            assert!(hyphen_run(&free) >= 2, "{composer:?} {justify:?}: lines in a row end with hyphens without a limit");
            for limit in [1, 2] {
                t.para.hyphenation.hyphen_limit = limit;
                let l = layout(db(), &t);
                assert!(!l.overflow);
                assert!(hyphen_run(&l) >= 1, "{composer:?} {justify:?} {limit}: still hyphenated");
                assert!(hyphen_run(&l) <= limit as usize, "{composer:?} {justify:?} {limit}: {}", hyphen_run(&l));
            }
        }
    }
}

#[test]
fn soft_hyphen_is_invisible_mid_line() {
    let l = layout(db(), &TextObject::point(Point::ZERO, "co\u{00AD}operate", style(20.0)));
    let sh = l.glyphs.iter().find(|g| g.byte == 2).unwrap();
    assert!(sh.outline.elements().is_empty());
    assert_eq!(sh.advance, 0.0);
}

// ---------- area type options ----------

#[test]
fn inset_and_first_baseline_options() {
    let frame = Rect::new(0.0, 0.0, 300.0, 300.0);
    let t = area(COPY, style(12.0), frame, Justify::Left);
    let base = layout(db(), &t);
    let inset = layout_with(db(), &t, &LayoutOptions { inset: 10.0, ..Default::default() });
    for li in &inset.lines {
        assert!(li.x0 >= 10.0 - 1e-6 && li.x1 <= 290.0 + 1e-6);
    }
    assert!((inset.lines[0].baseline - base.lines[0].baseline - 10.0).abs() < 1e-6);
    let lead = layout_with(db(), &t, &LayoutOptions { first_baseline: FirstBaseline::Leading, ..Default::default() });
    assert!((lead.lines[0].baseline - 14.4).abs() < 1e-6, "{}", lead.lines[0].baseline);
    let fixed = layout_with(db(), &t, &LayoutOptions { first_baseline: FirstBaseline::Fixed, first_baseline_min: 30.0, ..Default::default() });
    assert!((fixed.lines[0].baseline - 30.0).abs() < 1e-6);
    let cap = layout_with(db(), &t, &LayoutOptions { first_baseline: FirstBaseline::CapHeight, ..Default::default() });
    assert!(cap.lines[0].baseline < base.lines[0].baseline && cap.lines[0].baseline > 5.0);
}

/// `LineInfo::first_baseline`: each line that starts a column has its baseline that far below the
/// column's top inside the inset, for every first baseline option, with and without a minimum and
/// an inset, in both leading models, and for lines of several sizes (each line in one size).
#[test]
fn lines_starting_a_column_sit_at_their_first_baseline() {
    let paras = [
        ("A heading in large type\n", 24.0),
        ("Body copy in a smaller size that runs over two or three lines of a column.\n", 12.0),
        ("A middle size\n", 18.0),
    ];
    let runs: Vec<TextRun> = paras.repeat(4).into_iter().map(|(text, size)| run(text, size)).collect();
    let text: String = runs.iter().map(|r| r.text.as_str()).collect();
    let mut other_sizes = 0;
    for model in [vectorcraft_doc::LeadingModel::RomanBaseline, vectorcraft_doc::LeadingModel::EmBoxTop] {
        for first_baseline in [FirstBaseline::Ascent, FirstBaseline::CapHeight, FirstBaseline::XHeight, FirstBaseline::Leading, FirstBaseline::Fixed]
        {
            for (first_baseline_min, inset) in [(0.0, 0.0), (18.0, 6.0)] {
                for height in [100.0, 120.0, 140.0] {
                    let mut t = area(&text, style(12.0), Rect::new(0.0, 0.0, 420.0, height), Justify::Left);
                    t.runs = runs.clone();
                    t.para.leading_model = model;
                    let opts = LayoutOptions { columns: 3, gutter: 10.0, first_baseline, first_baseline_min, inset, ..Default::default() };
                    let l = layout_with(db(), &t, &opts);
                    let starts: Vec<&LineInfo> =
                        l.lines.iter().enumerate().filter(|(i, li)| *i == 0 || l.lines[i - 1].region != li.region).map(|(_, li)| li).collect();
                    assert_eq!(starts.len(), 3, "{model:?} {first_baseline:?} {height}");
                    other_sizes += starts.iter().filter(|li| li.ascent != starts[0].ascent).count();
                    for li in starts {
                        assert!(
                            (li.baseline - (inset + li.first_baseline)).abs() < 1e-9,
                            "{model:?} {first_baseline:?} {first_baseline_min} {height}: {li:?}"
                        );
                    }
                }
            }
        }
    }
    assert!(other_sizes >= 20, "lines of another size than the first start {other_sizes} columns");
}

#[test]
fn rows_and_columns_flow_in_order() {
    let text = COPY.repeat(2);
    let t = area(&text, style(12.0), Rect::new(0.0, 0.0, 400.0, 300.0), Justify::Left);
    let l = layout_with(db(), &t, &LayoutOptions { rows: 2, columns: 2, gutter: 10.0, ..Default::default() });
    assert_eq!(l.frames.len(), 4);
    // Text order: column 1 (top then bottom cell), then column 2.
    let first_col2 = l.lines.iter().position(|li| li.x0 > 200.0).expect("reaches column 2");
    assert!(l.lines[..first_col2].iter().any(|li| li.baseline > 155.0), "fills the lower cell of column 1 first");
    for w in l.lines.windows(2) {
        assert!(w[1].start >= w[0].start);
    }
}

/// Layout clamps area type's rows and columns to 100 each: 2^60 rows or columns with 100 of the
/// other, 2^32 of each, and `usize::MAX` of each lay out as 100 rows and 100 columns.
#[test]
fn area_type_lays_out_in_at_most_100_rows_and_100_columns() {
    // At 100 by 100, a cell is about 98 by 28 pt.
    let with = |rows: usize, columns: usize| {
        let mut t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 10_000.0, 3_000.0), Justify::Left);
        t.area.rows = rows;
        t.area.columns = columns;
        t.area.gutter = 2.0;
        layout_soon(&t)
    };
    let most = with(100, 100);
    assert_eq!(most.frames.len(), 10_000);
    assert!(!most.overflow && most.lines.len() > 10, "{:?}", line_places(&most));
    for (rows, columns) in [(1 << 60, 100), (100, 1 << 60), (1 << 32, 1 << 32), (usize::MAX, usize::MAX)] {
        let l = with(rows, columns);
        assert_eq!((&l.frames, line_places(&l)), (&most.frames, line_places(&most)), "{rows} by {columns}");
    }
}

// ---------- vertical alignment ----------

fn valign(a: VerticalAlign) -> LayoutOptions {
    LayoutOptions { vertical_align: a, ..Default::default() }
}

/// (space above the first line's ascent, space below the last line's descent) in `0..h`.
fn block_space(l: &TextLayout, h: f64) -> (f64, f64) {
    let (first, last) = (l.lines.first().unwrap(), l.lines.last().unwrap());
    (first.baseline - first.ascent, h - (last.baseline + last.descent))
}

#[test]
fn vertical_align_center_bottom_and_justify_in_a_rectangle() {
    let t = area("First line\nSecond line\nThird line", style(12.0), Rect::new(0.0, 0.0, 200.0, 300.0), Justify::Left);
    let top = layout(db(), &t);
    let (above, below) = block_space(&top, 300.0);
    assert!(above.abs() < 1e-6 && below > 200.0);
    let center = layout_with(db(), &t, &valign(VerticalAlign::Center));
    let (a, b) = block_space(&center, 300.0);
    assert!((a - b).abs() < 1e-6 && a > 100.0, "{a} {b}");
    // The model field reaches layout().
    let mut tc = t.clone();
    tc.area.vertical_align = VerticalAlign::Center;
    assert!((layout(db(), &tc).lines[0].baseline - center.lines[0].baseline).abs() < 1e-9);
    // Lines, glyph origins, outlines and transforms move together.
    let d = center.lines[0].baseline - top.lines[0].baseline;
    for (g, h) in center.glyphs.iter().zip(&top.glyphs) {
        assert!((g.origin.y - h.origin.y - d).abs() < 1e-9);
        assert!((g.xf.translation().y - h.xf.translation().y - d).abs() < 1e-9);
        if !h.outline.elements().is_empty() {
            assert!((g.outline.bounding_box().y0 - h.outline.bounding_box().y0 - d).abs() < 1e-6);
        }
    }
    assert!(center.bounds.y0 > top.bounds.y0 + 100.0);
    let bottom = layout_with(db(), &t, &valign(VerticalAlign::Bottom));
    let (a, b) = block_space(&bottom, 300.0);
    assert!(b.abs() < 1e-6 && a > 200.0);
    // Justify: the first line stays, the last one reaches the bottom, gaps are equal.
    let just = layout_with(db(), &t, &valign(VerticalAlign::Justify));
    let (a, b) = block_space(&just, 300.0);
    assert!(a.abs() < 1e-6 && b.abs() < 1e-6, "{a} {b}");
    let gaps: Vec<f64> = just.lines.windows(2).map(|w| w[1].baseline - w[0].baseline).collect();
    assert!(gaps.len() == 2 && (gaps[0] - gaps[1]).abs() < 1e-6 && gaps[0] > 100.0);
    // A single line justifies to the top.
    let one = area("Alone", style(12.0), Rect::new(0.0, 0.0, 200.0, 300.0), Justify::Left);
    assert_eq!(layout_with(db(), &one, &valign(VerticalAlign::Justify)).lines[0].baseline, layout(db(), &one).lines[0].baseline);
    // Inset: the space is measured inside it.
    let inset = layout_with(db(), &t, &LayoutOptions { inset: 10.0, vertical_align: VerticalAlign::Bottom, ..Default::default() });
    assert!(block_space(&inset, 290.0).1.abs() < 1e-6);
}

#[test]
fn vertical_align_leaves_overflowing_text_where_it_is() {
    let text = COPY.repeat(4);
    let t = area(&text, style(12.0), Rect::new(0.0, 0.0, 200.0, 100.0), Justify::Left);
    let top = layout(db(), &t);
    assert!(top.overflow);
    for a in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
        let l = layout_with(db(), &t, &valign(a));
        assert!(l.overflow);
        assert_eq!(l.lines.len(), top.lines.len());
        assert_eq!(l.lines.last().unwrap().end, top.lines.last().unwrap().end);
        // A full cell has less than a line of space left: the lines barely move.
        assert!(l.lines[0].baseline - top.lines[0].baseline < top.lines[0].ascent + top.lines[0].descent);
        assert!(block_space(&l, 100.0).1 >= -1e-6);
    }
}

#[test]
fn vertical_align_acts_on_each_column_on_its_own() {
    // Enough text for column 1 and a few lines of column 2.
    let t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 420.0, 120.0), Justify::Left);
    let opts = |a| LayoutOptions { columns: 2, gutter: 20.0, vertical_align: a, ..Default::default() };
    let top = layout_with(db(), &t, &opts(VerticalAlign::Top));
    let col = |l: &TextLayout, c: usize| l.lines.iter().filter(|li| li.region == c).cloned().collect::<Vec<_>>();
    assert!(!top.overflow && !col(&top, 0).is_empty() && !col(&top, 1).is_empty());
    assert!(col(&top, 1).iter().all(|li| li.x0 >= 220.0 - 1e-6));
    let bottom = layout_with(db(), &t, &opts(VerticalAlign::Bottom));
    for c in 0..2 {
        let last = col(&bottom, c).last().cloned().unwrap();
        assert!((last.baseline + last.descent - 120.0).abs() < 1e-6, "column {c} ends at the bottom");
    }
    let shift = |c: usize| col(&bottom, c)[0].baseline - col(&top, c)[0].baseline;
    assert!(shift(1) > shift(0) + 20.0, "the short column moves further: {} {}", shift(0), shift(1));
}

#[test]
fn vertical_align_in_a_circle_reflows_and_stays_inside() {
    let circle = Circle::new((150.0, 150.0), 120.0).to_path(0.1);
    let t = area_path("Centred text in a round frame flows again at its new height.", style(12.0), &circle, Justify::Left);
    let top = layout(db(), &t);
    for a in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
        let l = layout_with(db(), &t, &valign(a));
        assert!(!l.overflow, "{a:?}");
        assert_eq!(l.lines.last().unwrap().end, top.lines.last().unwrap().end);
        let moved = if a == VerticalAlign::Justify {
            l.lines.last().unwrap().baseline - top.lines.last().unwrap().baseline
        } else {
            l.lines[0].baseline - top.lines[0].baseline
        };
        assert!(moved > 20.0, "{a:?} {moved}");
        for g in l.glyphs.iter().filter(|g| !g.outline.elements().is_empty()) {
            let b = g.outline.bounding_box();
            for p in [Point::new(b.x0, b.y0), Point::new(b.x1, b.y1)] {
                assert!((p - Point::new(150.0, 150.0)).hypot() <= 121.0, "{a:?} {p:?}");
            }
        }
    }
    let c = layout_with(db(), &t, &valign(VerticalAlign::Center));
    let (above, below) = block_space(&c, 270.0);
    assert!((above - 30.0 - below).abs() < 10.0, "roughly centred: {above} {below}");
}

#[test]
fn vertical_align_with_text_wrap_flows_again_around_the_object() {
    let mut t = area(COPY, style(12.0), Rect::new(0.0, 0.0, 300.0, 400.0), Justify::Left);
    let obstacle = Rect::new(100.0, 250.0, 200.0, 330.0).to_path(0.1);
    t.wrap = vec![vectorcraft_doc::WrapShape { path: PathData::from_bezpath(&obstacle), wrap: Default::default() }];
    let top = layout(db(), &t);
    let bottom = layout_with(db(), &t, &valign(VerticalAlign::Bottom));
    assert!(!bottom.overflow);
    assert_eq!(bottom.lines.last().unwrap().end, top.lines.last().unwrap().end);
    let last = bottom.lines.last().unwrap();
    assert!(400.0 - (last.baseline + last.descent) < 15.0, "near the bottom: {}", last.baseline);
    // Lines beside the object leave room for it (offset 6 pt).
    for li in bottom.lines.iter().filter(|li| li.baseline + li.descent > 244.0 && li.baseline - li.ascent < 336.0) {
        assert!(li.x1 <= 94.0 + 1e-6 || li.x0 >= 206.0 - 1e-6, "{li:?}");
    }
}

#[test]
fn vertical_align_on_vertical_type_moves_columns_along_the_block_axis() {
    // Vertical type: the block axis runs right to left, so "bottom" is the frame's left edge.
    let mut t = area("Tate\nYoko", style(20.0), Rect::new(0.0, 0.0, 200.0, 100.0), Justify::Left);
    t.vertical = true;
    let top = layout(db(), &t);
    let bottom = layout_with(db(), &t, &valign(VerticalAlign::Bottom));
    let center = layout_with(db(), &t, &valign(VerticalAlign::Center));
    assert!(top.bounds.x1 > 190.0 && top.bounds.x0 > 100.0, "{:?}", top.bounds);
    assert!(bottom.bounds.x0.abs() < 0.5, "{:?}", bottom.bounds);
    assert!((center.bounds.x0 - (200.0 - center.bounds.x1)).abs() < 0.5, "{:?}", center.bounds);
    // Glyphs stay inside the frame; lines keep line-space coordinates.
    for g in &bottom.glyphs {
        assert!(g.origin.x >= -1e-6 && g.origin.x <= 200.0 + 1e-6);
    }
}

/// Top-to-Top leading (em box top to em box top): the first line hangs from the frame's top and a
/// line's leading is the space below it. Vertical alignment measures the space from the lines that
/// model placed, keeps their spacing when it shifts them, and justify adds its share on top of it.
#[test]
fn vertical_align_with_top_to_top_leading() {
    use vectorcraft_doc::LeadingModel;
    let st = |size: f64| CharStyle { size, leading: Some(size * 1.5), ..style(size) };
    let mut t = area("", st(40.0), Rect::new(0.0, 0.0, 200.0, 300.0), Justify::Left);
    t.runs = vec![TextRun { text: "大\n".into(), style: st(40.0), inline: None }, TextRun { text: "小\n中".into(), style: st(20.0), inline: None }];
    t.para.leading_model = LeadingModel::EmBoxTop;
    let top = layout(db(), &t);
    assert_eq!(top.lines.len(), 3);
    // The em box top sits 0.88 em above the baseline in the craft-fonts CJK faces; a system
    // fallback (no craft-fonts) has its own metrics, which the relative checks below don't need.
    if !crate::CRAFT_FONTS.is_empty() {
        assert!((top.lines[0].baseline - 0.88 * 40.0).abs() < 0.01, "first em box at the top: {}", top.lines[0].baseline);
    }
    let gaps = |l: &TextLayout| l.lines.windows(2).map(|w| w[1].baseline - w[0].baseline).collect::<Vec<_>>();
    let bottom = layout_with(db(), &t, &valign(VerticalAlign::Bottom));
    assert!(block_space(&bottom, 300.0).1.abs() < 1e-6, "{:?}", block_space(&bottom, 300.0));
    for (a, b) in gaps(&bottom).iter().zip(gaps(&top)) {
        assert!((a - b).abs() < 1e-6, "spacing kept: {a} {b}");
    }
    let center = layout_with(db(), &t, &valign(VerticalAlign::Center));
    let d = center.lines[0].baseline - top.lines[0].baseline;
    assert!((d - block_space(&top, 300.0).1 * 0.5).abs() < 1e-6, "{d}");
    let just = layout_with(db(), &t, &valign(VerticalAlign::Justify));
    assert!((just.lines[0].baseline - top.lines[0].baseline).abs() < 1e-9, "the first line stays");
    assert!(block_space(&just, 300.0).1.abs() < 1e-6);
    let extra: Vec<f64> = gaps(&just).iter().zip(gaps(&top)).map(|(a, b)| a - b).collect();
    assert!(extra.len() == 2 && (extra[0] - extra[1]).abs() < 1e-6 && extra[0] > 50.0, "{extra:?}");
    // A round frame reflows with Top-to-Top leading too, without losing text.
    let circle = Circle::new((150.0, 150.0), 120.0).to_path(0.1);
    let mut round = area_path("", st(20.0), &circle, Justify::Left);
    round.runs = t.runs.clone();
    round.para.leading_model = LeadingModel::EmBoxTop;
    let rtop = layout(db(), &round);
    for a in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
        let l = layout_with(db(), &round, &valign(a));
        assert!(!l.overflow, "{a:?}");
        assert_eq!(l.lines.last().unwrap().end, rtop.lines.last().unwrap().end);
        // The 40 pt line starts low in the circle already, so centring moves it a little.
        let want = if a == VerticalAlign::Center { 1.0 } else { 20.0 };
        assert!(l.lines.last().unwrap().baseline > rtop.lines.last().unwrap().baseline + want, "{a:?}");
    }
}

// ---------- OpenType ----------

#[test]
fn opentype_ligature_switches() {
    let serif = CharStyle { font_family: "Source Serif 4".into(), ..style(20.0) };
    let t = TextObject::point(Point::ZERO, "fi", serif.clone());
    assert_eq!(layout(db(), &t).glyphs.len(), 1, "fi ligature by default");
    let off = LayoutOptions { features: OtFeatures { ligatures: false, ..Default::default() }, ..Default::default() };
    assert_eq!(layout_with(db(), &t, &off).glyphs.len(), 2);
    let fi = |tracking: f64, features: &[&str]| {
        let st = CharStyle { tracking, features: features.iter().map(|s| s.to_string()).collect(), ..serif.clone() };
        layout(db(), &TextObject::point(Point::ZERO, "fi", st)).glyphs.len()
    };
    // Small tracking adjustments (fitting a line) keep the ligature.
    assert_eq!(fi(-5.0, &[]), 1, "tracking -5 keeps fi");
    assert_eq!(fi(20.0, &[]), 1, "tracking +20 keeps fi");
    let (tight, loose) = LIGATURE_TRACKING_LIMITS;
    assert_eq!((fi(tight, &[]), fi(loose, &[])), (1, 1), "the limits themselves keep fi");
    // Real letterspacing suppresses ligatures (as letterspaced type should), condensing sooner...
    assert_eq!(fi(100.0, &[]), 2, "tracking 100 drops fi");
    assert_eq!(fi(-30.0, &[]), 2, "tracking -30 drops fi");
    assert_eq!(fi(-100.0, &[]), 2, "tracking -100 drops fi");
    // ...unless the character turns them on explicitly.
    assert_eq!(fi(100.0, &["liga"]), 1, "explicit liga wins over tracking");
    assert_eq!(fi(100.0, &["+liga"]), 1);
    assert_eq!(fi(100.0, &["liga", "-liga"]), 2, "the last tag wins");
    assert_eq!(fi(0.0, &["-liga"]), 2, "explicit off still turns it off");
    let f = OtFeatures::from_tags(["dlig", "-liga", "smcp", "onum", "bogus"]);
    assert!(f.discretionary_ligatures && !f.ligatures && f.small_caps && f.oldstyle_figures && !f.fractions);
}

#[test]
fn face_charmap_for_glyphs_panel() {
    let f = db().face("Source Sans 3", "Regular").unwrap();
    let chars = f.chars();
    assert!(chars.len() > 200, "{}", chars.len());
    assert!(chars.windows(2).all(|w| w[0].0 < w[1].0));
    let a = f.glyph_for('A');
    assert!(a != 0 && chars.iter().any(|&(c, g)| c == 'A' && g == a));
    assert!(f.advance(a) > 0.0);
    assert!(!db().outline(&f, a).elements().is_empty());
    assert_eq!(f.units_per_em(), 1000.0);
}

#[test]
fn uncovered_characters_do_not_break_layout() {
    // Private-use code point: no font covers it (system fallback is tried at most once and
    // remembered); layout still places a .notdef-width glyph.
    let l = layout(db(), &TextObject::point(Point::ZERO, "a\u{F8FF}\u{E000}b", style(12.0)));
    assert_eq!(l.lines.len(), 1);
    assert!(l.glyphs.len() >= 3);
}

// ---------- kerning across style runs ----------

/// 40 pt "AVAV" in Source Sans 3, which kerns A and V, with the character at `byte` restyled by
/// `restyle`.
fn avav(byte: usize, restyle: impl FnMut(&mut CharStyle)) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, "AVAV", style(40.0));
    edit::style_range(&mut t.runs, byte, byte + 1, restyle);
    t
}

/// Each glyph's source byte, pen position along the line and advance.
fn pens(t: &TextObject) -> Vec<(usize, f64, f64)> {
    layout(db(), t).glyphs.iter().map(|g| (g.byte, g.origin.x, g.advance)).collect()
}

#[test]
fn restyling_one_character_keeps_its_kerning_with_its_neighbors() {
    let plain = pens(&TextObject::point(Point::ZERO, "AVAV", style(40.0)));
    let unkerned = pens(&TextObject::point(Point::ZERO, "AVAV", CharStyle { kerning: Some(0.0), ..style(40.0) }));
    assert!((plain[0].2 - unkerned[0].2).abs() > 0.1, "the font kerns A and V: {plain:?} {unkerned:?}");
    type Restyle = fn(&mut CharStyle);
    let restyles: [(&str, Restyle); 10] = [
        ("fill", |s| s.fill = Default::default()),
        ("stroke", |s| {
            s.stroke = s.fill.clone();
            s.stroke_width = 2.0;
        }),
        ("underline", |s| s.underline = true),
        ("strikethrough", |s| s.strikethrough = true),
        ("overprint", |s| s.overprint_fill = true),
        ("character style", |s| s.style_name = Some("Accent".into())),
        ("leading", |s| s.leading = Some(60.0)),
        ("baseline shift", |s| s.baseline_shift = 5.0),
        ("rotation", |s| s.rotation = 15.0),
        ("character alignment", |s| s.char_align = vectorcraft_doc::CharAlign::EmBoxCenter),
    ];
    for byte in 0..4 {
        for (what, restyle) in restyles {
            let t = avav(byte, restyle);
            assert_eq!(t.runs.len(), if byte == 0 || byte == 3 { 2 } else { 3 }, "{what} on byte {byte}");
            assert_eq!(pens(&t), plain, "{what} on byte {byte} moves no glyph");
            let l = layout(db(), &t);
            for g in &l.glyphs {
                assert_eq!(g.run, usize::from(byte > 0 && g.byte >= byte) + usize::from(g.byte > byte), "{what} on byte {byte}: glyph {g:?}");
                let x = g.origin.x + g.advance * 0.25;
                assert_eq!(hit_byte(&l, Point::new(x, l.lines[0].baseline)), g.byte, "{what} on byte {byte}: a click before glyph {}", g.byte);
            }
        }
    }
    // The V keeps its own baseline shift and rotation.
    let l = layout(db(), &avav(1, |s| s.baseline_shift = 5.0));
    let y: Vec<f64> = l.glyphs.iter().map(|g| g.xf.as_coeffs()[5]).collect();
    assert!((y[0] - y[1] - 5.0).abs() < 1e-9 && y[0] == y[2] && y[0] == y[3], "{y:?}");
    let l = layout(db(), &avav(1, |s| s.rotation = 90.0));
    assert!(l.glyphs[1].xf.as_coeffs()[1].abs() > l.glyphs[1].xf.as_coeffs()[0].abs(), "the V stands rotated: {:?}", l.glyphs[1].xf);
    assert_eq!(l.glyphs[0].xf.as_coeffs()[1], 0.0, "the A does not");
    // A V with 60 pt leading on the second line sets that line 60 pt below the first.
    let mut t = TextObject::point(Point::ZERO, "AVAV\nAVAV", style(40.0));
    edit::style_range(&mut t.runs, 6, 7, |s| s.leading = Some(60.0));
    let l = layout(db(), &t);
    assert!((l.lines[1].baseline - l.lines[0].baseline - 60.0).abs() < 1e-9, "{:?}", l.lines);
}

#[test]
fn tracking_or_kerning_on_one_character_keeps_the_kerning_before_it() {
    let plain = pens(&TextObject::point(Point::ZERO, "AVAV", style(40.0)));
    let unkerned = pens(&TextObject::point(Point::ZERO, "AVAV", CharStyle { kerning: Some(0.0), ..style(40.0) }));
    // Tracking of 50/1000 em on the V adds 2 pt after it and leaves the font's kerning on both sides.
    let tracked = pens(&avav(1, |s| s.tracking = 50.0));
    assert_eq!(tracked[0], plain[0], "the A keeps its kerning with the V");
    assert!((tracked[1].2 - (plain[1].2 + 2.0)).abs() < 1e-9, "{tracked:?} {plain:?}");
    // Manual kerning of 100/1000 em on the V sets the space between the V and the A after it, in
    // place of the font's; the A before the V keeps the font's kerning with it.
    let kerned = pens(&avav(1, |s| s.kerning = Some(100.0)));
    assert_eq!(kerned[0], plain[0], "the A keeps its kerning with the V");
    assert!((kerned[1].2 - (unkerned[1].2 + 4.0)).abs() < 1e-9, "{kerned:?} {unkerned:?}");
    assert!((kerned[2].1 - (plain[1].1 + unkerned[1].2 + 4.0)).abs() < 1e-9, "{kerned:?}");
    assert_eq!((kerned[2].2, kerned[3].2), (plain[2].2, plain[3].2), "the A and V after it keep their kerning");
}

#[test]
fn manual_kerning_after_a_small_caps_break_replaces_the_fonts_kerning() {
    // Small Caps shapes the lowercase a apart from "AVA", which has manual kerning while the a has
    // the font's: "AVA" is set as it is when all of the text has manual kerning.
    let caps = CharStyle { small_caps: Some(70.0), ..style(40.0) };
    let mut mixed = TextObject::point(Point::ZERO, "aAVA", caps.clone());
    edit::style_range(&mut mixed.runs, 1, 4, |s| s.kerning = Some(0.0));
    let manual = TextObject::point(Point::ZERO, "aAVA", CharStyle { kerning: Some(0.0), ..caps });
    let advances = |t: &TextObject| pens(t).into_iter().filter(|g| g.0 > 0).map(|g| g.2).collect::<Vec<_>>();
    assert_eq!(advances(&mixed), advances(&manual));
}

#[test]
fn manual_kerning_replaces_both_halves_of_the_fonts_pair_after_it() {
    use crate::test_fonts::{PAIR_KERNING_FAMILY, pair_kerning_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(pair_kerning_font().unwrap());
    // 100 pt in a 1000-unit font: the pair takes 6 pt off the A's advance and moves the V 10 pt left.
    let st = CharStyle { font_family: PAIR_KERNING_FAMILY.into(), ..style(100.0) };
    // Each A's advance and how far the V after it is drawn from its pen position, in `text` with
    // manual kerning on the bytes `manual`.
    let set = |text: &str, manual: std::ops::Range<usize>| {
        let mut t = TextObject::point(Point::ZERO, text, st.clone());
        edit::style_range(&mut t.runs, manual.start, manual.end, |s| s.kerning = Some(0.0));
        let l = layout(&db, &t);
        let g = |c: char| l.glyphs.iter().filter(move |g| text[g.byte..].starts_with(c));
        g('A').zip(g('V')).map(|(a, v)| (a.advance, v.xf.as_coeffs()[4] - v.origin.x)).collect::<Vec<_>>()
    };
    let (kerned, unkerned) = (set("AV", 0..0)[0], set("AV", 0..2)[0]);
    assert!((unkerned.0 - kerned.0 - 6.0).abs() < 1e-9 && (unkerned.1 - kerned.1 - 10.0).abs() < 1e-9, "{kerned:?} {unkerned:?}");
    assert_eq!(set("AV", 0..1), [unkerned], "manual kerning on the A turns off the whole A-V pair");
    assert_eq!(set("AV", 1..2), [kerned], "manual kerning on the V keeps the A-V pair");
    assert_eq!(set("HAV", 0..1), [kerned], "manual kerning on the H before them keeps the A-V pair");
    assert_eq!(set("AVAV", 1..4), [kerned, unkerned], "manual kerning from the first V on: the second pair is off");
    assert_eq!(set("AV\u{301}", 1..4), [kerned], "manual kerning on a V with a combining accent keeps the A-V pair");
    // An ß in All Caps is set as SS: with manual kerning on it, the font's S-S pair is off.
    let caps = |manual: std::ops::Range<usize>| {
        let mut t = TextObject::point(Point::ZERO, "Aß", CharStyle { all_caps: true, ..st.clone() });
        edit::style_range(&mut t.runs, manual.start, manual.end, |s| s.kerning = Some(0.0));
        let l = layout(&db, &t);
        l.glyphs.iter().filter(|g| g.byte == 1).map(|g| (g.advance, g.xf.as_coeffs()[4] - g.origin.x)).collect::<Vec<_>>()
    };
    let (auto, manual) = (caps(0..0), caps(0..3));
    assert_eq!(auto.len(), 2);
    assert_ne!(auto, manual, "the font kerns S-S");
    assert_eq!(caps(1..3), manual, "manual kerning on the ß turns off the S-S pair");
}

#[test]
fn a_size_change_still_shapes_the_character_apart() {
    let plain = pens(&TextObject::point(Point::ZERO, "AVAV", style(40.0)));
    let unkerned = pens(&TextObject::point(Point::ZERO, "AVAV", CharStyle { kerning: Some(0.0), ..style(40.0) }));
    let larger = pens(&avav(1, |s| s.size = 41.0));
    assert_eq!(larger[0], unkerned[0], "a V of another size is shaped on its own: {larger:?}");
    assert_ne!(larger[0], plain[0]);
}

#[test]
fn a_ligature_does_not_span_a_change_of_paint() {
    let serif = CharStyle { font_family: "Source Serif 4".into(), ..style(20.0) };
    let glyphs = |byte: Option<usize>| {
        let mut t = TextObject::point(Point::ZERO, "ofit", serif.clone());
        if let Some(byte) = byte {
            edit::style_range(&mut t.runs, byte, byte + 1, |s| s.fill = Default::default());
        }
        layout(db(), &t).glyphs.iter().map(|g| (g.byte, g.len)).collect::<Vec<_>>()
    };
    let fi = vec![(0, 1), (1, 2), (3, 1)];
    assert_eq!(glyphs(None), fi, "fi ligature in unstyled text");
    assert_eq!(glyphs(Some(0)), fi, "a restyled o leaves the fi ligature");
    assert_eq!(glyphs(Some(3)), fi, "a restyled t leaves the fi ligature");
    assert_eq!(glyphs(Some(2)), vec![(0, 1), (1, 1), (2, 1), (3, 1)], "an i in another paint is a glyph of its own");
}

// ---------- performance ----------

#[test]
fn layout_10k_area_text_is_fast() {
    let text: String = COPY.chars().cycle().take(10_000).collect();
    for (justify, composer) in
        [(Justify::JustifyLeft, Composer::EveryLine), (Justify::Left, Composer::EveryLine), (Justify::Left, Composer::SingleLine)]
    {
        let mut t = area(&text, style(10.0), Rect::new(0.0, 0.0, 400.0, 20_000.0), justify);
        t.para.hyphenate = true;
        t.para.composer = composer;
        let _ = layout(db(), &t);
        let n = 5;
        let start = std::time::Instant::now();
        for _ in 0..n {
            let l = layout(db(), &t);
            assert!(!l.overflow);
        }
        let per = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
        eprintln!("layout of 10k chars ({justify:?}, {composer:?}, hyphenated): {per:.3} ms");
        let budget = if cfg!(debug_assertions) { 400.0 } else { 10.0 };
        assert!(per < budget, "{justify:?} {composer:?}: {per} ms");
    }
}

/// Proportional Metrics (#966): in horizontal type full-width glyphs take the font's proportional
/// widths (`palt`), and Japanese composition takes nothing more off the punctuation `palt`
/// re-spaced (the flush bracket at a paragraph's start, consecutive punctuation, half width at a
/// line's end); punctuation it leaves full width is trimmed as before (vertical type: see
/// `tests_vertical`). Needs craft-fonts' Shippori Mincho, whose `palt` covers kana and most brackets but not
/// 〘〙 (skipped without it).
#[test]
fn proportional_metrics_set_full_width_glyphs_on_their_proportional_widths() {
    use vectorcraft_doc::Mojikumi;
    if db().face("Shippori Mincho", "Regular").is_none_or(|f| f.family != "Shippori Mincho") {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    }
    let st = |on: bool| CharStyle { size: 20.0, font_family: "Shippori Mincho".into(), proportional_metrics: on, ..CharStyle::default() };
    // Right-aligned in a frame ten ems wide, so a line's end trim moves the line.
    let lay = |text: &str, on: bool, m: Mojikumi| {
        let mut t = area(text, st(on), Rect::new(0.0, 0.0, 200.0, 200.0), Justify::Right);
        t.para.mojikumi = m;
        layout(db(), &t)
    };
    let adv = |l: &TextLayout| l.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>();
    let x0 = |l: &TextLayout| l.glyphs.iter().map(|g| g.outline.bounding_box().x0).collect::<Vec<_>>();
    let near = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01);
    let off = adv(&lay("「あ漢」", false, Mojikumi::None));
    let on = adv(&lay("「あ漢」", true, Mojikumi::None));
    assert!(off.iter().all(|a| (a - 20.0).abs() < 0.01), "{off:?}");
    assert!(on[0] < 15.0 && on[1] < 19.0 && on[3] < 15.0, "kana and brackets narrower: {on:?}");
    assert!((on[2] - 20.0).abs() < 0.01, "the kanji keeps its em: {on:?}");
    // Re-spaced punctuation: Line-end Punctuation Half Width changes nothing, at the paragraph's
    // start (「), between punctuation (」、) or at the line's end (」).
    for text in ["「あ」、漢", "漢「あ」"] {
        let (half, none) = (lay(text, true, Mojikumi::LineEndHalf), lay(text, true, Mojikumi::None));
        assert!(near(&adv(&half), &adv(&none)), "{text}: {:?} {:?}", adv(&half), adv(&none));
        assert!(near(&x0(&half), &x0(&none)), "{text}: {:?} {:?}", x0(&half), x0(&none));
    }
    // Punctuation `palt` leaves full width: flush at the paragraph's start, half width at the end.
    let (half, none) = (lay("〘あ〙", true, Mojikumi::LineEndHalf), lay("〘あ〙", true, Mojikumi::None));
    assert!((none.glyphs[0].advance - 20.0).abs() < 0.01 && (half.glyphs[0].advance - 10.0).abs() < 0.01, "{:?}", adv(&half));
    let moved = x0(&half)[1] - x0(&none)[1];
    assert!((moved - 10.0).abs() < 0.01, "〙's empty half goes past the line's end: あ moved {moved}");
}

/// The arrows pass a Thai consonant with its vowel and tone marks whole (น้ำ: no, mai tho, sara am),
/// as an accented letter written with a combining mark; plain text moves a character at a time.
#[test]
fn caret_moves_over_whole_clusters() {
    let s = "กน\u{0E49}\u{0E33}e\u{0301}x";
    let stops: Vec<usize> = std::iter::successors(Some(0), |&i| (i < s.len()).then(|| edit::next_cluster(s, i))).collect();
    assert_eq!(stops, [0, 3, 12, 15, 16]);
    let back: Vec<usize> = std::iter::successors(Some(s.len()), |&i| (i > 0).then(|| edit::prev_cluster(s, i))).collect();
    assert_eq!(back, [16, 15, 12, 3, 0]);
    // Inside a cluster (a caret left there by a click), the arrows go to its edges.
    assert_eq!((edit::prev_cluster(s, 6), edit::next_cluster(s, 6)), (3, 12));
    assert_eq!((edit::next_cluster("", 0), edit::prev_cluster("ab", 99)), (0, 1));
}

/// Word steps and double-click stop between Thai words, which no space separates: ภาษา|ไทย|ง่าย.
#[test]
fn thai_words_are_words_to_the_caret() {
    let s = "ภาษาไทยง่าย ok";
    let (thai, easy) = ("ภาษา".len(), "ภาษาไทย".len());
    let end = "ภาษาไทยง่าย".len();
    assert_eq!(edit::next_word(s, 0), thai);
    assert_eq!(edit::next_word(s, thai), easy);
    assert_eq!(edit::next_word(s, easy), end);
    assert_eq!(edit::prev_word(s, end), easy);
    assert_eq!(edit::prev_word(s, easy), thai);
    assert_eq!(edit::prev_word(s, thai), 0);
    assert_eq!(edit::word_at(s, thai + 3), thai..easy, "double-click inside ไทย");
    assert_eq!(edit::word_at(s, 0), 0..thai);
    // Latin keeps its words.
    assert_eq!(edit::word_at(s, s.len() - 1), end + 1..s.len());
    assert_eq!(edit::next_word("hello world", 0), 5);
}
