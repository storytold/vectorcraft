//! The text engine's newer features combined, two at a time and all together: ligatures under
//! tracking, the paragraph composer, area type vertical alignment, Shrink Text to Fit,
//! per-paragraph attributes and inline graphics. Each feature has its own tests; these check that
//! one doesn't undo or skip another (as vertical alignment once left inline art behind).

use super::*;
use kurbo::Shape;
use vectorcraft_doc::{AreaFit, CharStyle, Composer, InlineArt, Justify, TextKind, TextRun, VerticalAlign};
use vectorcraft_geom::PathData;

const COPY: &str = "Typography is the craft of arranging type to make written language legible, readable and appealing when displayed. \
The arrangement of type involves selecting typefaces, point sizes, line lengths, line spacing and letter spacing.";

/// Words full of `fi` (Source Serif 4 ligates it).
const FI: &str = "fine fish finish first in fifty fields of fireflies, a fitting finale for the final fiddler";

/// Two lines that single-line and every-line composers break differently (see
/// `every_line_composer_balances_ragged_lines`).
const RAG: &str = "Destroy all creatures with toughness X or less.";

fn db() -> &'static FontDb {
    FontDb::global()
}

fn sans(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

fn serif(size: f64, tracking: f64) -> CharStyle {
    CharStyle { font_family: "Source Serif 4".into(), size, tracking, ..CharStyle::default() }
}

fn run(text: &str, style: CharStyle) -> TextRun {
    TextRun::new(text, style)
}

/// A square symbol, `scale` × the run's size tall, raised `shift` points.
fn art(scale: f64, shift: f64) -> InlineArt {
    InlineArt { scale, baseline_shift: shift, bounds: Some(Rect::new(-5.0, -5.0, 5.0, 5.0)), ..InlineArt::new("sym") }
}

fn sym(scale: f64, shift: f64, style: CharStyle) -> TextRun {
    TextRun::inline(art(scale, shift), style)
}

/// Area type in `frame` holding `runs`.
fn area(runs: Vec<TextRun>, frame: Rect) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, "", sans(12.0));
    t.runs = runs;
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    t
}

/// `t` as Shrink Text to Fit lays it out at factor `f`, without fitting: sizes, explicit leading
/// and baseline shifts scaled (inline art's own shift too), paragraph spacing kept.
fn scaled_by(t: &TextObject, f: f64) -> TextObject {
    let mut s = t.clone();
    s.area.fit = AreaFit::None;
    for r in &mut s.runs {
        r.style.size *= f;
        r.style.leading = r.style.leading.map(|l| l * f);
        r.style.baseline_shift *= f;
        if let Some(a) = &mut r.inline {
            a.baseline_shift *= f;
        }
    }
    s
}

fn line_texts(t: &TextObject, l: &TextLayout) -> Vec<String> {
    let text = t.plain_text();
    l.lines.iter().map(|li| text[li.start..li.end].trim_end().to_string()).collect()
}

/// Ligature clusters: glyphs standing for more than one character.
fn ligatures(l: &TextLayout) -> usize {
    l.glyphs.iter().filter(|g| g.len > 1 && !g.outline.elements().is_empty()).count()
}

/// Every inline graphic drawn once, where its glyph is: left edge on the glyph's origin, the
/// scaled height, centred half the (scaled) cap height above the glyph's baseline plus its own
/// (scaled) shift.
fn assert_inlines_on_their_glyphs(t: &TextObject, l: &TextLayout, what: &str) {
    let want = t.runs.iter().filter(|r| r.inline.is_some()).count();
    assert_eq!(l.inlines.len(), want, "{what}: one placement per inline run");
    let mut glyphs: Vec<usize> = l.inlines.iter().map(|i| i.glyph).collect();
    glyphs.dedup();
    assert_eq!(glyphs.len(), want, "{what}: no inline drawn twice");
    for i in &l.inlines {
        let g = &l.glyphs[i.glyph];
        assert_eq!(g.run, i.run, "{what}");
        let r = &t.runs[i.run];
        let a = r.inline.as_ref().unwrap();
        let size = r.style.size * l.fit_scale;
        let face = db().face(&r.style.font_family, &r.style.font_style).unwrap();
        let cap = face.cap_height * size / face.upem;
        let b = i.bounds;
        assert!((b.height() - a.scale * size).abs() < 1e-6, "{what}: height {} vs {}", b.height(), a.scale * size);
        assert!((b.x0 - g.origin.x).abs() < 1e-6, "{what}: {b:?} vs origin {:?}", g.origin);
        let centre = g.origin.y - cap * 0.5 - a.baseline_shift * l.fit_scale;
        assert!((b.center().y - centre).abs() < 1e-6, "{what}: centre {} vs {centre} (line {})", b.center().y, g.line);
        assert!(l.lines[g.line].glyph_start <= i.glyph && i.glyph < l.lines[g.line].glyph_end, "{what}");
        assert!(l.bounds.contains(b.center()), "{what}: layout bounds {:?} miss {b:?}", l.bounds);
    }
}

/// Space left below the last line of a one-cell frame.
fn slack(l: &TextLayout, frame: Rect) -> f64 {
    let last = l.lines.last().unwrap();
    frame.y1 - (last.baseline + last.descent)
}

fn baselines(l: &TextLayout) -> Vec<f64> {
    l.lines.iter().map(|li| li.baseline).collect()
}

// ---------- ligatures under tracking × … ----------

#[test]
fn ligatures_x_composer() {
    // Justified (where every-line spreads word space) and ragged, with tracking inside the limits
    // (kept) and letterspaced (dropped), whichever composer breaks the lines.
    for justify in [Justify::Left, Justify::JustifyLeft] {
        for (tracking, kept) in [(0.0, true), (20.0, true), (100.0, false)] {
            let mut counts = vec![];
            for composer in [Composer::SingleLine, Composer::EveryLine] {
                let mut t = area(vec![run(&FI.repeat(3), serif(12.0, tracking))], Rect::new(0.0, 0.0, 170.0, 1000.0));
                t.para.justify = justify;
                t.para.composer = composer;
                let l = layout(db(), &t);
                assert!(l.lines.len() > 3, "several lines to compose");
                counts.push(ligatures(&l));
            }
            assert_eq!(counts[0], counts[1], "{justify:?} {tracking}: composers shape alike");
            assert_eq!(counts[0] > 0, kept, "{justify:?} {tracking}: {counts:?}");
        }
    }
}

#[test]
fn ligatures_x_vertical_align() {
    let frame = Rect::new(0.0, 0.0, 200.0, 400.0);
    for tracking in [20.0, 100.0] {
        let mut t = area(vec![run(FI, serif(12.0, tracking))], frame);
        let top = layout(db(), &t);
        for a in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
            t.area.vertical_align = a;
            let l = layout(db(), &t);
            let gids = |l: &TextLayout| l.glyphs.iter().map(|g| (g.gid, g.byte)).collect::<Vec<_>>();
            assert_eq!(gids(&l), gids(&top), "{tracking} {a:?}: moving lines doesn't reshape them");
            assert!(l.lines[0].baseline > top.lines[0].baseline + 1.0 || a == VerticalAlign::Justify, "{a:?}");
        }
    }
}

#[test]
fn ligatures_x_shrink_text() {
    // Tracking is in 1/1000 em, so scaling the size doesn't move it across the limits.
    let frame = Rect::new(0.0, 0.0, 150.0, 60.0);
    for (tracking, features, kept) in [(40.0, vec![], true), (100.0, vec![], false), (100.0, vec!["liga".to_string()], true)] {
        let st = CharStyle { features, ..serif(14.0, tracking) };
        let mut t = area(vec![run(&FI.repeat(2), st)], frame);
        t.area.fit = AreaFit::ShrinkText { min_percent: 10.0 };
        let l = layout(db(), &t);
        assert!(!l.overflow && l.fit_scale < 1.0, "{tracking}: {}", l.fit_scale);
        assert_eq!(ligatures(&l) > 0, kept, "{tracking}");
        assert_eq!(ligatures(&l), ligatures(&layout(db(), &scaled_by(&t, l.fit_scale))), "{tracking}");
    }
}

#[test]
fn ligatures_x_paragraph_attrs() {
    // Paragraph attributes don't touch shaping: a centred, indented, every-line paragraph keeps
    // its ligatures and a letterspaced run in the next paragraph loses them.
    let mut t = area(vec![run(&format!("{FI}\n"), serif(12.0, 10.0)), run(FI, serif(12.0, 120.0))], Rect::new(0.0, 0.0, 240.0, 600.0));
    let mut second = t.para.clone();
    second.justify = Justify::Center;
    second.left_indent = 20.0;
    second.space_before = 12.0;
    t.set_paragraph_styles(vec![t.para.clone(), second]);
    let l = layout(db(), &t);
    let split = FI.len() + 1;
    let ligs = |first: bool| l.glyphs.iter().filter(|g| g.len > 1 && (g.byte < split) == first).count();
    assert!(ligs(true) > 0, "tracking 10 keeps them");
    assert_eq!(ligs(false), 0, "tracking 120 drops them");
}

#[test]
fn ligatures_x_inline_graphics() {
    // An inline graphic between ligated words neither breaks their ligatures nor takes them: its
    // own letterspaced run tracks the art without touching its neighbours.
    let st = serif(14.0, 0.0);
    let runs = vec![run("fine fish ", st.clone()), sym(1.0, 0.0, serif(14.0, 200.0)), run(" first fifty", st.clone())];
    let t = area(runs, Rect::new(0.0, 0.0, 400.0, 100.0));
    let l = layout(db(), &t);
    let without = area(vec![run("fine fish ", st.clone()), run(" first fifty", st.clone())], Rect::new(0.0, 0.0, 400.0, 100.0));
    assert_eq!(ligatures(&l), ligatures(&layout(db(), &without)), "the same ligatures as without the art");
    assert!(ligatures(&l) >= 4, "fi in fine, fish, first and fifty");
    assert_inlines_on_their_glyphs(&t, &l, "between ligatures");
    let g = &l.glyphs[l.inlines[0].glyph];
    assert!((g.advance - (14.0 + 200.0 / 1000.0 * 14.0)).abs() < 1e-6, "art width plus its tracking: {}", g.advance);
}

// ---------- composer × … ----------

#[test]
fn composer_x_vertical_align() {
    let w = natural_width(RAG, 12.0) * 0.9;
    let frame = Rect::new(0.0, 0.0, w, 200.0);
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let mut t = area(vec![run(RAG, sans(12.0))], frame);
        t.para.composer = composer;
        let top = layout(db(), &t);
        for a in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
            t.area.vertical_align = a;
            let l = layout(db(), &t);
            assert_eq!(line_texts(&t, &l), line_texts(&t, &top), "{composer:?} {a:?}: the same breaks");
            let want = match a {
                VerticalAlign::Center => slack(&top, frame) * 0.5,
                _ => slack(&top, frame),
            };
            let moved = l.lines.last().unwrap().baseline - top.lines.last().unwrap().baseline;
            assert!((moved - want).abs() < 1e-6, "{composer:?} {a:?}: last line moved {moved}, want {want}");
        }
    }
    // And the two composers still differ there.
    let breaks = |c: Composer| {
        let mut t = area(vec![run(RAG, sans(12.0))], frame);
        t.para.composer = c;
        t.area.vertical_align = VerticalAlign::Center;
        line_texts(&t, &layout(db(), &t))
    };
    assert_ne!(breaks(Composer::SingleLine), breaks(Composer::EveryLine));
}

#[test]
fn composer_x_shrink_text() {
    // Each composer shrinks to the largest size at which its own breaks fit.
    for justify in [Justify::Left, Justify::JustifyLeft] {
        for composer in [Composer::SingleLine, Composer::EveryLine] {
            let mut t = area(vec![run(COPY, sans(14.0))], Rect::new(0.0, 0.0, 180.0, 90.0));
            t.para.composer = composer;
            t.para.justify = justify;
            t.area.fit = AreaFit::ShrinkText { min_percent: 20.0 };
            let l = layout(db(), &t);
            assert!(!l.overflow && l.fit_scale < 1.0, "{justify:?} {composer:?}");
            let size = 14.0 * l.fit_scale;
            assert!(layout(db(), &scaled_by(&t, (size + 0.1) / 14.0)).overflow, "{justify:?} {composer:?}: maximal at {size}");
            let same = layout(db(), &scaled_by(&t, l.fit_scale));
            assert_eq!(line_texts(&t, &same), line_texts(&t, &l), "{justify:?} {composer:?}");
        }
    }
}

#[test]
fn composer_x_paragraph_attrs() {
    // Each paragraph breaks with its own composer.
    let w = natural_width("Destroy all creatures with toughness X or", 12.0) + 2.0;
    let mut t = area(vec![run(&format!("{RAG}\n{RAG}\n{RAG}"), sans(12.0))], Rect::new(0.0, 0.0, w, 400.0));
    let p = |c: Composer| vectorcraft_doc::ParaStyle { composer: c, ..t.para.clone() };
    t.set_paragraph_styles(vec![p(Composer::SingleLine), p(Composer::EveryLine), p(Composer::SingleLine)]);
    let l = layout(db(), &t);
    let greedy = ["Destroy all creatures with toughness X or", "less."];
    let every = ["Destroy all creatures with toughness X", "or less."];
    let want: Vec<&str> = [greedy, every, greedy].concat();
    assert_eq!(line_texts(&t, &l), want);
    // The object-wide override still wins over every paragraph.
    let forced = layout_with(db(), &t, &LayoutOptions { composer: Some(Composer::SingleLine), ..Default::default() });
    assert_eq!(line_texts(&t, &forced), [greedy, greedy, greedy].concat());
}

#[test]
fn composer_x_inline_graphics() {
    // An inline graphic is a word's worth of box to the composers: each one breaks around it as
    // around the letter it replaces, and the art follows its glyph to whichever line it lands on.
    let st = sans(12.0);
    let runs = |x: TextRun| vec![run("Destroy all creatures with toughness ", st.clone()), x, run(" or less.", st.clone())];
    let with_art = runs(sym(0.7, 0.0, st.clone()));
    let mut probe = area(with_art.clone(), Rect::new(0.0, 0.0, 10_000.0, 100.0));
    probe.runs.truncate(2);
    probe.runs.push(run(" or", st.clone()));
    let w = layout(db(), &probe).lines[0].x1 + 2.0;
    let sym_text = INLINE.to_string();
    for (composer, first, second) in [
        (Composer::SingleLine, format!("Destroy all creatures with toughness {sym_text} or"), "less."),
        (Composer::EveryLine, format!("Destroy all creatures with toughness {sym_text}"), "or less."),
    ] {
        let mut t = area(with_art.clone(), Rect::new(0.0, 0.0, w, 200.0));
        t.para.composer = composer;
        let l = layout(db(), &t);
        assert_eq!(line_texts(&t, &l), [first.as_str(), second], "{composer:?}");
        assert_inlines_on_their_glyphs(&t, &l, &format!("{composer:?}"));
    }
}

#[test]
fn composer_x_mixed_sizes() {
    // A 24 pt word among 12 pt text: in a rectangle the every-line composer breaks the paragraph as
    // it breaks uniform 12 pt text in which that word is set at 200% horizontal scale, as wide as
    // at 24 pt, and not as the single-line composer does.
    let st = sans(12.0);
    let runs = |x: CharStyle| vec![run("Destroy all creatures with toughness ", st.clone()), run("X", x), run(" or less.", st.clone())];
    let big = sans(24.0);
    let wide = CharStyle { h_scale: 200.0, ..sans(12.0) };
    let mut probe = area(runs(big.clone()), Rect::new(0.0, 0.0, 10_000.0, 100.0));
    probe.runs[2] = run(" or", st.clone());
    let w = layout(db(), &probe).lines[0].x1 + 2.0;
    let breaks = |x: &CharStyle, composer: Composer| {
        let mut t = area(runs(x.clone()), Rect::new(0.0, 0.0, w, 200.0));
        t.para.composer = composer;
        line_texts(&t, &layout(db(), &t))
    };
    let greedy = ["Destroy all creatures with toughness X or", "less."];
    let every = ["Destroy all creatures with toughness X", "or less."];
    assert_eq!(breaks(&wide, Composer::EveryLine), every, "uniform sizes");
    assert_eq!(breaks(&wide, Composer::SingleLine), greedy, "uniform sizes");
    assert_eq!(breaks(&big, Composer::EveryLine), every, "mixed sizes");
    assert_eq!(breaks(&big, Composer::SingleLine), greedy, "mixed sizes");
    // The same in a rectangle split into two columns, one line in each.
    let mut t = area(runs(big.clone()), Rect::new(0.0, 0.0, 2.0 * w + 12.0, 34.0));
    (t.area.columns, t.area.gutter) = (2, 12.0);
    t.para.composer = Composer::EveryLine;
    let l = layout(db(), &t);
    assert_eq!(line_texts(&t, &l), every, "two columns");
    assert!(!l.overflow && l.lines[1].x0 > w, "{:?}", l.lines);
    // The 24 pt word's line is set at its own size.
    let mut t = area(runs(big.clone()), Rect::new(0.0, 0.0, w, 200.0));
    t.para.composer = Composer::EveryLine;
    let l = layout(db(), &t);
    let x = l.glyphs.iter().find(|g| t.plain_text()[g.byte..].starts_with('X')).unwrap();
    assert_eq!(x.line, 0);
    assert!(l.lines[0].ascent > l.lines[1].ascent + 1.0, "{:?}", l.lines);
    // A rectangle that wraps around an object, and a frame that isn't a rectangle (a corner cut),
    // both far below the text, break mixed sizes line by line and still break uniform sizes with
    // the every-line composer.
    let below = PathData::from_bezpath(&Rect::new(0.0, 180.0, 20.0, 190.0).to_path(0.1));
    for (x, want) in [(&wide, every), (&big, greedy)] {
        let mut t = area(runs(x.clone()), Rect::new(0.0, 0.0, w, 200.0));
        t.wrap = vec![vectorcraft_doc::WrapShape { path: below.clone(), wrap: Default::default() }];
        t.para.composer = Composer::EveryLine;
        assert_eq!(line_texts(&t, &layout(db(), &t)), want, "wrap, {}", x.size);
    }
    let cut = kurbo::BezPath::from_vec(vec![
        kurbo::PathEl::MoveTo((0.0, 0.0).into()),
        kurbo::PathEl::LineTo((w, 0.0).into()),
        kurbo::PathEl::LineTo((w, 190.0).into()),
        kurbo::PathEl::LineTo((w - 10.0, 200.0).into()),
        kurbo::PathEl::LineTo((0.0, 200.0).into()),
        kurbo::PathEl::ClosePath,
    ]);
    for (x, want) in [(&wide, every), (&big, greedy)] {
        let mut t = area(runs(x.clone()), Rect::ZERO);
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&cut) };
        t.para.composer = Composer::EveryLine;
        assert_eq!(line_texts(&t, &layout(db(), &t)), want, "cut corner, {}", x.size);
    }
}

#[test]
fn composer_x_mixed_line_metrics() {
    // In a rectangle the every-line composer also breaks a paragraph whose characters differ in
    // other line metrics: vertical scale, leading, font, and an inline graphic raised above the text.
    let st = sans(12.0);
    let wide = CharStyle { h_scale: 200.0, ..sans(12.0) };
    let xs = [
        ("vertical scale", run("X", CharStyle { v_scale: 200.0, ..wide.clone() })),
        ("leading", run("X", CharStyle { leading: Some(30.0), ..wide.clone() })),
        ("font", run("X", serif(12.0, 0.0))),
        ("inline graphic", sym(0.7, 20.0, st.clone())),
    ];
    let mut wrong = vec![];
    for (name, x) in xs {
        let runs = |end: &str| vec![run("Destroy all creatures with toughness ", st.clone()), x.clone(), run(end, st.clone())];
        let w = layout(db(), &area(runs(" or"), Rect::new(0.0, 0.0, 10_000.0, 100.0))).lines[0].x1 + 2.0;
        let greedy = [format!("Destroy all creatures with toughness {} or", x.text), "less.".to_string()];
        let every = [format!("Destroy all creatures with toughness {}", x.text), "or less.".to_string()];
        for (composer, want) in [(Composer::EveryLine, every), (Composer::SingleLine, greedy)] {
            let mut t = area(runs(" or less."), Rect::new(0.0, 0.0, w, 200.0));
            t.para.composer = composer;
            let got = line_texts(&t, &layout(db(), &t));
            if got != want {
                wrong.push((name, composer, got));
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn composer_x_lines_start_after_the_spaces() {
    // A line may end after a dash or between two spaces, but the every-line composer breaks after
    // the spaces that follow, as the single-line composer does: no line starts with a space, in
    // uniform 12 pt text and with a 24 pt word in it.
    let st = sans(12.0);
    let runs = |x: CharStyle| {
        vec![run("Destroy all creatures \u{2014} with toughness  ", st.clone()), run("X", x), run(" or less \u{2014} then draw a card.", st.clone())]
    };
    let mut wrong = vec![];
    for x in [sans(12.0), sans(24.0)] {
        for justify in [Justify::Left, Justify::JustifyLeft] {
            for w in (40..400).step_by(4) {
                let mut t = area(runs(x.clone()), Rect::new(0.0, 0.0, f64::from(w), 2000.0));
                t.para.composer = Composer::EveryLine;
                t.para.justify = justify;
                let text = t.plain_text();
                let l = layout(db(), &t);
                if l.lines.iter().any(|li| text[li.start..].starts_with(' ')) {
                    wrong.push((x.size, justify, w, line_texts(&t, &l)));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

const INLINE: char = vectorcraft_doc::text::INLINE_CHAR;

// ---------- vertical alignment × … ----------

#[test]
fn vertical_align_x_shrink_text() {
    let frame = Rect::new(0.0, 0.0, 200.0, 100.0);
    let mut t = area(vec![run(&COPY.repeat(2), sans(14.0))], frame);
    t.area.fit = AreaFit::ShrinkText { min_percent: 10.0 };
    let top = layout(db(), &t);
    assert!(!top.overflow && top.fit_scale < 1.0);
    for a in [VerticalAlign::Center, VerticalAlign::Bottom] {
        t.area.vertical_align = a;
        let l = layout(db(), &t);
        assert_eq!(l.fit_scale, top.fit_scale, "{a:?}: aligning doesn't change the fit");
        assert_eq!(line_texts(&t, &l), line_texts(&t, &top), "{a:?}");
        let want = if a == VerticalAlign::Center { slack(&top, frame) * 0.5 } else { slack(&top, frame) };
        for (b, b0) in baselines(&l).iter().zip(baselines(&top)) {
            assert!((b - b0 - want).abs() < 1e-6, "{a:?}: {b} vs {b0} + {want}");
        }
        if a == VerticalAlign::Bottom {
            assert!(slack(&l, frame).abs() < 1e-6, "the shrunk block sits on the bottom");
        }
    }
    // Still overflowing at the minimum: aligned like any full frame (the same lines, moved by
    // less than a line, inside the frame), at the same scale.
    let frame = Rect::new(0.0, 0.0, 120.0, 40.0);
    let mut t = area(vec![run(&COPY.repeat(4), sans(14.0))], frame);
    t.area.fit = AreaFit::ShrinkText { min_percent: 90.0 };
    let top = layout(db(), &t);
    assert!(top.overflow && (top.fit_scale - 0.9).abs() < 1e-9);
    for a in [VerticalAlign::Center, VerticalAlign::Bottom, VerticalAlign::Justify] {
        t.area.vertical_align = a;
        let l = layout(db(), &t);
        assert!(l.overflow && l.fit_scale == top.fit_scale, "{a:?}");
        assert_eq!(line_texts(&t, &l), line_texts(&t, &top), "{a:?}");
        let first = &top.lines[0];
        assert!(l.lines[0].baseline - first.baseline < first.ascent + first.descent, "{a:?}");
        assert!(slack(&l, frame) >= -1e-6, "{a:?}");
    }
}

#[test]
fn vertical_align_x_paragraph_attrs() {
    // Per-paragraph spacing and alignment survive the move: lines keep their x and their
    // paragraph gaps, and the block lands where the alignment puts it.
    let frame = Rect::new(0.0, 0.0, 260.0, 500.0);
    let mut t = area(vec![run(&format!("{COPY}\n{COPY}\n{COPY}"), sans(11.0))], frame);
    let mut mid = t.para.clone();
    mid.justify = Justify::Center;
    mid.space_before = 18.0;
    mid.left_indent = 24.0;
    let mut last = t.para.clone();
    last.justify = Justify::Right;
    last.space_before = 6.0;
    t.set_paragraph_styles(vec![t.para.clone(), mid, last]);
    let top = layout(db(), &t);
    for a in [VerticalAlign::Center, VerticalAlign::Bottom] {
        t.area.vertical_align = a;
        let l = layout(db(), &t);
        assert_eq!(line_texts(&t, &l), line_texts(&t, &top));
        let d = l.lines[0].baseline - top.lines[0].baseline;
        assert!(d > 1.0, "{a:?}");
        for (li, l0) in l.lines.iter().zip(&top.lines) {
            assert!((li.baseline - l0.baseline - d).abs() < 1e-6, "{a:?}: every line moves alike");
            assert!((li.x0 - l0.x0).abs() < 1e-6 && (li.x1 - l0.x1).abs() < 1e-6, "{a:?}: x stays");
        }
        for (g, g0) in l.glyphs.iter().zip(&top.glyphs) {
            assert!((g.origin.x - g0.origin.x).abs() < 1e-6 && (g.origin.y - g0.origin.y - d).abs() < 1e-6);
        }
    }
    // Justify: spacing is shared out on top of each paragraph's own space before, never less.
    t.area.vertical_align = VerticalAlign::Justify;
    let l = layout(db(), &t);
    assert!(slack(&l, frame).abs() < 1e-6, "justified to the bottom");
    for (pair, pair0) in l.lines.windows(2).zip(top.lines.windows(2)) {
        assert!(pair[1].baseline - pair[0].baseline >= pair0[1].baseline - pair0[0].baseline - 1e-6);
    }
}

#[test]
fn vertical_align_x_inline_graphics() {
    // Inline art moves with its line however the lines are aligned, in one cell or several, in a
    // rectangle or a frame the text flows into again.
    let st = sans(12.0);
    let mut runs = vec![];
    for i in 0..6 {
        runs.push(run(&format!("{} ", &COPY[i * 20..i * 20 + 20]), st.clone()));
        runs.push(sym(1.0 + i as f64 * 0.1, i as f64 - 2.0, st.clone()));
        if i == 2 {
            runs.push(run("\n", st.clone()));
        }
    }
    let mut tri = BezPath::new();
    tri.move_to((0.0, 0.0));
    tri.line_to((600.0, 0.0));
    tri.line_to((300.0, 400.0));
    tri.close_path();
    for (name, frame, columns) in [
        ("rect", Rect::new(0.0, 0.0, 160.0, 400.0).to_path(0.1), 1),
        ("columns", Rect::new(0.0, 0.0, 400.0, 300.0).to_path(0.1), 2),
        ("triangle", tri, 1),
    ] {
        for a in VerticalAlign::ALL {
            let mut t = area(runs.clone(), Rect::ZERO);
            t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame) };
            t.area.columns = columns;
            t.area.vertical_align = a;
            let l = layout(db(), &t);
            assert!(!l.overflow, "{name} {a:?}");
            assert_inlines_on_their_glyphs(&t, &l, &format!("{name} {a:?}"));
        }
    }
}

// ---------- Shrink Text to Fit × … ----------

#[test]
fn shrink_text_x_paragraph_attrs() {
    // Shrinking scales the type, not each paragraph's spacing and indents: the fitted layout is
    // the unfitted one at the scaled size.
    let mut t = area(vec![run(&format!("{COPY}\n{COPY}"), sans(13.0))], Rect::new(0.0, 0.0, 220.0, 160.0));
    let mut second = t.para.clone();
    second.space_before = 14.0;
    second.left_indent = 30.0;
    second.first_line_indent = 12.0;
    second.justify = Justify::JustifyLeft;
    second.composer = Composer::SingleLine;
    t.set_paragraph_styles(vec![t.para.clone(), second]);
    t.area.fit = AreaFit::ShrinkText { min_percent: 10.0 };
    let l = layout(db(), &t);
    assert!(!l.overflow && l.fit_scale < 1.0);
    let same = layout(db(), &scaled_by(&t, l.fit_scale));
    assert_eq!(baselines(&l), baselines(&same));
    assert_eq!(line_texts(&t, &l), line_texts(&t, &same));
    // The second paragraph's indents are unscaled points.
    let split = COPY.len() + 1;
    let first_of_second = l.lines.iter().find(|li| li.start == split).unwrap();
    assert!((first_of_second.x0 - 42.0).abs() < 1e-6, "{}", first_of_second.x0);
    assert!(l.lines.iter().filter(|li| li.start > split).all(|li| (li.x0 - 30.0).abs() < 1e-6));
    // A paragraph's space before alone can make the text overflow; shrinking still finds a fit.
    let mut tall = t.clone();
    tall.edit_paras(Some(1..2), |p| p.space_before = 100.0);
    let l = layout(db(), &tall);
    assert!(!l.overflow && l.fit_scale < layout(db(), &t).fit_scale);
}

#[test]
fn shrink_text_x_inline_graphics() {
    // Inline art shrinks with the type, its shift included, and stays centred on the cap height.
    let st = sans(14.0);
    let mut runs = vec![];
    for i in 0..8 {
        runs.push(run(&COPY[i * 25..i * 25 + 25], st.clone()));
        runs.push(sym(1.2, 1.5, st.clone()));
    }
    let mut t = area(runs, Rect::new(0.0, 0.0, 200.0, 80.0));
    t.area.fit = AreaFit::ShrinkText { min_percent: 10.0 };
    let l = layout(db(), &t);
    assert!(!l.overflow && l.fit_scale < 1.0, "{}", l.fit_scale);
    assert_inlines_on_their_glyphs(&t, &l, "shrunk");
    let same = layout(db(), &scaled_by(&t, l.fit_scale));
    for (i, i0) in l.inlines.iter().zip(&same.inlines) {
        assert!((i.bounds.y0 - i0.bounds.y0).abs() < 1e-6 && (i.bounds.x0 - i0.bounds.x0).abs() < 1e-6, "{:?} vs {:?}", i.bounds, i0.bounds);
    }
    // Large art makes lines taller; shrinking still fits it.
    for r in &mut t.runs {
        if let Some(a) = &mut r.inline {
            a.scale = 3.0;
        }
    }
    let big = layout(db(), &t);
    assert!(!big.overflow && big.fit_scale < l.fit_scale);
    assert_inlines_on_their_glyphs(&t, &big, "shrunk, big art");
}

// ---------- paragraph attributes × inline graphics ----------

#[test]
fn paragraph_attrs_x_inline_graphics() {
    // Inline art in paragraphs of each alignment and indent sits on its glyph, and an inline
    // graphic opening a paragraph takes that paragraph's first-line indent.
    let st = sans(12.0);
    let runs = vec![
        run("Left aligned ", st.clone()),
        sym(1.0, 0.0, st.clone()),
        run(" text.\n", st.clone()),
        sym(1.0, 0.0, st.clone()),
        run(" centred with art first\nright ", st.clone()),
        sym(1.5, -1.0, st.clone()),
        run(" aligned\nfull justified text with ", st.clone()),
        sym(1.0, 0.0, st.clone()),
        run(" in the middle of a long line that wraps", st.clone()),
    ];
    let mut t = area(runs, Rect::new(0.0, 0.0, 200.0, 400.0));
    let base = t.para.clone();
    let with = |j: Justify, first: f64| vectorcraft_doc::ParaStyle { justify: j, first_line_indent: first, left_indent: 6.0, ..base.clone() };
    t.set_paragraph_styles(vec![base.clone(), with(Justify::Center, 0.0), with(Justify::Right, 0.0), with(Justify::JustifyLeft, 18.0)]);
    let l = layout(db(), &t);
    assert_inlines_on_their_glyphs(&t, &l, "paragraphs");
    let opening = &l.glyphs[l.inlines[1].glyph];
    let line = &l.lines[opening.line];
    assert!((opening.origin.x - line.x0).abs() < 1e-6, "the art opens its line");
    assert!(((line.x0 + line.x1) * 0.5 - (6.0 + 200.0) * 0.5).abs() < 1e-6, "the centred paragraph: {line:?}");
    let right = &l.lines[l.glyphs[l.inlines[2].glyph].line];
    assert!((right.x1 - 200.0).abs() < 1e-6, "{right:?}");
    let full = l.lines.iter().find(|li| li.start == t.plain_text().find("full").unwrap()).unwrap();
    assert!((full.x0 - 24.0).abs() < 1e-6, "left indent + first-line indent: {full:?}");
}

// ---------- all of them ----------

/// One area type object with everything on: two paragraphs with their own attributes (the first
/// ragged every-line with inline symbols and lightly tracked ligatures, the second single-line,
/// justified and spaced away), centred vertically, shrunk to fit.
#[test]
fn every_feature_at_once() {
    let rules = serif(14.0, 15.0);
    let flavor = CharStyle { font_style: "Italic".into(), ..serif(14.0, 0.0) };
    let mut runs = vec![];
    for (i, words) in
        ["Sacrifice a fine fish", "First strike. When this finishes fighting, draw a card", "Fifty fireflies fill the field"].iter().enumerate()
    {
        runs.push(sym(0.8, 0.0, rules.clone()));
        runs.push(run(&format!(": {words}. "), rules.clone()));
        if i == 1 {
            runs.push(sym(0.8, 0.5, rules.clone()));
            runs.push(run(" Then fight again.", rules.clone()));
        }
    }
    runs.push(run("\nThe finest fishers find their fortune in the fjords, if they first fix their nets.", flavor));
    let frame = Rect::new(0.0, 0.0, 240.0, 110.0);
    let mut t = area(runs, frame);
    let mut flav = t.para.clone();
    flav.space_before = 10.0;
    flav.justify = Justify::JustifyLeft;
    flav.composer = Composer::SingleLine;
    t.para.composer = Composer::EveryLine;
    t.set_paragraph_styles(vec![t.para.clone(), flav]);
    t.area.fit = AreaFit::ShrinkText { min_percent: 30.0 };

    let top = layout(db(), &t);
    t.area.vertical_align = VerticalAlign::Center;
    let l = layout(db(), &t);
    assert!(!l.overflow && l.fit_scale < 1.0, "{}", l.fit_scale);
    assert_eq!(l.fit_scale, top.fit_scale);
    // Shrunk: the same as laying out the scaled text, then centring it.
    let mut unfit = scaled_by(&t, l.fit_scale);
    unfit.area.vertical_align = VerticalAlign::Top;
    let plain = layout(db(), &unfit);
    assert_eq!(line_texts(&t, &l), line_texts(&t, &plain));
    let d = slack(&plain, frame) * 0.5;
    assert!(d > 0.0);
    for (b, b0) in baselines(&l).iter().zip(baselines(&plain)) {
        assert!((b - b0 - d).abs() < 1e-6, "centred: {b} vs {b0} + {d}");
    }
    // Every symbol on its glyph; ligatures kept at tracking 15.
    assert_inlines_on_their_glyphs(&t, &l, "everything");
    assert_eq!(l.inlines.len(), 4);
    assert!(ligatures(&l) >= 6, "{}", ligatures(&l));
    // The flavor paragraph: justified (all but its last line fill the measure) and spaced away.
    let split = t.plain_text().find('\n').unwrap() + 1;
    let flavor_lines: Vec<&LineInfo> = l.lines.iter().filter(|li| li.start >= split).collect();
    assert!(flavor_lines.len() >= 2);
    for li in &flavor_lines[..flavor_lines.len() - 1] {
        assert!((li.x1 - li.x0 - 240.0).abs() < 0.5, "justified: {li:?}");
    }
    // Deterministic.
    let again = layout(db(), &t);
    assert_eq!(baselines(&again), baselines(&l));
    assert_eq!(again.inlines.iter().map(|i| i.bounds).collect::<Vec<_>>(), l.inlines.iter().map(|i| i.bounds).collect::<Vec<_>>());
}

/// The natural width of `s` set in one line at `size`.
fn natural_width(s: &str, size: f64) -> f64 {
    let l = layout(db(), &TextObject::point(Point::ZERO, s, sans(size)));
    l.lines[0].x1 - l.lines[0].x0
}
