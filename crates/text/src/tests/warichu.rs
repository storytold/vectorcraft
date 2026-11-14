//! Warichu (Character panel menu › Warichu and Warichu Settings): marked characters set smaller
//! and stacked on lines inside their line. Bundled fonts: in vertical type `§` stands upright like
//! CJK.
use super::{area, db, point, serif, style};
use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{CharStyle, Justify, TextKind, TextRun, WarichuAlign};
use vectorcraft_geom::PathData;

fn marked(size: f64) -> CharStyle {
    CharStyle { warichu: true, ..style(size) }
}

/// "ab", then `inside` in `st`, then "z", as point type at the origin.
fn with_block(inside: &str, st: CharStyle) -> TextObject {
    let mut t = point("", style(20.0));
    t.xf = Affine::IDENTITY;
    t.runs = vec![TextRun::new("ab", style(20.0)), TextRun::new(inside, st), TextRun::new("z", style(20.0))];
    t
}

fn ink(l: &TextLayout, i: usize) -> Rect {
    l.glyphs[i].outline.bounding_box()
}

/// Two lines by default, at 50% of the size: the first two of four characters on a line above the
/// other two, both lines starting where the block starts, and the text after the block starting
/// where the longer line ends. The lines are one warichu em apart, and the stack is centered
/// across the line on the line's ideographic em box.
#[test]
fn warichu_stacks_the_characters_on_two_lines_at_half_size() {
    let l = layout(db(), &with_block("cdef", marked(20.0)));
    let plain = layout(db(), &with_block("cdef", style(20.0)));
    let (c, d, e, f, z) = (&l.glyphs[2], &l.glyphs[3], &l.glyphs[4], &l.glyphs[5], &l.glyphs[6]);
    // Half size: half the ink height and half the advance of the same letter in the line.
    let ratio = ink(&l, 2).height() / ink(&plain, 2).height();
    assert!((ratio - 0.5).abs() < 1e-6, "{ratio}");
    assert!((c.advance - plain.glyphs[2].advance * 0.5).abs() < 1e-6);
    // Two lines: c and d on the first, e and f on the second, starting where the block starts.
    assert!((c.origin.y - d.origin.y).abs() < 1e-9 && (e.origin.y - f.origin.y).abs() < 1e-9);
    assert!((e.origin.y - c.origin.y - 10.0).abs() < 1e-6, "one em of 10 pt apart: {:?} {:?}", c.origin, e.origin);
    assert!((c.origin.x - e.origin.x).abs() < 1e-9);
    assert!((c.origin.x - plain.glyphs[2].origin.x).abs() < 1e-9, "the block starts where the text would");
    // The block is as long as its longer line, and the text after it follows on.
    let block = (c.advance + d.advance).max(e.advance + f.advance);
    assert!((z.origin.x - c.origin.x - block).abs() < 1e-6, "{} {}", z.origin.x - c.origin.x, block);
    assert!((z.origin.y - l.glyphs[0].origin.y).abs() < 1e-9, "the line's own baseline");
    // Centered on the line's ideographic em box (0.12 em below the baseline to 0.88 em above it
    // for a font without ideographs): the two 10 pt em boxes fill the 20 pt one, so the first
    // line's baseline is 8.8 pt above the line's and the second's 1.2 pt below it.
    let baseline = z.origin.y;
    assert!((c.origin.y - (baseline - 8.8)).abs() < 1e-6, "{} {baseline}", c.origin.y);
    assert!((e.origin.y - (baseline + 1.2)).abs() < 1e-6, "{} {baseline}", e.origin.y);
    // Every glyph of the block is on the first line of the text.
    assert!(l.glyphs.iter().all(|g| g.line == 0));
}

/// Warichu Settings: Lines sets how many lines the block is stacked on, Line Gap adds space
/// between them, and Scale sets the size as a percentage.
#[test]
fn warichu_settings_set_the_lines_their_gap_and_the_scale() {
    let st = CharStyle { warichu_lines: 3, warichu_gap: 4.0, warichu_scale: 30.0, ..marked(20.0) };
    let l = layout(db(), &with_block("cdefgh", st));
    let plain = layout(db(), &with_block("cdefgh", style(20.0)));
    let ys: Vec<f64> = l.glyphs[2..8].iter().map(|g| g.origin.y).collect();
    let mut lines = ys.clone();
    lines.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert_eq!(lines.len(), 3, "{ys:?}");
    for pair in lines.windows(2) {
        assert!((pair[1] - pair[0] - (6.0 + 4.0)).abs() < 1e-6, "an em of 6 pt plus the gap: {lines:?}");
    }
    let ratio = ink(&l, 2).height() / ink(&plain, 2).height();
    assert!((ratio - 0.3).abs() < 1e-6, "{ratio}");
    // One line: the characters keep to one smaller line centered on the line.
    let one = layout(db(), &with_block("cd", CharStyle { warichu_lines: 1, ..marked(20.0) }));
    assert!((one.glyphs[2].origin.y - one.glyphs[3].origin.y).abs() < 1e-9);
    assert!(one.glyphs[3].origin.x > one.glyphs[2].origin.x);
    // A block has no more lines than characters.
    let two = layout(db(), &with_block("cd", CharStyle { warichu_lines: 6, ..marked(20.0) }));
    assert!((two.glyphs[3].origin.y - two.glyphs[2].origin.y - 10.0).abs() < 1e-6);
}

/// Alignment: the shorter line is set at the start of the block, centered, at its end or spread
/// to its length; Auto spreads every line but the last.
#[test]
fn warichu_alignment_sets_the_lines_along_the_block() {
    // Where each line of a two-line block of four letters starts and ends along the block, and the
    // block's length.
    let at = |text: &str, align: WarichuAlign| {
        let l = layout(db(), &with_block(text, CharStyle { warichu_align: align, ..marked(20.0) }));
        let g = &l.glyphs[2..6];
        let line = |a: &PositionedGlyph, b: &PositionedGlyph| (a.origin.x - g[0].origin.x, b.origin.x + b.advance - g[0].origin.x);
        let block = l.glyphs[6].origin.x - g[0].origin.x;
        (line(&g[0], &g[1]), line(&g[2], &g[3]), block)
    };
    // "mm" then "mi": the second, last line is shorter.
    let (first, (start, end), block) = at("mmmi", WarichuAlign::Left);
    assert!(first.0.abs() < 1e-9 && (first.1 - block).abs() < 1e-6, "the longer line fills the block");
    assert!(start.abs() < 1e-9 && end < block - 1.0);
    let (_, (start, end), block) = at("mmmi", WarichuAlign::Right);
    assert!((end - block).abs() < 1e-6 && start > 1.0);
    let (_, (start, end), block) = at("mmmi", WarichuAlign::Center);
    assert!((start - (block - end)).abs() < 1e-6 && start > 0.5);
    // The last line of a justified block: set left, centered or right, or spread with JustifyAll.
    let (_, (start, _), _) = at("mmmi", WarichuAlign::JustifyLeft);
    assert!(start.abs() < 1e-9);
    let (_, (_, end), block) = at("mmmi", WarichuAlign::JustifyRight);
    assert!((end - block).abs() < 1e-6);
    let (_, (start, end), block) = at("mmmi", WarichuAlign::JustifyCenter);
    assert!((start - (block - end)).abs() < 1e-6 && start > 0.5);
    let (_, (start, end), block) = at("mmmi", WarichuAlign::JustifyAll);
    assert!(start.abs() < 1e-9 && (end - block).abs() < 1e-6);
    // "im" then "mm": the shorter first line is spread to the block's length by every justified
    // alignment and by Auto, and not by Left.
    for align in [WarichuAlign::Auto, WarichuAlign::JustifyLeft, WarichuAlign::JustifyCenter, WarichuAlign::JustifyRight, WarichuAlign::JustifyAll] {
        let ((start, end), _, block) = at("immm", align);
        assert!(start.abs() < 1e-9 && (end - block).abs() < 1e-6, "{align:?}: {start} {end} {block}");
    }
    let ((_, end), _, block) = at("immm", WarichuAlign::Left);
    assert!(end < block - 1.0);
    assert_eq!(at("mmmi", WarichuAlign::Auto), at("mmmi", WarichuAlign::JustifyLeft));
}

/// A warichu block stays whole on one line: area type breaks before or after it, never inside it,
/// and a block longer than the line runs past its end.
#[test]
fn a_warichu_block_does_not_break_across_lines() {
    let mut t = area("", style(20.0), Rect::new(0.0, 0.0, 60.0, 400.0), Justify::Left);
    t.runs = vec![TextRun::new("ab ", style(20.0)), TextRun::new("cd efgh ijkl mnop qrst", marked(20.0)), TextRun::new(" z", style(20.0))];
    let l = layout(db(), &t);
    let block: Vec<usize> = l.glyphs.iter().filter(|g| (3..25).contains(&g.byte)).map(|g| g.line).collect();
    assert!(!block.is_empty());
    assert!(block.iter().all(|&line| line == block[0]), "{block:?}");
    assert!(l.glyphs.iter().any(|g| g.line != block[0]), "the text around it breaks");
}

/// Vertical type: the lines of a block stand side by side across the column, the first to the
/// right, each running down from where the block starts.
#[test]
fn vertical_warichu_sets_the_lines_side_by_side() {
    let mut t = point("", style(20.0));
    t.xf = Affine::IDENTITY;
    t.vertical = true;
    t.runs = vec![TextRun::new("§", style(20.0)), TextRun::new("§§§§", marked(20.0)), TextRun::new("§", style(20.0))];
    let l = layout(db(), &t);
    let (a, b, c) = (ink(&l, 1), ink(&l, 2), ink(&l, 3));
    assert!((a.center().x - b.center().x).abs() < 1e-6, "one line down the column");
    assert!(c.center().x < a.center().x - 5.0, "the second line is to the left: {a:?} {c:?}");
    assert!((a.y0 - c.y0).abs() < 1e-6, "both lines start where the block starts");
    // The column's center lies between the two lines, and the character after the block follows
    // the longer line.
    let column = ink(&l, 0).center().x;
    assert!((((a.center().x + c.center().x) * 0.5) - column).abs() < 1.0, "{column} {a:?} {c:?}");
    assert!(ink(&l, 5).y0 > b.y1, "{:?} {b:?}", ink(&l, 5));
}

/// Vertical point type that starts with warichu keeps its anchor on the column's center line, as
/// the column has the size of its text.
#[test]
fn vertical_point_type_starting_with_warichu_keeps_its_anchor() {
    let column = |runs: Vec<TextRun>| {
        let mut t = point("", style(20.0));
        t.xf = Affine::IDENTITY;
        t.vertical = true;
        t.runs = runs;
        let l = layout(db(), &t);
        ink(&l, l.glyphs.len() - 1).center().x
    };
    let plain = column(vec![TextRun::new("§", style(20.0))]);
    let after = column(vec![TextRun::new("§§", marked(20.0)), TextRun::new("§", style(20.0))]);
    assert!((plain - after).abs() < 1e-6, "{plain} {after}");
}

/// Every character of a block, a space at its end too, keeps its advance inside the block, and a
/// block of spaces alone keeps their length.
#[test]
fn spaces_keep_their_advance_inside_a_block() {
    for inside in ["cdef ", "  "] {
        let l = layout(db(), &with_block(inside, marked(20.0)));
        let n = inside.len();
        let (start, end) = (l.glyphs[2].origin.x.min(l.glyphs[2 + n - 1].origin.x), l.glyphs[2 + n].origin.x);
        assert!(end > start, "{inside:?}");
        for g in &l.glyphs[2..2 + n] {
            assert!(g.origin.x >= start - 1e-9 && g.origin.x + g.advance <= end + 1e-9, "{inside:?}: {:?} {} {start} {end}", g.origin, g.advance);
        }
    }
}

/// In vertical type, characters marked Tate-chu-yoko as well are set across the column at their
/// own size, and digits marked Warichu are stacked rather than set across the column.
#[test]
fn vertical_tate_chu_yoko_wins_and_warichu_digits_stack() {
    let column = |runs: Vec<TextRun>| {
        let mut t = point("", style(20.0));
        t.xf = Affine::IDENTITY;
        t.vertical = true;
        t.runs = runs;
        layout(db(), &t)
    };
    let both = column(vec![TextRun::new("AB", CharStyle { tate_chu_yoko: true, ..marked(20.0) })]);
    let tcy = column(vec![TextRun::new("AB", CharStyle { tate_chu_yoko: true, ..style(20.0) })]);
    assert_eq!(ink(&both, 0), ink(&tcy, 0));
    assert_eq!(ink(&both, 1), ink(&tcy, 1));
    // "10" alone would stand across the column; marked Warichu its digits are stacked, the 1 to
    // the right of the 0.
    let digits = column(vec![TextRun::new("10", marked(20.0))]);
    assert!(ink(&digits, 0).center().x > ink(&digits, 1).center().x + 2.0, "{:?} {:?}", ink(&digits, 0), ink(&digits, 1));
    assert!((ink(&digits, 0).y0 - ink(&digits, 1).y0).abs() < 2.0, "both start where the block starts");
}

/// A line holding only a warichu block keeps the leading of the block's own size, with the default
/// Auto Leading and with the paragraph's own.
#[test]
fn a_warichu_line_keeps_the_auto_leading_of_its_own_size() {
    for auto in [120.0, 150.0] {
        let gap = |first: CharStyle| {
            let mut t = point("", style(20.0));
            t.xf = Affine::IDENTITY;
            t.para.justification.auto_leading = auto;
            t.runs = vec![TextRun::new("cd", first), TextRun::new("\u{2028}z", style(20.0))];
            let l = layout(db(), &t);
            l.lines[1].baseline - l.lines[0].baseline
        };
        let (block, plain) = (gap(marked(20.0)), gap(style(20.0)));
        assert!((block - plain).abs() < 1e-6, "{auto}%: {block} {plain}");
    }
}

/// Area type that starts with warichu sets its first baseline as for text of the block's own size,
/// whatever the First Baseline setting.
#[test]
fn the_first_baseline_measures_a_block_at_its_own_size() {
    use vectorcraft_doc::FirstBaseline;
    for first in [FirstBaseline::Ascent, FirstBaseline::CapHeight, FirstBaseline::XHeight, FirstBaseline::Leading] {
        let baseline = |inside: CharStyle| {
            let mut t = area("", style(20.0), Rect::new(0.0, 0.0, 300.0, 300.0), Justify::Left);
            t.area.first_baseline = first;
            t.runs = vec![TextRun::new("cd", inside)];
            layout(db(), &t).lines[0].baseline
        };
        let (block, plain) = (baseline(marked(20.0)), baseline(style(20.0)));
        assert!((block - plain).abs() < 1e-6 || block > plain, "{first:?}: {block} {plain}");
        if first != FirstBaseline::Ascent {
            assert!((block - plain).abs() < 1e-6, "{first:?}: {block} {plain}");
        }
    }
}

/// Mixed right-to-left and left-to-right text in a line of a block is set in its bidi order.
#[test]
fn a_warichu_line_sets_mixed_text_in_bidi_order() {
    let l = layout(db(), &with_block("אב12", CharStyle { warichu_lines: 1, ..marked(20.0) }));
    let at = |byte: usize| l.glyphs.iter().find(|g| g.byte == byte).unwrap().origin.x;
    let (alef, bet, one, two) = (at(2), at(4), at(6), at(7));
    assert!(one < two && two < bet && bet < alef, "{one} {two} {bet} {alef}");
}

/// Insert Aki before the block's first character and after its last is space before and after the
/// block.
#[test]
fn insert_aki_spaces_the_block() {
    let at = |aki: (f64, f64)| {
        let mut t = with_block("cd", marked(20.0));
        t.runs = vec![
            TextRun::new("ab", style(20.0)),
            TextRun::new("c", CharStyle { aki_left: aki.0, ..marked(20.0) }),
            TextRun::new("d", CharStyle { aki_right: aki.1, ..marked(20.0) }),
            TextRun::new("z", style(20.0)),
        ];
        let l = layout(db(), &t);
        (l.glyphs[2].origin.x, l.glyphs[4].origin.x)
    };
    let (c, z) = at((0.0, 0.0));
    let (c2, z2) = at((500.0, 250.0));
    assert!((c2 - c - 5.0).abs() < 1e-6, "half an em of 10 pt before: {c} {c2}");
    assert!((z2 - z - 7.5).abs() < 1e-6, "and a quarter after: {z} {z2}");
    // Right to left, the space before the first character is on the block's right.
    let rtl = |aki: f64| {
        let mut t = with_block("", marked(20.0));
        t.runs =
            vec![TextRun::new("ab", style(20.0)), TextRun::new("אב", CharStyle { aki_left: aki, ..marked(20.0) }), TextRun::new("z", style(20.0))];
        let l = layout(db(), &t);
        let at = |byte: usize| l.glyphs.iter().find(|g| g.byte == byte).unwrap().origin.x;
        (at(2).min(at(4)), at(6))
    };
    let ((left, z), (left2, z2)) = (rtl(0.0), rtl(500.0));
    assert!((left2 - left).abs() < 1e-6 && z2 > z, "{left} {left2} {z} {z2}");
}

/// Type on a path sets warichu characters at their own size along the path, without stacking them.
#[test]
fn type_on_a_path_does_not_stack_warichu() {
    let mut t = with_block("cdef", marked(20.0));
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (400.0, 0.0)).to_path(0.1)), start: 0.0, end: None };
    let mut plain = with_block("cdef", style(20.0));
    plain.kind = t.kind.clone();
    let (l, p) = (layout(db(), &t), layout(db(), &plain));
    // The block's characters are set where the same characters without the mark are (the pair
    // spacing after the block may differ, since the block is shaped apart from the text after it).
    let placed = |l: &TextLayout| l.glyphs[..6].iter().map(|g| g.origin).collect::<Vec<_>>();
    assert_eq!(placed(&l), placed(&p));
}

/// A line may break between two blocks with different settings, and a block longer than its line
/// stays whole.
#[test]
fn lines_break_between_blocks_and_a_long_block_stays_whole() {
    let mut t = area("", style(20.0), Rect::new(0.0, 0.0, 30.0, 400.0), Justify::Left);
    let other = CharStyle { warichu_lines: 3, ..marked(20.0) };
    t.runs = vec![TextRun::new("mmmm", marked(20.0)), TextRun::new("mmmmmm", other), TextRun::new("mmmmmmmmmmmmmmmmmmmm", marked(20.0))];
    let l = layout(db(), &t);
    let lines = |r: std::ops::Range<usize>| l.glyphs.iter().filter(|g| r.contains(&g.byte)).map(|g| g.line).collect::<Vec<_>>();
    let (a, b, c) = (lines(0..4), lines(4..10), lines(10..30));
    assert!(a.iter().all(|&x| x == a[0]) && b.iter().all(|&x| x == b[0]) && c.iter().all(|&x| x == c[0]));
    assert!(a[0] < b[0] && b[0] < c[0], "{a:?} {b:?} {c:?}");
}

/// Justification's Glyph Scaling leaves a block's glyphs at their size and places, as it leaves an
/// inline graphic.
#[test]
fn glyph_scaling_leaves_a_warichu_block_as_it_is() {
    let at = |scaling: [f64; 3]| {
        let mut t = with_block("cdef", marked(20.0));
        t.para.justification.glyph_scaling = scaling;
        let l = layout(db(), &t);
        let start = l.glyphs[2].origin.x;
        (l.glyphs[2..6].iter().map(|g| (g.origin.x - start, g.origin.y, g.advance)).collect::<Vec<_>>(), ink(&l, 3).width())
    };
    let (plain, plain_ink) = at([100.0, 100.0, 100.0]);
    let (scaled, scaled_ink) = at([120.0, 120.0, 120.0]);
    assert_eq!(plain, scaled);
    assert!((plain_ink - scaled_ink).abs() < 1e-9);
}

/// A block gets every line it asks for when its places to break are too few or too early: it is
/// split between characters instead.
#[test]
fn a_block_gets_its_lines_when_places_to_break_run_short() {
    let l = layout(db(), &with_block("a mmmmmmmm", CharStyle { warichu_lines: 3, ..marked(20.0) }));
    let mut ys: Vec<f64> = l.glyphs[2..12].iter().map(|g| g.origin.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert_eq!(ys.len(), 3, "{ys:?}");
}

/// With System Layout on, the block takes a whole number of points, and the text after it starts
/// where the block ends.
#[test]
fn system_layout_sets_the_block_in_whole_points() {
    let mut t = with_block("cdef", marked(20.0));
    t.system_layout = true;
    let l = layout(db(), &t);
    let (c, d, e, f, z) = (&l.glyphs[2], &l.glyphs[3], &l.glyphs[4], &l.glyphs[5], &l.glyphs[6]);
    let length = z.origin.x - c.origin.x;
    assert!((length - length.round()).abs() < 1e-9, "{length}");
    let longer = (d.origin.x + d.advance).max(f.origin.x + f.advance) - c.origin.x.min(e.origin.x);
    assert!((length - longer).abs() <= 0.5 + 1e-9, "{length} {longer}");
}

/// Character Alignment moves a block on a line of larger text as it moves text of the block's own
/// size: the block counts as text of the size of the line it sits in.
#[test]
fn character_alignment_moves_a_block_as_text_of_its_own_size() {
    let at = |align: vectorcraft_doc::CharAlign, inside: CharStyle| {
        let mut t = point("", style(40.0));
        t.xf = Affine::IDENTITY;
        t.runs = vec![TextRun::new("ab", style(40.0)), TextRun::new("cd", CharStyle { char_align: align, ..inside })];
        let l = layout(db(), &t);
        // Where the third character sits across the line, from the line's baseline.
        l.glyphs[2].origin.y - l.glyphs[0].origin.y
    };
    let plain = |a| at(a, style(20.0));
    let block = |a| at(a, marked(20.0));
    let roman = vectorcraft_doc::CharAlign::RomanBaseline;
    for a in [vectorcraft_doc::CharAlign::EmBoxTop, vectorcraft_doc::CharAlign::EmBoxCenter, vectorcraft_doc::CharAlign::EmBoxBottom] {
        let (moved, still) = (block(a) - block(roman), plain(a) - plain(roman));
        assert!((moved - still).abs() < 1e-6, "{a:?}: {moved} {still}");
    }
}

/// A tab ends a block: it keeps its tab stop, and the characters after it form a block of their own.
#[test]
fn a_tab_ends_a_warichu_block() {
    let l = layout(db(), &with_block("cd\tef", marked(20.0)));
    let (c, d, tab, e, f) = (&l.glyphs[2], &l.glyphs[3], &l.glyphs[4], &l.glyphs[5], &l.glyphs[6]);
    assert!(d.origin.y > c.origin.y && (d.origin.x - c.origin.x).abs() < 1e-9, "c and d are one block");
    assert!(f.origin.y > e.origin.y && (f.origin.x - e.origin.x).abs() < 1e-9, "e and f are another");
    assert!((tab.origin.y - l.glyphs[0].origin.y).abs() < 1e-9 && tab.origin.x > c.origin.x);
    assert!(e.origin.x >= tab.origin.x + tab.advance - 1e-6);
}

/// A right-to-left block reads from its right end: its first character is to the right of the
/// second on the first line.
#[test]
fn a_right_to_left_warichu_block_reads_from_its_right_end() {
    // Hebrew letters, two bytes each.
    let l = layout(db(), &with_block("אבגד", marked(20.0)));
    let at = |byte: usize| l.glyphs.iter().find(|g| g.byte == byte).unwrap();
    let (c, d, e) = (at(2), at(4), at(6));
    assert!((c.origin.y - d.origin.y).abs() < 1e-9 && e.origin.y > c.origin.y);
    assert!(c.origin.x > d.origin.x, "{:?} {:?}", c.origin, d.origin);
    assert!((c.origin.x - (d.origin.x + d.advance)).abs() < 1e-6, "c follows d to its right");
}

/// Shrink Text to Fit scales the Line Gap with the size, as it scales the baseline shift.
#[test]
fn shrink_text_to_fit_scales_the_warichu_line_gap() {
    let st = CharStyle { warichu_gap: 10.0, ..marked(20.0) };
    let t = with_block("cdef", st);
    let s = scaled(&t, 0.5);
    assert_eq!(s.runs[1].style.warichu_gap, 5.0);
    let l = layout(db(), &s);
    assert!((l.glyphs[4].origin.y - l.glyphs[2].origin.y - (5.0 + 5.0)).abs() < 1e-6);
}

/// Two blocks whose characters the bidi order draws in turn (Latin, then Hebrew in the first;
/// Hebrew, then Latin in the second) each keep where they start: both lines of a block start there.
#[test]
fn blocks_the_bidi_order_interleaves_keep_their_places() {
    let mut t = with_block("", marked(20.0));
    t.runs = vec![
        TextRun::new("ab", style(20.0)),
        TextRun::new("cא", marked(20.0)),
        TextRun::new("בd", CharStyle { warichu_lines: 3, ..marked(20.0) }),
        TextRun::new("z", style(20.0)),
    ];
    let l = layout(db(), &t);
    let at = |byte: usize| l.glyphs.iter().find(|g| g.byte == byte).unwrap().origin;
    let (c, alef, bet, d, z) = (at(2), at(3), at(5), at(7), at(8));
    assert!(alef.y > c.y && d.y > bet.y, "two lines each: {c:?} {alef:?} {bet:?} {d:?}");
    assert!((alef.x - c.x).abs() < 1e-9, "the first block's lines start together: {c:?} {alef:?}");
    assert!((d.x - bet.x).abs() < 1e-9, "the second block's lines start together: {bet:?} {d:?}");
    assert!(c.x < bet.x && bet.x < z.x, "{c:?} {bet:?} {z:?}");
}

/// A justified line spreads its letters with the same gap before a right-to-left block as before a
/// left-to-right one: the block starts at the first of its glyphs the line draws.
#[test]
fn justification_opens_the_same_gap_before_a_right_to_left_block() {
    for inside in ["cd", "אב"] {
        let at = |justify: Justify| {
            let mut t = area("", style(20.0), Rect::new(0.0, 0.0, 200.0, 400.0), justify);
            t.runs = vec![TextRun::new("ab", style(20.0)), TextRun::new(inside, marked(20.0)), TextRun::new("z", style(20.0))];
            let l = layout(db(), &t);
            let x = |byte: usize| l.glyphs.iter().filter(|g| g.byte == byte).map(|g| g.origin.x).fold(f64::INFINITY, f64::min);
            let block = l.glyphs.iter().filter(|g| g.warichu.is_some()).map(|g| g.origin.x).fold(f64::INFINITY, f64::min);
            (x(0), x(1), block, x(2 + inside.len()))
        };
        let (a, b, block, z) = at(Justify::Left);
        let (a2, b2, block2, z2) = at(Justify::JustifyAll);
        let gap = (b2 - a2) - (b - a);
        assert!(gap > 1.0, "{inside}: the line is spread: {gap}");
        assert!(((block2 - b2) - (block - b) - gap).abs() < 1e-6, "{inside}: the gap before the block: {} {gap}", (block2 - b2) - (block - b));
        assert!(((z2 - block2) - (z - block) - gap).abs() < 1e-6, "{inside}: the gap after the block: {} {gap}", (z2 - block2) - (z - block));
    }
}

/// The caret next to a character of a block spans the band of the character's block line, not
/// the whole line.
#[test]
fn the_caret_in_a_block_spans_its_block_line() {
    let l = layout(db(), &with_block("cdef", marked(20.0)));
    let (c, e) = (&l.glyphs[2], &l.glyphs[4]);
    let middle = (c.origin.y + e.origin.y) * 0.5;
    let line = &l.lines[0];
    // "c" and "d", on the upper line.
    for byte in [2, 3] {
        let (top, bottom) = caret_position(&l, byte);
        assert!((top.y - (line.baseline - line.ascent)).abs() < 1e-6 && (bottom.y - middle).abs() < 1e-6, "{byte}: {top:?} {bottom:?} {middle}");
    }
    // "e" and "f", on the lower line.
    for byte in [4, 5] {
        let (top, bottom) = caret_position(&l, byte);
        assert!((top.y - middle).abs() < 1e-6 && (bottom.y - (line.baseline + line.descent)).abs() < 1e-6, "{byte}: {top:?} {bottom:?} {middle}");
    }
    // Outside the block the caret spans the line.
    let (top, bottom) = caret_position(&l, 0);
    assert!((top.y - (line.baseline - line.ascent)).abs() < 1e-6 && (bottom.y - (line.baseline + line.descent)).abs() < 1e-6);
}

/// A click on a block's lower line goes to that line's characters, not to the character of the
/// upper line at the same place along the block.
#[test]
fn a_click_on_a_block_line_goes_to_its_own_characters() {
    let l = layout(db(), &with_block("cdef", marked(20.0)));
    let (c, d, e, f) = (&l.glyphs[2], &l.glyphs[3], &l.glyphs[4], &l.glyphs[5]);
    let near = |g: &PositionedGlyph, share: f64| Point::new(g.origin.x + g.advance * share, g.origin.y - 2.0);
    assert_eq!(hit_byte(&l, near(e, 0.25)), 4);
    assert_eq!(hit_byte(&l, near(f, 0.75)), 6);
    assert_eq!(hit_byte(&l, near(c, 0.25)), 2);
    assert_eq!(hit_byte(&l, near(d, 0.75)), 4);
    // Without a point across the line, the nearest stop along it, as before.
    assert_eq!(byte_in_line(&l, 0, l.glyphs[0].origin.x), 0);
}

/// Selecting characters of a block highlights each on its own line of the block: a character of
/// the upper line does not highlight the lower one, and a selection that runs from the upper line
/// to the lower one covers just the selected characters on each. A character with its own
/// Character Alignment stays on its block line for the highlight and for a click.
#[test]
fn a_selection_in_a_block_highlights_its_own_block_line() {
    let l = layout(db(), &with_block("cdef", marked(20.0)));
    let (c, d, e) = (&l.glyphs[2], &l.glyphs[3], &l.glyphs[4]);
    let middle = (c.origin.y + e.origin.y) * 0.5;
    let extent = |quads: &[[Point; 4]]| {
        let ys = quads.iter().flatten().map(|p| p.y);
        let xs = quads.iter().flatten().map(|p| p.x);
        (
            xs.clone().fold(f64::INFINITY, f64::min),
            xs.fold(f64::NEG_INFINITY, f64::max),
            ys.clone().fold(f64::INFINITY, f64::min),
            ys.fold(f64::NEG_INFINITY, f64::max),
        )
    };
    // "c", on the upper line.
    let (x0, x1, y0, y1) = extent(&selection_quads(&l, 2, 3));
    assert!(y0 < c.origin.y && y1 <= middle + 1e-9, "{y0} {y1} {middle}");
    assert!((x0 - c.origin.x).abs() < 1e-6 && (x1 - (c.origin.x + c.advance)).abs() < 1e-6, "{x0} {x1}");
    // "e", on the lower line.
    let (_, _, y0, y1) = extent(&selection_quads(&l, 4, 5));
    assert!(y0 >= middle - 1e-9 && y1 > e.origin.y, "{y0} {y1} {middle}");
    // "de": d on the upper line and e on the lower, nothing of c.
    let quads = selection_quads(&l, 3, 5);
    assert_eq!(quads.len(), 2, "{quads:?}");
    let (x0, _, _, _) = extent(&quads[..1]);
    assert!((x0 - d.origin.x).abs() < 1e-6, "{x0} {:?}", d.origin);
    // A selection that runs on past the line's break covers the break after the line's text too.
    let mut t = with_block("cdef", marked(20.0));
    t.runs.push(TextRun::new("\u{2028}y", style(20.0)));
    let l = layout(db(), &t);
    let z = &l.glyphs[6];
    let first: Vec<[Point; 4]> = selection_quads(&l, 2, 11).into_iter().filter(|q| q[0].y < l.lines[0].baseline).collect();
    let (_, x1, _, _) = extent(&first);
    assert!(x1 > z.origin.x + z.advance + 1.0, "{x1} {:?} {}", z.origin, z.advance);
    // A character of the upper line with its own Character Alignment stays on that line: on a line
    // of 40 pt text, where Character Alignment moves a block of 20 pt, "d" is set lower than "c".
    let mut t = with_block("", marked(20.0));
    t.runs = vec![
        TextRun::new("ab", style(40.0)),
        TextRun::new("c", marked(20.0)),
        TextRun::new("d", CharStyle { char_align: vectorcraft_doc::CharAlign::EmBoxBottom, ..marked(20.0) }),
        TextRun::new("ef", marked(20.0)),
        TextRun::new("z", style(40.0)),
    ];
    let l = layout(db(), &t);
    assert!((l.glyphs[3].origin.y - l.glyphs[2].origin.y).abs() > 0.5, "d is set lower: {:?} {:?}", l.glyphs[2].origin, l.glyphs[3].origin);
    // "c" and "d" alone take the same band, and "cd" one highlight over both.
    let band = |a: usize, b: usize| {
        let (_, _, y0, y1) = extent(&selection_quads(&l, a, b));
        (y0, y1)
    };
    let ((c0, c1), (d0, d1)) = (band(2, 3), band(3, 4));
    assert!((c0 - d0).abs() < 1e-9 && (c1 - d1).abs() < 1e-9, "{c0} {c1} {d0} {d1}");
    let quads = selection_quads(&l, 2, 4);
    assert!(quads.len() == 1 && (quads[0][2].y - c1).abs() < 1e-9, "{quads:?} {c1}");
    // The lower line's band starts where the upper line's ends.
    let (e0, _) = band(4, 5);
    assert!((e0 - c1).abs() < 1e-9, "{e0} {c1}");
    // A click on the upper line at the end of "d" goes there too, though "d" is set lower than "c".
    let (c, d) = (&l.glyphs[2], &l.glyphs[3]);
    assert_eq!(hit_byte(&l, Point::new(d.origin.x + d.advance * 0.75, c.origin.y - 2.0)), 4);
    // A justified block line's selected characters are highlighted as one piece, over the space
    // between them too: "im" spread to the length of "mm".
    let l = layout(db(), &with_block("immm", CharStyle { warichu_align: WarichuAlign::JustifyAll, ..marked(20.0) }));
    let (i, m) = (&l.glyphs[2], &l.glyphs[3]);
    assert!(m.origin.x > i.origin.x + i.advance + 1.0, "the line is spread: {:?} {} {:?}", i.origin, i.advance, m.origin);
    let quads = selection_quads(&l, 2, 4);
    let (x0, x1, _, _) = extent(&quads);
    assert!(quads.len() == 1 && (x0 - i.origin.x).abs() < 1e-6 && (x1 - (m.origin.x + m.advance)).abs() < 1e-6, "{quads:?}");
    // Characters selected together that the bidi order draws apart keep apart: "a" and "א" of
    // "aאבb", drawn "aבאb", leave "ב" between them out.
    let l = layout(db(), &with_block("aאבb", CharStyle { warichu_lines: 1, ..marked(20.0) }));
    let bet = l.glyphs.iter().find(|g| g.byte == 5).map(|g| (l.logical_point(g.origin).x, g.advance)).unwrap();
    let quads = selection_quads(&l, 2, 5);
    assert_eq!(quads.len(), 2, "{quads:?}");
    for q in &quads {
        assert!(q[1].x <= bet.0 + 1e-6 || q[0].x >= bet.0 + bet.1 - 1e-6, "{q:?} {bet:?}");
    }
}

/// Digits marked Warichu in vertical type on a path stand across the column as digits without the
/// mark do: type on a path does not stack warichu.
#[test]
fn vertical_type_on_a_path_keeps_its_digits_across_the_column() {
    let on_path = |st: CharStyle| {
        let mut t = point("", style(20.0));
        t.xf = Affine::IDENTITY;
        t.vertical = true;
        t.runs = vec![TextRun::new("12", st)];
        t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (0.0, 400.0)).to_path(0.1)), start: 0.0, end: None };
        let l = layout(db(), &t);
        (ink(&l, 0), ink(&l, 1))
    };
    assert_eq!(on_path(marked(20.0)), on_path(style(20.0)));
}

/// A block whose characters come from fonts of different heights gives its line the tallest ascent
/// and the deepest descent among them, whichever font comes first.
#[test]
fn a_block_of_several_fonts_keeps_the_tallest_of_them() {
    let fonts = ["Source Sans 3", "Source Serif 4", "Inter", "JetBrains Mono"];
    let st = |family: &str, warichu: bool| CharStyle { font_family: family.into(), warichu, ..style(20.0) };
    let line = |runs: Vec<TextRun>| {
        let mut t = point("", style(20.0));
        t.xf = Affine::IDENTITY;
        t.runs = runs;
        let l = layout(db(), &t);
        (l.lines[0].ascent, l.lines[0].descent)
    };
    let plain: Vec<(f64, f64)> = fonts.iter().map(|f| line(vec![TextRun::new("x", st(f, false))])).collect();
    // The two fonts whose ascents differ most.
    let mut pair = (0, 0);
    for i in 0..fonts.len() {
        for j in 0..fonts.len() {
            if plain[i].0 - plain[j].0 > plain[pair.0].0 - plain[pair.1].0 {
                pair = (i, j);
            }
        }
    }
    let (tall, short) = (fonts[pair.0], fonts[pair.1]);
    assert!(plain[pair.0].0 - plain[pair.1].0 > 0.1, "the bundled fonts differ in height: {plain:?}");
    let first_short = line(vec![TextRun::new("c", st(short, true)), TextRun::new("d", st(tall, true))]);
    let first_tall = line(vec![TextRun::new("c", st(tall, true)), TextRun::new("d", st(short, true))]);
    assert!((first_short.0 - first_tall.0).abs() < 1e-6 && (first_short.1 - first_tall.1).abs() < 1e-6, "{first_short:?} {first_tall:?}");
    let deepest = plain[pair.0].1.max(plain[pair.1].1);
    assert!(first_short.0 >= plain[pair.0].0 - 1e-6 && first_short.1 >= deepest - 1e-6, "{first_short:?} {plain:?}");
}

/// Warichu Settings on characters not marked Warichu leave their shaping alone: "fi" keeps its
/// ligature when only the f has other settings.
#[test]
fn warichu_settings_without_the_mark_keep_a_ligature() {
    let fi = |f: CharStyle| {
        let mut t = point("", serif(20.0));
        t.runs = vec![TextRun::new("f", f), TextRun::new("i", serif(20.0))];
        layout(db(), &t)
    };
    let glyphs = |l: &TextLayout| l.glyphs.iter().map(|g| (g.gid, g.len)).collect::<Vec<_>>();
    let plain = fi(serif(20.0));
    assert!(plain.glyphs.iter().any(|g| g.len == 2), "the bundled serif joins f and i: {:?}", glyphs(&plain));
    let settings = CharStyle { warichu_lines: 3, warichu_gap: 2.0, warichu_scale: 70.0, warichu_align: WarichuAlign::Center, ..serif(20.0) };
    assert_eq!(glyphs(&fi(settings)), glyphs(&plain));
}

/// Insert Kashidas leaves a warichu block alone: a justified line lengthens its other Arabic words
/// with kashidas, and the block keeps the length its lines set.
#[test]
fn kashidas_leave_a_warichu_block_alone() {
    use crate::test_fonts::{TATWEEL_FAMILY, tatweel_font};
    use vectorcraft_doc::text::Kashida;
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(tatweel_font().expect("the test font builds"));
    let face = db.face(TATWEEL_FAMILY, "Regular").expect("the test face");
    let arabic = |warichu: bool| CharStyle { font_family: TATWEEL_FAMILY.into(), warichu, ..style(20.0) };
    let (before, block) = ("تبد تبد ", "بتب");
    let mut t = area("", arabic(false), Rect::new(0.0, 0.0, 300.0, 400.0), Justify::JustifyAll);
    t.para.kashida = Kashida::Medium;
    t.runs = vec![TextRun::new(before, arabic(false)), TextRun::new(block, arabic(true)), TextRun::new(" تبد", arabic(false))];
    let l = layout(&db, &t);
    let tatweel = face.glyph_for('\u{0640}');
    let kashidas: Vec<usize> = l.glyphs.iter().filter(|g| g.len == 0 && g.gid == tatweel).map(|g| g.byte).collect();
    assert!(!kashidas.is_empty(), "the words around the block take kashidas");
    let inside = before.len()..before.len() + block.len();
    assert!(kashidas.iter().all(|b| !inside.contains(b)), "{kashidas:?} {inside:?}");
}
