//! The advances a reader gives live type ([`reader_advances`]): the same as when the type is laid
//! out again each time, which happens only when a reader sets the type differently from its layout.
//! The type writers draw one layout of the type ([`handed`]) and write what laying it out in each
//! writer writes.

use super::*;
use vectorcraft_doc::text::{Bullet, ParaList};
use vectorcraft_doc::{Document, JustificationOptions, Mojikumi, TextRun};
use vectorcraft_geom::{Point, shapes};

/// The advances with `t` laid out again as a reader sets it, whatever it holds.
fn laid_out_again(t: &TextObject, lay: &vectorcraft_text::TextLayout, kerning: bool) -> HashMap<usize, (f64, f64)> {
    let mut set = TextObject { kind: TextKind::Point, ..t.clone() };
    for p in std::iter::once(&mut set.para).chain(set.paras.iter_mut()) {
        p.mojikumi = Mojikumi::None;
        p.justification = JustificationOptions::default();
    }
    if !kerning {
        for r in &mut set.runs {
            r.style.kerning.get_or_insert(0.0);
        }
    }
    let mut out: HashMap<usize, (f64, f64)> = HashMap::new();
    for g in vectorcraft_text::layout(FontDb::global(), &set).glyphs.iter().filter(|g| g.len > 0) {
        let (advance, _) = out.entry(g.byte).or_insert((0.0, (g.xf.translation().x - g.origin.x) * lay.fit_scale));
        *advance += g.advance * lay.fit_scale;
    }
    out
}

fn style() -> CharStyle {
    CharStyle { size: 12.0, fill: Paint::solid(vectorcraft_color::Color::BLACK), ..CharStyle::default() }
}

/// Point type with Line-end Punctuation Half Width, the setting new type gets.
fn point(text: &str) -> TextObject {
    let mut t = TextObject::point(Point::new(20.0, 20.0), text, style());
    t.para.mojikumi = Mojikumi::LineEndHalf;
    t
}

/// How many times [`reader_advances`] laid out `t` again (`kerning` as it is given), after checking
/// that it gives what laying out again gives.
fn layouts_again(t: &TextObject, kerning: bool) -> usize {
    let lay = vectorcraft_text::layout(FontDb::global(), t);
    READER_LAYOUTS.with(|n| n.set(0));
    let got = reader_advances(t, &lay, kerning);
    let again = READER_LAYOUTS.with(std::cell::Cell::get);
    assert_eq!(got, laid_out_again(t, &lay, kerning), "{:?}", t.plain_text());
    again
}

/// SVG export of `texts`, each a type object, and how many times it laid type out again for its
/// reader advances.
fn export_layouts_again(texts: Vec<TextObject>) -> (String, usize) {
    let mut d = Document::new(300.0, 200.0);
    let l = d.layers[0].id;
    for t in texts {
        let n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        d.insert(Some(l), usize::MAX, n).unwrap();
    }
    READER_LAYOUTS.with(|n| n.set(0));
    let svg = crate::export(&d, &crate::ExportOptions::default());
    (svg, READER_LAYOUTS.with(std::cell::Cell::get))
}

#[test]
fn latin_type_with_mojikumi_is_not_laid_out_again() {
    assert_eq!(layouts_again(&point("Latin words, digits 2026 and (brackets)."), true), 0);
    let (svg, again) = export_layouts_again(vec![point("Ab0"), point("Ab1"), point("Ab2")]);
    assert_eq!(again, 0, "{svg}");
    assert!(svg.contains(">Ab1</tspan>"), "{svg}");
}

#[test]
fn empty_and_one_character_type() {
    assert_eq!(layouts_again(&point(""), true), 0);
    assert_eq!(layouts_again(&point("A"), true), 0);
    assert_eq!(layouts_again(&point("A"), false), 1);
    let (svg, again) = export_layouts_again(vec![point(""), point("A")]);
    assert_eq!(again, 0, "{svg}");
}

#[test]
fn japanese_type_with_mojikumi_is_laid_out_again() {
    for text in ["「かな」、漢字とLatin。", "雅", "、"] {
        assert_eq!(layouts_again(&point(text), true), 1, "{text}");
    }
    // Without Mojikumi, a reader sets it as the layout does.
    let mut none = point("「かな」、漢字とLatin。");
    none.para.mojikumi = Mojikumi::None;
    assert_eq!(layouts_again(&none, true), 0);
}

/// A middle dot is a character the Mojikumi sets space: type holding only middle dots is laid out
/// again with each set that spaces them.
#[test]
fn middle_dots_with_mojikumi_are_laid_out_again() {
    for set in [Mojikumi::YakumonoHankaku, Mojikumi::GyomatsuYakumonoHankaku] {
        let mut t = point("・・");
        t.para.mojikumi = set;
        assert_eq!(layouts_again(&t, true), 1, "{set:?}");
    }
}

#[test]
fn a_japanese_run_or_paragraph_lays_the_whole_object_out_again() {
    let mut runs = point("Latin ");
    runs.runs.push(TextRun::new("漢字", style()));
    assert_eq!(layouts_again(&runs, true), 1);
    let paras = point("Latin\n漢字");
    assert_eq!(layouts_again(&paras, true), 1);
}

#[test]
fn area_and_justified_type_is_laid_out_again() {
    let mut area = point("aa bb cc dd ee ff");
    area.kind = TextKind::Area { frame: shapes::rectangle(Rect::new(0.0, 0.0, 60.0, 160.0)) };
    area.para.justify = Justify::JustifyLeft;
    assert_eq!(layouts_again(&area, true), 1);
    let mut spaced = point("Latin words");
    spaced.para.justification = JustificationOptions { letter_spacing: [0.0, 5.0, 10.0], ..JustificationOptions::default() };
    assert_eq!(layouts_again(&spaced, true), 1);
}

#[test]
fn unkerned_advances_lay_out_again_only_for_runs_without_manual_kerning() {
    let mut wide = point("AVAV");
    wide.runs[0].style.h_scale = 150.0;
    assert_eq!(layouts_again(&wide, false), 1);
    wide.runs[0].style.kerning = Some(0.0);
    assert_eq!(layouts_again(&wide, false), 0);
}

#[test]
fn list_items_with_latin_markers_are_not_laid_out_again() {
    let mut bullets = point("first\nsecond");
    bullets.para.list = Some(ParaList::bulleted(Bullet::Dot));
    assert_eq!(layouts_again(&bullets, true), 0);
    let mut numbers = point("一つ\n二つ");
    numbers.para.list = Some(ParaList::numbered(vectorcraft_doc::text::NumberStyle::Arabic));
    assert_eq!(layouts_again(&numbers, true), 1);
}

#[test]
fn a_mixed_document_lays_out_again_only_the_japanese_type() {
    let (svg, again) = export_layouts_again(vec![point("Latin"), point("「かな」。"), point(""), point("Ab2")]);
    assert_eq!(again, 1, "{svg}");
}

/// Type `t`, with the object's own fills and strokes `items`, alone in a document.
fn doc_of(t: TextObject, items: Vec<AppearanceItem>) -> Document {
    let mut d = Document::new(300.0, 200.0);
    let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
    n.appearance.items = items;
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

/// Type on a straight path across the page.
fn on_path(text: &str, st: CharStyle) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, st);
    let mut line = BezPath::new();
    line.move_to((20.0, 100.0));
    line.line_to((280.0, 100.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    t
}

/// A character style with a stroke on side `align`.
fn stroked(align: StrokeAlign) -> CharStyle {
    CharStyle { stroke: Paint::solid(vectorcraft_color::Color::rgb8(200, 0, 0)), stroke_width: 2.0, stroke_align: align, ..style() }
}

fn red_fill() -> Vec<AppearanceItem> {
    vec![AppearanceItem::Fill(vectorcraft_doc::FillLayer::new(Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0))))]
}

/// The type the writers handle differently, each with how many times its export lays the type
/// out: once, plus the layout of the copy that checks for hanging glyphs on a path; and how many
/// times with each writer laying the type out on its own ([`LAY_OUT_AGAIN`]).
fn writer_cases() -> Vec<(&'static str, Document, crate::ExportOptions, u64, u64)> {
    let opts = crate::ExportOptions::default;
    let outlined = || crate::ExportOptions { outline_text: true, ..crate::ExportOptions::default() };
    let mut runs = point("Red ");
    runs.runs.push(TextRun::new("green ", CharStyle { underline: true, ..stroked(StrokeAlign::Outside) }));
    runs.runs.push(TextRun::new("blue", CharStyle { strikethrough: true, ..style() }));
    let mut many = point("");
    many.runs = (0..200).map(|i| TextRun::new(if i % 7 == 6 { " " } else { "x" }, CharStyle { size: 8.0 + f64::from(i % 5), ..style() })).collect();
    let mut rtl_run = point("abc");
    rtl_run.runs[0].style.char_direction = Some(vectorcraft_doc::ParaDirection::RightToLeft);
    let mut skewed = on_path("Skewed", style());
    skewed.path_effect = vectorcraft_doc::PathEffect::Skew;
    let mut hanging = on_path("\u{201C}Quoted.\u{201D}", style());
    hanging.optical_margin = true;
    let mut inside = point("Inside out");
    inside.runs.push(TextRun::new(" plain", style()));
    inside.runs[0].style = CharStyle { underline: true, ..stroked(StrokeAlign::Inside) };
    let outside = TextObject::point(Point::new(20.0, 20.0), "Outside", stroked(StrokeAlign::Outside));
    vec![
        ("point type", doc_of(point("Plain type"), vec![]), opts(), 1, 1),
        ("point type with a fill", doc_of(point("Filled type"), red_fill()), opts(), 1, 2),
        ("type on a path", doc_of(on_path("Type on a path", style()), vec![]), opts(), 1, 2),
        ("type on a path with a fill", doc_of(on_path("Filled path type", style()), red_fill()), opts(), 1, 3),
        ("bidirectional type", doc_of(point("abc \u{5D0}\u{5D1}\u{5D2} def"), vec![]), opts(), 1, 2),
        ("a run set right to left", doc_of(rtl_run, vec![]), opts(), 1, 2),
        ("bidirectional type on a path", doc_of(on_path("abc \u{5D0}\u{5D1}\u{5D2}", style()), vec![]), opts(), 1, 2),
        ("type on a path with Skew", doc_of(skewed, vec![]), opts(), 1, 2),
        ("type on a path that may hang", doc_of(hanging, vec![]), opts(), 2, 3),
        ("outside character stroke", doc_of(outside, vec![]), opts(), 1, 3),
        ("inside character stroke", doc_of(inside, vec![]), opts(), 1, 3),
        ("runs with a stroke and decorations", doc_of(runs.clone(), vec![]), opts(), 1, 3),
        ("runs as outlines", doc_of(runs.clone(), vec![]), outlined(), 1, 1),
        ("outside stroke on a path with a fill", doc_of(on_path("Stroked", stroked(StrokeAlign::Outside)), red_fill()), opts(), 1, 4),
        ("inside stroke on a path", doc_of(on_path("Stroked", stroked(StrokeAlign::Inside)), vec![]), opts(), 1, 4),
        ("many runs as outlines", doc_of(many.clone(), vec![]), outlined(), 1, 1),
        ("many runs", doc_of(many, vec![]), opts(), 1, 1),
    ]
}

/// SVG export of `d` with each type writer laying the type out again (`again`) or drawing the
/// layout it is handed, and how many times the export laid type out.
fn export_counting(d: &Document, opts: &crate::ExportOptions, again: bool) -> (crate::Output, u64) {
    LAY_OUT_AGAIN.with(|a| a.set(again));
    let before = vectorcraft_text::test_fonts::layouts_on_this_thread();
    let out = crate::export_full(d, opts, None);
    let laid = vectorcraft_text::test_fonts::layouts_on_this_thread() - before;
    LAY_OUT_AGAIN.with(|a| a.set(false));
    (out, laid)
}

#[test]
fn every_type_writer_draws_one_layout() {
    for (name, d, opts, layouts, own_layouts) in writer_cases() {
        let (out, laid) = export_counting(&d, &opts, false);
        assert_eq!(laid, layouts, "{name}: {}", out.svg);
        // The same file and warnings as each writer laying the type out on its own.
        let (again, laid_again) = export_counting(&d, &opts, true);
        assert_eq!(laid_again, own_layouts, "{name}");
        assert_eq!(out.svg, again.svg, "{name}");
        assert_eq!(out.warnings, again.warnings, "{name}");
        assert_eq!(out.setting_notes, again.setting_notes, "{name}");
    }
}

/// The CPU time this thread has run, from Linux's scheduler statistics; wall time elsewhere.
fn thread_time(start: std::time::Instant) -> std::time::Duration {
    std::fs::read_to_string("/proc/thread-self/schedstat")
        .ok()
        .and_then(|s| s.split_whitespace().next()?.parse().ok())
        .map_or_else(|| start.elapsed(), std::time::Duration::from_nanos)
}

fn median(mut v: Vec<std::time::Duration>) -> std::time::Duration {
    v.sort();
    v[v.len() / 2]
}

/// Timing, not a check: SVG export of a large document with each writer laying the type out again
/// and with one layout handed to them, alternating, seven runs each; and grouping the glyphs of
/// type with 1,000 runs by run, one filter per run against one pass. Run with
/// `cargo test --release -p vectorcraft-svg --lib svg_type_layout_timing -- --ignored --nocapture`.
#[test]
#[ignore = "timing"]
fn svg_type_layout_timing() {
    let mut d = Document::new(2000.0, 2000.0);
    let l = d.layers[0].id;
    let add = |d: &mut Document, t: TextObject, items: Vec<AppearanceItem>| {
        let mut n = Node::new(d.alloc_id(), NodeKind::Text(Box::new(t)));
        n.appearance.items = items;
        d.insert(Some(l), usize::MAX, n).unwrap();
    };
    let words: String = "Type set along a path for timing. ".chars().cycle().take(60).collect();
    for _ in 0..1000 {
        add(&mut d, on_path(&words, style()), vec![]);
    }
    let hebrew = "abc \u{5D0}\u{5D1}\u{5D2}\u{5D3} def ".repeat(6);
    for _ in 0..1000 {
        add(&mut d, point(&hebrew.chars().take(80).collect::<String>()), vec![]);
    }
    for _ in 0..300 {
        add(&mut d, TextObject::point(Point::new(20.0, 20.0), "Stroked outside", stroked(StrokeAlign::Outside)), vec![]);
    }
    let mut runs = point("");
    runs.runs = (0..1000).map(|i| TextRun::new(if i % 9 == 8 { " " } else { "x" }, CharStyle { size: 8.0 + f64::from(i % 3), ..style() })).collect();
    let mut outlined = Document::new(2000.0, 2000.0);
    let ol = outlined.layers[0].id;
    for _ in 0..300 {
        let n = Node::new(outlined.alloc_id(), NodeKind::Text(Box::new(runs.clone())));
        outlined.insert(Some(ol), usize::MAX, n).unwrap();
    }
    let docs = [
        ("1,000 on a path, 1,000 bidirectional, 300 outside strokes", &d, crate::ExportOptions::default()),
        ("300 texts of 1,000 runs as outlines", &outlined, crate::ExportOptions { outline_text: true, ..crate::ExportOptions::default() }),
    ];
    for (name, doc, opts) in docs {
        let (one, again) = (export_counting(doc, &opts, false), export_counting(doc, &opts, true));
        assert_eq!(one.0.svg, again.0.svg, "{name}");
        let (mut base, mut handed) = (vec![], vec![]);
        for _ in 0..7 {
            for (again, times) in [(true, &mut base), (false, &mut handed)] {
                let start = std::time::Instant::now();
                let t0 = thread_time(start);
                let _ = export_counting(doc, &opts, again);
                times.push(thread_time(start) - t0);
            }
        }
        let (from, to, base_median, handed_median) = (again.1, one.1, median(base.clone()), median(handed.clone()));
        println!(
            "{name}: layouts {from} -> {to}; laid out again {base:?} (median {base_median:?}); one layout {handed:?} (median {handed_median:?})"
        );
    }
    // The glyphs of one 1,000-run text, grouped by run 300 times.
    let lay = vectorcraft_text::layout(FontDb::global(), &runs);
    let (mut filtered, mut bucketed) = (vec![], vec![]);
    for _ in 0..7 {
        let start = std::time::Instant::now();
        let t0 = thread_time(start);
        for _ in 0..300 {
            for i in 0..runs.runs.len() {
                for g in lay.glyphs.iter().filter(|g| g.run == i) {
                    std::hint::black_box(g);
                }
            }
        }
        filtered.push(thread_time(start) - t0);
        let start = std::time::Instant::now();
        let t0 = thread_time(start);
        for _ in 0..300 {
            let mut run_glyphs = vec![Vec::new(); runs.runs.len()];
            for g in &lay.glyphs {
                if let Some(gs) = run_glyphs.get_mut(g.run) {
                    gs.push(g);
                }
            }
            for g in run_glyphs.iter().flatten() {
                std::hint::black_box(g);
            }
        }
        bucketed.push(thread_time(start) - t0);
    }
    println!(
        "grouping 300 texts of 1,000 runs: filter per run {filtered:?} (median {:?}); one pass {bucketed:?} (median {:?})",
        median(filtered.clone()),
        median(bucketed.clone())
    );
}
