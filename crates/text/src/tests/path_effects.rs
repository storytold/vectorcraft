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

/// `text` in `style` set as point type, and on `path` with `effect`, aligned as new type is (to the
/// start of its paragraph's direction).
fn point_and_on_path(text: &str, style: CharStyle, path: &vectorcraft_geom::PathData, effect: PathEffect) -> (TextLayout, TextLayout) {
    let mut t = TextObject::point(Point::ZERO, text, style);
    t.para.justify = vectorcraft_doc::Justify::Auto;
    t.para.direction = text.chars().any(|c| ('\u{600}'..='\u{6ff}').contains(&c)).then_some(ParaDirection::RightToLeft);
    let point = layout(FontDb::global(), &t);
    t.kind = TextKind::OnPath { path: path.clone(), start: 0.05, end: None };
    t.path_effect = effect;
    (point, layout(FontDb::global(), &t))
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

/// The drawn glyphs of `on` (type on a path) in blocks that each move as one, laid out as `point`
/// (the same type as point type) lays them out: each glyph with its point-type twin, and the map
/// from point-type text space to the block's place on the path. A block is a run of glyphs placed
/// by one transform; checking that each glyph is that transform of its twin checks that a joined
/// word keeps its point-type shape (joins and any offsets the font sets between joined letters)
/// whatever the font.
fn blocks<'a>(text: &str, point: &'a TextLayout, on: &'a TextLayout) -> Vec<(kurbo::Affine, Vec<(&'a PositionedGlyph, &'a PositionedGlyph)>)> {
    let drawn = on.glyphs.iter().filter(|g| text.get(g.byte..g.byte + g.len).is_some_and(|c| !c.trim().is_empty()));
    let mut out: Vec<(kurbo::Affine, Vec<(&PositionedGlyph, &PositionedGlyph)>)> = vec![];
    for g in drawn {
        let Some(twin) = point.glyphs.iter().find(|p| p.byte == g.byte && p.gid == g.gid) else { continue };
        let xf = g.xf * twin.xf.inverse();
        let same = |a: kurbo::Affine| a.as_coeffs().iter().zip(xf.as_coeffs()).all(|(x, y)| (x - y).abs() < 1e-6);
        match out.last_mut() {
            Some((block, members)) if same(*block) => members.push((g, twin)),
            _ => out.push((xf, vec![(g, twin)])),
        }
    }
    out
}

/// The middle of a block's baseline on the path: the middle of its glyphs' point-type baseline,
/// mapped by the block's transform.
fn baseline_middle(block: &(kurbo::Affine, Vec<(&PositionedGlyph, &PositionedGlyph)>)) -> Point {
    let (xf, members) = block;
    let left = members.iter().map(|(_, p)| (p.xf * Point::ZERO).x).fold(f64::INFINITY, f64::min);
    let right = members
        .iter()
        .map(|(_, p)| {
            let [a, b, ..] = p.xf.as_coeffs();
            (p.xf * Point::new(p.advance / a.hypot(b), 0.0)).x
        })
        .fold(f64::NEG_INFINITY, f64::max);
    *xf * Point::new((left + right) / 2.0, 0.0)
}

/// How far `p` is from `path`.
fn off_path(path: &vectorcraft_geom::PathData, p: Point) -> f64 {
    path.to_bezpath().segments().map(|s| kurbo::ParamCurveNearest::nearest(&s, p, 1e-4).distance_sq).fold(f64::INFINITY, f64::min).sqrt()
}

/// The words of `text` (by the spaces before a byte).
fn word(text: &str, b: usize) -> usize {
    text.get(..b).map_or(0, |t| t.chars().filter(|c| c.is_whitespace()).count())
}

/// Gravity: the middle of each glyph's baseline lies on the path (of each joined Arabic word's,
/// which tilts as one, keeping its point-type shape) and its vertical edges point at the path's
/// centre, for Latin, Arabic and mixed text on an open curve and a circle.
#[test]
fn gravity_points_vertical_edges_at_the_path_centre() {
    for path in paths() {
        let centre = path.bounds().unwrap_or_default().center();
        for (text, style) in texts() {
            let (point, l) = point_and_on_path(text, style, &path, PathEffect::Gravity);
            assert!(!l.overflow, "{text}");
            let bs = blocks(text, &point, &l);
            assert!(!bs.is_empty(), "{text}: nothing drawn");
            for b in &bs {
                let mid = baseline_middle(b);
                assert!(off_path(&path, mid) < 0.05, "{text}: a baseline middle {} pt off the path", off_path(&path, mid));
                let [_, _, c, d, ..] = b.0.as_coeffs();
                let up = -kurbo::Vec2::new(c, d).normalize();
                let out = (mid - centre).normalize();
                assert!(
                    up.dot(out).abs() > 1.0 - 1e-6,
                    "{text}: vertical edges at {}° to the centre",
                    up.dot(out).clamp(-1.0, 1.0).acos().to_degrees()
                );
            }
        }
    }
}

/// Stair Step: a joined Arabic word steps as one block, keeping its point-type shape, so its joins
/// stay closed; Latin glyphs each step on their own; the two Arabic words step apart.
#[test]
fn stair_step_keeps_joined_words_on_one_baseline() {
    let [curve, _] = paths();
    let latin = "Stairs";
    let (point, l) = point_and_on_path(latin, CharStyle { size: 24.0, ..CharStyle::default() }, &curve, PathEffect::StairStep);
    assert_eq!(blocks(latin, &point, &l).len(), latin.len(), "Latin steps glyph by glyph");
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    let (point, l) = point_and_on_path(TEXT, style(&face), &curve, PathEffect::StairStep);
    let bs = blocks(TEXT, &point, &l);
    assert_eq!(bs.len(), 2, "one block a word");
    for b in &bs {
        assert!(b.1.len() > 1);
        assert!(b.1.iter().all(|(g, _)| word(TEXT, g.byte) == word(TEXT, b.1[0].0.byte)), "a block is one word");
    }
    assert!((baseline_middle(&bs[0]).y - baseline_middle(&bs[1]).y).abs() > 0.5, "the two words step apart");
}

/// Stair Step: the middle of a joined Arabic word's baseline lies on the path, on an open curve and
/// a circle, alone or beside Latin.
#[test]
fn stair_step_sets_joined_words_on_the_path_by_their_middle() {
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    for path in paths() {
        for text in [TEXT, "سكة للتصميم gravity"] {
            let (point, l) = point_and_on_path(text, style(&face), &path, PathEffect::StairStep);
            for b in blocks(text, &point, &l).iter().filter(|b| b.1.len() > 1) {
                let off = off_path(&path, baseline_middle(b));
                assert!(off < 0.05, "{text}: the middle of a word's baseline is {off} pt off the path");
            }
        }
    }
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

/// 3D Ribbon: a joined Arabic word is placed as one unit, keeping its point-type shape (so its
/// joins meet), with the middle of its baseline on the path, on an open curve and on a circle.
#[test]
fn ribbon_keeps_joined_words_on_one_baseline() {
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    for curve in paths() {
        let (point, l) = point_and_on_path(TEXT, style(&face), &curve, PathEffect::Ribbon3d);
        let bs = blocks(TEXT, &point, &l);
        assert_eq!(bs.len(), 2, "one unit a word");
        for b in &bs {
            assert!(b.1.iter().all(|(g, _)| word(TEXT, g.byte) == word(TEXT, b.1[0].0.byte)), "a unit is one word");
            let off = off_path(&curve, baseline_middle(b));
            assert!(off < 0.05, "the middle of a word's baseline is {off} pt off the path");
        }
    }
}

/// Skew bends joined Arabic glyphs along a curved path, as Rainbow does, so where one ends along the
/// path the next begins and their joins meet; Latin glyphs stay unbent.
#[test]
fn skew_bends_joined_glyphs_so_their_joins_meet() {
    let [_, circle] = paths();
    let (_, latin) = point_and_on_path("Skewed", CharStyle { size: 24.0, ..CharStyle::default() }, &circle, PathEffect::Skew);
    assert!(latin.glyphs.iter().all(|g| g.bend.is_none()));
    let Some(face) = arabic_face() else {
        eprintln!("no Arabic font installed: skipped");
        return;
    };
    let (_, l) = point_and_on_path(TEXT, CharStyle { size: 48.0, ..style(&face) }, &circle, PathEffect::Skew);
    let bent: Vec<_> = l.glyphs.iter().filter_map(|g| g.bend.map(|b| (g.byte, b))).collect();
    assert!(bent.len() >= 6, "{bent:?}");
    let word = |b: usize| TEXT.get(..b).map_or(0, |t| t.chars().filter(|c| c.is_whitespace()).count());
    for w in bent.windows(2).filter(|w| word(w[0].0) == word(w[1].0)) {
        let (a, b) = (w[0].1, w[1].1);
        assert!((a.s + a.step - b.s).abs() < 1e-6, "a gap along the path between joined glyphs: {a:?} {b:?}");
    }
}

/// Gravity stays finite where the line from the path's centre runs along the path: on a nearly flat
/// path, an S-curve and a straight line (whose centre lies on it), every glyph's transform is
/// finite and invertible and every glyph lies within a few type sizes of the path's bounds.
#[test]
fn gravity_stays_finite_on_flat_and_s_shaped_paths() {
    let curve = |els: &[(f64, f64)]| {
        let mut b = kurbo::BezPath::new();
        b.move_to(els[0]);
        b.curve_to(els[1], els[2], els[3]);
        vectorcraft_geom::PathData::from_bezpath(&b)
    };
    let paths = [
        ("nearly flat", curve(&[(0.0, 0.0), (300.0, 0.5), (600.0, -0.5), (900.0, 0.0)])),
        ("S-curve", curve(&[(0.0, 100.0), (300.0, -200.0), (600.0, 400.0), (900.0, 100.0)])),
        ("straight", curve(&[(0.0, 0.0), (300.0, 0.0), (600.0, 0.0), (900.0, 0.0)])),
    ];
    for (name, path) in &paths {
        let b = path.bounds().unwrap_or_default().inflate(24.0 * 4.0, 24.0 * 4.0);
        for (text, style) in texts() {
            let (_, l) = point_and_on_path(text, style, path, PathEffect::Gravity);
            assert!(l.bounds.x0.is_finite() && l.bounds.y1.is_finite(), "{name} {text}: {:?}", l.bounds);
            for g in &l.glyphs {
                assert!(g.xf.as_coeffs().iter().all(|c| c.is_finite()) && g.xf.determinant().abs() > 0.0, "{name} {text}: {:?}", g.xf);
                if g.outline.elements().is_empty() {
                    continue;
                }
                let gb = g.outline.bounding_box();
                assert!(b.contains(gb.origin()) && b.contains(Point::new(gb.x1, gb.y1)), "{name} {text}: a glyph at {gb:?}, the path within {b:?}");
            }
        }
    }
}
