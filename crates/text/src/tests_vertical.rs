//! Vertical type (bundled fonts: `§` and `×` stand upright like CJK, letters lie on their side;
//! Japanese text only with craft-fonts' faces, skipped without them).
use super::*;
use kurbo::Shape;
use vectorcraft_doc::{CharStyle, Justify, TextKind};
use vectorcraft_geom::PathData;

fn vertical(text: &str) -> TextObject {
    vertical_styled(text, CharStyle { size: 20.0, ..CharStyle::default() })
}

fn vertical_styled(text: &str, style: CharStyle) -> TextObject {
    let mut t = TextObject::point(Point::ZERO, text, style);
    t.xf = Affine::IDENTITY;
    t.vertical = true;
    t
}

fn ink(l: &TextLayout, i: usize) -> Rect {
    l.glyphs[i].outline.bounding_box()
}

#[test]
fn columns_run_down_and_follow_each_other_to_the_left() {
    let l = layout(FontDb::global(), &vertical("§§\n§"));
    assert!(l.vertical);
    let (a, b, c) = (ink(&l, 0), ink(&l, 1), ink(&l, 2));
    assert!(b.center().y > a.center().y + 15.0, "down the column: {a:?} {b:?}");
    assert!((a.center().x - b.center().x).abs() < 0.5, "one column");
    assert!(c.center().x < a.center().x - 15.0, "the next column is to the left: {c:?}");
    // Point type: the anchor is on the first column's centre line.
    assert!(a.center().x.abs() < 4.0, "{a:?}");
    assert!(l.bounds.contains(a.center()) && l.bounds.contains(c.center()));
}

#[test]
fn upright_marks_stand_and_letters_lie_on_their_side() {
    let h = layout(FontDb::global(), &TextObject::point(Point::ZERO, "§l", CharStyle { size: 20.0, ..CharStyle::default() }));
    let v = layout(FontDb::global(), &vertical("§l"));
    let (hs, hl, vs, vl) = (ink(&h, 0), ink(&h, 1), ink(&v, 0), ink(&v, 1));
    assert!((vs.width() - hs.width()).abs() < 0.01 && (vs.height() - hs.height()).abs() < 0.01, "§ upright: {hs:?} {vs:?}");
    assert!((vl.width() - hl.height()).abs() < 0.01 && (vl.height() - hl.width()).abs() < 0.01, "l turned: {hl:?} {vl:?}");
    assert!(vl.width() > vl.height(), "a tall l lies across the column");
}

#[test]
fn caret_and_selection_turn_with_the_columns() {
    let l = layout(FontDb::global(), &vertical("§§§"));
    let (top, bottom) = caret_position(&l, "§".len());
    assert!((top.y - bottom.y).abs() < 1e-6 && (top.x - bottom.x).abs() > 10.0, "a horizontal caret across the column: {top:?} {bottom:?}");
    let between = (ink(&l, 0).center().y + ink(&l, 1).center().y) / 2.0;
    assert!((top.y - between).abs() < 3.0, "between the first two marks: {top:?} {between} {:?} {:?}", ink(&l, 0), ink(&l, 1));
    let q = selection_quads(&l, 0, "§§".len());
    let r = Rect::from_points(q[0][0], q[0][2]);
    assert!(r.height() > r.width() && r.contains(ink(&l, 0).center()) && r.contains(ink(&l, 1).center()) && !r.contains(ink(&l, 2).center()));
    // Clicking a mark finds it.
    let c = ink(&l, 2).center();
    assert_eq!(hit_byte(&l, c + Vec2::new(0.0, 6.0)), "§§§".len());
    assert_eq!(hit_byte(&l, c - Vec2::new(0.0, 6.0)), "§§".len());
}

#[test]
fn area_type_starts_at_the_right_edge_and_wraps_to_the_left() {
    let mut t = vertical(&"§".repeat(12));
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 100.0, 105.0).to_path(0.1)) };
    let l = layout(FontDb::global(), &t);
    let first = ink(&l, 0);
    assert!(first.x1 <= 100.0 + 1e-6 && first.x1 > 80.0 && first.y0 >= 0.0, "top right: {first:?}");
    let cols: Vec<f64> = l.glyphs.iter().map(|g| (g.outline.bounding_box().center().x * 10.0).round()).collect();
    let mut distinct = cols.clone();
    distinct.dedup();
    assert!(distinct.len() >= 2 && distinct.windows(2).all(|w| w[1] < w[0]), "columns right to left: {distinct:?}");
    assert!(l.glyphs.iter().all(|g| g.outline.bounding_box().y1 <= 105.0 + 1e-6), "inside the frame");
}

/// `baselines`: one per line that holds characters, along it for horizontal type and down the
/// column's centre line for vertical type (where point type's anchor is); none on a path.
#[test]
fn baselines_run_along_lines_and_down_column_centres() {
    let h = layout(
        FontDb::global(),
        &TextObject::point(
            Point::ZERO,
            "§§

§",
            CharStyle { size: 20.0, ..CharStyle::default() },
        ),
    );
    let hb = h.baselines();
    assert_eq!(hb.len(), 2, "the empty line has none: {hb:?}");
    assert!(hb[0].0 == Point::ZERO && hb[0].1.y == 0.0 && hb[0].1.x > 15.0, "the first baseline runs from the anchor: {hb:?}");
    assert!(hb[1].0.y > 40.0 && hb[1].0.y == hb[1].1.y && hb[1].1.x > hb[1].0.x, "the third line's, lower: {hb:?}");
    let v = layout(
        FontDb::global(),
        &vertical(
            "§§
§",
        ),
    );
    let vb = v.baselines();
    let (a, c) = (ink(&v, 0), ink(&v, 2));
    assert_eq!(vb.len(), 2, "{vb:?}");
    assert!(vb[0].0.x.abs() < 4.0 && vb[0].0.x == vb[0].1.x && vb[0].1.y > vb[0].0.y + 30.0, "down the first column's centre: {vb:?}");
    assert!((vb[0].0.x - a.center().x).abs() < 4.0 && (vb[1].0.x - c.center().x).abs() < 4.0, "through the glyphs: {vb:?} {a:?} {c:?}");
    let mut on_path = vertical("ab");
    on_path.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (100.0, 0.0)).to_path(0.1)), start: 0.0, end: None };
    assert!(layout(FontDb::global(), &on_path).baselines().is_empty());
}

#[test]
fn type_on_a_path_stays_horizontal() {
    let mut t = vertical("ab");
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (100.0, 0.0)).to_path(0.1)), start: 0.0, end: None };
    assert!(!layout(FontDb::global(), &t).vertical);
}

#[test]
fn three_digits_stand_across_the_column_squeezed_into_one_em() {
    let l = layout(FontDb::global(), &vertical("§100§"));
    let block: Vec<Rect> = (1..4).map(|i| ink(&l, i)).collect();
    assert!(block.windows(2).all(|w| w[1].center().x > w[0].center().x + 2.0 && (w[1].center().y - w[0].center().y).abs() < 1.0), "{block:?}");
    let span = block[0].union(block[2]);
    assert!(span.width() <= 20.0 + 1e-6, "squeezed into the 20 pt em: {span:?}");
    assert!(ink(&l, 4).center().y - ink(&l, 0).center().y < 45.0, "one em of the column");
}

#[test]
fn two_digits_stand_across_the_column_and_longer_numbers_lie_on_their_side() {
    let l = layout(FontDb::global(), &vertical("§12§2026"));
    let (one, two) = (ink(&l, 1), ink(&l, 2));
    assert!((one.center().y - two.center().y).abs() < 1.0 && two.center().x > one.center().x + 3.0, "12 side by side: {one:?} {two:?}");
    assert!(one.height() > one.width(), "upright digits are taller than wide: {one:?}");
    let (before, after) = (ink(&l, 0), ink(&l, 3));
    assert!(after.center().y - before.center().y < 45.0, "the block takes one em (20 pt) of the column");
    let year: Vec<Rect> = (4..8).map(|i| ink(&l, i)).collect();
    assert!(year.windows(2).all(|w| w[1].center().y > w[0].center().y + 5.0), "2026 runs down the column on its side");
}

/// Each line's text, from the layout's line ranges.
fn line_texts(t: &TextObject, l: &TextLayout) -> Vec<String> {
    let text = t.plain_text();
    l.lines.iter().map(|li| text.get(li.start..li.end).unwrap_or("").trim_end().to_string()).collect()
}

#[test]
fn kinsoku_keeps_closing_marks_off_line_starts_and_opening_brackets_off_line_ends() {
    for vertical_type in [false, true] {
        // Five characters fit a line: without kinsoku "。" would start the second line.
        let text = "あいうえお。かきくけ「こさしすせそ」";
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 101.0, 400.0).to_path(0.1)) };
        if vertical_type {
            t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 400.0, 101.0).to_path(0.1)) };
            t.vertical = true;
        }
        let l = layout(FontDb::global(), &t);
        let lines = line_texts(&t, &l);
        assert!(lines.len() > 2, "{lines:?}");
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                let first = line.chars().next().unwrap_or(' ');
                assert!(!crate::shape::no_line_start(first), "line {i} starts with {first}: {lines:?}");
            }
            let last = line.chars().last().unwrap_or(' ');
            assert!(!crate::shape::no_line_end(last), "line {i} ends with {last}: {lines:?}");
        }
    }
}

/// Each glyph's cell down the (single) column: from its origin over its advance. A glyph that
/// doesn't advance (the rest of a tate-chu-yoko block) shares the cell of the glyph before it.
fn column_cells(l: &TextLayout) -> Vec<(f64, f64)> {
    let mut cells: Vec<(f64, f64)> = Vec::with_capacity(l.glyphs.len());
    for g in &l.glyphs {
        let cell = match cells.last() {
            Some(&prev) if g.advance == 0.0 => prev,
            _ => (g.origin.y, g.origin.y + g.advance),
        };
        cells.push(cell);
    }
    cells
}

/// One column, cells one after the other, each glyph's ink inside its own cell (within `tol`)
/// and no two glyphs' ink overlapping.
fn assert_own_cells(l: &TextLayout, text: &str, tol: f64) {
    let label = |i: usize| &text[l.glyphs[i].byte..l.glyphs[i].byte + l.glyphs[i].len];
    let cells = column_cells(l);
    for (i, g) in l.glyphs.iter().enumerate() {
        assert!((g.origin.x - l.glyphs[0].origin.x).abs() < 1e-9, "{:?} leaves the column: {g:?}", label(i));
        if let Some(next) = l.glyphs.get(i + 1) {
            assert!((next.origin.y - (g.origin.y + g.advance)).abs() < 1e-9, "{:?} starts where {:?} ends", label(i + 1), label(i));
        }
        if g.outline.elements().is_empty() {
            continue;
        }
        let (r, (top, bottom)) = (ink(l, i), cells[i]);
        assert!(r.y0 >= top - tol && r.y1 <= bottom + tol, "{:?}: ink {r:?} outside its cell {top}..{bottom}", label(i));
        for j in (0..i).filter(|&j| !l.glyphs[j].outline.elements().is_empty()) {
            let o = ink(l, j);
            let (ox, oy) = (r.x1.min(o.x1) - r.x0.max(o.x0), r.y1.min(o.y1) - r.y0.max(o.y0));
            assert!(ox <= tol || oy <= tol, "{:?} {r:?} overlaps {:?} {o:?}", label(i), label(j));
        }
    }
}

/// Index of the glyph of `needle`'s first byte in `text`.
fn glyph_of(l: &TextLayout, text: &str, needle: &str) -> usize {
    let b = text.find(needle).unwrap();
    l.glyphs.iter().position(|g| g.byte == b).unwrap()
}

/// Issue #260 with bundled glyphs (`§` stands in for the upright CJK characters).
#[test]
fn numbers_keep_their_own_cells_down_the_column() {
    let text = "§§ §§§ 10§22§§ 1§ 100§ 2026§";
    let l = layout(FontDb::global(), &vertical(text));
    assert_own_cells(&l, text, 0.25);
    // Tate-chu-yoko: one em of the column, centred on the column's centre line (x = 0), the next
    // character right after it.
    for block in ["10§", "22§", "100§"] {
        let first = glyph_of(&l, text, block);
        let digits = block.len() - '§'.len_utf8();
        let after = first + digits;
        let g = &l.glyphs[first];
        assert!((g.advance - 20.0).abs() < 1e-9, "{block}: {g:?}");
        assert!(l.glyphs[first + 1..after].iter().all(|d| d.advance == 0.0), "{block}");
        assert!((l.glyphs[after].origin.y - (g.origin.y + 20.0)).abs() < 1e-9, "{block}: the next character follows the block's em");
        let inks = (first..after).map(|i| ink(&l, i)).reduce(|a, b| a.union(b)).unwrap();
        assert!(inks.center().x.abs() < 1.0, "{block} centred on the column: {inks:?}");
        assert!((inks.center().y - (g.origin.y + 10.0)).abs() < 1.5, "{block} centred in its em: {inks:?} {g:?}");
    }
    // A single digit and longer numbers lie on their side, advancing by their own length.
    let one = glyph_of(&l, text, "1§");
    let r = ink(&l, one);
    assert!(r.width() > r.height() && l.glyphs[one].advance < 20.0, "1 on its side: {r:?}");
    let year = glyph_of(&l, text, "2026");
    let len: f64 = l.glyphs[year..year + 4].iter().map(|g| g.advance).sum();
    assert!(len > 30.0 && (l.glyphs[year + 4].origin.y - l.glyphs[year].origin.y - len).abs() < 1e-9, "2026 takes its length: {len}");
    // Half-width spaces take their own (narrow) width.
    for (i, g) in l.glyphs.iter().enumerate().filter(|(_, g)| text[g.byte..].starts_with(' ')) {
        assert!(g.advance > 2.0 && g.advance < 10.0, "space {i}: {g:?}");
    }
}

#[test]
fn justified_columns_keep_tate_chu_yoko_blocks_whole_and_marks_centred() {
    let text = "§10§§";
    let mut t = vertical(text);
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 40.0, 200.0).to_path(0.1)) };
    t.para.justify = Justify::JustifyAll;
    let l = layout(FontDb::global(), &t);
    assert_eq!(l.lines.len(), 1);
    // 200 pt of column, four 20 pt cells: three gaps of 40 pt between them, none inside "10".
    let (one, zero) = (ink(&l, 1), ink(&l, 2));
    assert!((one.center().y - zero.center().y).abs() < 0.5 && zero.x0 > one.x1, "10 side by side: {one:?} {zero:?}");
    let marks: Vec<Rect> = [0, 3, 4].iter().map(|&i| ink(&l, i)).collect();
    assert!((marks[1].center().y - marks[0].center().y - 120.0).abs() < 1e-6, "{marks:?}");
    assert!((marks[2].center().y - marks[1].center().y - 60.0).abs() < 1e-6, "{marks:?}");
    assert!(marks.iter().all(|m| (m.center().x - marks[0].center().x).abs() < 1e-6), "upright marks stay on the centre line: {marks:?}");
    assert!(marks[2].y1 <= 200.0 + 1e-6, "inside the frame: {marks:?}");
}

#[test]
fn tracking_spaces_upright_marks_without_moving_them_off_the_centre_line() {
    let plain = layout(FontDb::global(), &vertical("§§"));
    let tracked = layout(FontDb::global(), &vertical_styled("§§", CharStyle { size: 20.0, tracking: 200.0, ..CharStyle::default() }));
    for i in 0..2 {
        assert!((ink(&tracked, i).center().x - ink(&plain, i).center().x).abs() < 1e-6, "{:?} {:?}", ink(&tracked, i), ink(&plain, i));
    }
    let step = ink(&tracked, 1).center().y - ink(&tracked, 0).center().y;
    assert!((step - 24.0).abs() < 1e-6, "one em plus 200/1000 em of tracking: {step}");
}

/// Issue #260 as reported, with a Japanese face of craft-fonts (whose own digits are proportional).
#[test]
fn japanese_numbers_keep_their_own_cells_down_the_column() {
    let Some(face) = crate::craft_fonts::japanese_ui_fonts(false).into_iter().next() else {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    };
    let db = FontDb::with_font_dirs(vec![]);
    let text = "はただ商店 秋のセール 10月22日まで";
    let t = vertical_styled(text, CharStyle { size: 28.0, font_family: face.family.into(), font_style: face.style.into(), ..CharStyle::default() });
    let l = layout(&db, &t);
    assert!(l.glyphs.iter().filter(|g| !text[g.byte..].starts_with(' ')).all(|g| g.gid != 0), "real glyphs, no tofu");
    assert_own_cells(&l, text, 0.5);
    for block in ["10月", "22日"] {
        let first = glyph_of(&l, text, block);
        let g = &l.glyphs[first];
        assert!((g.advance - 28.0).abs() < 1e-9 && l.glyphs[first + 1].advance == 0.0, "{block}: one em");
        assert!((l.glyphs[first + 2].origin.y - (g.origin.y + 28.0)).abs() < 1e-9, "{block}: the next character follows the em");
        let inks = ink(&l, first).union(ink(&l, first + 1));
        assert!(inks.center().x.abs() < 2.8, "{block} centred on the column: {inks:?}");
    }
}

/// Upright glyphs take their cell from the font's vertical metrics: the advance down the column,
/// and where the glyph hangs from its vertical origin (VORG, or the glyph's top and top side
/// bearing). Fonts without them keep one em and the em box centre.
#[test]
fn upright_glyphs_follow_the_fonts_vertical_metrics() {
    use crate::test_fonts::{VERTICAL_FAMILY, VERTICAL_TALL, vertical_font};
    for vorg in [false, true] {
        let db = FontDb::with_font_dirs(vec![]);
        db.add_font(vertical_font(vorg).unwrap());
        let text = format!("{VERTICAL_TALL}{VERTICAL_TALL}");
        let mut t = vertical(&text);
        t.runs[0].style.font_family = VERTICAL_FAMILY.into();
        let l = layout(&db, &t);
        let step = l.glyphs[1].origin.y - l.glyphs[0].origin.y;
        assert!((step - 28.0).abs() < 0.01, "1.4 em down the column at 20 pt (vorg {vorg}): {step}");
        // The glyph hangs from its vertical origin, 1.1 em above the baseline: its top is
        // (1.1 em − its own top) below the top of its cell.
        let face = db.face(VERTICAL_FAMILY, "Regular").unwrap();
        let (_, origin) = face.vertical_glyph(face.glyph_for(VERTICAL_TALL)).unwrap();
        assert!((origin - 1100.0).abs() < 1.0, "vorg {vorg}: {origin}");
        let top = db.outline(&face, face.glyph_for(VERTICAL_TALL)).bounding_box().y0; // y-down: −yMax
        let cell_top = l.glyphs[0].origin.y;
        let expected = cell_top + (1.1 + top / 1000.0) * 20.0;
        assert!((ink(&l, 0).y0 - expected).abs() < 0.05, "vorg {vorg}: ink top {} vs {expected}", ink(&l, 0).y0);
    }
    // The same glyph in the font without vertical metrics: one em, as before.
    let l = layout(FontDb::global(), &vertical("§§"));
    assert!((l.glyphs[1].origin.y - l.glyphs[0].origin.y - 20.0).abs() < 0.01);
}

/// Mojikumi (JLREQ 3.1): with Line-end Punctuation Half Width, a closing mark ending a line is set
/// half width, a closing mark followed by punctuation loses the space after it, and an opening
/// bracket after another loses the space before it; vertical type the same way down the column.
/// Needs a font with full-width Japanese punctuation (skipped without one).
#[test]
fn mojikumi_halves_line_end_and_consecutive_punctuation() {
    use vectorcraft_doc::Mojikumi;
    let lay = |text: &str, m: Mojikumi, vertical_type: bool| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = m;
        layout(FontDb::global(), &t)
    };
    let solid = lay("一。", Mojikumi::None, false);
    if (solid.glyphs[1].advance - 20.0).abs() > 2.0 {
        return; // no font with full-width punctuation here
    }
    for vertical_type in [false, true] {
        let adv = |text: &str, m| lay(text, m, vertical_type).glyphs.iter().map(|g| g.advance).collect::<Vec<_>>();
        // Line end: 。 half width.
        assert!((adv("一。", Mojikumi::LineEndHalf)[1] - 10.0).abs() < 0.01, "vertical {vertical_type}");
        assert!((adv("一。", Mojikumi::None)[1] - 20.0).abs() < 0.01);
        // 」 before 「: the closing mark's space goes; 「 keeps its own.
        let a = adv("一」「二", Mojikumi::LineEndHalf);
        assert!((a[1] - 10.0).abs() < 0.01 && (a[2] - 20.0).abs() < 0.01, "{a:?}");
        // 「「 inside a line: the second bracket loses the space before it and is drawn half an em
        // earlier (at a line's start the first one would lose its own too).
        let a = adv("一「「二", Mojikumi::LineEndHalf);
        assert!((a[1] - 20.0).abs() < 0.01 && (a[2] - 10.0).abs() < 0.01, "{a:?}");
        let (on, off) = (lay("一「「二", Mojikumi::LineEndHalf, vertical_type), lay("一「「二", Mojikumi::None, vertical_type));
        let along = |r: Rect| if vertical_type { r.y0 } else { r.x0 };
        let moved = along(off.glyphs[2].outline.bounding_box()) - along(on.glyphs[2].outline.bounding_box());
        assert!((moved - 10.0).abs() < 0.01, "vertical {vertical_type}: the second 「 is drawn half an em earlier ({moved})");
        let across = |r: Rect| if vertical_type { r.x0 } else { r.y0 };
        let drift = across(off.glyphs[2].outline.bounding_box()) - across(on.glyphs[2].outline.bounding_box());
        assert!(drift.abs() < 0.01, "vertical {vertical_type}: and not moved across the line ({drift})");
        // Text without punctuation is untouched.
        assert_eq!(adv("一二", Mojikumi::LineEndHalf), adv("一二", Mojikumi::None));
    }
}

/// Mojikumi (JLREQ 3.2.2): a quarter em between Japanese and Latin letters or digits, either way
/// round, horizontal and vertical; none inside a tate-chu-yoko block, none left at a line's end, and
/// none with Mojikumi None. Needs a font with full-width Japanese (skipped without one).
#[test]
fn mojikumi_spaces_japanese_from_latin_by_a_quarter_em() {
    use vectorcraft_doc::Mojikumi;
    let lay = |text: &str, m: Mojikumi, vertical_type: bool| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = m;
        layout(FontDb::global(), &t)
    };
    if (lay("雅", Mojikumi::None, false).glyphs[0].advance - 20.0).abs() > 2.0 {
        return; // no font with full-width Japanese here
    }
    for vertical_type in [false, true] {
        let extra = |text: &str| -> Vec<f64> {
            let (on, off) = (lay(text, Mojikumi::LineEndHalf, vertical_type), lay(text, Mojikumi::None, vertical_type));
            on.glyphs.iter().zip(&off.glyphs).map(|(a, b)| a.advance - b.advance).collect()
        };
        // 雅楽 2026 年: after 楽 and after 6.
        assert_eq!(extra("雅楽2026年").iter().map(|x| (x * 100.0).round() / 100.0).collect::<Vec<_>>(), [0.0, 5.0, 0.0, 0.0, 0.0, 5.0, 0.0]);
        // Latin first, and nothing after the last character of the line.
        assert_eq!(extra("AB雅").iter().map(|x| x.round()).collect::<Vec<_>>(), [0.0, 5.0, 0.0]);
        assert_eq!(extra("雅A").iter().map(|x| x.round()).collect::<Vec<_>>(), [5.0, 0.0]);
        // Punctuation takes no Japanese–Latin space (neither bracket is at the line's start or end).
        assert!(extra("で「A」です").iter().all(|x| x.abs() < 0.01), "{:?}", extra("で「A」です"));
    }
    // A tate-chu-yoko block is set as a Japanese character, with no space inside or around it.
    let on = lay("第10回", Mojikumi::LineEndHalf, true);
    let off = lay("第10回", Mojikumi::None, true);
    assert!(on.glyphs.iter().zip(&off.glyphs).all(|(a, b)| (a.advance - b.advance).abs() < 0.01));
}

/// Mojikumi (JLREQ 3.1.5): with Line-end Punctuation Half Width, an opening bracket starting a
/// wrapped line is set flush with the line's start (the space before it goes), giving the line half
/// an em more room; at the start of a paragraph too, so its face starts at the first-line indent
/// (JIS X 4051's principle, Figure 71 ①). Horizontal and vertical. Needs a font with full-width
/// Japanese punctuation (skipped without one).
#[test]
fn mojikumi_sets_an_opening_bracket_flush_at_the_start_of_a_wrapped_line() {
    use vectorcraft_doc::Mojikumi;
    // 20 pt type in a frame three and a half ems across the lines.
    let lay = |text: &str, m: Mojikumi, vertical_type: bool| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = m;
        let frame = if vertical_type { Rect::new(0.0, 0.0, 200.0, 70.0) } else { Rect::new(0.0, 0.0, 70.0, 200.0) };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        layout(FontDb::global(), &t)
    };
    if (lay("一「", Mojikumi::None, false).glyphs[1].advance - 20.0).abs() > 2.0 {
        return; // no font with full-width punctuation here
    }
    for vertical_type in [false, true] {
        let along = |r: Rect| if vertical_type { r.y0 } else { r.x0 };
        // 一二三 / 「四五六: the bracket can't end the first line, so it starts the second.
        let (on, off) = (lay("一二三「四五六", Mojikumi::LineEndHalf, vertical_type), lay("一二三「四五六", Mojikumi::None, vertical_type));
        let moved = along(off.glyphs[3].outline.bounding_box()) - along(on.glyphs[3].outline.bounding_box());
        assert!((moved - 10.0).abs() < 0.01, "vertical {vertical_type}: 「 is drawn half an em earlier ({moved})");
        let across = |r: Rect| if vertical_type { r.x0 } else { r.y0 };
        let drift = across(off.glyphs[3].outline.bounding_box()) - across(on.glyphs[3].outline.bounding_box());
        assert!(drift.abs() < 0.01, "vertical {vertical_type}: and not moved across the line ({drift})");
        assert!((on.glyphs[3].advance - 10.0).abs() < 0.01, "vertical {vertical_type}: {}", on.glyphs[3].advance);
        // The half em it gave up lets 六 stay on the line.
        assert_eq!(on.glyphs[6].line, on.glyphs[3].line, "vertical {vertical_type}");
        assert_ne!(off.glyphs[6].line, off.glyphs[3].line, "vertical {vertical_type}");
        // At the start of a paragraph too: half width, its face half an em earlier than with
        // Mojikumi None, so it starts at the first-line indent.
        let first = lay("「一」", Mojikumi::LineEndHalf, vertical_type);
        assert!((first.glyphs[0].advance - 10.0).abs() < 0.01, "vertical {vertical_type}: {}", first.glyphs[0].advance);
        let solid = lay("「一」", Mojikumi::None, vertical_type);
        let moved = along(solid.glyphs[0].outline.bounding_box()) - along(first.glyphs[0].outline.bounding_box());
        assert!((moved - 10.0).abs() < 0.01, "vertical {vertical_type}: the paragraph's 「 is drawn half an em earlier ({moved})");
    }
}

/// Burasagari: in a measure of exactly five ems, a comma that would be the sixth character goes to
/// the next line with the character before it (None) or hangs outside the line (Standard, Forced);
/// a full stop that is the fifth character stays inside (None, Standard) or hangs while the four
/// before it fill the measure (Forced). Closing brackets don't hang. Horizontal and vertical, with
/// Line-end Punctuation Half Width (the hanging mark is half width). Needs a font with full-width
/// Japanese punctuation (skipped without one).
#[test]
fn burasagari_hangs_a_comma_or_full_stop_outside_the_line() {
    use vectorcraft_doc::{Burasagari, Mojikumi};
    // 20 pt type, justified, in a frame five ems along the lines.
    let lay = |text: &str, b: Burasagari, vertical_type: bool| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = Mojikumi::LineEndHalf;
        t.para.justify = Justify::JustifyLeft;
        t.para.burasagari = b;
        let frame = if vertical_type { Rect::new(0.0, 0.0, 200.0, 100.0) } else { Rect::new(0.0, 0.0, 100.0, 200.0) };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        layout(FontDb::global(), &t)
    };
    if (lay("一、二", Burasagari::None, false).glyphs[1].advance - 20.0).abs() > 2.0 {
        return; // no font with full-width punctuation here
    }
    let advances = |l: &TextLayout, n: usize| l.glyphs.iter().take(n).map(|g| (g.advance * 100.0).round() / 100.0).collect::<Vec<_>>();
    for vertical_type in [false, true] {
        // 、 as the sixth character.
        let none = lay("一二三四五、六七", Burasagari::None, vertical_type);
        assert_ne!(none.glyphs[4].line, none.glyphs[0].line, "vertical {vertical_type}: 五、 go to the next line");
        for b in [Burasagari::Standard, Burasagari::Forced] {
            let l = lay("一二三四五、六七", b, vertical_type);
            assert_eq!(l.glyphs[5].line, l.glyphs[0].line, "vertical {vertical_type} {b:?}: 、 hangs");
            assert_eq!(advances(&l, 6), [20.0, 20.0, 20.0, 20.0, 20.0, 10.0], "vertical {vertical_type} {b:?}: five ems inside, half an em outside");
            assert_eq!(l.glyphs[6].line, l.glyphs[0].line + 1, "vertical {vertical_type} {b:?}");
        }
        // 。 as the fifth character: it fits.
        for b in [Burasagari::None, Burasagari::Standard] {
            let l = lay("一二三四。六七", b, vertical_type);
            assert_eq!(l.glyphs[5].line, l.glyphs[0].line + 1, "vertical {vertical_type} {b:?}");
            // Four ems and a half: the half em left is spread between the five characters.
            assert_eq!(advances(&l, 5), [22.5, 22.5, 22.5, 22.5, 10.0], "vertical {vertical_type} {b:?}: 。 inside");
        }
        let forced = lay("一二三四。六七", Burasagari::Forced, vertical_type);
        let a = advances(&forced, 5);
        assert!((a[..4].iter().sum::<f64>() - 100.0).abs() < 0.05, "vertical {vertical_type}: the four fill the measure: {a:?}");
        assert_eq!(a[4], 10.0, "vertical {vertical_type}");
        assert_eq!(forced.glyphs[4].line, forced.glyphs[0].line);
        // A closing bracket doesn't hang: 」 as the sixth character goes on with 五.
        for b in [Burasagari::Standard, Burasagari::Forced] {
            let l = lay("一二三四五」六七", b, vertical_type);
            assert_ne!(l.glyphs[4].line, l.glyphs[0].line, "vertical {vertical_type} {b:?}");
        }
        // The full-width comma and full stop hang too.
        for mark in ['，', '．'] {
            let l = lay(&format!("一二三四五{mark}六七"), Burasagari::Standard, vertical_type);
            assert_eq!(l.glyphs[5].line, l.glyphs[0].line, "vertical {vertical_type}: {mark} hangs");
        }
    }
    // Horizontal: the hanging mark starts at the frame's edge.
    let l = lay("一二三四五、六七", Burasagari::Standard, false);
    assert!((l.glyphs[5].origin.x - 100.0).abs() < 0.01, "{:?}", l.glyphs[5].origin);
}

/// Kinsoku Set: in a measure of two ems, あカッ breaks before ッ with Soft (a small kana may start a
/// line) and before カ with Hard (ッ goes with the character before it); None breaks as Soft here.
/// Horizontal and vertical. Needs a font with full-width Japanese (skipped without one).
#[test]
fn kinsoku_set_decides_whether_a_small_kana_may_start_a_line() {
    use vectorcraft_doc::Kinsoku;
    let lay = |text: &str, k: Kinsoku, vertical_type: bool| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.kinsoku = k;
        let frame = if vertical_type { Rect::new(0.0, 0.0, 200.0, 40.0) } else { Rect::new(0.0, 0.0, 40.0, 200.0) };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        layout(FontDb::global(), &t)
    };
    if (lay("雅", Kinsoku::Hard, false).glyphs[0].advance - 20.0).abs() > 2.0 {
        return; // no font with full-width Japanese here
    }
    for vertical_type in [false, true] {
        let lines = |k| lay("あカッ", k, vertical_type).glyphs.iter().map(|g| g.line).collect::<Vec<_>>();
        assert_eq!(lines(Kinsoku::Hard), [0, 1, 1], "vertical {vertical_type}");
        assert_eq!(lines(Kinsoku::Soft), [0, 0, 1], "vertical {vertical_type}");
        assert_eq!(lines(Kinsoku::None), [0, 0, 1], "vertical {vertical_type}");
        // A full stop never starts a line, Soft or not.
        let stop = |k| lay("あカ。", k, vertical_type).glyphs.iter().map(|g| g.line).collect::<Vec<_>>();
        assert_eq!(stop(Kinsoku::Soft), [0, 1, 1], "vertical {vertical_type}");
    }
}

/// Proportional Metrics in vertical type (#966): upright glyphs take the font's proportional
/// heights (`vpal`, from shaping them top to bottom) and move with them, so each glyph's ink stays in
/// its shorter cell. Line-end Punctuation Half Width takes nothing more off the punctuation `vpal`
/// re-spaced, and trims the punctuation it leaves full height (〘〙 in Shippori Mincho) as before.
/// Off, the column is as before, and a font without vertical metrics (the bundled Source Sans 3)
/// keeps today's cells. Needs craft-fonts' Shippori Mincho (skipped without it).
#[test]
fn proportional_metrics_set_upright_glyphs_on_their_proportional_heights() {
    use vectorcraft_doc::Mojikumi;
    let lay = |text: &str, font: &str, on: bool, m: Mojikumi| {
        let mut t = vertical_styled(text, CharStyle { size: 20.0, font_family: font.into(), proportional_metrics: on, ..CharStyle::default() });
        t.para.mojikumi = m;
        layout(FontDb::global(), &t)
    };
    let adv = |l: &TextLayout| l.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>();
    let y0 = |l: &TextLayout| l.glyphs.iter().map(|g| g.outline.bounding_box().y0).collect::<Vec<_>>();
    let near = |a: &[f64], b: &[f64]| a.len() == b.len() && a.iter().zip(b).all(|(a, b)| (a - b).abs() < 0.01);
    // Without vertical metrics the cells stay as they are.
    let (on, off) = (lay("§§", "Source Sans 3", true, Mojikumi::None), lay("§§", "Source Sans 3", false, Mojikumi::None));
    assert!(near(&adv(&on), &adv(&off)) && near(&y0(&on), &y0(&off)), "{:?} {:?}", adv(&on), adv(&off));
    if FontDb::global().face("Shippori Mincho", "Regular").is_none_or(|f| f.family != "Shippori Mincho") {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    }
    let text = "「あ」、ッ漢。";
    let off = lay(text, "Shippori Mincho", false, Mojikumi::None);
    assert!(off.glyphs.iter().all(|g| (g.advance - 20.0).abs() < 0.01), "off: an em each, as before: {:?}", adv(&off));
    let on = lay(text, "Shippori Mincho", true, Mojikumi::None);
    assert!(on.glyphs[0].advance < 12.0 && on.glyphs[4].advance < 16.0, "「 and ッ shorter: {:?}", adv(&on));
    assert!((on.glyphs[5].advance - 20.0).abs() < 0.01, "the kanji keeps its em: {:?}", adv(&on));
    // Each glyph's ink stays in its own cell, so none runs into the next.
    for g in &on.glyphs {
        let b = g.outline.bounding_box();
        assert!(b.y0 > g.origin.y - 0.5 && b.y1 < g.origin.y + g.advance + 0.5, "byte {}: ink {b:?} in [{}, +{}]", g.byte, g.origin.y, g.advance);
    }
    // Re-spaced punctuation: Line-end Punctuation Half Width changes nothing.
    let half = lay(text, "Shippori Mincho", true, Mojikumi::LineEndHalf);
    assert!(near(&adv(&half), &adv(&on)) && near(&y0(&half), &y0(&on)), "{:?} {:?}", adv(&half), adv(&on));
    // Punctuation `vpal` leaves full height: set flush at the paragraph's start, as before.
    let (half, none) = (lay("〘あ〙", "Shippori Mincho", true, Mojikumi::LineEndHalf), lay("〘あ〙", "Shippori Mincho", true, Mojikumi::None));
    assert!((none.glyphs[0].advance - 20.0).abs() < 0.01 && (half.glyphs[0].advance - 10.0).abs() < 0.01, "{:?}", adv(&half));
}
