//! Vertical type (bundled fonts: `§` and `×` stand upright like CJK, letters lie on their side;
//! Japanese text only with craft-fonts' faces, skipped without them).
use crate::*;
use kurbo::Shape;
use vectorcraft_doc::{AreaFit, CharStyle, Justify, TextKind, TextRun};
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

/// Unicode Vertical_Orientation Tu: without a vertical alternate, fullwidth !/? stay upright.
/// Tr controls (a bracket and the long vowel mark) still rotate when they have no alternate.
#[test]
fn fullwidth_punctuation_stays_upright_without_vertical_alternates() {
    check_fullwidth_punctuation(false);
}

#[test]
fn fullwidth_punctuation_keeps_vertical_alternates_upright() {
    check_fullwidth_punctuation(true);
}

fn check_fullwidth_punctuation(alternates: bool) {
    use crate::test_fonts::{VERTICAL_PUNCTUATION_FAMILY, vertical_punctuation_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(vertical_punctuation_font(alternates).unwrap());
    let style = CharStyle { font_family: VERTICAL_PUNCTUATION_FAMILY.into(), size: 20.0, ..CharStyle::default() };
    let face = db.face(VERTICAL_PUNCTUATION_FAMILY, "Regular").unwrap();
    for (ch, nominal, alternate) in [('！', '!', 'I'), ('？', '?', 'J'), ('「', 'L', 'L'), ('ー', '-', '-')] {
        let upright = matches!(ch, '！' | '？');
        let expected = if alternates { alternate } else { nominal };
        let h = layout(&db, &TextObject::point(Point::ZERO, &expected.to_string(), style.clone()));
        for area in [false, true] {
            let mut t = vertical_styled(&ch.to_string(), style.clone());
            if area {
                t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 100.0, 100.0).to_path(0.1)) };
            }
            let v = layout(&db, &t);
            assert_eq!(v.glyphs.len(), 1, "{ch} area {area}");
            assert!(!v.overflow);
            let g = &v.glyphs[0];
            assert_eq!(g.font_id, face.id());
            assert_eq!(g.gid, face.glyph_for(expected), "{ch}: the expected nominal or vert glyph");
            let reference = if upright { h.glyphs[0].xf } else { Affine::rotate(std::f64::consts::FRAC_PI_2) * h.glyphs[0].xf };
            for (actual, expected) in g.xf.as_coeffs().iter().zip(reference.as_coeffs().iter()).take(4) {
                assert!((actual - expected).abs() < 1e-9, "{ch} area {area}, alternates {alternates}: {:?} vs {reference:?}", g.xf);
            }
        }
        // Vertical substitutions must not change horizontal type.
        let h = layout(&db, &TextObject::point(Point::ZERO, &ch.to_string(), style.clone()));
        assert_eq!(h.glyphs[0].gid, face.glyph_for(nominal));
    }
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

/// A vertical frame too narrow for its first column is made wider from its right edge, where that
/// column is; its height stays.
#[test]
fn fit_first_line_widens_a_vertical_frame_from_its_right_edge() {
    let mut t = vertical("§§");
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(90.0, 0.0, 100.0, 105.0).to_path(0.1)) };
    assert!(layout(FontDb::global(), &t).lines.is_empty(), "a column doesn't fit");
    assert!(fit_first_line(FontDb::global(), &mut t));
    let TextKind::Area { frame } = &t.kind else { panic!("area") };
    let b = frame.bounds().unwrap();
    assert!((b.x1 - 100.0).abs() < 1e-6 && b.x0 < 90.0 && b.x0 > 60.0 && (b.y0, b.y1) == (0.0, 105.0), "{b:?}");
    let l = layout(FontDb::global(), &t);
    assert_eq!(l.lines.len(), 1);
    assert!(ink(&l, 0).x1 <= 100.0 + 1e-6, "the column at the right edge: {:?}", ink(&l, 0));
    // A fixed first baseline further in than the line's ascent: wide enough for it too.
    let mut t = vertical("§§");
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(90.0, 0.0, 100.0, 105.0).to_path(0.1)) };
    t.area.first_baseline = crate::FirstBaseline::Fixed;
    t.area.first_baseline_min = 40.0;
    assert!(fit_first_line(FontDb::global(), &mut t));
    let TextKind::Area { frame } = &t.kind else { panic!("area") };
    assert!(frame.bounds().unwrap().x0 < 60.0, "{:?}", frame.bounds());
    assert_eq!(layout(FontDb::global(), &t).lines.len(), 1);
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

/// `text` in vertical type on a straight path from `from` to `to`, with path effect `effect`.
fn on_vertical_path(text: &str, from: (f64, f64), to: (f64, f64), effect: vectorcraft_doc::PathEffect) -> TextObject {
    let mut t = vertical(text);
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new(from, to).to_path(0.1)), start: 0.0, end: None };
    t.path_effect = effect;
    t
}

/// Glyph `i`'s top (its ascender direction) and its own advance direction in text space, as unit
/// vectors.
fn top_and_ahead(l: &TextLayout, i: usize) -> (Vec2, Vec2) {
    let g = &l.glyphs[i];
    let pen = g.xf * Point::ZERO;
    let unit = |v: Vec2| v / v.hypot();
    (unit(g.xf * Point::new(0.0, -500.0) - pen), unit(g.xf * Point::new(500.0, 0.0) - pen))
}

/// On a vertical path with Rainbow, an upright glyph's top faces the path's start, and a letter lying
/// on its side is a quarter turn clockwise from it: its top faces the path's left and its baseline
/// runs along the path. Both turn with the path when it runs the other way. The effects that keep
/// the glyphs' axes fixed on the page keep that relation only on a path that runs to the right.
#[test]
fn on_a_vertical_path_letters_lie_a_quarter_turn_clockwise_from_upright_glyphs() {
    use vectorcraft_doc::PathEffect;
    for to in [(0.0, 1000.0), (0.0, -1000.0), (1000.0, 0.0), (-1000.0, 0.0), (700.0, 700.0)] {
        let l = layout(FontDb::global(), &on_vertical_path("§L", (0.0, 0.0), to, PathEffect::Rainbow));
        let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
        let left = Vec2::new(tangent.y, -tangent.x);
        let (upright, _) = top_and_ahead(&l, 0);
        let (side, ahead) = top_and_ahead(&l, 1);
        assert!((upright + tangent).hypot() < 1e-6, "{to:?}: the upright top faces the start: {upright:?}");
        // A clockwise quarter turn with y down takes (x, y) to (-y, x).
        assert!((side - Vec2::new(-upright.y, upright.x)).hypot() < 1e-6, "{to:?}: {side:?} against {upright:?}");
        assert!((side - left).hypot() < 1e-6 && (ahead - tangent).hypot() < 1e-6, "{to:?}: {side:?} {ahead:?}");
    }
    // Skew, 3D Ribbon and Stair Step keep axes of the glyphs fixed on the page: on a path that runs
    // to the right the relation holds. On one that runs to the left, Skew and 3D Ribbon mirror both
    // kinds, and Stair Step keeps letters level while upright glyphs still face the path's start, so
    // letters are a quarter turn counterclockwise from upright glyphs there.
    for effect in [PathEffect::Skew, PathEffect::Ribbon3d, PathEffect::StairStep] {
        for to in [(1000.0, 0.0), (700.0, 700.0), (0.0, 1000.0), (0.0, -1000.0), (-1000.0, 0.0), (-700.0, 300.0)] {
            // Skew and 3D Ribbon slant glyphs on a slanted path, where no turn relates them exactly.
            if to.1 != 0.0 && effect != PathEffect::StairStep {
                continue;
            }
            let l = layout(FontDb::global(), &on_vertical_path("§L", (0.0, 0.0), to, effect));
            let (upright, _) = top_and_ahead(&l, 0);
            let (side, _) = top_and_ahead(&l, 1);
            let clockwise = to.0 >= 0.0;
            let want = if clockwise { Vec2::new(-upright.y, upright.x) } else { Vec2::new(upright.y, -upright.x) };
            assert!((side - want).hypot() < 1e-6, "{effect:?} {to:?}: {side:?} against {upright:?}");
        }
    }
}

/// On a vertical path, letters lying on their side are set as horizontal type on the same path sets
/// them, with every path effect and on a path running either way, and they follow one another along
/// the path without overlapping where the effect keeps them square to it.
#[test]
fn on_a_vertical_path_letters_keep_the_horizontal_path_pose() {
    use vectorcraft_doc::PathEffect;
    for effect in [PathEffect::Rainbow, PathEffect::Skew, PathEffect::Ribbon3d, PathEffect::StairStep, PathEffect::Gravity] {
        for to in [(1000.0, 0.0), (-1000.0, 0.0), (700.0, 300.0)] {
            let mut vertical = on_vertical_path("LLl", (0.0, 0.0), to, effect);
            let v = layout(FontDb::global(), &vertical);
            vertical.vertical = false;
            let h = layout(FontDb::global(), &vertical);
            for i in 0..3 {
                let (a, b) = (v.glyphs[i].xf.as_coeffs(), h.glyphs[i].xf.as_coeffs());
                assert!(a.iter().zip(b).take(4).all(|(a, b)| (a - b).abs() < 1e-9), "{effect:?} {to:?} glyph {i}: {a:?} against {b:?}");
            }
            let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
            let span = |i: usize| {
                let along = v.glyphs[i].outline.elements().iter().filter_map(|el| el.end_point()).map(|p| p.to_vec2().dot(tangent));
                along.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), d| (lo.min(d), hi.max(d)))
            };
            // Skew slants the letters, Stair Step keeps them level and Gravity fans them toward the
            // path's center, so their ink can reach past the next letter's.
            if matches!(effect, PathEffect::Rainbow | PathEffect::Ribbon3d) {
                assert!((0..2).all(|i| span(i).1 <= span(i + 1).0 + 1e-6), "{effect:?} {to:?}: {:?}", (0..3).map(span).collect::<Vec<_>>());
            }
        }
    }
}

/// On a vertical path, the column is centered on the path: the em boxes of upright glyphs, of letters
/// lying on their side and of a tate-chu-yoko block straddle it, their centers on the path line, on
/// a path running any way.
#[test]
fn on_a_vertical_path_the_column_is_centered_on_the_path() {
    use vectorcraft_doc::PathEffect;
    let Some(face) = FontDb::global().face("Source Sans 3", "Regular") else { panic!("the bundled face") };
    let (em, centre) = (face.units_per_em(), face.ideographic_centre());
    for to in [(0.0, 1000.0), (0.0, -1000.0), (1000.0, 0.0), (-700.0, 700.0)] {
        let l = layout(FontDb::global(), &on_vertical_path("§H12§", (0.0, 0.0), to, PathEffect::Rainbow));
        let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
        let across = |p: Point| p.to_vec2().dot(Vec2::new(tangent.y, -tangent.x));
        // An upright glyph (the shaper centers a narrow one in its em box, as off a path) has its
        // ink centered on the path.
        for i in [0, 4] {
            let mid = across(ink(&l, i).center());
            assert!(mid.abs() < 1.0, "{to:?}: upright glyph {i} is {mid} off the path");
        }
        // A letter lying on its side: its em box's center is `centre` em above its baseline.
        let mid = across(l.glyphs[1].xf * Point::new(0.0, -centre * em));
        assert!(mid.abs() < 1e-6, "{to:?}: the letter's em box is {mid} off the path");
        // The digits of the block stand side by side across the path, the block centered on it.
        let block = (across(ink(&l, 2).center()) + across(ink(&l, 3).center())) * 0.5;
        assert!(block.abs() < 1.5, "{to:?}: the block is {block} off the path");
    }
}

/// On a vertical path, the caret and the selection span the column, half its em on each side of the
/// path, as the glyphs do.
#[test]
fn on_a_vertical_path_the_caret_and_selection_span_the_column() {
    use vectorcraft_doc::PathEffect;
    for to in [(0.0, 1000.0), (1000.0, 0.0), (-700.0, 700.0)] {
        let l = layout(FontDb::global(), &on_vertical_path("§L§", (0.0, 0.0), to, PathEffect::Rainbow));
        let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
        let across = |p: Point| p.to_vec2().dot(Vec2::new(tangent.y, -tangent.x));
        let (top, bottom) = caret_position(&l, "§".len());
        assert!((across(top) - 10.0).abs() < 1e-6 && (across(bottom) + 10.0).abs() < 1e-6, "{to:?}: caret {top:?} {bottom:?}");
        for q in selection_quads(&l, 0, "§L§".len()) {
            let (lo, hi) = q.iter().map(|p| across(*p)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), a| (lo.min(a), hi.max(a)));
            assert!((lo + 10.0).abs() < 1e-6 && (hi - 10.0).abs() < 1e-6, "{to:?}: highlight from {lo} to {hi}");
        }
    }
}

/// On a straight path running down from the anchor, vertical type sets every glyph where vertical
/// point type sets it: upright glyphs with a horizontal scale stay centered on the column, a baseline
/// shift moves an upright glyph across the column, every upright glyph keeps its own cell along the
/// path, and a combining mark stays on its upright letter.
#[test]
fn on_a_downward_vertical_path_glyphs_sit_where_vertical_point_type_sets_them() {
    use crate::test_fonts::{VERTICAL_FAMILY, vertical_font};
    let run = |text: &str, style: CharStyle| TextRun { text: text.into(), style, inline: None };
    let runs = |plain: CharStyle| {
        vec![
            run("§×", plain.clone()),
            run("§", CharStyle { h_scale: 150.0, ..plain.clone() }),
            run("§", CharStyle { baseline_shift: 3.0, ..plain.clone() }),
            run("§", CharStyle { rotation: 30.0, ..plain.clone() }),
            run("L", CharStyle { h_scale: 150.0, ..plain.clone() }),
            run("L", CharStyle { baseline_shift: 3.0, ..plain.clone() }),
            run("q\u{301}x", CharStyle { standard_vertical_roman: true, ..plain.clone() }),
            run("§", plain),
        ]
    };
    let compare = |db: &FontDb, point: &TextObject, name: &str| {
        let mut on_path = point.clone();
        on_path.kind =
            TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (0.0, 3000.0)).to_path(0.1)), start: 0.0, end: None };
        let (p, v) = (layout(db, point), layout(db, &on_path));
        assert_eq!(p.glyphs.len(), v.glyphs.len(), "{name}");
        let apart: Vec<_> = (p.glyphs.iter().zip(&v.glyphs).enumerate())
            .filter(|(_, (a, b))| a.xf.as_coeffs().iter().zip(b.xf.as_coeffs()).any(|(a, b)| (a - b).abs() > 1e-6))
            .map(|(i, (a, b))| format!("glyph {i}: {:?} on the path, {:?} in point type", b.xf.as_coeffs(), a.xf.as_coeffs()))
            .collect();
        assert!(apart.is_empty(), "{name}: {apart:#?}");
    };
    let mut point = vertical("");
    point.runs = runs(CharStyle { size: 20.0, ..CharStyle::default() });
    compare(FontDb::global(), &point, "the bundled face");
    // A face with vertical metrics, whose cells hang from their vertical origins.
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(vertical_font(true).unwrap());
    point.runs = runs(CharStyle { size: 20.0, font_family: VERTICAL_FAMILY.into(), ..CharStyle::default() });
    compare(&db, &point, "vertical metrics");
    // The horizontally scaled upright glyph is wider across the column, centered where the plain one is.
    point.kind = TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (0.0, 3000.0)).to_path(0.1)), start: 0.0, end: None };
    let l = layout(&db, &point);
    let (plain, wide) = (ink(&l, 0), ink(&l, 2));
    assert!(wide.width() > plain.width() * 1.4 && (wide.center().x - plain.center().x).abs() < 1e-6, "{plain:?} {wide:?}");
}

/// With Skew on a curved vertical path, every glyph's axis along the path follows the tangent and
/// its axis across the path stays upright on the page: an upright glyph's top runs back along the
/// tangent and its own x axis points up the page, and a letter lying on its side keeps its top up
/// the page and its baseline along the tangent.
#[test]
fn on_a_curved_vertical_path_skew_keeps_the_axis_across_the_path_upright() {
    use vectorcraft_doc::PathEffect;
    let mut arc = kurbo::BezPath::new();
    arc.move_to((0.0, 0.0));
    arc.curve_to((40.0, 0.0), (80.0, 40.0), (80.0, 120.0));
    let mut t = vertical("§L§L§");
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&arc), start: 0.0, end: None };
    t.path_effect = PathEffect::Skew;
    let l = layout(FontDb::global(), &t);
    let page_up = Vec2::new(0.0, -1.0);
    for (i, g) in l.glyphs.iter().enumerate() {
        let tangent = Vec2::new(g.angle.cos(), g.angle.sin());
        let (top, ahead) = top_and_ahead(&l, i);
        if i % 2 == 0 {
            assert!((top + tangent).hypot() < 1e-6 && (ahead - page_up).hypot() < 1e-6, "upright glyph {i}: {top:?} {ahead:?} at {tangent:?}");
        } else {
            assert!((top - page_up).hypot() < 1e-6 && (ahead - tangent).hypot() < 1e-6, "letter {i}: {top:?} {ahead:?} at {tangent:?}");
        }
    }
    let turns = l.glyphs.iter().map(|g| g.angle).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), a| (lo.min(a), hi.max(a)));
    assert!(turns.1 - turns.0 > 0.3, "the glyphs sit on the curve: {turns:?}");
}

/// A tate-chu-yoko block whose characters differ in size stays one block: every character stands on
/// the block's baseline in the em of its first character, in vertical type and on a vertical path.
#[test]
fn a_tate_chu_yoko_block_of_mixed_sizes_shares_its_first_characters_em() {
    let run = |text: &str, size: f64| TextRun { text: text.into(), style: CharStyle { size, ..CharStyle::default() }, inline: None };
    let mut t = vertical("");
    t.runs = vec![run("§", 20.0), run("1", 20.0), run("2", 10.0), run("§", 20.0)];
    let mut on_path = t.clone();
    on_path.kind =
        TextKind::OnPath { path: PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (0.0, 1000.0)).to_path(0.1)), start: 0.0, end: None };
    for (name, t) in [("vertical", &t), ("on a path", &on_path)] {
        let l = layout(FontDb::global(), t);
        assert!(l.glyphs[2].advance == 0.0, "{name}: 12 is one block: {:?}", l.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>());
        let (one, two) = (ink(&l, 1), ink(&l, 2));
        assert!((one.y1 - two.y1).abs() < 0.5 && two.center().x > one.center().x, "{name}: on one baseline: {one:?} {two:?}");
    }
}

/// On a vertical path, Tate-chu-yoko Settings Up/Down move a marked block toward the path's start
/// and Left/Right move it to the right of its upright characters (the path's left), with every
/// effect on a path running any way.
#[test]
fn on_a_vertical_path_tate_chu_yoko_settings_keep_their_directions() {
    use vectorcraft_doc::PathEffect;
    for effect in [PathEffect::Rainbow, PathEffect::Skew, PathEffect::Ribbon3d, PathEffect::StairStep, PathEffect::Gravity] {
        for to in [(1000.0, 0.0), (-1000.0, 0.0), (0.0, 1000.0), (-700.0, -700.0)] {
            let at = |up: f64, right: f64| {
                let marked = CharStyle { size: 20.0, tate_chu_yoko: true, tcy_up_down: up, tcy_left_right: right, ..CharStyle::default() };
                let mut t = on_vertical_path("", (0.0, 0.0), to, effect);
                t.runs = vec![
                    TextRun { text: "§".into(), style: CharStyle { size: 20.0, ..CharStyle::default() }, inline: None },
                    TextRun { text: "12".into(), style: marked, inline: None },
                ];
                ink(&layout(FontDb::global(), &t), 1).center().to_vec2()
            };
            let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
            let left = Vec2::new(tangent.y, -tangent.x);
            let (plain, up, right) = (at(0.0, 0.0), at(4.0, 0.0), at(0.0, 3.0));
            assert!((up - plain + tangent * 4.0).hypot() < 1e-6, "{effect:?} {to:?}: Up/Down moved it {:?}", up - plain);
            assert!((right - plain - left * 3.0).hypot() < 1e-6, "{effect:?} {to:?}: Left/Right moved it {:?}", right - plain);
        }
    }
}

/// On a vertical path, a tate-chu-yoko block stands upright like the glyphs around it: its digits
/// read across the column, their tops toward the path's start, centered on the column.
#[test]
fn on_a_vertical_path_tate_chu_yoko_stands_upright() {
    use vectorcraft_doc::PathEffect;
    for to in [(0.0, 1000.0), (0.0, -1000.0), (1000.0, 0.0)] {
        let l = layout(FontDb::global(), &on_vertical_path("§12§", (0.0, 0.0), to, PathEffect::Rainbow));
        assert!(l.glyphs[1].outline.elements().len() > 1, "{to:?}: the 1 draws");
        let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
        let left = Vec2::new(tangent.y, -tangent.x);
        let (upright, _) = top_and_ahead(&l, 0);
        for i in [1, 2] {
            let (top, ahead) = top_and_ahead(&l, i);
            assert!((top - upright).hypot() < 1e-6 && (ahead - left).hypot() < 1e-6, "{to:?} digit {i}: {top:?} {ahead:?}");
        }
        let centre = |i: usize| ink(&l, i).center().to_vec2();
        let (one, two) = (centre(1), centre(2));
        assert!((two - one).dot(left) > 3.0 && (two - one).dot(tangent).abs() < 1.0, "{to:?}: 12 side by side: {one:?} {two:?}");
        let across = |v: Vec2| v.dot(left);
        let block = (across(one) + across(two)) * 0.5;
        assert!((block - across(centre(0))).abs() < 2.0, "{to:?}: the block is centered on the column: {block} against {}", across(centre(0)));
        let (before, after) = ((centre(0)).dot(tangent), (centre(3)).dot(tangent));
        assert!(before < one.dot(tangent) && one.dot(tangent) < after, "{to:?}: the block stands between the marks");
    }
    // On a curve the block stays whole: both digits turn alike and stand on one baseline.
    let mut arc = kurbo::BezPath::new();
    arc.move_to((0.0, 0.0));
    arc.curve_to((0.0, 40.0), (20.0, 60.0), (60.0, 60.0));
    let mut t = vertical("§12§");
    t.kind = TextKind::OnPath { path: PathData::from_bezpath(&arc), start: 0.0, end: None };
    let l = layout(FontDb::global(), &t);
    let (a, b) = (l.glyphs[1].xf.as_coeffs(), l.glyphs[2].xf.as_coeffs());
    assert!(a.iter().zip(b).take(4).all(|(a, b)| (a - b).abs() < 1e-9), "{a:?} against {b:?}");
    let (top, ahead) = top_and_ahead(&l, 1);
    let (p1, p2) = (l.glyphs[1].xf * Point::ZERO, l.glyphs[2].xf * Point::ZERO);
    assert!((p2 - p1).dot(top).abs() < 1e-6 && (p2 - p1).dot(ahead) > 3.0, "{p1:?} {p2:?}");
    // With every effect, on a path running either way, the block's top faces the path's start.
    for effect in [PathEffect::Rainbow, PathEffect::Skew, PathEffect::Ribbon3d, PathEffect::StairStep, PathEffect::Gravity] {
        for to in [(1000.0, 0.0), (-1000.0, 0.0)] {
            let l = layout(FontDb::global(), &on_vertical_path("§12§", (0.0, 0.0), to, effect));
            let tangent = Vec2::new(to.0, to.1) / Vec2::new(to.0, to.1).hypot();
            for i in [1, 2] {
                let (top, _) = top_and_ahead(&l, i);
                assert!(top.dot(tangent) < -0.5, "{effect:?} {to:?} digit {i}: {top:?}");
            }
        }
    }
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

/// Vertical type keeps its tracking in thousandths of the em itself at any Horizontal Scale
/// (`spacing_scale`): its advances down the column don't all take the scale.
#[test]
fn vertical_tracking_keeps_the_em_at_any_horizontal_scale() {
    let step = |h_scale: f64, tracking: f64| {
        let l = layout(FontDb::global(), &vertical_styled("§§", CharStyle { size: 20.0, h_scale, tracking, ..CharStyle::default() }));
        ink(&l, 1).center().y - ink(&l, 0).center().y
    };
    for h_scale in [100.0, 50.0] {
        let more = step(h_scale, 200.0) - step(h_scale, 0.0);
        assert!((more - 4.0).abs() < 1e-6, "{h_scale}%: 200/1000 of the 20 pt em: {more}");
    }
}

/// Baseline shift moves an upright glyph across the column, as it moves a letter lying on its
/// side, at any Character Rotation, and leaves its neighbors where they are. The glyph turns about
/// its shifted pen position (`xf * pivot`), and `xf * unturned` places the shifted glyph without
/// its rotation.
#[test]
fn baseline_shift_moves_an_upright_glyph_across_the_column() {
    let at = |byte: usize, rotation: f64, shift: f64| {
        let mut t = vertical("§l§");
        edit::style_range(&mut t.runs, byte, byte + "§l§"[byte..].chars().next().map_or(1, char::len_utf8), |s| {
            s.rotation = rotation;
            s.baseline_shift = shift;
        });
        layout(FontDb::global(), &t)
    };
    let centre = |l: &TextLayout, byte: usize| l.glyphs.iter().find(|g| g.byte == byte).map(|g| g.outline.bounding_box().center()).unwrap();
    // The letter lying on its side moves across the column by the shift.
    let across = centre(&at(2, 0.0, 10.0), 2) - centre(&at(2, 0.0, 0.0), 2);
    assert!((across.x.abs() - 10.0).abs() < 1e-6 && across.y.abs() < 1e-6, "{across:?}");
    for rotation in [0.0, 30.0, 90.0, -90.0] {
        let (plain, shifted) = (at(3, rotation, 0.0), at(3, rotation, 10.0));
        let moved = centre(&shifted, 3) - centre(&plain, 3);
        assert!((moved - across).hypot() < 1e-6, "{rotation}°: {moved:?}, across {across:?}");
        for byte in [0, 2] {
            assert!((centre(&shifted, byte) - centre(&plain, byte)).hypot() < 1e-6, "{rotation}°: byte {byte} moved");
        }
        let (p, s) = (plain.glyphs.iter().find(|g| g.byte == 3).unwrap(), shifted.glyphs.iter().find(|g| g.byte == 3).unwrap());
        let turn_at = s.xf * s.pivot - p.xf * p.pivot;
        assert!((turn_at - across).hypot() < 1e-6, "{rotation}°: turns about a point moved by {turn_at:?}");
        let unrotated = at(3, 0.0, 10.0);
        let u = unrotated.glyphs.iter().find(|g| g.byte == 3).unwrap();
        let (a, b) = ((s.xf * s.unturned).as_coeffs(), u.xf.as_coeffs());
        assert!(a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-6), "{rotation}°: {a:?} vs {b:?}");
    }
}

/// Character panel menu › Tate-chu-yoko: a marked run stands across one em of the column, including
/// a number longer than three digits and Latin letters. Unmarked text keeps the automatic rule
/// (two or three digits only). Horizontal type stores the mark and does not move the letters.
#[test]
fn manual_tate_chu_yoko_sets_marked_characters_across_one_em() {
    let marked = CharStyle { size: 20.0, tate_chu_yoko: true, ..CharStyle::default() };
    let year = layout(FontDb::global(), &vertical_styled("2026", marked.clone()));
    assert_eq!(year.glyphs.len(), 4);
    assert!((year.glyphs[0].advance - 20.0).abs() < 1e-6, "{:?}", year.glyphs[0].advance);
    assert!(year.glyphs[1..].iter().all(|g| g.advance == 0.0));
    let inks: Vec<Rect> = (0..4).map(|i| ink(&year, i)).collect();
    for pair in inks.windows(2) {
        assert!((pair[0].center().y - pair[1].center().y).abs() < 1.0, "one row: {inks:?}");
        assert!(pair[1].center().x > pair[0].center().x, "left to right: {inks:?}");
    }
    let union = inks.iter().copied().reduce(|a, b| a.union(b)).unwrap();
    assert!(union.width() <= 21.0, "squeezed into the em: {union:?}");
    assert!(union.center().x.abs() < 1.5, "centred on the column: {union:?}");

    let plain = layout(FontDb::global(), &vertical("2026"));
    let down: f64 = plain.glyphs.iter().map(|g| g.advance).sum();
    assert!(down > 30.0 && plain.glyphs.iter().all(|g| g.advance > 0.0), "unmarked 2026 runs down the column: {down}");

    let letters = layout(FontDb::global(), &vertical_styled("AB", marked.clone()));
    assert!((letters.glyphs[0].advance - 20.0).abs() < 1e-6 && letters.glyphs[1].advance == 0.0);
    let (a, b) = (ink(&letters, 0), ink(&letters, 1));
    assert!((a.center().y - b.center().y).abs() < 1.0 && b.center().x > a.center().x, "letters side by side: {a:?} {b:?}");
    let side = layout(FontDb::global(), &vertical("AB"));
    assert!(ink(&side, 1).center().y > ink(&side, 0).center().y + 5.0, "unmarked letters run down the column");

    let mut horizontal = TextObject::point(Point::ZERO, "AB", marked);
    horizontal.xf = Affine::IDENTITY;
    let row = layout(FontDb::global(), &horizontal);
    assert!(!row.vertical);
    assert!(row.glyphs.iter().all(|g| g.advance > 0.0));
    assert!(ink(&row, 1).center().x > ink(&row, 0).center().x + 5.0, "horizontal type stays a row");
}

/// Character panel menu › Tate-chu-yoko Settings: a marked block moves up by a positive Up/Down
/// and right by a positive Left/Right, in points, as one piece (the offsets of its first
/// character), and keeps its one em down the column. An automatic two-digit block and horizontal
/// type do not move.
#[test]
fn tate_chu_yoko_settings_move_a_marked_block_up_down_left_and_right() {
    let marked = CharStyle { size: 20.0, tate_chu_yoko: true, ..CharStyle::default() };
    let union = |l: &TextLayout, r: std::ops::Range<usize>| r.map(|i| ink(l, i)).reduce(|a, b| a.union(b)).unwrap();
    let at = |up: f64, right: f64| {
        let mut t = vertical_styled("", marked.clone());
        t.runs = vec![
            TextRun::new("AB", CharStyle { tcy_up_down: up, tcy_left_right: right, ..marked.clone() }),
            TextRun::new("C", CharStyle { tcy_up_down: -up, tcy_left_right: -right, ..marked.clone() }),
            TextRun::new("§", CharStyle { size: 20.0, ..CharStyle::default() }),
        ];
        layout(FontDb::global(), &t)
    };
    let still = at(0.0, 0.0);
    for (up, right) in [(5.0, 0.0), (-3.5, 0.0), (0.0, 4.0), (0.0, -2.0), (6.0, -7.0)] {
        let moved = at(up, right);
        let (a, b) = (union(&still, 0..3), union(&moved, 0..3));
        assert!((b.y0 - (a.y0 - up)).abs() < 1e-6 && (b.y1 - (a.y1 - up)).abs() < 1e-6, "up {up}: {a:?} {b:?}");
        assert!((b.x0 - (a.x0 + right)).abs() < 1e-6 && (b.x1 - (a.x1 + right)).abs() < 1e-6, "right {right}: {a:?} {b:?}");
        assert!((moved.glyphs[0].advance - 20.0).abs() < 1e-6 && moved.glyphs[1..3].iter().all(|g| g.advance == 0.0), "one em");
        assert_eq!(ink(&moved, 3), ink(&still, 3), "the mark after the block stays");
        assert_eq!(moved.glyphs[0].origin, still.glyphs[0].origin, "the caret and hit region stay");
    }

    // An automatic block of two digits takes no offsets, and horizontal type does not move.
    let shifted = CharStyle { size: 20.0, tcy_up_down: 5.0, tcy_left_right: 5.0, ..CharStyle::default() };
    let digits = layout(FontDb::global(), &vertical_styled("10", shifted.clone()));
    let plain = layout(FontDb::global(), &vertical("10"));
    assert_eq!(union(&digits, 0..2), union(&plain, 0..2));
    let row = |st: CharStyle| {
        let mut t = TextObject::point(Point::ZERO, "AB", st);
        t.xf = Affine::IDENTITY;
        layout(FontDb::global(), &t)
    };
    assert_eq!(union(&row(CharStyle { tate_chu_yoko: true, ..shifted }), 0..2), union(&row(marked.clone()), 0..2));
}

/// Shrink Text to Fit scales a marked block's Up/Down and Left/Right offsets with the size, as it
/// scales the baseline shift.
#[test]
fn shrink_text_to_fit_scales_tate_chu_yoko_offsets() {
    let marked = CharStyle { size: 20.0, tate_chu_yoko: true, ..CharStyle::default() };
    let union = |l: &TextLayout, r: std::ops::Range<usize>| r.map(|i| ink(l, i)).reduce(|a, b| a.union(b)).unwrap();
    let (up, right) = (6.0, -4.0);
    let at = |up: f64, right: f64| {
        let mut t = vertical_styled("", marked.clone());
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 60.0, 60.0).to_path(0.1)) };
        t.area.fit = AreaFit::ShrinkText { min_percent: 10.0 };
        t.runs = vec![
            TextRun::new("AB", CharStyle { tcy_up_down: up, tcy_left_right: right, ..marked.clone() }),
            TextRun::new("§§§§§§§§§§§§", CharStyle { size: 20.0, ..CharStyle::default() }),
        ];
        layout(FontDb::global(), &t)
    };
    let (still, moved) = (at(0.0, 0.0), at(up, right));
    let f = moved.fit_scale;
    assert!(f < 1.0 && f == still.fit_scale, "{f} {}", still.fit_scale);
    let (a, b) = (union(&still, 0..2), union(&moved, 0..2));
    assert!((b.y0 - (a.y0 - up * f)).abs() < 1e-6 && (b.x0 - (a.x0 + right * f)).abs() < 1e-6, "{f}: {a:?} {b:?}");
}

/// A marked run is one block; the unmarked digits beside it still follow the automatic rule.
/// A column shorter than two ems holds the marked number on one line.
#[test]
fn manual_tate_chu_yoko_leaves_unmarked_digits_and_stays_on_one_line() {
    let plain = CharStyle { size: 20.0, ..CharStyle::default() };
    let marked = CharStyle { tate_chu_yoko: true, ..plain.clone() };
    let mut t = vertical_styled("", plain.clone());
    t.runs = vec![TextRun::new("X", plain), TextRun::new("2026", marked), TextRun::new("10", CharStyle { size: 20.0, ..CharStyle::default() })];
    let l = layout(FontDb::global(), &t);
    assert_eq!(l.glyphs.len(), 7);
    assert!(l.glyphs[0].advance > 0.0 && l.glyphs[0].advance < 20.0, "X on its side: {:?}", l.glyphs[0].advance);
    assert!((l.glyphs[1].advance - 20.0).abs() < 1e-6 && l.glyphs[2].advance == 0.0 && l.glyphs[3].advance == 0.0 && l.glyphs[4].advance == 0.0);
    assert!((l.glyphs[5].advance - 20.0).abs() < 1e-6 && l.glyphs[6].advance == 0.0, "unmarked 10 stays automatic");

    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 80.0, 30.0).to_path(0.1)) };
    t.runs = vec![TextRun::new("2026", CharStyle { size: 20.0, tate_chu_yoko: true, ..CharStyle::default() })];
    let one = layout(FontDb::global(), &t);
    assert_eq!(one.lines.len(), 1, "the block is one em, so it fits a 30 pt column");
    t.runs = vec![TextRun::new("2026", CharStyle { size: 20.0, ..CharStyle::default() })];
    let wrapped = layout(FontDb::global(), &t);
    assert!(wrapped.lines.len() > 1, "unmarked 2026 is longer than the column");
}

/// Character panel menu › Standard Vertical Roman Alignment: marked Latin letters and digits stand
/// upright one below the other, each in a cell of one em (the bundled font has no vertical
/// metrics), centred on the column. A space keeps its width, two digits stay tate-chu-yoko,
/// unmarked letters lie on their side, and horizontal type is unchanged.
#[test]
fn standard_vertical_roman_alignment_stands_half_width_characters_upright() {
    let plain = CharStyle { size: 20.0, ..CharStyle::default() };
    let marked = CharStyle { standard_vertical_roman: true, ..plain.clone() };
    let h = layout(FontDb::global(), &TextObject::point(Point::ZERO, "l", plain.clone()));
    let side = layout(FontDb::global(), &vertical_styled("lA l", plain.clone()));
    let up = layout(FontDb::global(), &vertical_styled("lA l", marked.clone()));
    let (hl, sl, ul) = (ink(&h, 0), ink(&side, 0), ink(&up, 0));
    assert!(sl.width() > sl.height(), "unmarked l lies across the column: {sl:?}");
    assert!((ul.width() - hl.width()).abs() < 0.01 && (ul.height() - hl.height()).abs() < 0.01, "marked l upright: {hl:?} {ul:?}");
    assert!(ul.height() > ul.width());
    for i in [0, 1, 3] {
        assert!((up.glyphs[i].advance - 20.0).abs() < 1e-6, "one em down the column: {:?}", up.glyphs[i].advance);
        assert!(ink(&up, i).center().x.abs() < 1.5, "centred on the column: {:?}", ink(&up, i));
        assert!(side.glyphs[i].advance < 20.0, "unmarked advance: {:?}", side.glyphs[i].advance);
    }
    assert!(ink(&up, 1).center().y > ink(&up, 0).center().y + 15.0, "one below the other");
    assert!((up.glyphs[2].advance - side.glyphs[2].advance).abs() < 1e-6, "the space keeps its width");

    let digits = layout(FontDb::global(), &vertical_styled("10", marked.clone()));
    assert!((digits.glyphs[0].advance - 20.0).abs() < 1e-6 && digits.glyphs[1].advance == 0.0, "two digits stay tate-chu-yoko");
    let (a, b) = (ink(&digits, 0), ink(&digits, 1));
    assert!((a.center().y - b.center().y).abs() < 1.0 && b.center().x > a.center().x, "side by side: {a:?} {b:?}");

    let mut horizontal = TextObject::point(Point::ZERO, "lA", marked);
    horizontal.xf = Affine::IDENTITY;
    let row = layout(FontDb::global(), &horizontal);
    let plain_row = layout(FontDb::global(), &TextObject::point(Point::ZERO, "lA", plain));
    assert_eq!(row.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>(), plain_row.glyphs.iter().map(|g| g.advance).collect::<Vec<_>>());
    assert!((ink(&row, 0).height() - hl.height()).abs() < 0.01, "horizontal type stays as it is");
}

/// A combining mark shaped as a glyph of its own turns upright with its letter and stays over it,
/// as in horizontal type, and takes no cell of its own down the column.
#[test]
fn standard_vertical_roman_alignment_keeps_combining_marks_on_their_letter() {
    let plain = CharStyle { size: 20.0, ..CharStyle::default() };
    let text = "q\u{301}x";
    let h = layout(FontDb::global(), &TextObject::point(Point::ZERO, text, plain.clone()));
    assert_eq!(h.glyphs.len(), 3, "the bundled font sets the acute as a glyph of its own");
    let up = layout(FontDb::global(), &vertical_styled(text, CharStyle { standard_vertical_roman: true, ..plain }));
    assert_eq!(up.glyphs.len(), 3);
    let (hq, ha, q, a, x) = (ink(&h, 0), ink(&h, 1), ink(&up, 0), ink(&up, 1), ink(&up, 2));
    let offset = |base: Rect, mark: Rect| (mark.center().x - base.center().x, mark.center().y - base.center().y);
    let (want, got) = (offset(hq, ha), offset(q, a));
    assert!((want.0 - got.0).abs() < 0.01 && (want.1 - got.1).abs() < 0.01, "the acute sits on the q as in a row: {want:?} {got:?}");
    assert!((up.glyphs[0].advance - 20.0).abs() < 1e-6 && up.glyphs[1].advance == 0.0, "one cell for the cluster");
    assert!(x.center().y > q.center().y + 10.0, "the next letter below: {q:?} {x:?}");
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

/// Mojikumi applies to type holding Japanese punctuation or a Japanese letter; type without either
/// lays out the same with Line-end Punctuation Half Width as with Mojikumi None, across lines and
/// list markers too.
#[test]
fn mojikumi_applies_only_to_japanese_punctuation_and_letters() {
    use vectorcraft_doc::Mojikumi;
    use vectorcraft_doc::text::{Bullet, ParaList};
    let styled = |text: &str, m: Mojikumi| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.para.mojikumi = m;
        t
    };
    let placed = |l: &TextLayout| l.glyphs.iter().map(|g| (g.byte, g.advance, g.origin, g.xf)).collect::<Vec<_>>();
    for text in ["", "A", "Latin words, digits 2026 and (brackets).", "first\nsecond"] {
        let on = styled(text, Mojikumi::LineEndHalf);
        let lay = layout(FontDb::global(), &on);
        assert!(!lay.mojikumi_applies(&on.plain_text()), "{text:?}");
        let off = layout(FontDb::global(), &styled(text, Mojikumi::None));
        assert_eq!(placed(&lay), placed(&off), "{text:?}");
        assert_eq!(lay.bounds, off.bounds, "{text:?}");
    }
    let mut listed = styled("first\nsecond", Mojikumi::LineEndHalf);
    listed.para.list = Some(ParaList::bulleted(Bullet::Dot));
    assert!(!layout(FontDb::global(), &listed).mojikumi_applies(&listed.plain_text()));
    for text in ["、", "「A」", "雅", "々", "Latin 漢字"] {
        let t = styled(text, Mojikumi::LineEndHalf);
        assert!(layout(FontDb::global(), &t).mojikumi_applies(&t.plain_text()), "{text:?}");
    }
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

/// Mojikumi: with Line-end Punctuation Half Width, a line is measured as it is set. A closing
/// bracket, comma or full stop that fits only at half width ends the line in a measure of three
/// and a half ems, with both composers, horizontal and vertical; with Mojikumi None it goes to the
/// next line with the character before it. A closing mark that already lost its half to the
/// punctuation after it takes nothing more off, and the space between Japanese and Latin text
/// counts only where the line ends. Uses a test font whose ideographs and punctuation are one em.
#[test]
fn mojikumi_line_end_half_width_counts_when_a_line_breaks() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::{Burasagari, Composer, Kinsoku, KinsokuShoriType, Mojikumi};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let lay = |text: &str, m: Mojikumi, composer: Composer, burasagari: Burasagari, vertical_type: bool, measure: f64| {
        let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = m;
        t.para.composer = composer;
        t.para.burasagari = burasagari;
        t.para.kinsoku = Kinsoku::Hard;
        // Push Out Only lets the every-line composer take the paragraph.
        if composer == Composer::EveryLine {
            t.para.kinsoku_type = KinsokuShoriType::PushOutOnly;
        }
        let frame = if vertical_type { Rect::new(0.0, 0.0, 200.0, measure) } else { Rect::new(0.0, 0.0, measure, 200.0) };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        layout(&db, &t)
    };
    let ends = |l: &TextLayout, text: &str| l.lines.iter().map(|line| text[line.start..line.end].to_string()).collect::<Vec<_>>();
    let widest = |l: &TextLayout| {
        l.lines.iter().map(|line| l.glyphs[line.glyph_start..line.glyph_end].iter().map(|g| g.advance).sum::<f64>()).fold(0.0, f64::max)
    };
    assert!((lay("一", Mojikumi::None, Composer::SingleLine, Burasagari::None, false, 70.0).glyphs[0].advance - 20.0).abs() < 1e-9, "one em");
    for vertical_type in [false, true] {
        for composer in [Composer::SingleLine, Composer::EveryLine] {
            let at = format!("vertical {vertical_type} {composer:?}");
            let marks = [("一二三」四五六", "一二三」"), ("一二三。四五六", "一二三。"), ("一二三、四五六", "一二三、")];
            for (text, first) in marks {
                let l = lay(text, Mojikumi::LineEndHalf, composer, Burasagari::None, vertical_type, 70.0);
                assert_eq!(ends(&l, text), [first, "四五六"], "{at}: {text}");
                assert!((l.glyphs[3].advance - 10.0).abs() < 1e-9, "{at}: {text}: the mark ends the line half width");
                assert!(widest(&l) <= 70.0 + 1e-9, "{at}: {text}: {}", widest(&l));
                // Standard burasagari: the mark fits inside, so it doesn't hang.
                let standard = lay(text, Mojikumi::LineEndHalf, composer, Burasagari::Standard, vertical_type, 70.0);
                assert_eq!(ends(&standard, text), [first, "四五六"], "{at}: {text} standard");
                assert!(widest(&standard) <= 70.0 + 1e-9, "{at}: {text} standard");
                // Full width it doesn't fit: kinsoku takes 三 to the next line with it.
                let solid = lay(text, Mojikumi::None, composer, Burasagari::None, vertical_type, 70.0);
                assert_eq!(ends(&solid, text).first().map(String::as_str), Some("一二"), "{at}: {text} none");
            }
            // 」 lost its half to 。 after it: the line would be four ems, so 三」。 go on.
            let text = "一二三」。四五";
            let l = lay(text, Mojikumi::LineEndHalf, composer, Burasagari::None, vertical_type, 70.0);
            assert_eq!(ends(&l, text).first().map(String::as_str), Some("一二"), "{at}: compressed");
            assert!(widest(&l) <= 70.0 + 1e-9, "{at}: compressed");
        }
    }
    // The quarter em between 三 and A goes where the line ends there: 一二三 fits three ems and a
    // tenth, with or without the space at its end.
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        let text = "一二三ABC";
        let on = lay(text, Mojikumi::LineEndHalf, composer, Burasagari::None, false, 62.0);
        assert_eq!(ends(&on, text).first().map(String::as_str), Some("一二三"), "{composer:?}");
        assert!((on.glyphs[2].advance - 20.0).abs() < 1e-9, "{composer:?}: no space at the line's end");
        let wide = lay(text, Mojikumi::LineEndHalf, composer, Burasagari::None, false, 200.0);
        assert!((wide.glyphs[2].advance - 25.0).abs() < 1e-9, "{composer:?}: the space inside the line");
    }
}

/// The predefined Mojikumi sets space punctuation by their own rules, horizontal and vertical:
/// YakumonoHankaku sets every bracket, comma and full stop half width and a middle dot a half em
/// (a quarter off each side, drawn a quarter em earlier); GyomatsuYakumonoHankaku keeps them full
/// width inside the line, sets an opening bracket starting the line flush and a closing mark ending
/// it half width, and takes the quarter em after a middle dot ending the line off;
/// GyomatsuYakumonoZenkaku and YakumonoZenkaku keep every mark full width. All but YakumonoZenkaku
/// put a quarter em between Japanese and Latin letters. Uses a test font whose ideographs and
/// punctuation are one em.
#[test]
fn mojikumi_predefined_sets_space_punctuation_by_their_rules() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::Mojikumi;
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let lay = |text: &str, m: Mojikumi, vertical_type: bool| {
        let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = m;
        layout(&db, &t)
    };
    let round = |v: Vec<f64>| v.into_iter().map(|x| (x * 100.0).round() / 100.0).collect::<Vec<_>>();
    for vertical_type in [false, true] {
        let adv = |text: &str, m| round(lay(text, m, vertical_type).glyphs.iter().map(|g| g.advance).collect());
        let at = |m: Mojikumi| format!("vertical {vertical_type} {m:?}");
        let middle = "一「二」三・四、五。";
        let cases = [
            (Mojikumi::None, [20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0]),
            (Mojikumi::YakumonoHankaku, [20.0, 10.0, 20.0, 10.0, 20.0, 10.0, 20.0, 10.0, 20.0, 10.0]),
            (Mojikumi::GyomatsuYakumonoHankaku, [20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 20.0, 10.0]),
            (Mojikumi::GyomatsuYakumonoZenkaku, [20.0; 10]),
            (Mojikumi::YakumonoZenkaku, [20.0; 10]),
        ];
        for (m, want) in cases {
            assert_eq!(adv(middle, m), want, "{}", at(m));
        }
        // Consecutive punctuation: YakumonoHankaku halves both marks, GyomatsuYakumonoHankaku
        // neither (Line-end Punctuation Half Width only the first).
        assert_eq!(adv("一」「二", Mojikumi::YakumonoHankaku), [20.0, 10.0, 10.0, 20.0], "vertical {vertical_type}");
        assert_eq!(adv("一」「二", Mojikumi::GyomatsuYakumonoHankaku), [20.0; 4], "vertical {vertical_type}");
        assert_eq!(adv("一」「二", Mojikumi::LineEndHalf), [20.0, 10.0, 20.0, 20.0], "vertical {vertical_type}");
        // A line's start and end: the opening bracket flush and the middle dot's outer quarter off.
        assert_eq!(adv("「一・", Mojikumi::GyomatsuYakumonoHankaku), [10.0, 20.0, 15.0], "vertical {vertical_type}");
        assert_eq!(adv("「一・", Mojikumi::YakumonoHankaku), [10.0, 20.0, 10.0], "vertical {vertical_type}");
        assert_eq!(adv("「一・", Mojikumi::GyomatsuYakumonoZenkaku), [20.0; 3], "vertical {vertical_type}");
        // YakumonoHankaku draws an opening bracket half an em and a middle dot a quarter em
        // earlier than its pen.
        let along = |r: Rect| if vertical_type { r.y0 } else { r.x0 };
        let (on, off) = (lay("一「二", Mojikumi::YakumonoHankaku, vertical_type), lay("一「二", Mojikumi::None, vertical_type));
        let moved = along(off.glyphs[1].outline.bounding_box()) - along(on.glyphs[1].outline.bounding_box());
        assert!((moved - 10.0).abs() < 0.01, "vertical {vertical_type}: 「 drawn half an em earlier ({moved})");
        let (on, off) = (lay("一・二", Mojikumi::YakumonoHankaku, vertical_type), lay("一・二", Mojikumi::None, vertical_type));
        let moved = along(off.glyphs[1].outline.bounding_box()) - along(on.glyphs[1].outline.bounding_box());
        assert!((moved - 5.0).abs() < 0.01, "vertical {vertical_type}: ・ drawn a quarter em earlier ({moved})");
        // The space between Japanese and Latin letters.
        let none = adv("一A二", Mojikumi::None);
        for (m, extra) in [
            (Mojikumi::YakumonoHankaku, 5.0),
            (Mojikumi::GyomatsuYakumonoHankaku, 5.0),
            (Mojikumi::GyomatsuYakumonoZenkaku, 5.0),
            (Mojikumi::YakumonoZenkaku, 0.0),
        ] {
            let got = adv("一A二", m);
            assert!((got[0] - none[0] - extra).abs() < 0.01 && (got[1] - none[1] - extra).abs() < 0.01, "{}: {got:?} {none:?}", at(m));
        }
    }
}

/// The predefined Mojikumi sets measure a line as they set it, with both composers, horizontal
/// and vertical: in a measure of three and a half ems, a closing bracket that fits only half width
/// ends the line with YakumonoHankaku and GyomatsuYakumonoHankaku, and goes to the next line with
/// the character before it (kinsoku) with GyomatsuYakumonoZenkaku and YakumonoZenkaku. A middle dot
/// that fits only without the quarter em after it ends the line with GyomatsuYakumonoHankaku.
#[test]
fn mojikumi_predefined_sets_count_when_a_line_breaks() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::{Burasagari, Composer, Kinsoku, KinsokuShoriType, Mojikumi};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let lay = |text: &str, m: Mojikumi, composer: Composer, vertical_type: bool, measure: f64| {
        let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.mojikumi = m;
        t.para.composer = composer;
        t.para.burasagari = Burasagari::None;
        t.para.kinsoku = Kinsoku::Hard;
        t.para.kinsoku_type = KinsokuShoriType::PushOutOnly;
        let frame = if vertical_type { Rect::new(0.0, 0.0, 200.0, measure) } else { Rect::new(0.0, 0.0, measure, 200.0) };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        layout(&db, &t)
    };
    let first = |l: &TextLayout, text: &str| l.lines.first().map(|line| text[line.start..line.end].to_string()).unwrap_or_default();
    let widest = |l: &TextLayout| {
        l.lines.iter().map(|line| l.glyphs[line.glyph_start..line.glyph_end].iter().map(|g| g.advance).sum::<f64>()).fold(0.0, f64::max)
    };
    for vertical_type in [false, true] {
        for composer in [Composer::SingleLine, Composer::EveryLine] {
            let text = "一二三」四五六";
            for (m, line) in [
                (Mojikumi::YakumonoHankaku, "一二三」"),
                (Mojikumi::GyomatsuYakumonoHankaku, "一二三」"),
                (Mojikumi::GyomatsuYakumonoZenkaku, "一二"),
                (Mojikumi::YakumonoZenkaku, "一二"),
            ] {
                let l = lay(text, m, composer, vertical_type, 70.0);
                assert_eq!(first(&l, text), line, "vertical {vertical_type} {composer:?} {m:?}");
                assert!(widest(&l) <= 70.0 + 1e-9, "vertical {vertical_type} {composer:?} {m:?}: {}", widest(&l));
            }
            let text = "一二三・四五六";
            for (m, line) in [(Mojikumi::GyomatsuYakumonoHankaku, "一二三・"), (Mojikumi::GyomatsuYakumonoZenkaku, "一二"), (Mojikumi::None, "一二")]
            {
                let l = lay(text, m, composer, vertical_type, 76.0);
                assert_eq!(first(&l, text), line, "vertical {vertical_type} {composer:?} {m:?}");
                assert!(widest(&l) <= 76.0 + 1e-9, "vertical {vertical_type} {composer:?} {m:?}: {}", widest(&l));
            }
        }
    }
}

/// A right-to-left paragraph measures its lines at full width with every Mojikumi set, and draws
/// them as it measures them: a closing bracket ending a line keeps its full em, with both composers.
#[test]
fn mojikumi_right_to_left_lines_are_drawn_as_measured() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::{Burasagari, Composer, Kinsoku, KinsokuShoriType, Mojikumi, ParaDirection};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let lay = |text: &str, m: Mojikumi, composer: Composer, measure: f64| {
        let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        t.xf = Affine::IDENTITY;
        t.para.direction = Some(ParaDirection::RightToLeft);
        t.para.mojikumi = m;
        t.para.composer = composer;
        t.para.burasagari = Burasagari::None;
        t.para.kinsoku = Kinsoku::Hard;
        t.para.kinsoku_type = KinsokuShoriType::PushOutOnly;
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, measure, 200.0).to_path(0.1)) };
        layout(&db, &t)
    };
    let text = "一二三」四五六";
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        for m in [Mojikumi::LineEndHalf, Mojikumi::GyomatsuYakumonoHankaku] {
            let at = format!("{composer:?} {m:?}");
            // Three and a half ems: the bracket does not fit at full width, so kinsoku takes 三 on.
            let narrow = lay(text, m, composer, 70.0);
            let first = narrow.lines.first().map(|line| &text[line.start..line.end]);
            assert_eq!(first, Some("一二"), "{at}");
            // Four and a quarter ems: the line is measured four ems wide and drawn so.
            let l = lay(text, m, composer, 85.0);
            let first = l.lines.first().map(|line| &text[line.start..line.end]);
            assert_eq!(first, Some("一二三」"), "{at}");
            let bracket = text.find('」').and_then(|byte| l.glyphs.iter().find(|g| g.byte == byte)).map(|g| g.advance);
            assert_eq!(bracket.map(|a| (a * 100.0).round() / 100.0), Some(20.0), "{at}");
        }
    }
}

/// A right-to-left line ending between a Japanese and a Latin character does not keep the quarter
/// em the set adds there: in three ems, 一二三 fits before Latin text, measured and drawn three ems
/// wide, with both composers and every set that adds the space.
#[test]
fn mojikumi_right_to_left_lines_end_without_the_japanese_latin_space() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::{Composer, Mojikumi, ParaDirection};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let text = "一二三AB";
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        for m in [Mojikumi::LineEndHalf, Mojikumi::YakumonoHankaku, Mojikumi::GyomatsuYakumonoHankaku, Mojikumi::GyomatsuYakumonoZenkaku] {
            let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
            let mut t = TextObject::point(Point::ZERO, text, style);
            t.xf = Affine::IDENTITY;
            t.para.direction = Some(ParaDirection::RightToLeft);
            t.para.mojikumi = m;
            t.para.composer = composer;
            t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 60.0, 200.0).to_path(0.1)) };
            let l = layout(&db, &t);
            let line = l.lines.first();
            assert_eq!(line.map(|line| &text[line.start..line.end]), Some("一二三"), "{composer:?} {m:?}");
            let drawn = line.map(|line| l.glyphs[line.glyph_start..line.glyph_end].iter().map(|g| g.advance).sum::<f64>());
            assert_eq!(drawn.map(|w| (w * 100.0).round() / 100.0), Some(60.0), "{composer:?} {m:?}");
        }
    }
}

/// With a Desired glyph scaling other than 100%, the space a Mojikumi set takes off before an
/// opening bracket or a middle dot scales with the glyph: the glyph is drawn that much earlier,
/// with YakumonoHankaku and with Line-end Punctuation Half Width's consecutive brackets.
#[test]
fn mojikumi_space_taken_off_scales_with_desired_glyph_scaling() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::Mojikumi;
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let lay = |text: &str, m: Mojikumi| {
        let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        t.xf = Affine::IDENTITY;
        t.para.mojikumi = m;
        t.para.justification.glyph_scaling = [50.0, 50.0, 100.0];
        layout(&db, &t)
    };
    for (text, m, k, want) in
        [("一「二", Mojikumi::YakumonoHankaku, 1, 5.0), ("一・二", Mojikumi::YakumonoHankaku, 1, 2.5), ("一「「二", Mojikumi::LineEndHalf, 2, 5.0)]
    {
        let (on, off) = (lay(text, m), lay(text, Mojikumi::None));
        let pen = |l: &TextLayout| l.glyphs[..k].iter().map(|g| g.advance).sum::<f64>();
        let drawn = |l: &TextLayout| l.glyphs[k].outline.bounding_box().x0 - pen(l);
        let moved = drawn(&off) - drawn(&on);
        assert!((moved - want).abs() < 0.01, "{text} {m:?}: drawn {moved} earlier, want {want}");
    }
}

/// With a Desired glyph scaling other than 100%, a line ending before Latin text is measured
/// without the scaled quarter em the set added there, not an unscaled one: at 50%, 一二三 is 30 pt
/// wide and does not fit 29 pt, with both composers and every set that adds the space.
#[test]
fn mojikumi_line_end_japanese_latin_space_scales_with_desired_glyph_scaling() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::{Composer, Mojikumi};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    let text = "一二三AB";
    for composer in [Composer::SingleLine, Composer::EveryLine] {
        for m in [Mojikumi::LineEndHalf, Mojikumi::YakumonoHankaku, Mojikumi::GyomatsuYakumonoHankaku, Mojikumi::GyomatsuYakumonoZenkaku] {
            let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
            let mut t = TextObject::point(Point::ZERO, text, style);
            t.xf = Affine::IDENTITY;
            t.para.mojikumi = m;
            t.para.composer = composer;
            t.para.justification.glyph_scaling = [50.0, 50.0, 100.0];
            t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 29.0, 200.0).to_path(0.1)) };
            let l = layout(&db, &t);
            let first = l.lines.first().map(|line| &text[line.start..line.end]);
            assert_eq!(first, Some("一二"), "{composer:?} {m:?}");
        }
    }
}

/// Mojikumi: in a frame that is narrow at the top, and in a frame a wrap object narrows at the top,
/// a line's first word is measured as the line sets it. A word ending with a closing bracket that
/// fits the narrow lines only with the bracket half width, and an opening bracket that fits them
/// only flush with the line's start, start on the first line, with both composers. An opening
/// bracket that kinsoku keeps with the character after it, and a No Break run, are measured whole.
#[test]
fn mojikumi_narrows_a_lines_first_word_when_looking_for_room() {
    use crate::test_fonts::{FULL_WIDTH_FAMILY, full_width_font};
    use vectorcraft_doc::{Composer, Kinsoku, KinsokuShoriType, Mojikumi, TextWrap, WrapShape};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(full_width_font().expect("test font"));
    // The top 40 pt of the frame are `top` wide, the rest 200: an L-shaped frame, or a rectangle
    // with a wrap object over the top right.
    let lay = |text: &str, styled: fn(&mut TextObject), composer: Composer, top: f64, wrapped: bool| {
        let style = CharStyle { size: 20.0, font_family: FULL_WIDTH_FAMILY.into(), font_style: "Regular".into(), ..CharStyle::default() };
        let mut t = TextObject::point(Point::ZERO, text, style);
        t.xf = Affine::IDENTITY;
        styled(&mut t);
        t.para.mojikumi = Mojikumi::LineEndHalf;
        t.para.composer = composer;
        t.para.kinsoku = Kinsoku::Hard;
        t.para.kinsoku_type = KinsokuShoriType::PushOutOnly;
        let frame = if wrapped {
            t.wrap = vec![WrapShape {
                path: PathData::from_bezpath(&Rect::new(top, -10.0, 210.0, 40.0).to_path(0.1)),
                wrap: TextWrap { offset: 0.0, invert: false },
            }];
            Rect::new(0.0, 0.0, 200.0, 200.0).to_path(0.1)
        } else {
            let mut p = kurbo::BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((top, 0.0));
            p.line_to((top, 40.0));
            p.line_to((200.0, 40.0));
            p.line_to((200.0, 200.0));
            p.line_to((0.0, 200.0));
            p.close_path();
            p
        };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame) };
        layout(&db, &t)
    };
    let plain: fn(&mut TextObject) = |_| {};
    let wide_bracket: fn(&mut TextObject) = |t| edit::style_range(&mut t.runs, 0, '「'.len_utf8(), |s| s.h_scale = 250.0);
    let no_break: fn(&mut TextObject) = |t| {
        let n = t.plain_text().len();
        edit::style_range(&mut t.runs, 0, n, |s| s.no_break = true);
    };
    let no_break_but_the_last_word: fn(&mut TextObject) = |t| {
        let n = t.plain_text().rfind(' ').map_or(0, |k| k + 1);
        edit::style_range(&mut t.runs, 0, n, |s| s.no_break = true);
    };
    // "ABCD」" is `full` wide inside a line; ending one, 」 is half width.
    let closing = "ABCD」EFGH";
    let open = lay(closing, plain, Composer::SingleLine, 200.0, false);
    let full: f64 = open.glyphs[..5].iter().map(|g| g.advance).sum();
    assert!((open.glyphs[4].advance - 20.0).abs() < 1e-9, "」 is one em inside a line");
    for wrapped in [false, true] {
        for composer in [Composer::SingleLine, Composer::EveryLine] {
            let at = format!("wrapped {wrapped} {composer:?}");
            let top = full - 5.0;
            let l = lay(closing, plain, composer, top, wrapped);
            let first = l.lines.first().expect("a line");
            assert_eq!(&closing[first.start..first.end], "ABCD」", "{at}");
            assert!(first.baseline < 40.0, "{at}: the word starts on the narrow lines: {}", first.baseline);
            let set: f64 = l.glyphs[first.glyph_start..first.glyph_end].iter().map(|g| g.advance).sum();
            assert!(set <= top + 1e-9, "{at}: {set} in {top}");
            // 「 is 50 pt at 250%, 25 pt flush with the line's start: 「一 is 45 pt.
            let opening = "「一二三四五六";
            let l = lay(opening, wide_bracket, composer, 47.0, wrapped);
            let first = l.lines.first().expect("a line");
            assert_eq!(&opening[first.start..first.end], "「一", "{at}");
            assert!(first.baseline < 40.0, "{at}: 「 starts on the narrow lines: {}", first.baseline);
            let set: f64 = l.glyphs[first.glyph_start..first.glyph_end].iter().map(|g| g.advance).sum();
            assert!(set <= 47.0 + 1e-9, "{at}: {set}");
            // Kinsoku keeps 「 with 一: in 40 pt they don't fit, and the line starts below, whole.
            let l = lay(opening, wide_bracket, composer, 40.0, wrapped);
            let first = l.lines.first().expect("a line");
            assert!(first.baseline > 40.0 && &opening[first.start..first.end] != "「", "{at}: kinsoku: {:?}", placed_lines(&l, opening));
            // A No Break run is one word, its space included: it doesn't fit the narrow lines.
            let run_text = "ABCD EFGH";
            let run: f64 = lay(run_text, no_break, composer, 200.0, false).glyphs.iter().map(|g| g.advance).sum();
            let l = lay(run_text, no_break, composer, run - 5.0, wrapped);
            let first = l.lines.first().expect("a line");
            assert!(first.baseline > 40.0 && &run_text[first.start..first.end] == run_text, "{at}: no break: {:?}", placed_lines(&l, run_text));
            // …but the space ending a No Break run is not set at the line's end: the run fits a
            // line 2 pt wider than it is without that space.
            let ending = "ABCD EFGH IJ";
            let l = lay(ending, no_break_but_the_last_word, composer, run + 2.0, wrapped);
            let first = l.lines.first().expect("a line");
            assert!(first.baseline < 40.0 && ending[first.start..first.end].trim_end() == run_text, "{at}: {:?}", placed_lines(&l, ending));
        }
    }
}

/// Each line's baseline and text.
fn placed_lines(l: &TextLayout, text: &str) -> Vec<(f64, String)> {
    l.lines.iter().map(|line| (line.baseline, text[line.start..line.end].to_string())).collect()
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

/// Kinsoku Shori Type: in a measure of fifteen ems, the closing bracket after the fifteenth
/// character ends the first line with Push In First, which closes the line's gaps to keep it inside
/// the frame whatever its alignment; Push Out Only moves the fifteenth character to the next line
/// with the bracket. Horizontal and vertical. Needs a font with full-width Japanese (skipped
/// without one).
#[test]
fn kinsoku_shori_type_pushes_a_closing_bracket_in_or_out() {
    use vectorcraft_doc::KinsokuShoriType;
    let text = "一二三四五六七八九十一二三四五」六";
    let lay = |ty: KinsokuShoriType, justify: Justify, vertical_type: bool| {
        let mut t = TextObject::point(Point::ZERO, text, CharStyle { size: 20.0, ..CharStyle::default() });
        t.xf = Affine::IDENTITY;
        t.vertical = vertical_type;
        t.para.kinsoku_type = ty;
        t.para.justify = justify;
        let frame = if vertical_type { Rect::new(0.0, 0.0, 200.0, 300.0) } else { Rect::new(0.0, 0.0, 300.0, 200.0) };
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&frame.to_path(0.1)) };
        layout(FontDb::global(), &t)
    };
    if (lay(KinsokuShoriType::PushOutOnly, Justify::Left, false).glyphs[0].advance - 20.0).abs() > 2.0 {
        return; // no font with full-width Japanese here
    }
    for vertical_type in [false, true] {
        let lines = |ty| lay(ty, Justify::Left, vertical_type).glyphs.iter().map(|g| g.line).collect::<Vec<_>>();
        let mut pushed_in = vec![0; 16];
        pushed_in.push(1);
        assert_eq!(lines(KinsokuShoriType::PushInFirst), pushed_in, "vertical {vertical_type}");
        let mut pushed_out = vec![0; 14];
        pushed_out.extend([1, 1, 1]);
        assert_eq!(lines(KinsokuShoriType::PushOutOnly), pushed_out, "vertical {vertical_type}");
        assert_eq!(lines(KinsokuShoriType::PushOutFirst), pushed_out, "vertical {vertical_type}: a push-out leaves one em");
    }
    for justify in [Justify::Left, Justify::Center, Justify::Right, Justify::JustifyLeft] {
        let l = lay(KinsokuShoriType::PushInFirst, justify, false);
        let (first, bracket) = (&l.glyphs[0], &l.glyphs[15]);
        assert!(first.origin.x.abs() < 0.01, "{justify:?}: the line starts at the frame's left: {:?}", first.origin);
        assert!(bracket.origin.x + bracket.advance <= 300.01, "{justify:?}: 」 ends inside the frame: {:?}", bracket.origin);
        assert!(l.glyphs[1].origin.x < 20.0 - 0.5, "{justify:?}: the gaps close up");
        for g in &l.glyphs[..15] {
            assert!(g.advance >= 18.0 - 0.01, "{justify:?}: a gap closes by at most a tenth of an em: {}", g.advance);
        }
    }
    // A No Break run wider than the frame did not push anything in, and keeps its gaps.
    let mut t = TextObject::point(Point::ZERO, "一二三四五六七八九十", CharStyle { size: 20.0, no_break: true, ..CharStyle::default() });
    t.xf = Affine::IDENTITY;
    t.para.kinsoku_type = KinsokuShoriType::PushInFirst;
    t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 150.0, 200.0).to_path(0.1)) };
    let l = layout(FontDb::global(), &t);
    assert!((l.glyphs[1].origin.x - 20.0).abs() < 0.01, "{:?}", l.glyphs[1].origin);
}

/// Kinsoku Shori Type at 200% Vertical Scale in a column of 225 pt: the font's vertical metrics set
/// each character 40 pt long, and its five gaps close by up to 4 pt each, room for the 15 pt 」
/// overflows by, so 」 is pushed in; Push Out Only moves 五 to the next column with it. Needs
/// craft-fonts' Shippori Mincho (skipped without it).
#[test]
fn kinsoku_shori_type_closes_a_column_by_the_em_down_the_column() {
    use vectorcraft_doc::KinsokuShoriType;
    if FontDb::global().face("Shippori Mincho", "Regular").is_none_or(|f| f.family != "Shippori Mincho") {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    }
    let lay = |ty: KinsokuShoriType| {
        let mut t = vertical_styled(
            "一二三四五」六",
            CharStyle { size: 20.0, font_family: "Shippori Mincho".into(), v_scale: 200.0, ..CharStyle::default() },
        );
        t.para.kinsoku_type = ty;
        t.kind = TextKind::Area { frame: PathData::from_bezpath(&Rect::new(0.0, 0.0, 200.0, 225.0).to_path(0.1)) };
        layout(FontDb::global(), &t)
    };
    let lines = |ty| lay(ty).glyphs.iter().map(|g| g.line).collect::<Vec<_>>();
    let out = lay(KinsokuShoriType::PushOutOnly);
    assert!((out.glyphs[0].advance - 40.0).abs() < 0.01, "40 pt down the column: {}", out.glyphs[0].advance);
    assert_eq!(lines(KinsokuShoriType::PushInFirst), [0, 0, 0, 0, 0, 0, 1]);
    let bracket = &lay(KinsokuShoriType::PushInFirst).glyphs[5];
    assert!(bracket.origin.y + bracket.advance <= 225.01, "」 ends inside the column: {:?} +{}", bracket.origin, bracket.advance);
    assert_eq!(lines(KinsokuShoriType::PushOutOnly), [0, 0, 0, 0, 1, 1, 1]);
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

/// Proportional Metrics turns on the kerning that goes with it (#966): `vkrn` and `vapk` in
/// vertical type with `vpal`, `apkn` with `palt`, only while the style's kerning is automatic.
/// Without Proportional Metrics vertical type isn't kerned, as before. Needs craft-fonts' Noto Sans
/// CJK SC, which has `vpal` and `vkrn` (skipped without it; it isn't a document font, so it's loaded
/// here).
#[test]
fn proportional_metrics_kern_the_glyphs_they_respace() {
    let tags = |v: Vec<harfrust::Feature>| v.iter().map(|f| (f.tag.to_be_bytes(), f.value)).collect::<Vec<_>>();
    let st = |pm: bool, kerning: Option<f64>| CharStyle { size: 20.0, proportional_metrics: pm, kerning, ..CharStyle::default() };
    let h = crate::OtFeatures::default();
    assert!(tags(h.resolve(&st(true, None))).contains(&(*b"apkn", 1)));
    assert!(tags(h.resolve(&st(true, Some(0.0)))).contains(&(*b"apkn", 0)), "manual kerning: none");
    assert!(!tags(h.resolve_fixed_width(&st(true, None))).iter().any(|(t, _)| t == b"apkn" || t == b"palt"));
    let v = crate::OtFeatures { vertical: true, ..Default::default() };
    let with = tags(v.resolve_vertical(&st(true, None), true, true));
    assert!(with.contains(&(*b"vpal", 1)) && with.contains(&(*b"vkrn", 1)) && with.contains(&(*b"vapk", 1)), "{with:?}");
    assert!(tags(v.resolve_vertical(&st(true, Some(0.0)), true, true)).contains(&(*b"vkrn", 0)), "manual kerning: none");
    assert!(tags(v.resolve_vertical(&st(true, None), false, false)).contains(&(*b"vkrn", 0)));

    let Some(noto) = crate::CRAFT_FONTS.iter().find(|f| f.family == "Noto Sans CJK SC") else {
        eprintln!("skipped: built without craft-fonts (set CRAFT_FONTS_DIR to a craft-fonts checkout to run it)");
        return;
    };
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(noto.bytes.to_vec());
    let lay = |pm: bool, kerning: Option<f64>| {
        let s = CharStyle { font_family: "Noto Sans CJK SC".into(), ..st(pm, kerning) };
        layout(&db, &vertical_styled("「カタカナ」とトップ、ヤッホー。", s)).glyphs.iter().map(|g| g.advance).collect::<Vec<_>>()
    };
    let (auto, manual) = (lay(true, None), lay(true, Some(0.0)));
    assert!(auto.iter().zip(&manual).any(|(a, b)| (a - b).abs() > 0.01), "kerned: {auto:?} {manual:?}");
    let optical = {
        let s = CharStyle { font_family: "Noto Sans CJK SC".into(), optical_kerning: true, ..st(true, None) };
        layout(&db, &vertical_styled("「カタカナ」とトップ、ヤッホー。", s)).glyphs.iter().map(|g| g.advance).collect::<Vec<_>>()
    };
    assert!(auto.iter().zip(&optical).all(|(a, b)| (a - b).abs() < 0.01), "vertical Optical uses font metrics: {auto:?} {optical:?}");
    assert!(lay(false, None).iter().all(|a| (a - 20.0).abs() < 0.01), "off: an em each, as before");
}

#[test]
fn vertical_optical_to_auto_keeps_the_fonts_pair() {
    use crate::test_fonts::{PAIR_KERNING_FAMILY, pair_kerning_font};
    let db = FontDb::with_font_dirs(vec![]);
    db.add_font(pair_kerning_font().unwrap());
    let style = CharStyle { font_family: PAIR_KERNING_FAMILY.into(), size: 100.0, ..CharStyle::default() };
    let positions =
        |t: &TextObject| layout(&db, t).glyphs.iter().map(|g| (g.advance, g.origin, g.xf.as_coeffs()[4], g.xf.as_coeffs()[5])).collect::<Vec<_>>();
    let auto = vertical_styled("AV", style.clone());
    let mut mixed = auto.clone();
    edit::style_range(&mut mixed.runs, 0, 1, |s| s.optical_kerning = true);
    let mut unkerned = auto.clone();
    edit::style_range(&mut unkerned.runs, 0, 1, |s| s.kerning = Some(0.0));
    let expected = positions(&auto);
    assert_eq!(expected.len(), 2);
    assert_ne!(expected, positions(&unkerned), "the font's A-V pair changes vertical positions");
    assert_eq!(positions(&mixed), expected, "vertical Optical keeps the A-V font pair across the style boundary");
}
