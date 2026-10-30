//! Type on a path's effects with Latin, Arabic and mixed text. Arabic gets the glyphs point type
//! gets (joined forms, ligatures) in the same visual order along the path, on a circle and on a
//! reversed path too; every effect keeps each glyph's size. Arabic fonts are installed ones (none is
//! kept in the repository): without one the Arabic tests say so and stop.
use crate::*;
use kurbo::{Point, Shape};
use std::sync::Arc;
use vectorcraft_doc::{CharStyle, ParaDirection, PathEffect, TextKind};

const TEXT: &str = "سكة للتصميم";

fn arabic_face() -> Option<Arc<FontFace>> {
    let db = FontDb::global();
    ["Noto Sans Arabic", "Geeza Pro", "Arial"]
        .into_iter()
        .filter_map(|name| db.face(name, "Regular"))
        .find(|f| TEXT.chars().filter(|c| !c.is_whitespace()).all(|c| f.covers(c)))
}

fn style(face: &FontFace) -> CharStyle {
    CharStyle { font_family: face.family.clone(), font_style: face.style.clone(), size: 24.0, ..CharStyle::default() }
}

/// `TEXT` set right to left as point type, or on `kind`.
fn set(face: &FontFace, kind: Option<TextKind>) -> TextLayout {
    let mut t = TextObject::point(Point::ZERO, TEXT, style(face));
    t.para.direction = Some(ParaDirection::RightToLeft);
    if let Some(kind) = kind {
        t.kind = kind;
    }
    layout(FontDb::global(), &t)
}

/// A straight path from (0, 0) to (`len`, 0), or the reverse.
fn line(len: f64, reversed: bool) -> vectorcraft_geom::PathData {
    let (a, b) = if reversed { (Point::new(len, 0.0), Point::ZERO) } else { (Point::ZERO, Point::new(len, 0.0)) };
    vectorcraft_geom::PathData::from_bezpath(&kurbo::Line::new(a, b).to_path(1e-3))
}

fn circle(r: f64) -> vectorcraft_geom::PathData {
    vectorcraft_geom::PathData::from_bezpath(&kurbo::Circle::new((0.0, 0.0), r).to_path(1e-3))
}

/// The glyphs drawn (not spaces), as (glyph id, byte), in the order they are laid out.
fn glyphs(l: &TextLayout) -> Vec<(u32, usize)> {
    l.glyphs.iter().filter(|g| TEXT.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty())).map(|g| (g.gid, g.byte)).collect()
}

/// The point type's glyphs, left to right on the page.
fn point_glyphs(face: &FontFace) -> Vec<(u32, usize)> {
    let l = set(face, None);
    let mut g: Vec<_> = l
        .glyphs
        .iter()
        .filter(|g| TEXT.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()))
        .map(|g| (g.origin.x, g.gid, g.byte))
        .collect();
    g.sort_by(|a, b| a.0.total_cmp(&b.0));
    g.into_iter().map(|(_, gid, byte)| (gid, byte)).collect()
}

#[test]
fn arabic_on_a_line_has_the_point_types_shaped_glyphs_in_its_order() {
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    let point = point_glyphs(&face);
    assert!(point.len() >= 6, "{point:?}");
    let l = set(&face, Some(TextKind::OnPath { path: line(600.0, false), start: 0.0, end: None }));
    // Along the path (left to right here): the same glyphs as point type left to right.
    let mut along: Vec<_> = l
        .glyphs
        .iter()
        .filter(|g| TEXT.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()))
        .map(|g| (g.origin.x, g.gid, g.byte))
        .collect();
    along.sort_by(|a, b| a.0.total_cmp(&b.0));
    let along: Vec<_> = along.into_iter().map(|(_, gid, byte)| (gid, byte)).collect();
    assert_eq!(along, point, "glyph ids and clusters along the path");
    assert_eq!(glyphs(&l).len(), point.len());
}

#[test]
fn arabic_on_a_circle_and_flipped_keeps_its_shaped_glyphs() {
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    let mut point = point_glyphs(&face);
    point.sort();
    for kind in
        [TextKind::OnPath { path: circle(150.0), start: 0.25, end: None }, TextKind::OnPath { path: line(600.0, true), start: 0.0, end: None }]
    {
        let l = set(&face, Some(kind.clone()));
        let mut got = glyphs(&l);
        got.sort();
        assert_eq!(got, point, "the same shaped glyphs: {kind:?}");
        assert!(!l.overflow);
        let b = l.bounds;
        assert!(b.width() > 0.0 && b.height() > 0.0);
    }
}

/// The open curve and the circle the effects are tried on.
fn paths() -> [vectorcraft_geom::PathData; 2] {
    let mut curve = kurbo::BezPath::new();
    curve.move_to((80.0, 160.0));
    curve.curve_to((300.0, 60.0), (500.0, 260.0), (820.0, 140.0));
    [vectorcraft_geom::PathData::from_bezpath(&curve), circle(150.0)]
}

/// `text` in `style` set as point type, and on `path` with `effect`.
fn point_and_on_path(text: &str, style: CharStyle, path: &vectorcraft_geom::PathData, effect: PathEffect) -> (TextLayout, TextLayout) {
    let mut t = TextObject::point(Point::ZERO, text, style);
    t.para.direction = text.chars().any(|c| ('\u{600}'..='\u{6ff}').contains(&c)).then_some(ParaDirection::RightToLeft);
    let point = layout(FontDb::global(), &t);
    t.kind = TextKind::OnPath { path: path.clone(), start: 0.05, end: None };
    t.path_effect = effect;
    (point, layout(FontDb::global(), &t))
}

/// Each drawn glyph's area on the path over its area as point type (1: the same size), by byte.
fn size_ratios(text: &str, point: &TextLayout, on_path: &TextLayout) -> Vec<(usize, f64)> {
    let area = |g: &PositionedGlyph| g.xf.determinant().abs();
    on_path
        .glyphs
        .iter()
        .filter(|g| text.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()))
        .filter_map(|g| point.glyphs.iter().find(|p| p.byte == g.byte && p.gid == g.gid).map(|p| (g.byte, area(g) / area(p))))
        .collect()
}

/// The texts tried: Latin, Arabic and mixed (the Arabic ones only with an Arabic font installed).
fn texts() -> Vec<(&'static str, CharStyle)> {
    let latin = CharStyle { size: 24.0, ..CharStyle::default() };
    let mut out = vec![("Gravity sample", latin)];
    if let Some(face) = arabic_face() {
        out.push((TEXT, style(&face)));
        out.push(("سكة للتصميم gravity", style(&face)));
    } else {
        eprintln!("no Arabic font installed: Latin only");
    }
    out
}

/// Gravity keeps every glyph whole: at its size (it leans toward the path's centre, it is not
/// crushed), drawn, and near the path, for Latin, Arabic and mixed text on an open curve and a
/// circle.
#[test]
fn gravity_keeps_every_glyph_whole() {
    for path in paths() {
        let along = path.to_bezpath();
        for (text, style) in texts() {
            let (point, l) = point_and_on_path(text, style, &path, PathEffect::Gravity);
            assert!(!l.overflow, "{text}");
            let ratios = size_ratios(text, &point, &l);
            assert!(!ratios.is_empty(), "{text}: nothing drawn");
            for (byte, r) in &ratios {
                assert!((r - 1.0).abs() < 1e-6, "{text}: the glyph at byte {byte} is drawn at {r} of its area");
            }
            // Vertical edges lean at most 45° off the path's normal.
            for g in l.glyphs.iter().filter(|g| text.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty())) {
                let [a, b, c, d, ..] = g.xf.as_coeffs();
                let (x, y) = (kurbo::Vec2::new(a, b), kurbo::Vec2::new(c, d));
                let lean = (x.dot(y) / (x.hypot() * y.hypot())).abs();
                assert!(lean <= std::f64::consts::FRAC_1_SQRT_2 + 1e-9, "{text}: a glyph leans {}°", lean.acos().to_degrees());
            }
            for g in l.glyphs.iter().filter(|g| !g.outline.elements().is_empty()) {
                let b = g.outline.bounding_box();
                let near = along
                    .segments()
                    .map(|s| kurbo::ParamCurveNearest::nearest(&s, b.center(), 1e-3).distance_sq)
                    .fold(f64::INFINITY, f64::min)
                    .sqrt();
                assert!(near < 40.0, "{text}: a glyph {near} pt off the path");
            }
        }
    }
}

/// Stair Step: the glyphs of a joined Arabic word step as one block, on one baseline, so their joins
/// stay closed; Latin glyphs each step on their own.
#[test]
fn stair_step_keeps_joined_words_on_one_baseline() {
    let [curve, _] = paths();
    let baselines = |text: &str, style: CharStyle| -> Vec<(usize, f64)> {
        let (_, l) = point_and_on_path(text, style, &curve, PathEffect::StairStep);
        l.glyphs
            .iter()
            .filter(|g| text.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()))
            .map(|g| (g.byte, g.xf.translation().y))
            .collect()
    };
    let latin = baselines("Stairs", CharStyle { size: 24.0, ..CharStyle::default() });
    assert!(latin.windows(2).any(|w| (w[0].1 - w[1].1).abs() > 0.5), "Latin steps glyph by glyph: {latin:?}");
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    let text = "سكة للتصميم";
    let words: Vec<_> = baselines(text, style(&face));
    let word = |b: usize| text.get(..b).map_or(0, |t| t.chars().filter(|c| c.is_whitespace()).count());
    for w in 0..2 {
        let ys: Vec<f64> = words.iter().filter(|(b, _)| word(*b) == w).map(|(_, y)| *y).collect();
        assert!(ys.len() > 1, "{words:?}");
        assert!(ys.iter().all(|y| (y - ys[0]).abs() < 1e-6), "word {w} on one baseline: {ys:?}");
    }
    let first = |w: usize| words.iter().find(|(b, _)| word(*b) == w).map(|(_, y)| *y).unwrap_or_default();
    assert!((first(0) - first(1)).abs() > 0.5, "the two words step apart: {words:?}");
}

/// 3D Ribbon: every glyph keeps its width (horizontal edges stay horizontal, vertical edges stand
/// across the path); a Latin glyph sits with the middle of its baseline on the path, and a joined
/// Arabic word stands level on one baseline centred on the path, so its joins meet.
#[test]
fn ribbon_sets_every_glyph_on_the_path_at_its_size() {
    for path in paths() {
        let along = path.to_bezpath();
        for (text, style) in texts() {
            let (point, l) = point_and_on_path(text, style, &path, PathEffect::Ribbon3d);
            for g in l.glyphs.iter().filter(|g| text.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty())) {
                let Some(p) = point.glyphs.iter().find(|p| p.byte == g.byte && p.gid == g.gid) else { continue };
                let [a, b, ..] = g.xf.as_coeffs();
                let [pa, pb, ..] = p.xf.as_coeffs();
                assert!(
                    (a.hypot(b) / pa.hypot(pb) - 1.0).abs() < 1e-6,
                    "{text}: the glyph at byte {} is {} as wide",
                    g.byte,
                    a.hypot(b) / pa.hypot(pb)
                );
                // The middle of its baseline, in font units: half its advance.
                let mid = g.xf * Point::new(g.advance / 2.0 / pa.hypot(pb), 0.0);
                if !g.rtl {
                    let off =
                        along.segments().map(|s| kurbo::ParamCurveNearest::nearest(&s, mid, 1e-4).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
                    assert!(off < 0.05, "{text}: the glyph at byte {} stands {off} pt off the path", g.byte);
                }
            }
        }
    }
}

/// 3D Ribbon: the glyphs of a joined Arabic word share one baseline (the same line through each
/// glyph's origin, in the same direction), so neighbours meet where they join, and the middle of
/// that baseline lies on the path.
#[test]
fn ribbon_keeps_joined_words_on_one_baseline() {
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    let [curve, _] = paths();
    let (_, l) = point_and_on_path(TEXT, style(&face), &curve, PathEffect::Ribbon3d);
    let word = |b: usize| TEXT.get(..b).map_or(0, |t| t.chars().filter(|c| c.is_whitespace()).count());
    for w in 0..2 {
        let g: Vec<_> =
            l.glyphs.iter().filter(|g| TEXT.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()) && word(g.byte) == w).collect();
        assert!(g.len() > 1);
        let (first, rest) = g.split_first().unwrap();
        let [a, b, c, d, ..] = first.xf.as_coeffs();
        for h in rest {
            let [ha, hb, hc, hd, ..] = h.xf.as_coeffs();
            assert!((ha - a).abs() < 1e-9 && (hb - b).abs() < 1e-9 && (hc - c).abs() < 1e-9 && (hd - d).abs() < 1e-9, "word {w}: one frame");
            // Its origin lies on the first glyph's baseline: no step across the path.
            let local = first.xf.inverse() * (h.xf * Point::ZERO);
            assert!(local.y.abs() < 1e-6, "word {w}: a glyph {} font units off the word's baseline", local.y);
        }
        let scale = {
            let [a, b, ..] = first.xf.as_coeffs();
            a.hypot(b)
        };
        let width: f64 = g.iter().map(|g| g.advance).sum::<f64>() / scale;
        let left = g.iter().map(|h| first.xf.inverse() * (h.xf * Point::ZERO)).map(|p| p.x).fold(f64::INFINITY, f64::min);
        let mid = first.xf * Point::new(left + width / 2.0, 0.0);
        let off =
            curve.to_bezpath().segments().map(|s| kurbo::ParamCurveNearest::nearest(&s, mid, 1e-4).distance_sq).fold(f64::INFINITY, f64::min).sqrt();
        assert!(off < 0.05, "word {w}: the middle of its baseline is {off} pt off the path");
    }
}
