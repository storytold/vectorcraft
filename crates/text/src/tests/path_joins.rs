//! Joined (cursive) type on a curved path: neighbouring glyphs must meet where they join. The gap
//! between one glyph's edge and the next one's, at the baseline and at the height of the joining
//! stroke, is the measure. Fonts are installed ones (none is kept in the repository): the tests
//! that need an Arabic font say so and stop without one.
use crate::*;
use kurbo::{Affine, BezPath, Point, Shape};
use std::sync::Arc;
use vectorcraft_doc::{CharStyle, TextKind};

fn arabic_face() -> Option<Arc<FontFace>> {
    let db = FontDb::global();
    ["Noto Sans Arabic", "Arial", "Geeza Pro"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular"))
        .find(|f| ['س', 'ل', 'ة', 'م', 'ت', 'ص'].iter().all(|c| f.covers(*c)))
}

/// A circle of radius `r` round the origin, one closed path.
fn circle(r: f64) -> vectorcraft_geom::PathData {
    vectorcraft_geom::PathData::from_bezpath(&kurbo::Circle::new((0.0, 0.0), r).to_path(1e-3))
}

/// Type `s` in `style` on a circle of radius `r`, the whole loop for it.
fn on_circle(s: &str, style: CharStyle, r: f64) -> TextLayout {
    let mut t = TextObject::point(Point::ZERO, s, style);
    t.kind = TextKind::OnPath { path: circle(r), start: 0.25, end: None };
    layout(FontDb::global(), &t)
}

/// For each pair of neighbouring glyphs on the line (in visual order, inside a word), how far
/// apart the right edge of the left one and the left edge of the right one are, at the baseline
/// and `h` above it.
fn gaps(l: &TextLayout, text: &str, h: f64) -> Vec<(f64, f64)> {
    // Visual order: the glyphs as placed, sorted along the line by where their left edge is on it.
    let edges: Vec<(Point, Point, Point, Point, usize)> = l
        .glyphs
        .iter()
        .filter(|g| text.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()))
        .map(|g| {
            let [a, b, ..] = g.xf.as_coeffs();
            let sx = a.hypot(b);
            let w = g.advance / sx;
            let up = h / sx;
            (g.xf * Point::new(0.0, 0.0), g.xf * Point::new(w, 0.0), g.xf * Point::new(0.0, -up), g.xf * Point::new(w, -up), g.byte)
        })
        .collect();
    // Which word a byte is in: the spaces before it.
    let word = |b: usize| text.get(..b).map_or(0, |t| t.chars().filter(|c| c.is_whitespace()).count());
    edges.windows(2).filter(|w| word(w[0].4) == word(w[1].4)).map(|w| ((w[0].1 - w[1].0).hypot(), (w[0].3 - w[1].2).hypot())).collect()
}

/// The largest gaps (baseline, joining height) of type `s` on a circle of radius `r` at `size`.
fn worst(face: &FontFace, s: &str, size: f64, r: f64) -> (f64, f64) {
    let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size, ..CharStyle::default() };
    let l = on_circle(s, style, r);
    gaps(&l, s, size * 0.25).into_iter().fold((0.0, 0.0), |(a, b), (x, y)| (f64::max(a, x), f64::max(b, y)))
}

/// The measure, printed: joined Arabic and separate Latin on circles at three sizes.
#[test]
fn joins_on_a_circle_are_measured() {
    let Some(face) = arabic_face() else {
        eprintln!("SKIPPED: joins_on_a_circle_are_measured; no installed font covers Arabic");
        return;
    };
    eprintln!("ASSERTIONS RUN: joins_on_a_circle_are_measured; Arabic font {} {}", face.family, face.style);
    for size in [12.0, 29.0, 72.0] {
        for r in [60.0, 110.0, 300.0] {
            let (b, j) = worst(&face, "سلسلة متصلة", size, r);
            let (lb, lj) = worst(&face, "minimum", size, r);
            eprintln!("size {size:>4} r {r:>5}: arabic baseline {b:.4} join {j:.4} | latin baseline {lb:.4} at {lj:.4}");
        }
    }
    let _ = BezPath::new();
    let _ = Affine::IDENTITY;
}

/// Chord placement, worked out here only (not in the layout): each glyph's baseline ends on the
/// circle, the glyph turned to the chord between them. Neighbours then meet at the baseline, and `h`
/// above it they stand apart by `h` times the angle between their chords, (a_i + a_{i+1}) / 2R.
#[test]
fn chord_placement_leaves_the_join_gap_above_the_baseline() {
    let Some(face) = arabic_face() else {
        eprintln!("SKIPPED: chord_placement_leaves_the_join_gap_above_the_baseline; no installed font covers Arabic");
        return;
    };
    let (size, r) = (29.0, 110.0);
    let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size, ..CharStyle::default() };
    let s = "سلسلة";
    let l = on_circle(s, style, r);
    let h = size * 0.25;
    let advances: Vec<f64> = l.glyphs.iter().map(|g| g.advance).filter(|a| *a > 1e-6).collect();
    let mut worst: f64 = 0.0;
    for w in advances.windows(2) {
        let (a, b) = (w[0], w[1]);
        // The chords' directions differ by the angle between their middles on the circle.
        let turn = (a + b) / (2.0 * r);
        let gap = 2.0 * h * (turn * 0.5).sin();
        let predicted = h * (a + b) / (2.0 * r);
        assert!((gap - predicted).abs() < 0.01, "{gap} vs {predicted}");
        worst = worst.max(gap);
    }
    eprintln!("chord placement at {size} pt on radius {r}: baseline gap 0, join-height gap {worst:.3}");
    assert!(worst > 0.5, "chord placement alone leaves a visible gap: {worst}");
}

/// A circle of radius `r` run the other way (clockwise): type on it stands on the inside, its
/// tops towards the centre, as Flip puts it.
fn circle_reversed(r: f64) -> vectorcraft_geom::PathData {
    let mut bp = kurbo::Circle::new((0.0, 0.0), r).to_path(1e-3);
    bp = bp.reverse_subpaths();
    vectorcraft_geom::PathData::from_bezpath(&bp)
}

fn on(path: vectorcraft_geom::PathData, s: &str, style: CharStyle) -> TextLayout {
    let mut t = TextObject::point(Point::ZERO, s, style);
    t.kind = TextKind::OnPath { path, start: 0.25, end: None };
    layout(FontDb::global(), &t)
}

/// The largest gap between neighbouring bent glyphs of a word at the baseline and `h` above it,
/// where their bends put their edges; and how many glyphs were bent.
fn bent_gaps(l: &TextLayout, text: &str, path: &vectorcraft_geom::PathData, h: f64) -> (f64, f64, usize) {
    let ap = vectorcraft_geom::ArcPath::new(path);
    let at = |b: &GlyphBend, x: f64, y: f64| {
        let (p, d) = ap.at(b.s + x * b.step / b.advance);
        p + kurbo::Vec2::new(-d.y, d.x) * (y + b.rise)
    };
    let word = |byte: usize| text.get(..byte).map_or(0, |t| t.chars().filter(|c| c.is_whitespace()).count());
    let bent: Vec<(&GlyphBend, usize)> = l.glyphs.iter().filter_map(|g| g.bend.as_ref().map(|b| (b, g.byte))).collect();
    let (mut base, mut join) = (0.0f64, 0.0f64);
    for w in bent.windows(2) {
        let ((a, ab), (b, bb)) = (w[0], w[1]);
        if word(ab) != word(bb) {
            continue;
        }
        base = base.max((at(a, a.advance, 0.0) - at(b, 0.0, 0.0)).hypot());
        join = join.max((at(a, a.advance, -h) - at(b, 0.0, -h)).hypot());
    }
    (base, join, bent.len())
}

/// Joined Arabic on circles of three radii at three sizes, on both sides of the path: neighbouring
/// glyphs meet, at the baseline and at the height of the joining stroke.
#[test]
fn joined_glyphs_meet_on_a_curved_path() {
    let Some(face) = arabic_face() else {
        eprintln!("SKIPPED: joined_glyphs_meet_on_a_curved_path; no installed font covers Arabic");
        return;
    };
    eprintln!("ASSERTIONS RUN: joined_glyphs_meet_on_a_curved_path; Arabic font {} {}", face.family, face.style);
    let s = "سلسلة متصلة";
    for size in [12.0, 29.0, 72.0] {
        let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size, ..CharStyle::default() };
        for r in [60.0, 110.0, 300.0] {
            for path in [circle(r), circle_reversed(r)] {
                let l = on(path.clone(), s, style.clone());
                let (base, join, n) = bent_gaps(&l, s, &path, size * 0.25);
                assert!(n >= 8, "the Arabic glyphs are bent ({n})");
                assert!(base <= 0.01 && join <= 0.01, "size {size} r {r}: baseline {base}, join {join}");
                for g in &l.glyphs {
                    let finite = g.outline.elements().iter().all(|e| e.end_point().is_none_or(|p| p.x.is_finite() && p.y.is_finite()));
                    assert!(finite, "a bent outline that isn't finite");
                }
            }
        }
    }
}

/// Latin type on the same path is placed as before: straight glyphs, not bent, each outline its
/// font's glyph through `xf`.
#[test]
fn latin_on_a_curved_path_stays_straight() {
    let db = FontDb::global();
    let style = CharStyle { size: 29.0, ..CharStyle::default() };
    let l = on(circle(110.0), "minimum", style);
    assert!(!l.glyphs.is_empty());
    for g in &l.glyphs {
        assert!(g.bend.is_none());
        let face = db.face_by_id(g.font_id).unwrap();
        let straight = g.xf * (*db.outline(&face, g.gid)).clone();
        assert_eq!(g.outline, straight, "glyph {} drawn through its xf", g.gid);
    }
}

/// On a straight path nothing turns: joined type is laid out exactly as before, straight.
#[test]
fn joined_type_on_a_straight_path_is_unchanged() {
    let Some(face) = arabic_face() else {
        eprintln!("SKIPPED: joined_type_on_a_straight_path_is_unchanged; no installed font covers Arabic");
        return;
    };
    let db = FontDb::global();
    let mut line = BezPath::new();
    line.move_to((0.0, 0.0));
    line.line_to((400.0, 0.0));
    let style = CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 29.0, ..CharStyle::default() };
    let l = on(vectorcraft_geom::PathData::from_bezpath(&line), "سلسلة متصلة", style);
    for g in &l.glyphs {
        assert!(g.bend.is_none());
        let face = db.face_by_id(g.font_id).unwrap();
        assert_eq!(g.outline, g.xf * (*db.outline(&face, g.gid)).clone());
    }
}
