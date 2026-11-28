//! Area type fitting (Shrink Text to Fit) and type on a path fitting (Dynamic Text).

use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{AreaFit, CharStyle, TextKind, TextRun};
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

/// A size that measures the cap height shrinks in steps of 0.1 pt of the size the Character panel
/// shows, not of the em size the layout sets it at.
#[test]
fn shrink_text_steps_the_size_the_character_panel_shows() {
    let mut t = area(COPY, 14.0, Rect::new(0.0, 0.0, 200.0, 90.0), shrink(30.0));
    t.runs[0].style.font_height_reference = vectorcraft_doc::text::FontHeightReference::CapHeight;
    let l = layout(db(), &t);
    assert!(!l.overflow && l.fit_scale < 1.0, "{}", l.fit_scale);
    let shown = 14.0 * l.fit_scale;
    assert!((shown * 10.0 - (shown * 10.0).round()).abs() < 1e-6, "{shown}");
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

/// Type on a path `path` set at `size`, with Dynamic Text on.
fn dynamic_on(text: &str, size: f64, path: &kurbo::BezPath, end: Option<f64>) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, CharStyle { size, ..CharStyle::default() });
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(path), start: 0.0, end };
    t.xf = Affine::IDENTITY;
    t.dynamic_text = true;
    t
}

/// Type on a straight path `len` points long, set at `size`, with Dynamic Text on.
fn dynamic(text: &str, size: f64, len: f64) -> TextObject {
    dynamic_on(text, size, &kurbo::Line::new((0.0, 100.0), (len, 100.0)).to_path(0.1), None)
}

/// `t` laid out at `f` times its sizes with Dynamic Text off.
fn fixed_at(t: &TextObject, f: f64) -> TextLayout {
    let mut s = scaled(t, f);
    s.dynamic_text = false;
    layout(db(), &s)
}

/// Checks that Dynamic Text laid `t` out at the largest tenth of a point that fits: the first run's
/// drawn size is whole tenths, the layout is that of `t` at that size, and a tenth more overflows.
fn assert_fills(t: &TextObject, what: &str) -> TextLayout {
    let l = layout(db(), t);
    let size = t.runs[0].style.size * l.fit_scale;
    assert!((size * 10.0 - (size * 10.0).round()).abs() < 1e-6, "{what}: {size}");
    assert!(!l.overflow, "{what}: fits at {size}");
    let same = fixed_at(t, l.fit_scale);
    assert_eq!((same.glyphs.len(), same.overflow), (l.glyphs.len(), false), "{what}");
    assert!(fixed_at(t, (size + 0.1) / t.runs[0].style.size).overflow, "{what}: {size} + 0.1 pt still fits");
    l
}

/// Dynamic Text: short text grows and long text shrinks to fill the span of its path, at the
/// largest size in tenths of a point that does not run past the end bracket, and the line then
/// takes the span's length within one step.
/// <https://helpx.adobe.com/photoshop/desktop/text-typography/work-with-dynamic-text/dynamic-text-overview.html>
#[test]
fn dynamic_text_fills_the_span_of_its_path() {
    let short = dynamic("Fill me", 12.0, 400.0);
    let mut off = short.clone();
    off.dynamic_text = false;
    assert_eq!(layout(db(), &off).fit_scale, 1.0, "off, the text keeps its size");
    let l = assert_fills(&short, "short");
    assert!(l.fit_scale > 2.0, "short text grows: {}", l.fit_scale);
    let (used, room) = l.path_fill.unwrap_or_default();
    let size = 12.0 * l.fit_scale;
    assert!(used <= room + 1e-6 && room - used <= used * 0.1 / size * 1.5 + 1e-6, "{used} of {room} at {size}");
    let long = dynamic(COPY, 12.0, 400.0);
    assert!(fixed_at(&long, 1.0).overflow);
    let l = assert_fills(&long, "long");
    assert!(l.fit_scale < 1.0, "long text shrinks: {}", l.fit_scale);
    // Deterministic.
    assert_eq!(layout(db(), &long).fit_scale, l.fit_scale);
}

/// Dynamic Text refits to the brackets: an end bracket nearer the start makes the text smaller, and
/// a closed path with no end bracket holds the text once round.
/// <https://helpx.adobe.com/photoshop/desktop/text-typography/work-with-dynamic-text/reposition-start-and-end-points.html>
#[test]
fn dynamic_text_follows_its_brackets() {
    let line = kurbo::Line::new((0.0, 100.0), (400.0, 100.0)).to_path(0.1);
    let whole = layout(db(), &dynamic_on("Brackets", 12.0, &line, None));
    let half = dynamic_on("Brackets", 12.0, &line, Some(0.5));
    let l = assert_fills(&half, "half");
    assert!(l.fit_scale < whole.fit_scale * 0.6, "{} vs {}", l.fit_scale, whole.fit_scale);
    let circle = kurbo::Circle::new((200.0, 200.0), 100.0).to_path(0.1);
    let round = dynamic_on("Round and round the path", 12.0, &circle, None);
    let l = assert_fills(&round, "circle");
    let (used, room) = l.path_fill.unwrap_or_default();
    assert!((room - std::f64::consts::TAU * 100.0).abs() < 1.0, "{room}");
    assert!(used > room * 0.95, "{used} of {room}");
}

/// The runs keep their proportions: one factor scales every size.
#[test]
fn dynamic_text_keeps_the_runs_proportions() {
    let mut t = dynamic("Small", 10.0, 400.0);
    t.runs.push(TextRun::new(" LARGE", CharStyle { size: 20.0, ..CharStyle::default() }));
    let l = assert_fills(&t, "two runs");
    let d = drawn(&t);
    assert!((d.runs[1].style.size / d.runs[0].style.size - 2.0).abs() < 1e-9);
    assert!((d.runs[0].style.size - 10.0 * l.fit_scale).abs() < 1e-9);
    // A size Dynamic Text does not scale is drawn as stored.
    let mut point = t.clone();
    point.dynamic_text = false;
    assert!(matches!(drawn(&point), std::borrow::Cow::Borrowed(_)));
}

/// Nothing to fit: empty text, a path with no length and a span with no length lay out as they are.
/// Past the size limits the text stays at 0.1 pt (and overflows) or at 1,296 pt.
#[test]
fn dynamic_text_edges_and_limits() {
    let empty = layout(db(), &dynamic("", 12.0, 400.0));
    assert_eq!((empty.fit_scale, empty.overflow), (1.0, false));
    let spaces = assert_fills(&dynamic("   ", 12.0, 400.0), "spaces");
    assert!(spaces.fit_scale > 1.0);
    let one = assert_fills(&dynamic("W", 12.0, 100.0), "one character");
    assert!(one.fit_scale > 1.0);
    let point = kurbo::Line::new((50.0, 50.0), (50.0, 50.0)).to_path(0.1);
    let l = layout(db(), &dynamic_on("No room", 12.0, &point, None));
    assert_eq!((l.fit_scale, l.overflow), (1.0, true));
    let line = kurbo::Line::new((0.0, 100.0), (400.0, 100.0)).to_path(0.1);
    let mut closed_span = dynamic_on("No span", 12.0, &line, Some(0.5));
    closed_span.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.5, end: Some(0.5) };
    let l = layout(db(), &closed_span);
    assert_eq!((l.fit_scale, l.overflow), (1.0, true));
    // A span of a hundredth of a point: down at 0.1 pt the text still overflows.
    let tiny = layout(db(), &dynamic(COPY, 12.0, 0.01));
    assert!(tiny.overflow && (12.0 * tiny.fit_scale - 0.1).abs() < 1e-9, "{}", 12.0 * tiny.fit_scale);
    // A span of 100,000 points: one letter stops at 1,296 pt.
    let huge = layout(db(), &dynamic("i", 12.0, 100_000.0));
    assert!(!huge.overflow && (12.0 * huge.fit_scale - 1296.0).abs() < 1e-9, "{}", 12.0 * huge.fit_scale);
    // The limits apply to the first character's size, past an empty run before it.
    let mut empty_first = dynamic("", 0.1, 100_000.0);
    empty_first.runs.push(TextRun::new("i", CharStyle { size: 100.0, ..CharStyle::default() }));
    let l = layout(db(), &empty_first);
    assert!((100.0 * l.fit_scale - 1296.0).abs() < 1e-9, "{}", 100.0 * l.fit_scale);
    // A size that measures the cap height steps and stops as the Character panel shows it.
    let mut cap = dynamic("i", 12.0, 100_000.0);
    cap.runs[0].style.font_height_reference = vectorcraft_doc::text::FontHeightReference::CapHeight;
    let l = layout(db(), &cap);
    assert!((12.0 * l.fit_scale - 1296.0).abs() < 1e-9, "{}", 12.0 * l.fit_scale);
    let cap_short =
        TextObject { kind: dynamic("Cap height", 12.0, 400.0).kind, runs: vec![TextRun::new("Cap height", cap.runs[0].style.clone())], ..cap };
    let l = layout(db(), &cap_short);
    let shown = 12.0 * l.fit_scale;
    assert!(!l.overflow && (shown * 10.0 - (shown * 10.0).round()).abs() < 1e-6, "{shown}");
    // The same document size whatever the local size: a 12 pt run stored at 0.012 under a scale of 1,000.
    let mut scaled_xf = dynamic("Fill me", 0.012, 0.4);
    scaled_xf.xf = Affine::scale(1000.0);
    let l = layout(db(), &scaled_xf);
    let document_size = 12.0 * l.fit_scale;
    assert!(!l.overflow && (document_size * 10.0 - (document_size * 10.0).round()).abs() < 1e-6, "{document_size}");
}

/// Right-to-left text, vertical type, a bulleted paragraph, Optical Margin Alignment and Spacing
/// fit without overflowing, through the layout's own overflow rule.
#[test]
fn dynamic_text_fits_every_kind_of_line() {
    let arc = {
        let mut p = kurbo::BezPath::new();
        p.move_to((0.0, 200.0));
        p.quad_to((200.0, 0.0), (400.0, 200.0));
        p
    };
    let rtl = dynamic_on("שלום עולם", 12.0, &arc, None);
    assert!(assert_fills(&rtl, "right to left").fit_scale > 1.0);
    let mut vertical = dynamic_on("Vertical type", 12.0, &kurbo::Line::new((0.0, 0.0), (0.0, 400.0)).to_path(0.1), None);
    vertical.vertical = true;
    assert!(assert_fills(&vertical, "vertical").fit_scale > 1.0);
    let mut bullet = dynamic_on("Bulleted", 12.0, &arc, None);
    bullet.para.list = Some(vectorcraft_doc::text::ParaList::bulleted(vectorcraft_doc::text::Bullet::Dot));
    let l = assert_fills(&bullet, "bulleted");
    assert!(!l.markers.is_empty());
    let mut optical = dynamic_on("“Quoted.”", 12.0, &arc, None);
    optical.optical_margin = true;
    assert!(assert_fills(&optical, "optical margin").fit_scale > 1.0);
    let mut spaced = dynamic_on("Spaced around a curve", 12.0, &arc, None);
    spaced.path_spacing = 10.0;
    assert!(assert_fills(&spaced, "spacing").fit_scale > 1.0);
}

/// The search lays the text out at most 11 times past the first, however far the size moves.
#[test]
fn dynamic_text_lays_out_a_bounded_number_of_times() {
    for (text, len) in [("Fill me", 400.0), (COPY, 400.0), (COPY, 0.01), ("i", 100_000.0), ("Round", 37.3)] {
        let before = crate::layout::fit_passes();
        let l = layout(db(), &dynamic(text, 12.0, len));
        let passes = crate::layout::fit_passes() - before;
        assert!((1..=11).contains(&passes), "{text:?} on {len}: {passes} passes (fit {})", l.fit_scale);
    }
    // A straight span with no spacing starts from the right size: a few passes.
    let before = crate::layout::fit_passes();
    layout(db(), &dynamic(COPY, 12.0, 300.0));
    assert!(crate::layout::fit_passes() - before <= 4);
}
