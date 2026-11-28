//! Ruby (furigana) tests: geometry over the base text in horizontal and vertical type, line
//! growth, overhang/squeeze, splitting over a line break, and the byte ranges following edits.

use crate::*;
use kurbo::{Affine, Point, Rect, Shape};
use vectorcraft_doc::{CharStyle, RubyAlign, RubyPosition, RubySpan, TextKind};
use vectorcraft_geom::PathData;

fn db() -> &'static FontDb {
    FontDb::global()
}

fn style(size: f64) -> CharStyle {
    CharStyle { size, ..CharStyle::default() }
}

/// An outline's bounds, or `None` for a glyph with no ink (a space).
fn ink(g: &PositionedGlyph) -> Option<Rect> {
    (!g.outline.elements().is_empty()).then(|| g.outline.bounding_box())
}

fn ruby_ink(lay: &TextLayout) -> Rect {
    union_rect(lay.ruby.iter()).expect("ruby glyphs with ink")
}

fn union_rect<'a>(glyphs: impl Iterator<Item = &'a PositionedGlyph>) -> Option<Rect> {
    let mut b: Option<Rect> = None;
    for g in glyphs {
        if let Some(r) = ink(g) {
            b = Some(b.map_or(r, |a| a.union(r)));
        }
    }
    b
}

/// A point text object with `text`, and ruby `reading` over its first `base` bytes.
fn ruby_text(text: &str, base: usize, reading: &str, size: f64) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style(size));
    t.ruby = vec![RubySpan::new(0, base, reading)];
    t
}

#[test]
fn horizontal_ruby_sits_above_the_base_and_grows_the_line() {
    let base = TextObject::point(Point::ZERO, "漢字", style(20.0));
    let plain = layout(db(), &base);
    assert!(plain.ruby.is_empty());
    let base_glyphs: Vec<_> = plain.glyphs.iter().map(|g| (g.font_id, g.gid, g.origin, g.advance)).collect();

    let t = ruby_text("漢字", 6, "かんじ", 20.0);
    let lay = layout(db(), &t);
    assert_eq!(lay.ruby.len(), 3, "one glyph per kana");
    assert!(lay.ruby.iter().all(|g| g.run == 0 && g.line == 0 && g.len == 0));
    // The base text itself is unchanged: same glyphs, same places.
    let now: Vec<_> = lay.glyphs.iter().map(|g| (g.font_id, g.gid, g.origin, g.advance)).collect();
    assert_eq!(now, base_glyphs);
    // The reading is above the base's ink and inside the line's ascent.
    let above = ruby_ink(&lay);
    let below = union_rect(lay.glyphs.iter()).expect("base ink");
    assert!(above.y1 <= below.y0 + 0.5, "ruby bottom {} above base top {}", above.y1, below.y0);
    assert!(above.y0 < below.y0, "ruby rises above the base");
    assert!(lay.lines[0].ascent > plain.lines[0].ascent + 0.2 * 20.0, "the line grew: {} vs {}", lay.lines[0].ascent, plain.lines[0].ascent);
    // Centred over the base text by default.
    let centre = (above.x0 + above.x1) * 0.5;
    let base_centre = (below.x0 + below.x1) * 0.5;
    assert!((centre - base_centre).abs() < 1.0, "{centre} vs {base_centre}");
    // Above the base's em box, not touching its ink: a visible gap of a few points.
    assert!(above.y1 <= below.y0 - 1.0, "a gap between ruby and base ink: {} vs {}", above.y1, below.y0);
}

/// Bullets: a reading over a list paragraph's first characters is centered over those characters,
/// not over the bullet and the gap before them.
#[test]
fn ruby_over_a_list_item_leaves_out_its_bullet() {
    use vectorcraft_doc::text::{Bullet, ParaList};
    let mut t = ruby_text("漢字", 6, "かんじ", 20.0);
    t.para.list = Some(ParaList::bulleted(Bullet::Dot));
    t.para.left_indent = 18.0;
    t.para.first_line_indent = -18.0;
    let lay = layout(db(), &t);
    assert_eq!(lay.markers.len(), 1);
    let above = ruby_ink(&lay);
    let base = union_rect(lay.glyphs.iter().filter(|g| g.len > 0)).expect("base ink");
    let (centre, base_centre) = ((above.x0 + above.x1) * 0.5, (base.x0 + base.x1) * 0.5);
    assert!((centre - base_centre).abs() < 1.0, "{centre} vs {base_centre}");
}

#[test]
fn ruby_below_grows_the_descent() {
    let mut t = ruby_text("漢字", 6, "かんじ", 20.0);
    t.ruby[0].position = RubyPosition::Below;
    let lay = layout(db(), &t);
    let below = union_rect(lay.glyphs.iter()).expect("base ink");
    let ruby = ruby_ink(&lay);
    assert!(ruby.y0 >= below.y1 - 0.5, "ruby top {} below base bottom {}", ruby.y0, below.y1);
    let plain = layout(db(), &TextObject::point(Point::ZERO, "漢字", style(20.0)));
    assert!(lay.lines[0].descent > plain.lines[0].descent + 0.2 * 20.0);
}

#[test]
fn ruby_is_half_the_base_size_by_default_and_size_follows_the_span() {
    let half = ruby_text("漢", 3, "かん", 20.0);
    let mut whole = half.clone();
    whole.ruby[0].size_percent = 100.0;
    let half = ruby_ink(&layout(db(), &half));
    let whole = ruby_ink(&layout(db(), &whole));
    assert!(half.height() < whole.height() * 0.75, "{} vs {}", half.height(), whole.height());
}

#[test]
fn a_reading_wider_than_its_base_overhangs_or_is_squeezed() {
    // One base character, a three-kana reading: wider than the base at the default half size.
    let mut t = ruby_text("漢", 3, "かんじ", 20.0);
    let wide = ruby_ink(&layout(db(), &t));
    // The measure is the base character's advance (its em), not its ink.
    let base_lay = layout(db(), &TextObject::point(Point::ZERO, "漢", style(20.0)));
    let base = (base_lay.lines[0].x1 - base_lay.lines[0].x0).max(0.0);
    assert!(wide.width() > base, "overhanging ruby {} vs base {}", wide.width(), base);
    // Without overhang the reading is squeezed to the base's width.
    t.ruby[0].overhang = false;
    let narrow = ruby_ink(&layout(db(), &t));
    assert!(narrow.width() <= base + 0.5, "squeezed ruby {} vs base {}", narrow.width(), base);
}

#[test]
fn ruby_aligns_start_centre_and_end_over_its_base() {
    let mut t = ruby_text("漢字", 6, "かん", 20.0);
    // Alignment uses advances, not ink bearings, which differ between installed fallback fonts.
    let base_lay = layout(db(), &TextObject::point(Point::ZERO, "漢字", style(20.0)));
    let base_line = &base_lay.lines[0];
    let base = Rect::new(base_line.x0, 0.0, base_line.x1, 0.0);
    let reading = layout(db(), &TextObject::point(Point::ZERO, "かん", style(10.0)));
    let reading_line = &reading.lines[0];
    let width = reading_line.x1 - reading_line.x0;
    for (align, check) in [
        (RubyAlign::Start, (|r: Rect, b: Rect| (r.x0 - b.x0).abs() < 1e-6) as fn(Rect, Rect) -> bool),
        (RubyAlign::Center, |r: Rect, b: Rect| (((r.x0 + r.x1) - (b.x0 + b.x1)) * 0.5).abs() < 1e-6),
        (RubyAlign::End, |r: Rect, b: Rect| (r.x1 - b.x1).abs() < 1e-6),
    ] {
        t.ruby[0].align = align;
        let lay = layout(db(), &t);
        // Ruby glyphs are attachments with zero advance: use their pen origin and the reading's
        // standalone advance width. A wrong Start/Center/End placement still fails this test.
        let x0 = lay.ruby[0].origin.x - reading.glyphs[0].origin.x + reading_line.x0;
        let ruby = Rect::new(x0, 0.0, x0 + width, 0.0);
        assert!(check(ruby, base), "{align:?}: {ruby:?} over {base:?}");
    }
}

#[test]
fn vertical_ruby_is_upright_to_the_right_of_the_column() {
    let mut t = ruby_text("漢字", 6, "かんじ", 20.0);
    t.vertical = true;
    let lay = layout(db(), &t);
    assert!(lay.vertical);
    assert!(!lay.ruby.is_empty());
    // The base columns sit around the origin; the reading is to their right (larger x).
    let base = union_rect(lay.glyphs.iter()).expect("base ink");
    let ruby = ruby_ink(&lay);
    assert!(ruby.x0 > base.x0, "ruby right of the column: {ruby:?} vs {base:?}");
    assert!(ruby.x0 >= base.x1 - 1.0, "ruby outside the column: {ruby:?} vs {base:?}");
    // Upright: each reading glyph's ink has the width and height of its outline at 10 pt (half the
    // base size); a glyph on its side has them swapped.
    let mut not_square = false;
    for g in &lay.ruby {
        let Some(r) = ink(g) else { continue };
        let face = db().face_by_id(g.font_id).expect("the ruby glyph's face");
        let o = db().outline(&face, g.gid).bounding_box();
        let (w, h) = (o.width() * 10.0 / face.units_per_em(), o.height() * 10.0 / face.units_per_em());
        assert!((r.width() - w).abs() < 0.01 && (r.height() - h).abs() < 0.01, "upright ruby glyph {r:?}, its outline {w} x {h}");
        not_square |= (w - h).abs() > 0.5;
    }
    assert!(not_square, "one reading glyph's outline is not square");
    // Down the column: the reading's glyphs have distinct y positions.
    let ys: Vec<f64> = lay.ruby.iter().filter_map(|g| ink(g).map(|b| b.center().y)).collect();
    assert!(ys.windows(2).any(|w| w[0] != w[1]), "the reading runs down the column: {ys:?}");
}

#[test]
fn ruby_moves_the_lines_after_it_down() {
    // A narrow area type: "漢字" wraps to two lines; ruby on either line needs room.
    let frame = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 24.0, 200.0).to_path(0.1)) };
    let mut plain = TextObject::point(Point::ZERO, "漢字", style(20.0));
    plain.kind = frame.clone();
    plain.xf = Affine::IDENTITY;
    let plain = layout(db(), &plain);
    assert_eq!(plain.lines.len(), 2, "two lines to compare");
    // Ruby over the second line needs room above it between the two lines: the line moves down.
    let mut t = plain_text_with(frame.clone());
    t.ruby = vec![RubySpan::new(3, 6, "じ")];
    let lay = layout(db(), &t);
    assert_eq!(lay.lines.len(), 2);
    let gap_plain = plain.lines[1].baseline - plain.lines[0].baseline;
    let gap_ruby = lay.lines[1].baseline - lay.lines[0].baseline;
    assert!(gap_ruby > gap_plain + 2.0, "the ruby pushes the line on: {gap_ruby} vs {gap_plain}");
    // Ruby over the first line stays inside the frame: area type moves the first line down too.
    let mut first = plain_text_with(frame.clone());
    first.ruby = vec![RubySpan::new(0, 3, "かん")];
    let lay = layout(db(), &first);
    assert!(lay.lines[0].baseline > plain.lines[0].baseline + 2.0, "{} vs {}", lay.lines[0].baseline, plain.lines[0].baseline);
}

fn plain_text_with(frame: TextKind) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, "漢字", style(20.0));
    t.kind = frame;
    t.xf = Affine::IDENTITY;
    t
}

#[test]
fn a_reading_over_a_line_break_is_split_between_the_lines() {
    let frame = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 24.0, 200.0).to_path(0.1)) };
    let mut t = plain_text_with(frame);
    t.ruby = vec![RubySpan::new(0, 6, "かんじ")];
    let lay = layout(db(), &t);
    assert_eq!(lay.lines.len(), 2);
    assert!(!lay.ruby.is_empty());
    let mut lines: Vec<usize> = lay.ruby.iter().map(|g| g.line).collect();
    lines.sort_unstable();
    lines.dedup();
    assert_eq!(lines, vec![0, 1], "part of the reading on each line: {lines:?}");
}

#[test]
fn ruby_is_not_laid_out_for_type_on_a_path() {
    let mut t = TextObject::point(Point::ZERO, "漢字", style(20.0));
    t.kind =
        TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new(Point::ZERO, Point::new(200.0, 0.0)).to_path(0.1)), start: 0.0, end: None };
    t.ruby = vec![RubySpan::new(0, 6, "かんじ")];
    let lay = layout(db(), &t);
    assert!(lay.ruby.is_empty(), "no above/below on a path yet");
    assert_eq!(lay.glyphs.len(), 2, "the base text is laid out as before");
}

#[test]
fn ruby_ranges_follow_edits_of_the_text() {
    let mut t = TextObject::point(Point::ZERO, "漢字です", style(20.0));
    t.ruby = vec![RubySpan::new(0, 6, "かんじ")];
    // Typing two bytes before the base moves the ruby.
    t.ruby_after_replace(0, 0, 2);
    assert_eq!((t.ruby[0].start, t.ruby[0].end), (2, 8));
    // Deleting the first base character shrinks it.
    t.ruby_after_replace(2, 5, 0);
    assert_eq!((t.ruby[0].start, t.ruby[0].end), (2, 5));
    // Editing outside leaves it alone.
    t.ruby_after_replace(8, 9, 1);
    assert_eq!((t.ruby[0].start, t.ruby[0].end), (2, 5));
    // An emptied reading no longer lays anything out.
    t.ruby[0].text.clear();
    t.normalize_ruby();
    assert!(t.ruby.is_empty());
}
