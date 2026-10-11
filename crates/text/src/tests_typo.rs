//! Typography tests: rich-text editing, caret navigation, selection, area flow, composer,
//! hyphenation, OpenType features, area type options.

use super::*;
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
