use super::*;
use kurbo::{Affine, Shape};
use vectorcraft_doc::{CharStyle, Justify, TextKind, TextRun};
use vectorcraft_geom::PathData;

/// The bundled fonts only: what these tests measure doesn't change with the fonts installed.
fn db() -> &'static FontDb {
    static DB: std::sync::OnceLock<FontDb> = std::sync::OnceLock::new();
    DB.get_or_init(|| FontDb::with_font_dirs(vec![]))
}

fn style(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

fn serif(size: f64) -> CharStyle {
    CharStyle { font_family: "Source Serif 4".into(), ..style(size) }
}

fn point(text: &str, st: CharStyle) -> TextObject {
    TextObject::point(Point::ZERO, text, st)
}

fn area(text: &str, st: CharStyle, frame: Rect, justify: Justify) -> TextObject {
    let mut t = point(text, st);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
    t.xf = Affine::IDENTITY;
    t.para.justify = justify;
    t
}

fn width(t: &TextObject) -> f64 {
    let l = layout(db(), t);
    l.glyphs.iter().map(|g| g.advance).sum()
}

const LOREM: &str = "Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.";

#[test]
fn bundled_families_and_styles() {
    let f = db().families();
    for fam in ["Source Sans 3", "Source Serif 4", "Inter", "JetBrains Mono"] {
        assert!(f.iter().any(|x| x == fam), "{fam} missing from {f:?}");
    }
    let s = db().styles("Source Sans 3");
    assert_eq!(s.first().map(String::as_str), Some("Regular"), "{s:?}");
    assert!(s.iter().any(|x| x == "Bold") && s.iter().any(|x| x == "Italic") && s.iter().any(|x| x == "Semibold"), "{s:?}");
}

#[test]
fn style_lookup_and_fallback() {
    let f = db().face("Inter", "Semibold").unwrap();
    assert_eq!((f.family.as_str(), f.style.as_str()), ("Inter", "SemiBold"));
    let f = db().face("inter", "Bold").unwrap();
    assert_eq!(f.style, "SemiBold", "closest weight");
    let f = db().face("No Such Font", "Regular").unwrap();
    assert_eq!((f.family.as_str(), f.style.as_str()), ("Source Sans 3", "Regular"));
    let f = db().face("No Such Font", "Bold").unwrap();
    assert_eq!(f.style, "Bold");
    assert_eq!(db().face("Source Sans 3", "Italic").unwrap().style, "Italic");
}

#[test]
fn add_font_rejects_garbage_and_duplicates() {
    assert_eq!(db().add_font(vec![1, 2, 3]), 0);
    let bytes = include_bytes!("../../../assets/fonts/Inter-Regular.ttf").to_vec();
    assert_eq!(db().add_font(bytes), 0, "already bundled");
}

/// Character panel menu › System Layout: each character that takes space advances by a whole
/// number of points, at least one. Off (the default) keeps the font's fractional advances. A
/// tate-chu-yoko continuation stays at zero.
/// <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/turn-fractional-character-widths-off-or-on.html>
#[test]
fn system_layout_rounds_positive_advances_to_whole_points() {
    let mut off = point("Hi", style(13.0));
    off.runs[0].style.tracking = 50.0;
    assert!(!off.system_layout);
    let fractional: Vec<f64> = layout(db(), &off).glyphs.iter().map(|g| g.advance).collect();
    assert!(fractional.iter().any(|a| (a - a.round()).abs() > 1e-6), "{fractional:?}");
    let mut on = off.clone();
    on.system_layout = true;
    let whole: Vec<f64> = layout(db(), &on).glyphs.iter().map(|g| g.advance).collect();
    assert!(whole.iter().all(|a| *a >= 1.0 && (a - a.round()).abs() < 1e-9), "{whole:?}");
    assert!(fractional.iter().zip(&whole).any(|(a, b)| (a - b).abs() > 0.05), "{fractional:?} {whole:?}");
    let mut tiny = point("i", style(0.4));
    tiny.system_layout = true;
    let laid = layout(db(), &tiny);
    assert!(laid.glyphs.iter().any(|g| (g.advance - 1.0).abs() < 1e-9), "{:?}", laid.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>());
    let mut tcy = point("10", style(20.0));
    tcy.vertical = true;
    tcy.system_layout = true;
    let adv: Vec<f64> = layout(db(), &tcy).glyphs.iter().map(|g| g.advance).collect();
    assert!(adv.contains(&0.0), "a tate-chu-yoko continuation stays at zero: {adv:?}");
    assert!(adv.iter().all(|a| *a == 0.0 || (a - a.round()).abs() < 1e-9), "{adv:?}");
}

/// Character panel menu › Fractional Widths, off: each character that takes space advances by a
/// whole number of points, at least one. On (the default) keeps the font's fractional advances,
/// including tracking. A tate-chu-yoko continuation stays at zero.
/// <https://helpx.adobe.com/illustrator/desktop/design-with-text/edit-format-text/turn-fractional-character-widths-off-or-on.html>
#[test]
fn fractional_widths_off_rounds_positive_advances_to_whole_points() {
    let mut on = point("Hi", style(13.0));
    on.runs[0].style.tracking = 50.0;
    assert!(on.fractional_widths);
    let fractional: Vec<f64> = layout(db(), &on).glyphs.iter().map(|g| g.advance).collect();
    assert!(fractional.iter().any(|a| (a - a.round()).abs() > 1e-6), "{fractional:?}");
    let mut off = on.clone();
    off.fractional_widths = false;
    let whole: Vec<f64> = layout(db(), &off).glyphs.iter().map(|g| g.advance).collect();
    assert!(whole.iter().all(|a| *a >= 1.0 && (a - a.round()).abs() < 1e-9), "{whole:?}");
    assert!(fractional.iter().zip(&whole).any(|(a, b)| (a - b).abs() > 0.05), "{fractional:?} {whole:?}");
    // Smaller than half a point: rounding alone would be 0, which would run the type together.
    let mut tiny = point("i", style(0.4));
    tiny.fractional_widths = false;
    let laid = layout(db(), &tiny);
    assert!(laid.glyphs.iter().any(|g| (g.advance - 1.0).abs() < 1e-9), "{:?}", laid.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>());
    let mut tcy = point("10", style(20.0));
    tcy.vertical = true;
    tcy.fractional_widths = false;
    let adv: Vec<f64> = layout(db(), &tcy).glyphs.iter().map(|g| g.advance).collect();
    assert!(adv.contains(&0.0), "a tate-chu-yoko continuation stays at zero: {adv:?}");
    assert!(adv.iter().all(|a| *a == 0.0 || (a - a.round()).abs() < 1e-9), "{adv:?}");
}

#[test]
fn glyph_count_and_advances() {
    let l = layout(db(), &point("Hello", style(24.0)));
    assert_eq!(l.glyphs.len(), 5);
    assert_eq!(l.lines.len(), 1);
    for (i, g) in l.glyphs.iter().enumerate() {
        assert!(g.advance > 0.0);
        assert_eq!(g.byte, i);
        assert!(!g.outline.elements().is_empty());
    }
    // Glyphs sit on the baseline (y = 0), ascenders above it (y < 0, y-down).
    let h = l.glyphs[0].outline.bounding_box();
    assert!(h.y0 < -10.0 && h.y1.abs() < 1.0, "{h:?}");
    assert!(l.bounds.width() > 30.0);
}

#[test]
fn ligatures_and_kerning() {
    // "fi" forms a ligature in Source Serif 4: one glyph covering two bytes.
    let l = layout(db(), &point("fi", serif(20.0)));
    assert_eq!(l.glyphs.len(), 1);
    assert_eq!(l.glyphs[0].len, 2);
    // Metrics kerning tightens "AV"; manual kerning 0 disables it.
    let auto = width(&point("AV", style(100.0)));
    let off = width(&point("AV", CharStyle { kerning: Some(0.0), ..style(100.0) }));
    assert!(auto < off - 1.0, "auto {auto} off {off}");
}

#[test]
fn point_text_lines_and_auto_leading() {
    let l = layout(db(), &point("one\ntwo\n", style(10.0)));
    assert_eq!(l.lines.len(), 3);
    assert_eq!(l.lines[0].baseline, 0.0);
    assert!((l.lines[1].baseline - 12.0).abs() < 1e-9);
    assert!((l.lines[2].baseline - 24.0).abs() < 1e-9);
    assert_eq!((l.lines[0].start, l.lines[0].end), (0, 3));
    assert_eq!((l.lines[1].start, l.lines[1].end), (4, 7));
    assert_eq!((l.lines[2].start, l.lines[2].end), (8, 8));
}

#[test]
fn explicit_leading() {
    let l = layout(db(), &point("a\nb", CharStyle { leading: Some(30.0), ..style(10.0) }));
    assert!((l.lines[1].baseline - 30.0).abs() < 1e-9);
}

#[test]
fn point_alignment() {
    let mut t = point("Centered", style(20.0));
    let w = width(&t);
    t.para.justify = Justify::Center;
    let l = layout(db(), &t);
    assert!((l.lines[0].x0 + w / 2.0).abs() < 1e-6 && (l.lines[0].x1 - w / 2.0).abs() < 1e-6);
    t.para.justify = Justify::Right;
    let l = layout(db(), &t);
    assert!(l.lines[0].x1.abs() < 1e-6);
    assert!(l.glyphs[0].origin.x < -w + 1e-6);
}

#[test]
fn tracking_and_scale() {
    let base = width(&point("abcd", style(10.0)));
    let tracked = width(&point("abcd", CharStyle { tracking: 100.0, ..style(10.0) }));
    assert!((tracked - base - 4.0 * 1.0).abs() < 1e-6, "{base} {tracked}");
    let wide = width(&point("abcd", CharStyle { h_scale: 200.0, ..style(10.0) }));
    assert!((wide - 2.0 * base).abs() < 0.05);
    let l = layout(db(), &point("x", CharStyle { baseline_shift: 5.0, ..style(10.0) }));
    let b = l.glyphs[0].outline.bounding_box();
    assert!(b.y1 < -4.0, "shifted up: {b:?}");
}

#[test]
fn underline_and_strikethrough_bars() {
    let mut t = point("ab", style(20.0));
    t.runs = vec![
        TextRun { text: "ab".into(), style: style(20.0), inline: None },
        TextRun { text: "cd".into(), style: CharStyle { underline: true, strikethrough: true, ..style(20.0) }, inline: None },
    ];
    let l = layout(db(), &t);
    let bars = decorations(&l, db(), &t);
    assert_eq!(bars.len(), 2, "one underline and one strikethrough: {bars:?}");
    assert!(bars.iter().all(|(run, _)| *run == 1), "only the styled run");
    let (under, strike) = (bars[0].1.bounding_box(), bars[1].1.bounding_box());
    // The bars span the run's glyphs: from "c" to the end of "d".
    let (c, d) = (&l.glyphs[2], &l.glyphs[3]);
    assert!((under.x0 - c.origin.x).abs() < 1e-6 && (under.x1 - d.origin.x - d.advance).abs() < 1e-6, "{under:?}");
    // y runs down: the underline sits below the baseline, the strikethrough above it, both thin.
    assert!(under.y0 > 0.0 && under.y1 < 5.0, "{under:?}");
    assert!(strike.y1 < 0.0 && strike.y0 > -10.0, "{strike:?}");
    assert!(under.height() < 3.0 && strike.height() < 3.0);
    // No bars for plain text.
    assert!(decorations(&layout(db(), &point("ab", style(20.0))), db(), &point("ab", style(20.0))).is_empty());
}

#[test]
fn all_caps() {
    let a = width(&point("abc", CharStyle { all_caps: true, ..style(10.0) }));
    let b = width(&point("ABC", style(10.0)));
    assert!((a - b).abs() < 1e-9);
    let l = layout(db(), &point("\u{df}", CharStyle { all_caps: true, ..style(10.0) }));
    assert_eq!(l.glyphs.len(), 2, "ß -> SS");
}

#[test]
fn area_line_breaking_within_frame() {
    let frame = Rect::new(10.0, 10.0, 110.0, 400.0);
    let l = layout(db(), &area(LOREM, style(12.0), frame, Justify::Left));
    assert!(l.lines.len() > 3);
    assert!(!l.overflow);
    for line in &l.lines {
        assert!(line.x0 >= frame.x0 - 1e-6 && line.x1 <= frame.x1 + 1e-6, "{line:?}");
    }
    // Lines tile the text.
    for w in l.lines.windows(2) {
        assert_eq!(w[0].end, w[1].start);
    }
    assert_eq!(l.lines.last().unwrap().end, LOREM.len());
    // First baseline = frame top + ascent; subsequent lines advance by the leading.
    assert!((l.lines[0].baseline - (frame.y0 + l.lines[0].ascent)).abs() < 1e-6);
    assert!((l.lines[1].baseline - l.lines[0].baseline - 14.4).abs() < 1e-6);
    // Lines start at word boundaries.
    for line in &l.lines[1..] {
        assert_eq!(&LOREM[line.start - 1..line.start], " ");
    }
}

#[test]
fn area_overflow() {
    let small = layout(db(), &area(LOREM, style(12.0), Rect::new(0.0, 0.0, 100.0, 40.0), Justify::Left));
    assert!(small.overflow);
    assert!(!small.lines.is_empty() && small.lines.last().unwrap().end < LOREM.len());
    let big = layout(db(), &area(LOREM, style(12.0), Rect::new(0.0, 0.0, 1000.0, 400.0), Justify::Left));
    assert!(!big.overflow);
    assert_eq!(big.glyphs.len(), LOREM.chars().count());
}

#[test]
fn area_alignment_and_justify() {
    let frame = Rect::new(0.0, 0.0, 150.0, 400.0);
    let l = layout(db(), &area(LOREM, style(12.0), frame, Justify::Right));
    for line in &l.lines {
        assert!((line.x1 - frame.x1).abs() < 1e-6);
    }
    let l = layout(db(), &area(LOREM, style(12.0), frame, Justify::JustifyLeft));
    let n = l.lines.len();
    for line in &l.lines[..n - 1] {
        assert!((line.x0 - frame.x0).abs() < 1e-6 && (line.x1 - frame.x1).abs() < 1e-6, "{line:?}");
    }
    assert!(l.lines[n - 1].x1 < frame.x1 - 1.0, "last line not justified");
    let l = layout(db(), &area("Wide", style(12.0), frame, Justify::JustifyAll));
    assert!((l.lines[0].x1 - frame.x1).abs() < 1e-6);
}

#[test]
fn indents_and_paragraph_spacing() {
    let frame = Rect::new(0.0, 0.0, 200.0, 400.0);
    let mut t = area("para one\npara two", style(10.0), frame, Justify::Left);
    t.para.left_indent = 10.0;
    t.para.first_line_indent = 5.0;
    t.para.space_before = 6.0;
    let l = layout(db(), &t);
    assert!((l.lines[0].x0 - 15.0).abs() < 1e-9);
    assert!((l.lines[1].baseline - l.lines[0].baseline - 18.0).abs() < 1e-9);
}

#[test]
fn each_paragraph_has_its_own_attributes() {
    let frame = Rect::new(0.0, 0.0, 200.0, 400.0);
    let mut t = area("left\ncentred\nright", style(10.0), frame, Justify::Left);
    let centred = vectorcraft_doc::ParaStyle { justify: Justify::Center, space_before: 20.0, ..Default::default() };
    let right = vectorcraft_doc::ParaStyle { justify: Justify::Right, left_indent: 7.0, ..Default::default() };
    t.set_paragraph_styles(vec![t.para.clone(), centred, right]);
    let l = layout(db(), &t);
    assert_eq!(l.lines.len(), 3);
    assert!(l.lines[0].x0.abs() < 1e-9, "the first paragraph stays left");
    let mid = (l.lines[1].x0 + l.lines[1].x1) / 2.0;
    assert!((mid - 100.0).abs() < 1e-6, "the second is centred: {mid}");
    assert!((l.lines[2].x1 - 200.0).abs() < 1e-6, "the third is right-aligned");
    // Space before applies to the second paragraph only (leading 12 + 20).
    assert!((l.lines[1].baseline - l.lines[0].baseline - 32.0).abs() < 1e-9);
    assert!((l.lines[2].baseline - l.lines[1].baseline - 12.0).abs() < 1e-9);
    // Paragraph 0's style is `para` (what readers that predate per-paragraph styles see).
    assert_eq!(t.paras.len(), 3);
    assert_eq!(t.para_at(0), &t.para);
}

#[test]
fn non_rect_frame_narrows_lines() {
    // Triangle pointing up: lines get wider towards the bottom.
    let tri = PathData::from_bezpath(&{
        let mut p = BezPath::new();
        p.move_to((100.0, 0.0));
        p.line_to((200.0, 200.0));
        p.line_to((0.0, 200.0));
        p.close_path();
        p
    });
    let mut t = point(&LOREM.repeat(2), style(10.0));
    t.kind = TextKind::Area { frame: tri };
    let l = layout(db(), &t);
    assert!(l.lines.len() > 5);
    let w0 = l.lines[0].x1 - l.lines[0].x0;
    let wl = l.lines[l.lines.len() - 2].x1 - l.lines[l.lines.len() - 2].x0;
    assert!(w0 < wl, "{w0} {wl}");
    for line in &l.lines {
        let y = line.baseline - line.ascent;
        let half = y / 2.0; // half-width of the triangle at y
        assert!(line.x0 >= 100.0 - half - 1e-3 && line.x1 <= 100.0 + half + 1e-3, "{line:?}");
    }
}

#[test]
fn caret_mapping_round_trip() {
    let l = layout(db(), &point("Hi there\nnext", style(20.0)));
    let (top, bot) = caret_position(&l, 0);
    assert!(top.x.abs() < 1e-9 && top.y < 0.0 && bot.y > 0.0);
    let (end_top, _) = caret_position(&l, 8);
    assert!((end_top.x - l.lines[0].x1).abs() < 1e-6);
    for b in [0, 1, 3, 8, 9, 11, 13] {
        let (t, bo) = caret_position(&l, b);
        let mid = t.midpoint(bo);
        assert_eq!(hit_byte(&l, mid + Vec2::new(0.1, 0.0)), b, "byte {b}");
    }
    // Second line.
    let (t2, b2) = caret_position(&l, 10);
    assert!(t2.y > 0.0 && b2.y > 24.0);
    assert_eq!(hit_byte(&l, Point::new(1000.0, 24.0)), 13);
    assert_eq!(hit_byte(&l, Point::new(-50.0, -100.0)), 0);
}

#[test]
fn caret_inside_ligature_and_empty_text() {
    let l = layout(db(), &point("fi", serif(20.0)));
    let (a, _) = caret_position(&l, 1);
    let g = &l.glyphs[0];
    assert!((a.x - g.advance / 2.0).abs() < 1e-6);
    let e = layout(db(), &point("", style(20.0)));
    assert_eq!(e.lines.len(), 1);
    let (t, b) = caret_position(&e, 0);
    assert!(t.y < 0.0 && b.y > 0.0);
}

#[test]
fn on_path_placement() {
    let mut line = BezPath::new();
    line.move_to((0.0, 50.0));
    line.line_to((500.0, 50.0));
    let mut t = point("Path", style(20.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.1, end: None };
    let l = layout(db(), &t);
    assert!(l.on_path && !l.overflow);
    assert_eq!(l.glyphs.len(), 4);
    assert!((l.glyphs[0].origin.x - 50.0).abs() < 1e-6);
    for g in &l.glyphs {
        assert!((g.origin.y - 50.0).abs() < 1e-6 && g.angle.abs() < 1e-6);
    }
    // Vertical path downward: glyphs rotated 90°.
    let mut v = BezPath::new();
    v.move_to((0.0, 0.0));
    v.line_to((0.0, 300.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&v), start: 0.0, end: None };
    let l = layout(db(), &t);
    for g in &l.glyphs {
        assert!((g.angle - std::f64::consts::FRAC_PI_2).abs() < 1e-3);
        assert!(g.origin.x.abs() < 1e-6);
    }
    let (top, bot) = caret_position(&l, 0);
    assert!(top.x > 0.0 && bot.x < 0.0, "caret perpendicular to path: {top:?} {bot:?}");
    let mid = l.glyphs[1].origin + Vec2::new(0.0, 0.2);
    assert_eq!(hit_byte(&l, mid), 1);
}

#[test]
fn on_path_effects_orient_glyphs() {
    use vectorcraft_doc::PathEffect;
    let mut diag = BezPath::new();
    diag.move_to((0.0, 0.0));
    diag.line_to((400.0, 400.0));
    let mut t = point("H", style(40.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&diag), start: 0.1, end: None };
    let bbox = |t: &TextObject| layout(db(), t).glyphs[0].outline.bounding_box();
    let rainbow = bbox(&t);
    t.path_effect = PathEffect::StairStep;
    let stair = bbox(&t);
    // Unrotated: as tall as the cap height, narrower than the rotated glyph's box.
    assert!(stair.height() < rainbow.height() && stair.height() > 20.0, "{stair:?} {rainbow:?}");
    t.path_effect = PathEffect::Skew;
    let skew = bbox(&t);
    // Vertical stems stay vertical: the box spans the cap height plus the slant of the baseline.
    assert!(skew.height() > stair.height() && skew.width() < rainbow.width() + 1e-6, "{skew:?}");
    // Gravity on a circle: glyphs point away from the centre (same as Rainbow on a circle).
    let circle = kurbo::Circle::new((0.0, 0.0), 100.0).to_path(0.1);
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&circle), start: 0.0, end: None };
    t.path_effect = PathEffect::Rainbow;
    let a = layout(db(), &t).glyphs[0].outline.bounding_box();
    t.path_effect = PathEffect::Gravity;
    let b = layout(db(), &t).glyphs[0].outline.bounding_box();
    assert!((a.center() - b.center()).hypot() < 2.0, "{a:?} {b:?}");
    assert_eq!(PathEffect::parse("3D Ribbon"), Some(PathEffect::Ribbon3d));
    assert_eq!(PathEffect::parse("stair step"), Some(PathEffect::StairStep));
}

#[test]
fn on_path_circle_and_overflow() {
    let circle = kurbo::Circle::new((0.0, 0.0), 100.0).to_path(0.1);
    let mut t = point("Around the circle", style(14.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&circle), start: 0.0, end: None };
    let l = layout(db(), &t);
    assert!(!l.overflow);
    for g in &l.glyphs {
        let r = g.origin.to_vec2().hypot();
        assert!((r - 100.0).abs() < 2.0, "{r}");
    }
    let mut short = BezPath::new();
    short.move_to((0.0, 0.0));
    short.line_to((30.0, 0.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&short), start: 0.0, end: None };
    let l = layout(db(), &t);
    assert!(l.overflow && l.glyphs.len() < t.plain_text().len());
}

#[test]
fn fallback_font_per_character() {
    let primary = db().face("Source Sans 3", "Regular").unwrap();
    // Find a character the primary lacks but another bundled font has.
    let c = ['\u{2500}', '\u{2192}', '\u{25B6}', '\u{2588}', '\u{21E5}', '\u{2318}']
        .into_iter()
        .find(|&c| !primary.covers(c) && db().fallback_for(c, primary.id()).is_some())
        .expect("some symbol only covered by a non-primary bundled font");
    let text = format!("a{c}b");
    let l = layout(db(), &point(&text, style(12.0)));
    assert_eq!(l.glyphs.len(), 3);
    assert_eq!(l.glyphs[0].font_id, primary.id());
    assert_ne!(l.glyphs[1].font_id, primary.id());
    assert!(!l.glyphs[1].outline.elements().is_empty());
    assert_eq!(l.glyphs[2].font_id, primary.id());
}

/// Characters drawn from a fallback face keep the weight and slant asked for (#2491): bold text
/// takes a bold face of the fallback family when it has one.
#[test]
fn fallback_glyphs_keep_the_weight_of_the_text() {
    let primary = db().face("Source Sans 3", "Bold").unwrap();
    let regular = db().face("Source Sans 3", "Regular").unwrap();
    let c = ['\u{2500}', '\u{2192}', '\u{25B6}', '\u{2588}', '\u{21E5}', '\u{2318}']
        .into_iter()
        .find(|&c| {
            !primary.covers(c)
                && db()
                    .fallback_for(c, regular.id())
                    .is_some_and(|f| db().fallback_for_style(c, regular.id(), 700.0, false).is_some_and(|b| b.weight > f.weight))
        })
        .expect("some symbol covered by a family of the bundled fonts with more than one weight");
    let mut st = style(12.0);
    st.font_style = "Bold".into();
    let l = layout(db(), &point(&format!("a{c}b"), st));
    assert_eq!(l.glyphs[0].font_id, primary.id());
    let drawn = db().face_by_id(l.glyphs[1].font_id).unwrap();
    assert!(drawn.weight >= 600.0, "{} {} for a bold run", drawn.family, drawn.style);
}

/// A missing family's Bold Italic is drawn bold when the fallback family has Bold and Italic but no
/// Bold Italic, not as plain Italic (#2491).
#[test]
fn bold_italic_of_a_missing_family_keeps_the_bold() {
    let face = db().face("No Such Family", "Bold Italic").unwrap();
    assert_eq!(face.family, FALLBACK_FAMILY);
    assert!(face.weight >= 600.0, "{} {}", face.family, face.style);
    let italic = db().face("No Such Family", "Italic").unwrap();
    assert!(italic.italic && italic.weight < 600.0);
}

/// A space the primary font doesn't map goes to the fallback face like any other character
/// (#1844): an ideographic space in a face without U+3000 keeps its em, so an indent made with it
/// isn't lost. A tab, which the layout handles itself, stays with the primary face. Needs a bundled
/// face with U+3000 and あ (craft-fonts; skipped without it).
#[test]
fn whitespace_the_primary_font_lacks_takes_the_fallback_face() {
    let primary = db().face("Source Sans 3", "Regular").unwrap();
    assert!(!primary.covers('\u{3000}'));
    if ['\u{3000}', 'あ'].iter().any(|&c| db().fallback_for(c, primary.id()).is_none()) {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    }
    let l = layout(db(), &point("あ\u{3000}あ", style(20.0)));
    let width: f64 = l.glyphs.iter().map(|g| g.advance).sum();
    assert!((width - 60.0).abs() < 0.01, "あ　あ sets at 3 em: {:?}", l.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>());
    assert_ne!(l.glyphs[1].font_id, primary.id());
    let l = layout(db(), &point("a\tb", style(20.0)));
    assert!(l.glyphs.iter().all(|g| g.font_id == primary.id()));
}

#[test]
fn multiple_runs_and_styles() {
    let mut t = point("", style(10.0));
    t.runs = vec![
        TextRun { text: "Big".into(), style: style(40.0), inline: None },
        TextRun { text: "small".into(), style: CharStyle { font_family: "Inter".into(), ..style(10.0) }, inline: None },
    ];
    let l = layout(db(), &t);
    assert_eq!(l.glyphs.len(), 8);
    assert_eq!(l.glyphs[3].run, 1);
    assert!(l.lines[0].ascent > 30.0);
    assert_ne!(l.glyphs[0].font_id, l.glyphs[3].font_id);
}

#[test]
fn layout_is_fast() {
    let text: String = LOREM.chars().cycle().take(1000).collect();
    let t = area(&text, style(12.0), Rect::new(0.0, 0.0, 300.0, 2000.0), Justify::JustifyLeft);
    let _ = layout(db(), &t); // warm caches
    let n = 20;
    let start = std::time::Instant::now();
    for _ in 0..n {
        let l = layout(db(), &t);
        assert_eq!(l.glyphs.len(), 1000);
    }
    let per = start.elapsed().as_secs_f64() * 1000.0 / n as f64;
    eprintln!("layout of 1000 chars: {per:.3} ms");
    let budget = if cfg!(debug_assertions) { 100.0 } else { 5.0 };
    assert!(per < budget, "{per} ms");
}

#[test]
fn tab_stops_position_text() {
    use vectorcraft_doc::{TabAlign, TabStop};
    let stop = |position: f64, align: TabAlign| TabStop { position, align, leader: String::new(), align_on: '.' };
    let x_of = |t: &TextObject, byte: usize| layout(db(), t).glyphs.iter().find(|g| g.byte == byte).map(|g| g.origin.x).unwrap();
    // Default stops every 36 pt.
    let mut t = point("a\tb", style(12.0));
    assert!((x_of(&t, 2) - 36.0).abs() < 1e-6);
    // A left stop at 100.
    t.para.tabs = vec![stop(100.0, TabAlign::Left)];
    assert!((x_of(&t, 2) - 100.0).abs() < 1e-6);
    // A right stop: the text after the tab ends at 200.
    let mut r = point("a\t12345", style(12.0));
    r.para.tabs = vec![stop(200.0, TabAlign::Right)];
    let lay = layout(db(), &r);
    let last = lay.glyphs.last().unwrap();
    assert!((last.origin.x + last.advance - 200.0).abs() < 1e-6);
    // A decimal stop: the decimal points of two lines line up.
    let mut d = point("x\t12.5\ny\t1234.75", style(12.0));
    d.para.tabs = vec![stop(150.0, TabAlign::Decimal)];
    let lay = layout(db(), &d);
    let dots: Vec<f64> = lay.glyphs.iter().filter(|g| d.plain_text()[g.byte..].starts_with('.')).map(|g| g.origin.x).collect();
    assert_eq!(dots.len(), 2);
    assert!((dots[0] - 150.0).abs() < 1e-6 && (dots[1] - 150.0).abs() < 1e-6, "{dots:?}");
    // Past the last explicit stop, default stops resume.
    let mut p = point("a\tb\tc", style(12.0));
    p.para.tabs = vec![stop(50.0, TabAlign::Left)];
    assert!((x_of(&p, 4) - 72.0).abs() < 1e-6);
}

#[test]
fn a_leading_tab_takes_its_width_in_the_line_and_the_justification() {
    // A tab at the start of area type is 36 pt wide: the line breaks and justifies around that,
    // so no line reaches past the frame's right edge (#2457).
    for justify in [Justify::Left, Justify::JustifyLeft] {
        let t =
            area("\tSample copy flows here while the layout takes shape. Swap these words", style(12.0), Rect::new(0.0, 0.0, 200.0, 400.0), justify);
        let lay = layout(db(), &t);
        let right = lay.glyphs.iter().filter(|g| !t.plain_text()[g.byte..].starts_with(' ')).map(|g| g.origin.x + g.advance).fold(0.0, f64::max);
        assert!(right <= 200.0 + 1e-6, "{justify:?}: the type reaches {right}");
        let first = lay.glyphs.iter().find(|g| g.byte == 1).map(|g| g.origin.x).unwrap();
        assert!((first - 36.0).abs() < 1e-6, "{justify:?}: the text after the tab starts at {first}");
    }
}

#[test]
fn a_centred_or_right_aligned_line_sets_its_tabs_as_if_it_started_at_the_left() {
    use vectorcraft_doc::{TabAlign, TabStop};
    // "A<tab>B" with a stop at 100 pt is 100 pt plus B wide whatever the alignment: the whole line
    // moves, the gap between A and B stays 100 pt, and a right aligned line ends on the edge (#2457).
    for justify in [Justify::Center, Justify::Right, Justify::JustifyCenter, Justify::JustifyRight] {
        let mut t = area("A\tB", style(12.0), Rect::new(0.0, 0.0, 200.0, 100.0), justify);
        t.para.tabs = vec![TabStop { position: 100.0, align: TabAlign::Left, leader: String::new(), align_on: '.' }];
        let lay = layout(db(), &t);
        let at = |byte: usize| lay.glyphs.iter().find(|g| g.byte == byte).map(|g| g.origin.x).unwrap();
        assert!((at(2) - at(0) - 100.0).abs() < 1e-6, "{justify:?}: A at {}, B at {}", at(0), at(2));
        if matches!(justify, Justify::Right | Justify::JustifyRight) {
            let b = lay.glyphs.iter().find(|g| g.byte == 2).unwrap();
            assert!((b.origin.x + b.advance - 200.0).abs() < 1e-6, "{justify:?}: the line ends at {}", b.origin.x + b.advance);
        }
    }
}

#[test]
fn tab_leaders_line_up_from_line_to_line_and_stop_short_of_the_stop() {
    use vectorcraft_doc::{TabAlign, TabStop};
    let mut t = point("A\t1\nAAAAA\t2", style(12.0));
    t.para.tabs = vec![TabStop { position: 120.0, align: TabAlign::Left, leader: ".".into(), align_on: '.' }];
    let lay = layout(db(), &t);
    // The left edges of the dots of each line's tab.
    let dots = |byte: usize| -> Vec<f64> {
        let tab = lay.glyphs.iter().find(|g| g.byte == byte).unwrap();
        let mut xs: Vec<f64> = vec![];
        for el in tab.outline.elements() {
            if let kurbo::PathEl::MoveTo(p) = el {
                xs.push(p.x);
            }
        }
        xs.sort_by(f64::total_cmp);
        xs.dedup_by(|a, b| (*a - *b).abs() < 1e-6);
        xs
    };
    let (short, long) = (dots(1), dots(9));
    assert!(short.len() > long.len() && long.len() > 10, "{short:?} {long:?}");
    // The longer line's dots are the shorter line's last ones, at the same x.
    let tail = &short[short.len() - long.len()..];
    assert!(tail.iter().zip(&long).all(|(a, b)| (a - b).abs() < 1e-6), "{tail:?} {long:?}");
    assert!(short.last().is_some_and(|&x| x < 120.0));
}

/// Bullets: a list paragraph's first line starts with its bullet, glyphs that stand for no
/// character, in the run and size of the paragraph's first character. After the bullet the text
/// starts at the left indent, and the lines it wraps to start there too. The caret before the
/// first character stands at the text, a click on the bullet puts it there, an underline leaves
/// the bullet out, and a paragraph without text has no bullet.
#[test]
fn a_list_paragraph_starts_with_its_bullet_and_its_text_at_the_left_indent() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let bulleted = |text: &str, size: f64| {
        let mut t = area(text, style(size), Rect::new(0.0, 0.0, 200.0, 400.0), Justify::Left);
        t.para.left_indent = 18.0;
        t.para.first_line_indent = -18.0;
        t.para.list = Some(ParaList::bulleted(Bullet::Dot));
        t
    };
    let t = bulleted(LOREM, 12.0);
    let lay = layout(db(), &t);
    assert_eq!(lay.markers.len(), 1, "{:?}", lay.markers);
    let m = &lay.markers[0];
    assert_eq!((m.text.as_str(), m.glyphs.start, m.glyphs.len()), ("\u{2022}", lay.lines[0].glyph_start, 2));
    let marker = &lay.glyphs[m.glyphs.clone()];
    assert!(marker.iter().all(|g| g.len == 0 && g.byte == 0 && g.run == 0 && g.line == 0), "{marker:?}");
    assert!(marker[0].origin.x.abs() < 1e-6, "the bullet starts the first line at its first-line indent: {}", marker[0].origin.x);
    assert!(!marker[0].outline.elements().is_empty() && marker[1].outline.elements().is_empty(), "the bullet draws, its tab doesn't");
    assert!((marker[1].origin.x + marker[1].advance - 18.0).abs() < 1e-6, "the tab reaches the left indent");
    // The text starts at the left indent on the first line and on every line it wraps to.
    assert!(lay.lines.len() > 2, "the paragraph wraps");
    for (i, line) in lay.lines.iter().enumerate() {
        let first = lay.glyphs[line.glyph_start..line.glyph_end].iter().find(|g| g.len > 0).unwrap();
        assert!((first.origin.x - 18.0).abs() < 1e-6, "line {i} starts at {}", first.origin.x);
    }
    let (top, _) = caret_position(&lay, 0);
    assert!((top.x - 18.0).abs() < 1e-6, "the caret before the first character: {top:?}");
    assert_eq!(hit_byte(&lay, Point::new(marker[0].origin.x + 1.0, lay.lines[0].baseline - 3.0)), 0);
    // A selection from the paragraph's start highlights the text, not the bullet.
    let quads = selection_quads(&lay, 0, 5);
    assert!((quads[0][0].x - 18.0).abs() < 1e-6, "the selection starts at the text: {:?}", quads[0]);
    // The bullet takes the size of the paragraph's first character.
    let big = layout(db(), &bulleted(LOREM, 24.0));
    assert!((big.glyphs[0].advance - 2.0 * lay.glyphs[0].advance).abs() < 1e-6, "{} {}", big.glyphs[0].advance, lay.glyphs[0].advance);
    // A baseline shift or rotation of the first character leaves the bullet on the baseline.
    let mut raised = bulleted("Raised", 12.0);
    raised.runs[0].style.baseline_shift = 5.0;
    raised.runs[0].style.rotation = 30.0;
    let lay = layout(db(), &raised);
    let (bullet, first) = (&lay.glyphs[0], lay.glyphs.iter().find(|g| g.len > 0).unwrap());
    assert!((bullet.xf.translation().y - lay.lines[0].baseline).abs() < 1e-6, "{:?}", bullet.xf);
    assert!((bullet.xf.as_coeffs()[1]).abs() < 1e-9 && first.xf.as_coeffs()[1].abs() > 1e-3, "the bullet isn't rotated");
    // An underline starts at the text, not under the bullet.
    let mut underlined = bulleted("Underlined", 12.0);
    underlined.runs[0].style.underline = true;
    let lay = layout(db(), &underlined);
    let bars = decorations(&lay, db(), &underlined);
    assert_eq!(bars.len(), 1);
    assert!((bars[0].1.bounding_box().x0 - 18.0).abs() < 1e-6, "{:?}", bars[0].1.bounding_box());
    // Only the paragraphs with text have a bullet.
    let lay = layout(db(), &bulleted("One\n\nThree", 12.0));
    assert_eq!(lay.markers.iter().map(|m| lay.glyphs[m.glyphs.start].byte).collect::<Vec<_>>(), [0, 5]);
}

/// Bullets: in vertical type the bullet is drawn the same whether or not the paragraph's first
/// characters are set tate-chu-yoko.
#[test]
fn a_vertical_list_paragraphs_bullet_is_not_set_tate_chu_yoko() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let bullet = |tcy: bool| {
        let mut t = point("", style(12.0));
        t.vertical = true;
        t.runs = vec![TextRun::new("12", CharStyle { tate_chu_yoko: tcy, ..style(12.0) }), TextRun::new(" vertical", style(12.0))];
        t.para.list = Some(ParaList::bulleted(Bullet::Dot));
        let lay = layout(db(), &t);
        let g = &lay.glyphs[lay.markers[0].glyphs.start];
        (g.advance, g.xf)
    };
    let (plain, set) = (bullet(false), bullet(true));
    let same = (plain.0 - set.0).abs() < 1e-9 && plain.1.as_coeffs().iter().zip(set.1.as_coeffs()).all(|(a, b)| (a - b).abs() < 1e-9);
    assert!(same, "{plain:?} {set:?}");
}

/// Bullets: a right-to-left list paragraph starts at its right with the bullet, then the gap, then
/// the text, and the caret before its first character stands at the text.
#[test]
fn a_right_to_left_list_paragraph_starts_with_its_bullet_at_the_right() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let mut t = area("שלום עולם", style(12.0), Rect::new(0.0, 0.0, 200.0, 100.0), Justify::Auto);
    t.para.list = Some(ParaList::bulleted(Bullet::Dot));
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let lay = layout(db(), &t);
    let m = &lay.markers[0];
    assert_eq!(m.tab, m.glyphs.start, "the tab is drawn first, left of the bullet");
    assert_eq!((lay.marker_text(m.tab), lay.marker_text(m.glyphs.start + 1)), (Some("\t"), Some("\u{2022}")));
    let (tab, bullet) = (&lay.glyphs[m.tab], &lay.glyphs[m.glyphs.start + 1]);
    assert!((bullet.origin.x + bullet.advance - 200.0).abs() < 1e-6, "the bullet ends the line at the right: {bullet:?}");
    assert!((tab.origin.x + tab.advance - bullet.origin.x).abs() < 1e-6, "the gap is right before the bullet");
    let text_right = lay.glyphs.iter().filter(|g| g.len > 0).map(|g| g.origin.x + g.advance).fold(0.0, f64::max);
    assert!((text_right - tab.origin.x).abs() < 1e-6, "the text ends at the gap: {text_right} {}", tab.origin.x);
    let (top, _) = caret_position(&lay, 0);
    assert!((top.x - text_right).abs() < 1e-6, "the caret before the first character: {top:?}");
    // A selection across two right-to-left list items highlights their characters alone.
    let mut two = area("שלום\nעולם", style(12.0), Rect::new(0.0, 0.0, 200.0, 100.0), Justify::Auto);
    two.para.list = Some(ParaList::bulleted(Bullet::Dot));
    let lay = layout(db(), &two);
    assert_eq!(lay.markers.len(), 2);
    let quads = selection_quads(&lay, 0, two.plain_text().len());
    assert_eq!(quads.len(), lay.glyphs.iter().filter(|g| g.len > 0).count(), "one quad per character glyph: {quads:?}");
}

/// Bullets: a right-to-left list paragraph has its indents mirrored. Its bullet hangs at the right,
/// its right indent is kept at the left, and the lines it wraps to start at the right where the
/// first line's text starts after the bullet. A right-to-left paragraph without a list keeps its
/// left and first-line indents at the left, and a left-to-right list keeps its bullet at the left.
#[test]
fn a_right_to_left_list_paragraph_mirrors_its_hanging_indent() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let hebrew = "שלום עולם ".repeat(12);
    let para = |text: &str, justify: Justify, list: bool| {
        let mut t = area(text.trim_end(), style(12.0), Rect::new(0.0, 0.0, 200.0, 400.0), justify);
        t.para.left_indent = 18.0;
        t.para.first_line_indent = -18.0;
        t.para.right_indent = 5.0;
        t.para.list = list.then(|| ParaList::bulleted(Bullet::Dot));
        t
    };
    // The left and right ends of line `k`'s glyphs, its spaces left out.
    let ends = |t: &TextObject, lay: &TextLayout, k: usize| {
        let line = &lay.lines[k];
        let text = t.plain_text();
        let glyphs = lay.glyphs[line.glyph_start..line.glyph_end].iter().filter(|g| !text[g.byte..].starts_with(' '));
        let (l, r) = glyphs.fold((f64::MAX, f64::MIN), |(l, r), g| (l.min(g.origin.x), r.max(g.origin.x + g.advance)));
        (l, r)
    };
    let close = |(l, r): (f64, f64), (el, er): (f64, f64)| (l - el).abs() < 1e-6 && (r - er).abs() < 1e-6;
    for justify in [Justify::Auto, Justify::JustifyLeft] {
        let t = para(&hebrew, justify, true);
        let lay = layout(db(), &t);
        assert!(lay.lines.len() > 2, "{justify:?}: the paragraph wraps");
        let m = &lay.markers[0];
        let (tab, bullet) = (&lay.glyphs[m.tab], &lay.glyphs[m.glyphs.start + 1]);
        assert!((bullet.origin.x + bullet.advance - 200.0).abs() < 1e-6, "{justify:?}: the bullet ends the first line at the right: {bullet:?}");
        assert!(
            (tab.origin.x - 182.0).abs() < 1e-6,
            "{justify:?}: the text after the bullet starts at the left indent from the right: {}",
            tab.origin.x
        );
        // Justify with Last Line Aligned Left sets the last line at the left.
        let wrapped = if justify == Justify::JustifyLeft { 1..lay.lines.len() - 1 } else { 1..lay.lines.len() };
        for k in wrapped {
            let (_, r) = ends(&t, &lay, k);
            assert!((r - tab.origin.x).abs() < 1e-6, "{justify:?}: line {k}'s right edge is at {r}, the first line's text's at {}", tab.origin.x);
        }
        if justify == Justify::JustifyLeft {
            assert!(close(ends(&t, &lay, 0), (5.0, 200.0)), "the first line: {:?}", ends(&t, &lay, 0));
            assert!(close(ends(&t, &lay, 1), (5.0, 182.0)), "a wrapped line: {:?}", ends(&t, &lay, 1));
        }
    }
    // Without a list a right-to-left paragraph keeps its indents: the left and first-line indents
    // at the left, the right indent at the right.
    let t = para(&hebrew, Justify::JustifyLeft, false);
    let lay = layout(db(), &t);
    assert!(lay.markers.is_empty() && lay.lines.len() > 2);
    assert!(close(ends(&t, &lay, 0), (0.0, 195.0)), "the first line: {:?}", ends(&t, &lay, 0));
    assert!(close(ends(&t, &lay, 1), (18.0, 195.0)), "a wrapped line: {:?}", ends(&t, &lay, 1));
    // A left-to-right list: the bullet at the left, every line's text at the left indent.
    let t = para(LOREM, Justify::JustifyLeft, true);
    let lay = layout(db(), &t);
    assert!(lay.lines.len() > 2);
    assert!(lay.glyphs[lay.markers[0].glyphs.start].origin.x.abs() < 1e-6, "the bullet starts the first line");
    assert!(close(ends(&t, &lay, 0), (0.0, 195.0)), "the first line: {:?}", ends(&t, &lay, 0));
    assert!(close(ends(&t, &lay, 1), (18.0, 195.0)), "a wrapped line: {:?}", ends(&t, &lay, 1));
}

/// Bullets: area type in a frame that isn't a rectangle, centered vertically, flows again until it
/// settles, and keeps one marker for each list paragraph, at that paragraph's bullet.
#[test]
fn vertically_centered_list_paragraphs_in_a_round_frame_keep_one_bullet_each() {
    use vectorcraft_doc::VerticalAlign;
    use vectorcraft_doc::text::{Bullet, ParaList};
    let text = format!("{LOREM}\n{LOREM}");
    let frame = kurbo::Ellipse::new((150.0, 150.0), (150.0, 150.0), 0.0).to_path(0.1);
    let mut t = area(&text, style(12.0), Rect::new(0.0, 0.0, 300.0, 300.0), Justify::Left);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame) };
    t.para.list = Some(ParaList::bulleted(Bullet::Dot));
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let top = layout(db(), &t);
    t.area.vertical_align = VerticalAlign::Center;
    let lay = layout(db(), &t);
    assert!(lay.lines[0].baseline > top.lines[0].baseline + 1.0, "the text moves down");
    let starts: Vec<usize> = lay.markers.iter().map(|m| m.glyphs.start).collect();
    assert_eq!(starts.len(), 2, "{:?}", lay.markers);
    for (m, para_start) in lay.markers.iter().zip([0, LOREM.len() + 1]) {
        let glyphs = &lay.glyphs[m.glyphs.clone()];
        assert!(glyphs.iter().all(|g| g.len == 0 && g.byte == para_start), "{m:?}: {glyphs:?}");
        assert!(!glyphs[0].outline.elements().is_empty(), "the bullet draws");
    }
}

/// Bullets: in a frame narrower at the top than the bullet, its gap and the first word, the first
/// line goes down to where they fit, and every line stays inside the frame.
#[test]
fn a_list_paragraph_starts_where_its_bullet_and_first_word_fit() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let mut triangle = BezPath::new();
    triangle.move_to((150.0, 0.0));
    triangle.line_to((300.0, 300.0));
    triangle.line_to((0.0, 300.0));
    triangle.close_path();
    let mut t = area("Extraordinarily long words follow", style(12.0), Rect::new(0.0, 0.0, 300.0, 300.0), Justify::Left);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&triangle) };
    t.para.list = Some(ParaList::bulleted(Bullet::Dot));
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let lay = layout(db(), &t);
    assert_eq!(lay.markers.len(), 1);
    for (i, line) in lay.lines.iter().enumerate() {
        for g in &lay.glyphs[line.glyph_start..line.glyph_end] {
            let (l, r) = (g.origin.x, g.origin.x + g.advance);
            assert!(l >= line.avail.0 - 0.01 && r <= line.avail.1 + 0.01, "line {i}: {l}..{r} outside {:?}", line.avail);
        }
    }
}

/// Bullets: type on a path starts with its bullet and the gap after it, the caret before its
/// first character stands at the text, and a selection of the text highlights the characters
/// alone.
#[test]
fn type_on_a_path_starts_with_its_bullet() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let mut line = BezPath::new();
    line.move_to((0.0, 50.0));
    line.line_to((500.0, 50.0));
    let mut t = point("Path", style(20.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    t.para.list = Some(ParaList::bulleted(Bullet::Dot));
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let lay = layout(db(), &t);
    assert_eq!(lay.markers.len(), 1, "{:?}", lay.markers);
    let m = &lay.markers[0];
    assert_eq!((m.glyphs.clone(), m.tab, lay.marker_text(0)), (0..2, 1, Some("\u{2022}")));
    assert!(lay.glyphs[0].origin.x.abs() < 1e-6 && !lay.glyphs[0].outline.elements().is_empty(), "{:?}", lay.glyphs[0].origin);
    let first = lay.glyphs.iter().find(|g| g.len > 0).unwrap();
    assert!((first.origin.x - 18.0).abs() < 1e-6, "the text starts at {}", first.origin.x);
    let (top, _) = caret_position(&lay, 0);
    assert!((top.x - 18.0).abs() < 1e-6, "the caret before the first character: {top:?}");
    let quads = selection_quads(&lay, 0, 4);
    assert_eq!(quads.len(), 4, "one quad per character");
    assert!(quads.iter().all(|q| q[0].x >= 18.0 - 1e-6), "{quads:?}");
    // A right-to-left list item on a path reads from its bullet: the bullet is drawn right of the
    // text, with the gap between them.
    let mut rtl = point("שלום", style(20.0));
    rtl.kind = t.kind.clone();
    rtl.para = t.para.clone();
    let lay = layout(db(), &rtl);
    let m = &lay.markers[0];
    let (tab, bullet) = (&lay.glyphs[m.tab], &lay.glyphs[m.glyphs.start + 1]);
    let text_right = lay.glyphs.iter().filter(|g| g.len > 0).map(|g| g.origin.x + g.advance).fold(f64::MIN, f64::max);
    assert!((tab.origin.x - text_right).abs() < 1e-6 && (bullet.origin.x - tab.origin.x - tab.advance).abs() < 1e-6, "{tab:?} {bullet:?}");
    assert!((tab.advance + bullet.advance - 18.0).abs() < 1e-6, "the bullet and its gap take the left indent: {} {}", tab.advance, bullet.advance);
    // Optical Margin Alignment leaves a dash bullet at the start bracket, with room before it.
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.1, end: None };
    t.para.list = Some(ParaList::bulleted(Bullet::Dash));
    for optical in [false, true] {
        t.optical_margin = optical;
        let lay = layout(db(), &t);
        assert!((lay.glyphs[0].origin.x - 50.0).abs() < 1e-6, "optical {optical}: the dash starts at {}", lay.glyphs[0].origin.x);
    }
}

/// Bullets: after the bullet the text starts at an explicit tab stop that comes before the left
/// indent, at the next stop past the bullet when the bullet reaches past the left indent, and a
/// line holds the bullet and the start of its first word however narrow the frame. Justified lines
/// and Optical Margin Alignment leave the bullet and the gap after it as they are.
#[test]
fn a_list_paragraph_starts_its_text_at_the_next_tab_stop_after_the_bullet() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    use vectorcraft_doc::{TabAlign, TabStop};
    let text_x = |t: &TextObject| layout(db(), t).glyphs.iter().find(|g| g.byte == 0 && g.len > 0).map(|g| g.origin.x).unwrap();
    for (text, justify, optical) in
        [(LOREM, Justify::JustifyLeft, false), (LOREM, Justify::Left, true), (LOREM, Justify::JustifyAll, true), ("Item", Justify::JustifyAll, false)]
    {
        let mut t = area(text, style(12.0), Rect::new(0.0, 0.0, 200.0, 400.0), justify);
        t.para.list = Some(ParaList::bulleted(Bullet::Dash));
        t.para.left_indent = 18.0;
        t.para.first_line_indent = -18.0;
        t.optical_margin = optical;
        let lay = layout(db(), &t);
        let marker = &lay.glyphs[lay.markers[0].glyphs.clone()];
        assert!(marker[0].origin.x.abs() < 1e-6, "{justify:?}, optical {optical}: the dash starts at {}", marker[0].origin.x);
        assert!((text_x(&t) - 18.0).abs() < 1e-6, "{justify:?}, optical {optical}: the text starts at {}", text_x(&t));
        if justify != Justify::Left && !optical {
            let line = &lay.lines[0];
            let ends = lay.glyphs[line.glyph_start..line.glyph_end]
                .iter()
                .filter(|g| !t.plain_text()[g.byte..].starts_with(' '))
                .map(|g| g.origin.x + g.advance)
                .fold(0.0, f64::max);
            assert!((ends - 200.0).abs() < 1e-6, "{justify:?}: the justified first line ends at {ends}");
        }
    }
    // A right-aligned item ends at the frame's right edge, its bullet and gap before it.
    let mut right = area("Item", style(12.0), Rect::new(0.0, 0.0, 200.0, 100.0), Justify::Right);
    right.para.list = Some(ParaList::bulleted(Bullet::Dot));
    right.para.left_indent = 18.0;
    right.para.first_line_indent = -18.0;
    let lay = layout(db(), &right);
    let last = lay.glyphs.last().unwrap();
    assert!((last.origin.x + last.advance - 200.0).abs() < 1e-6, "the line ends at {}", last.origin.x + last.advance);
    let marker = &lay.glyphs[lay.markers[0].glyphs.clone()];
    assert!((marker[1].origin.x + marker[1].advance - text_x(&right)).abs() < 1e-6, "the text follows the gap");
    let mut t = area("Item", style(12.0), Rect::new(0.0, 0.0, 200.0, 100.0), Justify::Left);
    t.para.list = Some(ParaList::bulleted(Bullet::Dash));
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    t.para.tabs = vec![TabStop { position: 12.0, align: TabAlign::Left, leader: String::new(), align_on: '.' }];
    assert!((text_x(&t) - 12.0).abs() < 1e-6, "{}", text_x(&t));
    // No indents: the bullet starts the line and the text goes to the next default stop.
    t.para.tabs.clear();
    t.para.left_indent = 0.0;
    t.para.first_line_indent = 0.0;
    assert!((text_x(&t) - 36.0).abs() < 1e-6, "{}", text_x(&t));
    // A frame with no room for a letter after the bullet: the first line still holds the bullet and
    // the word's first letter.
    let mut narrow = area("Pneumonoultramicroscopic", style(12.0), Rect::new(0.0, 0.0, 20.0, 400.0), Justify::Left);
    narrow.para.list = Some(ParaList::bulleted(Bullet::Dot));
    narrow.para.left_indent = 18.0;
    narrow.para.first_line_indent = -18.0;
    let lay = layout(db(), &narrow);
    let first = &lay.glyphs[lay.lines[0].glyph_start..lay.lines[0].glyph_end];
    assert!(first.iter().any(|g| g.len > 0), "the first line holds more than the bullet: {:?}", lay.lines);
}

/// The list markers of `t`'s layout, in text order.
fn marker_texts(t: &TextObject) -> Vec<String> {
    layout(db(), t).markers.iter().map(|m| m.text.clone()).collect()
}

/// Area type `text` whose paragraphs have the lists `lists` and an 18 pt hanging indent.
fn listed(text: &str, lists: &[Option<vectorcraft_doc::text::ParaList>]) -> TextObject {
    let mut t = area(text, style(12.0), Rect::new(0.0, 0.0, 200.0, 400.0), Justify::Left);
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let styles = lists.iter().map(|&list| vectorcraft_doc::ParaStyle { list, ..t.para.clone() }).collect();
    t.set_paragraph_styles(styles);
    t
}

/// Numbered lists: each list paragraph starts with its number and a period, counted through the
/// list paragraphs before it, and its text starts at the left indent after the number. An empty
/// paragraph has no number and is not counted, a bulleted paragraph keeps its bullet and is not
/// counted, and a paragraph in no list starts the count again. Each number style writes its
/// numbers and rolls over, and a number wider than the indent takes the text to the next tab stop.
#[test]
fn numbered_list_paragraphs_are_numbered_in_order() {
    use vectorcraft_doc::text::{Bullet, NumberStyle, ParaList};
    let numbered = Some(ParaList::numbered(NumberStyle::Arabic));
    let t = listed("One\nTwo\nThree", &[numbered; 3]);
    assert_eq!(marker_texts(&t), ["1.", "2.", "3."]);
    let lay = layout(db(), &t);
    for (k, m) in lay.markers.iter().enumerate() {
        let number = &lay.glyphs[m.glyphs.start];
        let line = &lay.lines[number.line];
        let text = lay.glyphs[line.glyph_start..line.glyph_end].iter().find(|g| g.len > 0).unwrap();
        assert!(number.origin.x.abs() < 1e-6 && (text.origin.x - 18.0).abs() < 1e-6, "item {k}: {} {}", number.origin.x, text.origin.x);
        assert_eq!((m.tab, lay.marker_text(m.tab)), (m.glyphs.end - 1, Some("\t")), "item {k}");
    }
    assert_eq!(marker_texts(&listed("One\n\nTwo", &[numbered; 3])), ["1.", "2."]);
    assert_eq!(marker_texts(&listed("A\nB\nC\nD\nE", &[numbered, numbered, None, numbered, numbered])), ["1.", "2.", "1.", "2."]);
    assert_eq!(marker_texts(&listed("A\nB\nC", &[numbered, Some(ParaList::bulleted(Bullet::Dash)), numbered])), ["1.", "\u{2013}", "2."]);
    // The count goes on from the earlier frames of a thread.
    let mut t = listed("One\nTwo", &[numbered; 2]);
    t.list_count = 9;
    assert_eq!(marker_texts(&t), ["10.", "11."]);
    // Each style, from the count before its rollover.
    for (style, before, numbers) in [
        (NumberStyle::Arabic, 0, ["1.", "2."]),
        (NumberStyle::UpperLetters, 25, ["Z.", "AA."]),
        (NumberStyle::LowerLetters, 25, ["z.", "aa."]),
        (NumberStyle::UpperRoman, 3, ["IV.", "V."]),
        (NumberStyle::LowerRoman, 8, ["ix.", "x."]),
        (NumberStyle::TwoDigits, 98, ["99.", "100."]),
        (NumberStyle::ThreeDigits, 998, ["999.", "1000."]),
    ] {
        let mut t = listed("One\nTwo", &[Some(ParaList::numbered(style)); 2]);
        t.list_count = before;
        assert_eq!(marker_texts(&t), numbers, "{style:?}");
    }
    // "1000." is wider than the 18 pt before the left indent: the text goes to the next default stop.
    let mut wide = listed("One", &[Some(ParaList::numbered(NumberStyle::ThreeDigits))]);
    wide.list_count = 999;
    let lay = layout(db(), &wide);
    let number_width: f64 = lay.glyphs[..lay.markers[0].tab].iter().map(|g| g.advance).sum();
    let text = lay.glyphs.iter().find(|g| g.len > 0).unwrap();
    assert!(number_width > 18.0 && (text.origin.x - 36.0).abs() < 1e-6, "{number_width} {}", text.origin.x);
    // Type on a path numbers its paragraphs too.
    let mut line = BezPath::new();
    line.move_to((0.0, 50.0));
    line.line_to((500.0, 50.0));
    let mut on_path = listed("One\nTwo", &[numbered; 2]);
    on_path.kind = TextKind::OnPath { path: PathData::from_bezpath(&line), start: 0.0, end: None };
    assert_eq!(marker_texts(&on_path), ["1.", "2."]);
}

/// Numbered lists: a right-to-left list paragraph's number ends its first line at the right and
/// reads left to right, with its period at its left and the gap before the text left of that.
#[test]
fn a_right_to_left_numbered_paragraph_has_its_period_left_of_its_number() {
    use vectorcraft_doc::text::{NumberStyle, ParaList};
    let numbered = Some(ParaList::numbered(NumberStyle::Arabic));
    // The glyphs of "1", "2" and "." in a left-to-right "12.".
    let mut ltr = listed("One", &[numbered]);
    ltr.list_count = 11;
    let lay = layout(db(), &ltr);
    let gids: Vec<u32> = lay.glyphs[..lay.markers[0].tab].iter().map(|g| g.gid).collect();
    assert_eq!(gids.len(), 3, "{gids:?}");
    let mut rtl = listed("שלום עולם", &[numbered]);
    rtl.edit_paras(None, |p| p.justify = Justify::Auto);
    rtl.list_count = 11;
    let lay = layout(db(), &rtl);
    let m = &lay.markers[0];
    assert_eq!(m.text, "12.");
    assert_eq!(m.tab, m.glyphs.start, "the gap is drawn first, left of the number");
    let drawn = &lay.glyphs[m.glyphs.start + 1..m.glyphs.end];
    assert_eq!(drawn.iter().map(|g| g.gid).collect::<Vec<_>>(), [gids[2], gids[0], gids[1]], "drawn left to right: the period, 1, 2");
    assert!(drawn.windows(2).all(|w| w[0].origin.x < w[1].origin.x), "{drawn:?}");
    let last = drawn.last().unwrap();
    assert!((last.origin.x + last.advance - 200.0).abs() < 1e-6, "the number ends the line at the right: {last:?}");
}

/// Are the Japanese craft-fonts faces built in? Tests of Japanese glyphs skip (and say so) when
/// they aren't: the bundled fonts have none, and [`db`] reads no system fonts.
fn japanese_fonts() -> bool {
    let built_in = CRAFT_FONTS.iter().any(|f| f.is_japanese());
    if !built_in {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
    }
    built_in
}

#[test]
fn japanese_text_uses_the_craft_fonts_mincho_after_the_bundled_fonts() {
    if !japanese_fonts() {
        return;
    }
    let l = layout(db(), &point("日本語の文字", style(20.0)));
    assert_eq!(l.glyphs.len(), 6);
    assert!(l.glyphs.iter().all(|g| g.gid != 0 && !g.outline.elements().is_empty()), "real glyphs, no tofu");
    let face = db().face_covering('日').unwrap();
    assert!(face.family.contains("Mincho"), "document text falls back to a Mincho face: {}", face.family);
    assert!(CRAFT_FONTS.iter().any(|f| f.family == face.family));
    // Latin keeps the bundled fallback family.
    assert_eq!(db().face_covering('a').unwrap().family, FALLBACK_FAMILY);
}

#[test]
fn all_supplied_craft_font_families_are_registered_without_system_fonts() {
    if CRAFT_FONTS.is_empty() {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a checkout)");
        return;
    }
    let db = FontDb::with_font_dirs(vec![]);
    for font in CRAFT_FONTS {
        let face = db.face(font.family, font.style).unwrap();
        assert_eq!(face.family, font.family, "{} {}", font.family, font.style);
    }
}

#[test]
fn japanese_text_lays_out_without_the_craft_fonts() {
    // Built either way, Japanese text lays out (as missing glyphs when no font has them).
    let l = layout(db(), &point("日本語の文字 abc", style(20.0)));
    assert_eq!(l.lines.len(), 1);
    assert!(l.glyphs.iter().all(|g| g.advance.is_finite()));
    if CRAFT_FONTS.is_empty() {
        assert!(db().families().iter().all(|f| !f.contains("Mincho") && !f.contains("Gothic")), "no Japanese font is bundled");
    }
}

#[test]
fn vertical_japanese_columns_have_upright_ink_and_edit_geometry() {
    if !japanese_fonts() {
        return;
    }
    let mut t = point("日本語\n縦書き", style(30.0));
    t.vertical = true;
    let l = layout(db(), &t);
    assert!(l.vertical);
    assert_eq!(l.lines.len(), 2);
    assert!(l.glyphs.iter().all(|g| g.gid != 0 && !g.outline.elements().is_empty()));
    assert!(l.glyphs[3].origin.x < l.glyphs[0].origin.x);
    assert!(l.glyphs[1].origin.y > l.glyphs[0].origin.y);
    for g in &l.glyphs {
        let p = g.origin + dir(g.angle) * (g.advance * 0.1);
        assert_eq!(hit_byte(&l, p), g.byte);
        let (a, b) = caret_position(&l, g.byte);
        assert!((a.y - b.y).abs() < 1e-6);
        assert!(a.x > b.x);
    }
    let selection = selection_quads(&l, 0, "日本語".len());
    assert_eq!(selection.len(), 1);
}

#[test]
fn vertical_area_type_wraps_into_columns_inside_the_frame() {
    let mut t = area("日本語日本語日本語", style(20.0), Rect::new(0.0, 0.0, 100.0, 65.0), Justify::Left);
    t.vertical = true;
    let l = layout(db(), &t);
    assert!(l.lines.len() > 1);
    assert!(l.glyphs.iter().all(|g| g.origin.x >= 0.0 && g.origin.x <= 100.0 && g.origin.y >= 0.0 && g.origin.y <= 65.0));
    assert!(l.glyphs[l.lines[1].glyph_start].origin.x < l.glyphs[0].origin.x);
}

/// Justified Japanese lines (no word spaces) spread the leftover room over the gaps between their
/// characters (JLREQ 3.8), not inside a Latin word; the last line of a paragraph stays set solid.
#[test]
fn justified_japanese_lines_spread_between_characters() {
    // The ideographs' advance in the fonts at hand, measured; five fit a line, 10 pt left over.
    let measure = layout(db(), &point("一", style(20.0)));
    let a = measure.glyphs[0].advance;
    let width = 5.0 * a + 10.0;
    let t = area("一二三四五六七", style(20.0), Rect::new(0.0, 0.0, width, 400.0), Justify::JustifyLeft);
    let l = layout(db(), &t);
    let first = &l.glyphs[l.lines[0].glyph_start..l.lines[0].glyph_end];
    assert_eq!(first.len(), 5);
    for w in first.windows(2) {
        assert!((w[1].origin.x - w[0].origin.x - (a + 2.5)).abs() < 0.01, "10 pt over 4 gaps");
    }
    let end = first.last().map(|g| g.origin.x + a).unwrap();
    assert!((end - width).abs() < 0.01, "the line reaches the frame's edge: {end} vs {width}");
    // The last line keeps its natural spacing.
    let last = &l.glyphs[l.lines[1].glyph_start..l.lines[1].glyph_end];
    assert!((last[1].origin.x - last[0].origin.x - a).abs() < 0.01);
    // A Latin word inside a Japanese line keeps its letters together; the gaps around it share.
    let t = area("一二ABC三四五六七八九十一二三", style(20.0), Rect::new(0.0, 0.0, 8.0 * a + 10.0, 400.0), Justify::JustifyLeft);
    let l = layout(db(), &t);
    let g = &l.glyphs[l.lines[0].glyph_start..l.lines[0].glyph_end];
    let at = g.iter().position(|g| g.byte == "一二".len()).unwrap();
    let solid =
        |k: usize| (g[k + 1].origin.x - g[k].origin.x - layout(db(), &point(&"ABC"[k - at..=k - at], style(20.0))).glyphs[0].advance).abs() < 0.01;
    assert!(solid(at) && solid(at + 1), "A–B–C set solid");
    assert!(g[at].origin.x - g[at - 1].origin.x > a + 0.1, "二–A takes its share");
}

/// Leading measured from em box top to em box top (Japanese layout's model): area type's first line
/// touches the frame's top; lines of one size are as far apart as with baseline-to-baseline leading;
/// with sizes mixed, a line's leading is the space below it (baseline leading: above it).
#[test]
fn em_box_top_leading_hangs_lines_from_the_line_above() {
    use vectorcraft_doc::{LeadingModel, TextRun};
    let st = |size: f64| CharStyle { size, leading: Some(size * 1.5), ..style(size) };
    let lay = |t: &TextObject, m: LeadingModel| {
        let mut t = t.clone();
        t.para.leading_model = m;
        layout(db(), &t)
    };
    // The em box top: 0.88 em above the baseline (fonts without vertical metrics).
    let top = |size: f64| 0.88 * size;
    let one_size = area("一行目\n二行目\n三行目", st(20.0), Rect::new(0.0, 0.0, 300.0, 300.0), Justify::Left);
    let (roman, em) = (lay(&one_size, LeadingModel::RomanBaseline), lay(&one_size, LeadingModel::EmBoxTop));
    assert!((em.lines[0].baseline - top(20.0)).abs() < 0.01, "the first line's em box touches the top: {}", em.lines[0].baseline);
    assert!((roman.lines[0].baseline - em.lines[0].baseline).abs() > 0.5, "baseline leading keeps Area Type Options' first baseline");
    for l in [&roman, &em] {
        assert!((l.lines[1].baseline - l.lines[0].baseline - 30.0).abs() < 0.01 && (l.lines[2].baseline - l.lines[1].baseline - 30.0).abs() < 0.01);
    }
    // 40 pt over 20 pt (leading 60 and 30).
    let mut mixed = point("", st(40.0));
    mixed.runs = vec![TextRun { text: "大\n".into(), style: st(40.0), inline: None }, TextRun { text: "小".into(), style: st(20.0), inline: None }];
    let roman = lay(&mixed, LeadingModel::RomanBaseline);
    assert!((roman.lines[1].baseline - roman.lines[0].baseline - 30.0).abs() < 0.01, "the small line's leading, above it");
    let em = lay(&mixed, LeadingModel::EmBoxTop);
    let want = -top(40.0) + 60.0 + top(20.0);
    assert!(
        (em.lines[1].baseline - em.lines[0].baseline - want).abs() < 0.01,
        "the big line's leading, below it: {} vs {want}",
        em.lines[1].baseline - em.lines[0].baseline
    );
}

/// The leading model is a paragraph attribute: in "大 / 小 / 大 / 小" (40 pt over 20 pt) with
/// only the second paragraph top-to-top, its line hangs from the big line's em box (the big
/// line's leading, below it) while the fourth, baseline to baseline, takes its own leading above
/// it; in area type a top-to-top first paragraph puts its em box on the frame's top.
#[test]
fn leading_model_is_per_paragraph() {
    use vectorcraft_doc::{LeadingModel, ParaStyle, TextRun};
    let st = |size: f64| CharStyle { size, leading: Some(size * 1.5), ..style(size) };
    let model = |m: LeadingModel| ParaStyle { leading_model: m, ..ParaStyle::default() };
    let top = |size: f64| 0.88 * size;
    let mut t = point("", st(40.0));
    t.runs = vec![
        TextRun { text: "大\n".into(), style: st(40.0), inline: None },
        TextRun { text: "小\n".into(), style: st(20.0), inline: None },
        TextRun { text: "大\n".into(), style: st(40.0), inline: None },
        TextRun { text: "小".into(), style: st(20.0), inline: None },
    ];
    t.set_paragraph_styles(vec![
        model(LeadingModel::RomanBaseline),
        model(LeadingModel::EmBoxTop),
        model(LeadingModel::RomanBaseline),
        model(LeadingModel::RomanBaseline),
    ]);
    assert_eq!(t.paras.len(), 4);
    let l = layout(db(), &t);
    let gap = |i: usize| l.lines[i].baseline - l.lines[i - 1].baseline;
    let em = -top(40.0) + 60.0 + top(20.0);
    assert!((gap(1) - em).abs() < 0.01, "top to top: {} vs {em}", gap(1));
    assert!((gap(3) - 30.0).abs() < 0.01, "baseline to baseline: {}", gap(3));
    // Area type: only a top-to-top first paragraph moves the first line up to the frame's top.
    let mut a = area("一\n二", st(20.0), Rect::new(0.0, 0.0, 300.0, 300.0), Justify::Left);
    a.set_paragraph_styles(vec![model(LeadingModel::RomanBaseline), model(LeadingModel::EmBoxTop)]);
    let roman_first = layout(db(), &a).lines[0].baseline;
    a.set_paragraph_styles(vec![model(LeadingModel::EmBoxTop), model(LeadingModel::RomanBaseline)]);
    let em_first = layout(db(), &a);
    assert!((em_first.lines[0].baseline - top(20.0)).abs() < 0.01, "{}", em_first.lines[0].baseline);
    assert!((roman_first - em_first.lines[0].baseline).abs() > 0.5);
    assert!((em_first.lines[1].baseline - em_first.lines[0].baseline - 30.0).abs() < 0.01);
}

/// Character Alignment: a 20 pt character next to a 40 pt one lines its em box top, centre or
/// bottom up with the big one's (the em box from 0.12 em below the baseline to 0.88 em above it,
/// in fonts without vertical metrics), or stays on the baseline; in vertical type the same across
/// the column (top → right).
#[test]
fn character_alignment_lines_small_characters_up_with_the_largest_em_box() {
    use vectorcraft_doc::{CharAlign, TextRun};
    for vertical_type in [false, true] {
        let place = |a: CharAlign| {
            let mut t = point("", style(40.0));
            t.vertical = vertical_type;
            t.runs = vec![
                TextRun { text: "大".into(), style: style(40.0), inline: None },
                TextRun { text: "小".into(), style: CharStyle { char_align: a, ..style(20.0) }, inline: None },
            ];
            let l = layout(db(), &t);
            // How far the small character's origin sits above the big one's (to the right, vertical).
            let (big, small) = (l.glyphs[0].origin, l.glyphs[1].origin);
            if vertical_type { small.x - big.x } else { big.y - small.y }
        };
        let near = |a: f64, b: f64| (a - b).abs() < 0.01;
        assert!(near(place(CharAlign::RomanBaseline), 0.0), "vertical {vertical_type}");
        assert!(near(place(CharAlign::EmBoxTop), 0.88 * 20.0), "top: {}", place(CharAlign::EmBoxTop));
        assert!(near(place(CharAlign::EmBoxCenter), 0.38 * 20.0), "centre: {}", place(CharAlign::EmBoxCenter));
        assert!(near(place(CharAlign::EmBoxBottom), -0.12 * 20.0), "bottom: {}", place(CharAlign::EmBoxBottom));
    }
}

/// Character Alignment on the ICF: a 20 pt character next to a 40 pt one lines its ideographic
/// character face's top (right, vertical) or bottom (left) up with the big one's, which lies inside
/// the em box by the face's ICF margins. A face without ideographs has none (the ICF is the em
/// box). The Japanese faces need craft-fonts (skipped without them).
#[test]
fn character_alignment_on_the_icf_lines_small_characters_up_with_the_largest_face() {
    use vectorcraft_doc::{CharAlign, TextRun};
    let place = |family: &str, a: CharAlign, vertical_type: bool| {
        let st = |size: f64| CharStyle { font_family: family.into(), ..style(size) };
        let mut t = point("", st(40.0));
        t.vertical = vertical_type;
        t.runs = vec![
            TextRun { text: "大".into(), style: st(40.0), inline: None },
            TextRun { text: "小".into(), style: CharStyle { char_align: a, ..st(20.0) }, inline: None },
        ];
        let l = layout(db(), &t);
        let (big, small) = (l.glyphs[0].origin, l.glyphs[1].origin);
        if vertical_type { small.x - big.x } else { big.y - small.y }
    };
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    // No ideographs: the ICF is the em box.
    assert_eq!(db().face("Source Sans 3", "Regular").unwrap().icf_margins(), crate::IcfMargins::default());
    let Some(face) = db().face("Shippori Mincho", "Regular").filter(|f| f.family == "Shippori Mincho") else {
        return; // no Japanese font here
    };
    let m = face.icf_margins();
    assert!([m.top, m.bottom, m.right, m.left].iter().all(|x| *x > 0.01 && *x < 0.2), "{m:?}");
    for vertical_type in [false, true] {
        let (top, bottom) = if vertical_type { (m.right, m.left) } else { (m.top, m.bottom) };
        // The big character's ICF top is `top` ems below its em box top: 20 pt of difference.
        let em_top = place("Shippori Mincho", CharAlign::EmBoxTop, vertical_type);
        let icf_top = place("Shippori Mincho", CharAlign::IcfTop, vertical_type);
        assert!(near(em_top - icf_top, top * 20.0), "vertical {vertical_type}: {em_top} {icf_top} {m:?}");
        let em_bottom = place("Shippori Mincho", CharAlign::EmBoxBottom, vertical_type);
        let icf_bottom = place("Shippori Mincho", CharAlign::IcfBottom, vertical_type);
        assert!(near(icf_bottom - em_bottom, bottom * 20.0), "vertical {vertical_type}: {em_bottom} {icf_bottom} {m:?}");
    }
}

/// Burasagari leaves Latin punctuation alone: in a measure of exactly "abcd", the full stop of
/// "abcd. ef" doesn't hang with Standard or Forced (the line breaks as it does with None), and a
/// Latin line ending in a full stop isn't shortened by Forced.
#[test]
fn burasagari_leaves_latin_commas_and_full_stops_inside_the_line() {
    use vectorcraft_doc::Burasagari;
    let measure = width(&point("abcd", style(20.0)));
    let lay = |text: &str, b: Burasagari| {
        let mut t = area(text, style(20.0), Rect::new(0.0, 0.0, measure + 0.01, 400.0), Justify::JustifyLeft);
        t.para.burasagari = b;
        layout(db(), &t).glyphs.iter().map(|g| (g.line, (g.origin.x * 100.0).round())).collect::<Vec<_>>()
    };
    for text in ["abcd. ef", "a b. efgh", "ab, cd, efgh"] {
        let none = lay(text, Burasagari::None);
        for b in [Burasagari::Standard, Burasagari::Forced] {
            assert_eq!(lay(text, b), none, "{text} {b:?}");
        }
    }
}

/// Optical Margin Alignment hangs the glyphs at a line's ends outside its margins: wholly for a
/// quotation mark, hyphen, full stop or comma, by half for an em dash, by a tenth of the advance
/// for an A, at the left of a left-aligned line, at the right of a right-aligned line, and at
/// both ends of centered and justified lines. A full stop and the closing quotation mark after it
/// hang together. Off, the lines are placed as before.
#[test]
fn optical_margin_alignment_hangs_punctuation_and_letter_edges_outside_the_margins() {
    let frame = Rect::new(0.0, 0.0, 300.0, 400.0);
    let lay = |text: &str, justify: Justify, on: bool| {
        let mut t = area(text, style(20.0), frame, justify);
        t.optical_margin = on;
        layout(db(), &t).glyphs
    };
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let end = |g: &PositionedGlyph| g.origin.x + g.advance;
    // Left aligned: the opening quotation mark hangs wholly, so the H starts at the edge.
    let (on, off) = (lay("“Hello” there", Justify::Left, true), lay("“Hello” there", Justify::Left, false));
    assert!(near(off[0].origin.x, 0.0) && near(on[1].origin.x, 0.0), "{} {}", off[0].origin.x, on[1].origin.x);
    assert!(near(on[0].origin.x, -on[0].advance) && on[0].advance > 1.0);
    // An em dash hangs by half its advance.
    let dash = lay("—ab", Justify::Left, true);
    assert!(near(dash[0].origin.x, -0.5 * dash[0].advance) && dash[0].advance > 1.0, "{}", dash[0].origin.x);
    // An A hangs by a tenth of its advance; a letter of no class doesn't hang.
    let a = lay("AVA", Justify::Left, true);
    assert!(near(a[0].origin.x, -0.1 * a[0].advance), "{}", a[0].origin.x);
    let v = lay("van", Justify::Left, true);
    assert!(near(v[0].origin.x, -0.1 * v[0].advance), "{}", v[0].origin.x);
    assert!(near(lay("Hello", Justify::Left, true)[0].origin.x, 0.0));
    assert!(near(lay("and", Justify::Left, true)[0].origin.x, 0.0));
    // A line of one glyph doesn't hang.
    assert!(near(lay(".", Justify::Left, true)[0].origin.x, 0.0));
    // Right aligned: the full stop hangs wholly at the right; with a closing quotation mark after
    // it, both do.
    let (on, off) = (lay("Hello there.", Justify::Right, true), lay("Hello there.", Justify::Right, false));
    let n = on.len();
    assert!(near(end(&off[n - 1]), 300.0) && near(on[n - 1].origin.x, 300.0), "{} {}", end(&off[n - 1]), on[n - 1].origin.x);
    let on = lay("Say “hi.”", Justify::Right, true);
    let n = on.len();
    assert!(near(on[n - 2].origin.x, 300.0) && near(end(&on[n - 3]), 300.0), "{}", on[n - 2].origin.x);
    // At a line's start, a single quotation mark followed by a double one hangs with it.
    assert!(near(lay("‘“Hi", Justify::Left, true)[2].origin.x, 0.0));
    // A right-aligned line doesn't hang at its left, nor a left-aligned one at its right.
    assert!(near(lay("“Hello", Justify::Right, true).last().map_or(0.0, end), 300.0));
    assert!(near(lay("Hello.", Justify::Left, true)[0].origin.x, 0.0));
    // Centered: the letters between the hanging hyphens are centered in the frame.
    let on = lay("-ab-", Justify::Center, true);
    assert!(near((on[1].origin.x + end(&on[2])) * 0.5, 150.0), "{on:?}");
    let off = lay("-ab-", Justify::Center, false);
    assert!(near((off[0].origin.x + end(&off[3])) * 0.5, 150.0));
    // Justified: both hyphens hang and the words fill the frame between them.
    let on = lay("-ab cd-", Justify::JustifyAll, true);
    let n = on.len();
    assert!(near(on[1].origin.x, 0.0) && near(on[n - 1].origin.x, 300.0), "{} {}", on[1].origin.x, on[n - 1].origin.x);
    let off = lay("-ab cd-", Justify::JustifyAll, false);
    assert!(near(off[0].origin.x, 0.0) && near(end(&off[n - 1]), 300.0));
    // A line without spaces is justified between its letters, not next to the hanging marks.
    for text in ["“Hi”", "“Hi.”", "“漢字”"] {
        let on = lay(text, Justify::JustifyAll, true);
        let n = on.len();
        // The glyphs that hang at the right: the closing mark, and the full stop before it.
        let right = if text.contains('.') { 2 } else { 1 };
        assert!(near(on[1].origin.x, 0.0) && near(end(&on[n - 1 - right]), 300.0), "{text}: {on:?}");
    }
    // Letters that hang by part of their width keep their gaps: the line still fills the frame.
    let a = lay("AVA", Justify::JustifyAll, true);
    assert!(near(end(&a[2]), 300.0 + 0.1 * a[2].advance), "{}", end(&a[2]));
    // Point type hangs at its anchor.
    let mut t = point("“Hi", style(20.0));
    t.optical_margin = true;
    let l = layout(db(), &t);
    assert!(near(l.glyphs[1].origin.x, 0.0) && l.glyphs[0].origin.x < -1.0, "{:?}", l.glyphs[0].origin);
    // Vertical type: the letter after the quotation mark takes the mark's place at the column's top.
    let vertical = |on: bool| {
        let mut t = area("“Hello", style(20.0), frame, Justify::Left);
        (t.vertical, t.optical_margin) = (true, on);
        layout(db(), &t).glyphs
    };
    let (on, off) = (vertical(true), vertical(false));
    assert!((on[1].origin - off[0].origin).hypot() < 1e-6, "{:?} {:?}", on[1].origin, off[0].origin);
}

/// Right-to-left paragraphs hang at the margins their alignment places their lines against: Auto
/// and right-aligned lines hang at the right, left-aligned lines hang at the left, and justified
/// lines hang at both. The line is read from right to left, so `."` ending it hangs both glyphs at its left, and `".`
/// starting it hangs only the quotation mark at its right. Roman Hanging Punctuation hangs them
/// the same way. In vertical type the characters run from the top down, and the pair is read
/// from the top.
#[test]
fn optical_margin_alignment_hangs_right_to_left_paragraphs() {
    let frame = Rect::new(0.0, 0.0, 300.0, 400.0);
    let object = |text: &str, justify: Justify, on: bool| {
        let mut t = area(text, style(20.0), frame, justify);
        t.optical_margin = on;
        t
    };
    let lay = |text: &str, justify: Justify, on: bool| layout(db(), &object(text, justify, on)).glyphs;
    let at = |glyphs: &[PositionedGlyph], b: usize| glyphs.iter().find(|g| g.byte == b).cloned().unwrap();
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let end = |g: &PositionedGlyph| g.origin.x + g.advance;
    let alef = 'א'.len_utf8();
    // Auto aligns the line at the right: the opening quotation mark (logically first, drawn at the
    // right end) hangs wholly past the right edge, and the letter after it ends at the edge.
    let text = "\"שלום\" עולם";
    let (on, off) = (lay(text, Justify::Auto, true), lay(text, Justify::Auto, false));
    assert!(off.iter().all(|g| g.rtl) && near(end(&at(&off, 0)), 300.0), "{off:?}");
    assert!(near(at(&on, 0).origin.x, 300.0) && at(&on, 0).advance > 1.0, "{:?}", at(&on, 0).origin);
    assert!(near(end(&at(&on, 1)), 300.0), "{}", end(&at(&on, 1)));
    // Left aligned: the full stop ending the line is drawn at its left and hangs there.
    let text = "שלום, עולם.";
    let stop = text.len() - 1;
    let (on, off) = (lay(text, Justify::Left, true), lay(text, Justify::Left, false));
    assert!(near(off[0].origin.x, 0.0) && near(at(&on, stop).origin.x, -at(&on, stop).advance), "{:?}", at(&on, stop).origin);
    // Justified: `."` ending the line hangs both glyphs at the left; the right end doesn't hang.
    let text = "שלום עולם.\"";
    let (quote, stop) = (text.len() - 1, text.len() - 2);
    let on = lay(text, Justify::JustifyAll, true);
    let (q, s) = (at(&on, quote), at(&on, stop));
    assert!(near(q.origin.x, -q.advance - s.advance) && near(s.origin.x, -s.advance), "{:?} {:?}", q.origin, s.origin);
    assert!(near(end(&at(&on, 0)), 300.0), "{}", end(&at(&on, 0)));
    // A justified line of one word, spread between its letters: the opening quotation mark hangs
    // wholly past the right edge, the letter after it ends at the edge, and the last letter starts
    // at the left edge.
    let on = lay("\"שלום", Justify::JustifyAll, true);
    let mem = 1 + 3 * alef;
    assert!(near(at(&on, 0).origin.x, 300.0) && near(end(&at(&on, 1)), 300.0), "{:?} {:?}", at(&on, 0).origin, at(&on, 1).origin);
    assert!(near(at(&on, mem).origin.x, 0.0), "{:?}", at(&on, mem).origin);
    // `".` starting a right-aligned line: the quotation mark hangs and the full stop doesn't.
    let on = lay("\".שלום", Justify::Right, true);
    assert!(near(at(&on, 0).origin.x, 300.0) && near(end(&at(&on, 1)), 300.0), "{:?} {:?}", at(&on, 0).origin, at(&on, 1).origin);
    // A wrapped left-aligned line: the comma ending it hangs, and the space after it stays to its left.
    let first = width(&point("אבגד, ", style(20.0)));
    let mut t = area("אבגד, הוזח", style(20.0), Rect::new(0.0, 0.0, first + 1.0, 400.0), Justify::Left);
    t.optical_margin = true;
    let l = layout(db(), &t);
    let (comma, space) = (at(&l.glyphs, 4 * alef), at(&l.glyphs, 4 * alef + 1));
    assert!(l.lines.len() == 2 && near(comma.origin.x, -comma.advance), "{:?} {:?}", l.lines, comma.origin);
    assert!(near(end(&space), comma.origin.x), "{:?} {:?}", space.origin, comma.origin);
    // Point type: Auto grows the line left from its anchor, and the opening quotation mark hangs
    // to the right of the anchor.
    let mut t = point("\"שלום", style(20.0));
    (t.para.justify, t.optical_margin) = (Justify::Auto, true);
    let l = layout(db(), &t);
    assert!(near(at(&l.glyphs, 0).origin.x, 0.0) && near(end(&at(&l.glyphs, 1)), 0.0), "{:?}", at(&l.glyphs, 0).origin);
    // Roman Hanging Punctuation hangs the full stop the same way.
    let mut t = area("שלום, עולם.", style(20.0), frame, Justify::Left);
    t.para.roman_hanging = true;
    let l = layout(db(), &t);
    let stop = "שלום, עולם".len();
    assert!(near(at(&l.glyphs, stop).origin.x, -at(&l.glyphs, stop).advance), "{:?}", at(&l.glyphs, stop).origin);
    // Vertical type: the letter after the quotation mark takes the mark's place at the column's top.
    let vertical = |text: &str, justify: Justify, on: bool| {
        let mut t = object(text, justify, on);
        t.vertical = true;
        layout(db(), &t).glyphs
    };
    let (on, off) = (vertical("\"שלום", Justify::Left, true), vertical("\"שלום", Justify::Left, false));
    assert!((at(&on, 1).origin - at(&off, 0).origin).hypot() < 1e-6, "{:?} {:?}", at(&on, 1).origin, at(&off, 0).origin);
    // …and `."` ending a line at the column's bottom hangs both glyphs there.
    let text = "שלום.\"";
    let (on, off) = (vertical(text, Justify::Right, true), vertical(text, Justify::Right, false));
    let (stop, mem) = (text.len() - 2, text.len() - 2 - alef);
    let shift = at(&off, stop).advance + at(&off, stop + 1).advance;
    assert!(near((at(&on, mem).origin - at(&off, mem).origin).hypot(), shift), "{:?} {:?}", at(&on, mem).origin, at(&off, mem).origin);
}

/// Roman Hanging Punctuation (a paragraph attribute) hangs the punctuation Optical Margin
/// Alignment hangs, by the same amounts, and no letters; only the paragraphs that have it hang.
#[test]
fn roman_hanging_punctuation_hangs_punctuation_but_not_letters() {
    let frame = Rect::new(0.0, 0.0, 300.0, 400.0);
    let lay = |text: &str, justify: Justify, on: bool| {
        let mut t = area(text, style(20.0), frame, justify);
        t.para.roman_hanging = on;
        layout(db(), &t).glyphs
    };
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let end = |g: &PositionedGlyph| g.origin.x + g.advance;
    // Left aligned: the opening quotation mark hangs wholly.
    let on = lay("“Hello” there", Justify::Left, true);
    assert!(near(on[1].origin.x, 0.0) && near(on[0].origin.x, -on[0].advance) && on[0].advance > 1.0, "{:?}", on[0].origin);
    // An em dash hangs by half its advance.
    let dash = lay("—ab", Justify::Left, true);
    assert!(near(dash[0].origin.x, -0.5 * dash[0].advance), "{}", dash[0].origin.x);
    // Letters don't hang, where Optical Margin Alignment hangs an A by a tenth.
    assert!(near(lay("AVA", Justify::Left, true)[0].origin.x, 0.0));
    // Right aligned: the full stop and the closing quotation mark after it hang at the right.
    let r = lay("“Hi.”", Justify::Right, true);
    assert!(near(end(&r[2]), 300.0), "{}", end(&r[2]));
    // Justified: a hyphen at each end hangs, the words fill the frame between them.
    let j = lay("-ab cd-", Justify::JustifyAll, true);
    let n = j.len();
    assert!(near(j[1].origin.x, 0.0) && near(j[n - 1].origin.x, 300.0), "{} {}", j[1].origin.x, j[n - 1].origin.x);
    // Off: nothing hangs.
    assert!(near(lay("“Hello” there", Justify::Left, false)[0].origin.x, 0.0));
    // Per paragraph: the second paragraph, without it, doesn't hang.
    let mut t = area("“One\n“Two", style(20.0), frame, Justify::Left);
    let mut first = t.para.clone();
    first.roman_hanging = true;
    t.set_paragraph_styles(vec![first, t.para.clone()]);
    let l = layout(db(), &t);
    let at = |b: usize| l.glyphs.iter().find(|g| g.byte == b).unwrap().origin.x;
    assert!(at(0) < -1.0, "first paragraph hangs: {}", at(0));
    assert!(near(at("“One\n".len()), 0.0), "second doesn't: {}", at("“One\n".len()));
}

/// Type on a path hangs at its brackets, measured along the path: a left-aligned line at the
/// start bracket, a right-aligned line at the end bracket, a centered line at both, and the
/// justified alignments where they place the line. A mark hanging past the end bracket doesn't
/// overflow. A hang never takes a glyph off an open path's ends, nor past the other bracket of a
/// closed path. Right-to-left type hangs at the bracket Auto places it at and is read from right to
/// left. Roman Hanging Punctuation hangs path type the same way, without the letters.
#[test]
fn optical_margin_alignment_hangs_type_on_a_path_at_its_brackets() {
    // A straight path along y = 50 from x = 0 to 500: distance along it is x.
    let mut line = BezPath::new();
    line.move_to((0.0, 50.0));
    line.line_to((500.0, 50.0));
    let path = PathData::from_bezpath(&line);
    let on_path = |text: &str, justify: Justify, start: f64, end: Option<f64>, on: bool| {
        let mut t = point(text, style(20.0));
        t.kind = TextKind::OnPath { path: path.clone(), start, end };
        t.para.justify = justify;
        t.optical_margin = on;
        t
    };
    let lay = |text: &str, justify: Justify, on: bool| layout(db(), &on_path(text, justify, 0.1, Some(0.9), on));
    let near = |a: f64, b: f64| (a - b).abs() < 1e-6;
    let end = |g: &PositionedGlyph| g.origin.x + g.advance;
    // Left aligned: the opening quotation mark hangs wholly before the start bracket (x = 50).
    let (on, off) = (lay("“Hello", Justify::Left, true), lay("“Hello", Justify::Left, false));
    assert!(near(off.glyphs[0].origin.x, 50.0) && near(on.glyphs[1].origin.x, 50.0), "{:?}", on.glyphs[1].origin);
    assert!(near(on.glyphs[0].origin.x, 50.0 - on.glyphs[0].advance) && on.glyphs[0].advance > 1.0);
    // Auto, Justify All and Justify with Last Line Aligned Left place the line at the start bracket
    // and hang there too.
    for justify in [Justify::Auto, Justify::JustifyAll, Justify::JustifyLeft] {
        assert!(near(lay("“Hello", justify, true).glyphs[1].origin.x, 50.0), "{justify:?}");
    }
    // Right aligned: the full stop hangs past the end bracket (x = 450) without overflowing.
    for justify in [Justify::Right, Justify::JustifyRight] {
        let (on, off) = (lay("Hello there.", justify, true), lay("Hello there.", justify, false));
        let n = on.glyphs.len();
        assert!(near(end(&off.glyphs[n - 1]), 450.0) && near(on.glyphs[n - 1].origin.x, 450.0), "{justify:?}: {:?}", on.glyphs[n - 1].origin);
        assert!(!on.overflow);
    }
    // A right-aligned line doesn't hang at its start, nor a left-aligned one at its end.
    assert!(near(lay("“Hello", Justify::Right, true).glyphs.last().map_or(0.0, end), 450.0));
    assert!(near(lay("Hello.", Justify::Left, true).glyphs[0].origin.x, 50.0));
    // Centered: the letters between the hanging hyphens are centered between the brackets.
    for justify in [Justify::Center, Justify::JustifyCenter] {
        let on = lay("-ab-", justify, true).glyphs;
        assert!(near((on[1].origin.x + end(&on[2])) * 0.5, 250.0), "{justify:?}: {on:?}");
    }
    // The hanging full stop is room: a line that overflows by part of its full stop fits with it.
    let wide = lay("Hello.", Justify::Left, false).glyphs;
    let w: f64 = wide.iter().map(|g| g.advance).sum();
    let tight = |on: bool| layout(db(), &on_path("Hello.", Justify::Left, 0.1, Some((50.0 + w - 0.5 * wide[5].advance) / 500.0), on));
    let (on, off) = (tight(true), tight(false));
    assert!(off.overflow && off.glyphs.len() == 5, "{} {}", off.overflow, off.glyphs.len());
    assert!(!on.overflow && on.glyphs.len() == 6, "{} {}", on.overflow, on.glyphs.len());
    // Only the glyphs that hang take that room: a letter ending past the end bracket by less than
    // the overset story's last full stop still overflows.
    let story = lay("Hello there.", Justify::Left, false).glyphs;
    let past = (end(&story[3]) - 0.5 * story[11].advance) / 500.0;
    let overset = |on: bool| layout(db(), &on_path("Hello there.", Justify::Left, 0.1, Some(past), on));
    assert!(overset(true).overflow && overset(true).glyphs.len() == 3, "{}", overset(true).glyphs.len());
    assert_eq!(overset(false).glyphs.len(), 3);
    // A hang stops at an open path's first point: no glyph is held there.
    let at_start = |on: bool| layout(db(), &on_path("“Hello", Justify::Left, 0.0, None, on)).glyphs;
    assert!(near(at_start(true)[0].origin.x, 0.0) && near(at_start(true)[1].origin.x, at_start(false)[1].origin.x));
    let quote = at_start(false)[0].advance;
    let part = layout(db(), &on_path("“Hello", Justify::Left, 0.5 * quote / 500.0, None, true)).glyphs;
    assert!(near(part[0].origin.x, 0.0), "cut to the room before the bracket: {:?}", part[0].origin);
    // Vertical type on the path: the letter after the quotation mark takes the mark's place.
    let vertical = |on: bool| {
        let mut t = on_path("“Hello", Justify::Left, 0.1, Some(0.9), on);
        t.vertical = true;
        layout(db(), &t).glyphs
    };
    let (on, off) = (vertical(true), vertical(false));
    assert!((on[1].origin - off[0].origin).hypot() < 1e-6 && (on[0].origin - off[0].origin).hypot() > 1.0, "{:?} {:?}", on[1].origin, off[0].origin);
    // With Spacing on a curve, the letter after the hanging quotation mark still starts at the
    // bracket: the rest of the line is drawn as the same text without the mark.
    let mut curve = BezPath::new();
    curve.move_to((20.0, 150.0));
    curve.curve_to((60.0, 40.0), (140.0, 40.0), (180.0, 150.0));
    let spaced = |text: &str, on: bool| {
        let mut t = point(text, style(20.0));
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&curve), start: 0.2, end: None };
        (t.path_spacing, t.optical_margin) = (12.0, on);
        layout(db(), &t).glyphs.iter().map(|g| (g.origin, g.angle)).collect::<Vec<_>>()
    };
    let (on, off) = (spaced("“Hello", true), spaced("Hello", false));
    assert!(on.len() == 6 && on[1..].iter().zip(&off).all(|(a, b)| (a.0 - b.0).hypot() < 1e-6 && (a.1 - b.1).abs() < 1e-6), "{on:?} {off:?}");
    // A closed path whose brackets take all of it leaves no room to hang.
    let circle = PathData::from_bezpath(&kurbo::Circle::new((0.0, 0.0), 100.0).to_path(0.1));
    let round = |on: bool| {
        let mut t = point("“Around the circle”", style(14.0));
        t.kind = TextKind::OnPath { path: circle.clone(), start: 0.0, end: None };
        t.optical_margin = on;
        layout(db(), &t).glyphs.iter().map(|g| g.origin).collect::<Vec<_>>()
    };
    assert_eq!(round(true), round(false));
    // On a closed path whose brackets leave a stretch narrower than a quotation mark, a
    // right-aligned line hangs its closing mark into that stretch; its opening mark doesn't take it.
    let mut square = BezPath::new();
    square.move_to((0.0, 0.0));
    square.line_to((500.0, 0.0));
    square.line_to((500.0, 10.0));
    square.line_to((0.0, 10.0));
    square.close_path();
    let square = PathData::from_bezpath(&square);
    let gap = 0.5 * quote;
    let closed = |justify: Justify, on: bool| {
        // 1,020 pt round; the brackets are at 102 and at 102 - gap on the top side.
        let mut t = on_path("“Hi”", justify, 0.1, None, on);
        t.kind = TextKind::OnPath { path: square.clone(), start: 0.1, end: Some((102.0 - gap) / 1020.0) };
        layout(db(), &t).glyphs
    };
    let (on, off) = (closed(Justify::Right, true), closed(Justify::Right, false));
    assert!(near(off.last().map_or(0.0, end), 102.0 - gap) && near(on.last().map_or(0.0, end), 102.0), "{on:?}");
    // Centered, each mark hangs into half of it, and the line stays where it was.
    let origins = |on: bool| closed(Justify::Center, on).iter().map(|g| g.origin).collect::<Vec<_>>();
    let (on, off) = (origins(true), origins(false));
    assert!(on.iter().zip(&off).all(|(a, b)| (*a - *b).hypot() < 1e-6), "{on:?} {off:?}");
    // Right-to-left type on a path hangs too. Auto places it at the end bracket, where its opening
    // quotation mark, drawn last, hangs wholly past the bracket.
    let (on, off) = (lay("\"שלום\" עולם", Justify::Auto, true), lay("\"שלום\" עולם", Justify::Auto, false));
    let n = on.glyphs.len();
    assert!(near(end(&off.glyphs[n - 1]), 450.0) && near(on.glyphs[n - 1].origin.x, 450.0), "{:?}", on.glyphs[n - 1].origin);
    assert!(!on.overflow);
    // The line is read from right to left: left aligned, `."` ending it hangs both glyphs before the
    // start bracket.
    let (on, off) = (lay("עולם.\"", Justify::Left, true), lay("עולם.\"", Justify::Left, false));
    assert!(near(off.glyphs[0].origin.x, 50.0) && near(on.glyphs[2].origin.x, 50.0), "{:?}", on.glyphs[2].origin);
    // On a closed path, Auto right-to-left type takes the stretch past the end bracket first, as
    // right-aligned type does.
    let closed_rtl = |on: bool| {
        let mut t = on_path("\"שלום\"", Justify::Auto, 0.1, None, on);
        t.kind = TextKind::OnPath { path: square.clone(), start: 0.1, end: Some((102.0 - gap) / 1020.0) };
        layout(db(), &t).glyphs
    };
    let (on, off) = (closed_rtl(true), closed_rtl(false));
    assert!(near(off.last().map_or(0.0, end), 102.0 - gap) && near(on.last().map_or(0.0, end), 102.0), "{on:?}");
    // Vertical type on a path whose paragraph runs right to left reads from the top: the letter
    // after the quotation mark takes the mark's place.
    let vertical_rtl = |on: bool| {
        let mut t = on_path("\"שלום", Justify::Left, 0.1, Some(0.9), on);
        t.vertical = true;
        layout(db(), &t).glyphs
    };
    let (on, off) = (vertical_rtl(true), vertical_rtl(false));
    assert!((on[1].origin - off[0].origin).hypot() < 1e-6 && (on[0].origin - off[0].origin).hypot() > 1.0, "{:?} {:?}", on[1].origin, off[0].origin);
    // Roman Hanging Punctuation hangs the quotation mark and no letters.
    let roman = |text: &str| {
        let mut t = on_path(text, Justify::Left, 0.1, Some(0.9), false);
        t.para.roman_hanging = true;
        layout(db(), &t).glyphs
    };
    assert!(near(roman("“Hello")[1].origin.x, 50.0) && near(roman("AVA")[0].origin.x, 50.0));
    assert!(near(lay("AVA", Justify::Left, true).glyphs[0].origin.x, 50.0 - 0.1 * lay("AVA", Justify::Left, true).glyphs[0].advance));
}

/// With craft-fonts and no installed fonts (the web), Thai is set in Noto Sans Thai, not as empty
/// boxes. Skipped when built without craft-fonts or with a checkout that has no Thai font.
#[test]
fn thai_without_installed_fonts_takes_the_craft_fonts_thai_font() {
    if !crate::CRAFT_FONTS.iter().any(|f| f.is_thai()) {
        eprintln!("skipping: no Thai font in craft-fonts (set CRAFT_FONTS_DIR to a checkout that has one)");
        return;
    }
    let l = layout(db(), &point("ภาษาไทย", style(20.0)));
    assert!(!l.glyphs.is_empty() && l.glyphs.iter().all(|g| g.gid != 0), "no .notdef");
    let families: Vec<String> = l.glyphs.iter().filter_map(|g| db().face_by_id(g.font_id)).map(|f| f.family.clone()).collect();
    assert!(families.iter().all(|f| f == "Noto Sans Thai"), "{families:?}");
}

/// Thai is written without spaces between words: area type breaks its lines between words (as a
/// model of Thai words finds them), never inside a word nor before a vowel or tone mark.
#[test]
fn thai_lines_break_between_words() {
    let text = "สวัสดีครับ ภาษาไทยง่ายนิดเดียว";
    // Narrower than the shortest words: each line holds one word.
    let l = layout(db(), &area(text, style(12.0), Rect::new(0.0, 0.0, 70.0, 2000.0), Justify::Left));
    let lines: Vec<&str> = l.lines.iter().map(|ln| &text[ln.start..ln.end]).collect();
    assert!(lines.len() >= 3, "{lines:?}");
    // Its words: สวัสดี ครับ, ภาษา ไทย ง่าย นิดเดียว ("hello", "the Thai language is very easy").
    let words = ["สวัสดี", "ครับ ", "ภาษา", "ไทย", "ง่าย", "นิดเดียว"];
    let ends: Vec<usize> = words
        .iter()
        .scan(0, |at, w| {
            *at += w.len();
            Some(*at)
        })
        .collect();
    assert_eq!(ends.last(), Some(&text.len()));
    for ln in &l.lines {
        assert!(ends.contains(&ln.end), "a line ends inside a word: {lines:?}");
    }
    for w in l.lines.windows(2) {
        assert_eq!(w[0].end, w[1].start, "lines tile the text");
    }
    let mark = |c: char| matches!(c, '\u{0E31}' | '\u{0E34}'..='\u{0E3A}' | '\u{0E47}'..='\u{0E4E}');
    assert!(lines.iter().all(|s| s.chars().next().is_some_and(|c| !mark(c))), "a line starts with a mark: {lines:?}");
}

/// The desktop's dictionaries keep loanwords whole that a smaller model splits: โปรแกรม
/// ("program").
#[test]
fn thai_loanwords_stay_whole() {
    let text = "ยินดีต้อนรับสู่โปรแกรมวาดภาพเวกเตอร์";
    let l = layout(db(), &area(text, style(12.0), Rect::new(0.0, 0.0, 70.0, 2000.0), Justify::Left));
    let lines: Vec<&str> = l.lines.iter().map(|ln| &text[ln.start..ln.end]).collect();
    // No line ends inside the word, whatever the Thai font's widths put beside it.
    let word = text.find("โปรแกรม").unwrap();
    assert!(l.lines.iter().all(|ln| ln.end <= word || ln.end >= word + "โปรแกรม".len()), "{lines:?}");
}

/// No Break (Character panel menu) keeps the marked words on one line, where the same text wraps
/// at the space without the mark.
#[test]
fn no_break_keeps_marked_words_on_one_line() {
    let plain = style(20.0);
    let marked = CharStyle { no_break: true, ..plain.clone() };
    let hello: f64 = layout(db(), &point("hello ", plain.clone())).glyphs.iter().map(|g| g.advance).sum();
    let whole: f64 = layout(db(), &point("hello world", plain.clone())).glyphs.iter().map(|g| g.advance).sum();
    assert!(whole > hello + 1.0, "the second word takes room");
    let frame = Rect::new(0.0, 0.0, hello + 1.0, 200.0);
    let wrapped = layout(db(), &area("hello world", plain, frame, Justify::Left));
    assert!(wrapped.lines.len() > 1, "it wraps at the space");
    let kept = layout(db(), &area("hello world", marked, frame, Justify::Left));
    assert_eq!(kept.lines.len(), 1, "the marked words stay together");
    assert_eq!(kept.lines[0].end, "hello world".len());
}
