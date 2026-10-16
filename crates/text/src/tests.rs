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
