//! Area type fitting: Shrink Text to Fit.

use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{AreaFit, CharStyle, TextKind};
use vectorcraft_geom::PathData;

const COPY: &str = "Typography is the craft of arranging type to make written language legible, readable and appealing when displayed. \
The arrangement of type involves selecting typefaces, point sizes, line lengths, line spacing and letter spacing, and adjusting the space \
between pairs of letters.";

fn db() -> &'static FontDb {
    FontDb::global()
}

fn area(text: &str, size: f64, frame: Rect, fit: AreaFit) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, CharStyle { size, ..CharStyle::default() });
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    t.area.fit = fit;
    t
}

fn shrink(min_percent: f64) -> AreaFit {
    AreaFit::ShrinkText { min_percent }
}

/// `t` laid out without fitting at `size` (every run).
fn at_size(t: &TextObject, size: f64) -> TextLayout {
    let mut p = t.clone();
    p.area.fit = AreaFit::None;
    for r in &mut p.runs {
        r.style.size = size;
    }
    layout(db(), &p)
}

#[test]
fn shrink_text_makes_overflowing_text_fit() {
    let t = area(COPY, 14.0, Rect::new(0.0, 0.0, 200.0, 90.0), shrink(30.0));
    assert!(at_size(&t, 14.0).overflow, "the text overflows at full size");
    let l = layout(db(), &t);
    assert!(!l.overflow, "shrunk to fit");
    assert!(l.fit_scale < 1.0 && l.fit_scale >= 0.3, "{}", l.fit_scale);
    assert!(l.glyphs.len() > 100);
    // The first run's scaled size is a whole number of tenths of a point.
    let size = 14.0 * l.fit_scale;
    assert!((size * 10.0 - (size * 10.0).round()).abs() < 1e-6, "{size}");
    // Maximal: a tenth of a point larger overflows; and the layout is the one at that size.
    assert!(at_size(&t, size + 0.1).overflow, "{size} + 0.1 pt still fits");
    let same = at_size(&t, size);
    assert_eq!((same.lines.len(), same.glyphs.len(), same.overflow), (l.lines.len(), l.glyphs.len(), false));
    // Deterministic.
    assert_eq!(layout(db(), &t).fit_scale, l.fit_scale);
}

#[test]
fn scaled_area_shrink_accepts_small_local_sizes_and_uses_document_tenths() {
    // A 14 pt story represented in a scaled local coordinate system. Its local size is
    // below 0.1, as happens when setting an exact size after enlarging a text group.
    let mut t = area(COPY, 0.014, Rect::new(0.0, 0.0, 0.2, 0.09), shrink(30.0));
    t.xf = Affine::scale(1000.0);
    assert!(at_size(&t, 0.014).overflow);
    let l = layout(db(), &t);
    assert!(l.fit_scale < 1.0 && l.fit_scale >= 0.3, "{}", l.fit_scale);
    let document_size = 14.0 * l.fit_scale;
    assert!((document_size * 10.0 - (document_size * 10.0).round()).abs() < 1e-6);
}

#[test]
fn shrink_text_respects_its_minimum() {
    let t = area(COPY, 14.0, Rect::new(0.0, 0.0, 120.0, 40.0), shrink(90.0));
    let l = layout(db(), &t);
    assert!(l.overflow, "still overflows at 90 %");
    assert!((l.fit_scale - 0.9).abs() < 1e-9, "{}", l.fit_scale);
    // Out-of-range minimums are clamped (10 %).
    let l = layout(db(), &area(COPY, 14.0, Rect::new(0.0, 0.0, 120.0, 40.0), shrink(0.0)));
    assert!(l.fit_scale >= 0.1 - 1e-9, "{}", l.fit_scale);
}

#[test]
fn shrink_text_is_a_no_op_when_the_text_fits() {
    let t = area("Short text", 14.0, Rect::new(0.0, 0.0, 200.0, 90.0), shrink(50.0));
    let l = layout(db(), &t);
    let plain = at_size(&t, 14.0);
    assert!(!l.overflow);
    assert_eq!(l.fit_scale, 1.0);
    let o = |l: &TextLayout| l.glyphs.iter().map(|g| g.origin).collect::<Vec<_>>();
    assert_eq!(o(&l), o(&plain));
}

#[test]
fn shrink_text_fills_columns() {
    let mut t = area(COPY, 14.0, Rect::new(0.0, 0.0, 300.0, 70.0), shrink(20.0));
    t.area.columns = 2;
    t.area.gutter = 12.0;
    let l = layout(db(), &t);
    assert!(!l.overflow);
    assert!(l.fit_scale < 1.0);
    assert_eq!(l.frames.len(), 2);
    let second = l.frames[1];
    assert!(l.glyphs.iter().any(|g| g.origin.x >= second.x0), "text flows into the second column");
    let size = 14.0 * l.fit_scale;
    assert!(at_size(&t, size + 0.1).overflow, "maximal across columns too");
}

#[test]
fn shrink_text_scales_leading_and_baseline_shift_not_paragraph_spacing() {
    let mut t = area(&COPY.repeat(2), 12.0, Rect::new(0.0, 0.0, 200.0, 200.0), shrink(10.0));
    for r in &mut t.runs {
        r.style.leading = Some(20.0);
    }
    let l = layout(db(), &t);
    assert!(!l.overflow && l.fit_scale < 1.0);
    let gap = l.lines[2].baseline - l.lines[1].baseline;
    assert!((gap - 20.0 * l.fit_scale).abs() < 1e-6, "{gap} vs {}", 20.0 * l.fit_scale);
}

#[test]
fn point_type_and_type_on_a_path_ignore_fit() {
    let mut t = TextObject::point(Point::ZERO, COPY, CharStyle::default());
    t.area.fit = shrink(10.0);
    assert_eq!(layout(db(), &t).fit_scale, 1.0);
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (50.0, 0.0)).to_path(0.1)), start: 0.0, end: None };
    let l = layout(db(), &t);
    assert!(l.overflow, "text runs past the path end");
    assert_eq!(l.fit_scale, 1.0);
}

#[test]
fn shrink_text_guards_degenerate_sizes() {
    for size in [0.0, 0.01, f64::NAN, 1296.0] {
        let l = layout(db(), &area(COPY, size, Rect::new(0.0, 0.0, 50.0, 20.0), shrink(10.0)));
        assert!(l.fit_scale.is_finite() && l.fit_scale > 0.0 && l.fit_scale <= 1.0, "{size}: {}", l.fit_scale);
    }
    // A zero-height frame.
    let l = layout(db(), &area(COPY, 12.0, Rect::new(0.0, 0.0, 50.0, 0.0), shrink(10.0)));
    assert!(l.overflow);
}

#[test]
fn shrink_text_follows_top_to_top_leading_and_character_alignment() {
    use vectorcraft_doc::{CharAlign, LeadingModel, TextRun};
    // Mixed sizes aligned on the em box centre, leading measured from em box top to top.
    let mut t = area(COPY, 14.0, Rect::new(0.0, 0.0, 200.0, 90.0), shrink(20.0));
    t.para.leading_model = LeadingModel::EmBoxTop;
    let big = CharStyle { size: 28.0, leading: Some(32.0), char_align: CharAlign::EmBoxCenter, ..CharStyle::default() };
    t.runs.insert(0, TextRun { text: "Big ".into(), style: big, inline: None });
    for r in &mut t.runs {
        r.style.char_align = CharAlign::EmBoxCenter;
    }
    let l = layout(db(), &t);
    assert!(!l.overflow, "shrunk to fit under the top-to-top model");
    assert!(l.fit_scale < 1.0);
    // The shrunk layout is the same text laid out at the scaled sizes and leading.
    let mut manual = t.clone();
    manual.area.fit = AreaFit::None;
    for r in &mut manual.runs {
        r.style.size *= l.fit_scale;
        r.style.leading = r.style.leading.map(|v| v * l.fit_scale);
    }
    let m = layout(db(), &manual);
    let o = |l: &TextLayout| l.glyphs.iter().map(|g| (g.origin.x, g.origin.y)).collect::<Vec<_>>();
    assert_eq!(o(&l), o(&m));
    // First line still touches the frame top (top-to-top: em box top at y = 0, scaled).
    let first = l.lines.first().map(|li| li.baseline).unwrap_or(0.0);
    assert!(first > 0.0 && first <= 28.0 * l.fit_scale + 1e-6, "{first}");
    // Maximal: the next 0.1 pt step of the first run overflows.
    let step = 0.1 / 28.0;
    let mut bigger = manual.clone();
    for (r, orig) in bigger.runs.iter_mut().zip(&t.runs) {
        r.style.size = orig.style.size * (l.fit_scale + step);
        r.style.leading = orig.style.leading.map(|v| v * (l.fit_scale + step));
    }
    assert!(layout(db(), &bigger).overflow);
}
