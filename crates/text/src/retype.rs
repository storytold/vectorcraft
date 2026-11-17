//! Retype's glyph matching (Type › Retype, #4934): which glyph of which loaded font an outlined
//! glyph's shape is, the same up to uniform scale, rotation and translation.
//!
//! Phase 1 is exact matching only: a shape matches a glyph when, after the best similarity
//! transform (a Procrustes fit over points sampled at equal steps of arc length along each
//! contour), the leftover distance is a tiny fraction of the shape's size. That holds for outlines
//! made from a font the app has (Create Outlines' output, and other apps' that keep each glyph's
//! contours whole and in the font's order). Near matches (other versions of a font, hinted or
//! simplified outlines, faux styles, glyphs merged into one path) are left unmatched; fuzzy
//! identification is phase 2.

use std::collections::HashMap;
use std::sync::Arc;

use kurbo::{BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, PathEl, PathSeg, Point, QuadBez, Shape};

use crate::{FontDb, FontFace};

/// At most this many faces are indexed.
pub const MAX_FACES: usize = 96;
/// At most this many encoded characters of one face are indexed (in code point order).
pub const MAX_GLYPHS_PER_FACE: usize = 65_536;
/// At most this many glyphs are indexed in all: faces past it are left out.
pub const MAX_GLYPHS: usize = 262_144;
/// Shapes with more path elements than this are not matched.
pub const MAX_ELEMENTS: usize = 4096;
/// Shapes (and glyphs) with more contours than this are not matched.
const MAX_CONTOURS: usize = 48;
/// Points sampled along each contour.
const SAMPLES: usize = 32;
/// The largest leftover distance (root mean square over the samples) for a match, as a fraction of
/// the shape's spread about its centre.
const TOLERANCE: f64 = 2e-3;
/// How far apart two shapes' spread-to-length ratios may be and still be compared.
const INVARIANT_SLACK: f64 = 0.02;

/// One glyph an outlined shape matched.
#[derive(Clone)]
pub struct GlyphMatch {
    pub face: Arc<FontFace>,
    pub gid: u32,
    /// The character the face's cmap maps to the glyph (the lowest, when several do).
    pub ch: char,
    /// Document units per font unit: the font size is `scale` × units per em.
    pub scale: f64,
    /// The glyph's turn, radians (as [`kurbo::Affine::rotate`] turns).
    pub angle: f64,
    /// Where the glyph's origin (its baseline start) lies.
    pub origin: Point,
    /// The leftover distance, relative to the shape's spread.
    pub residual: f64,
}

/// A shape reduced for matching: points along each contour, centred.
struct Signature {
    contours: usize,
    /// Centred points, [`SAMPLES`] per contour.
    pts: Vec<Point>,
    centre: Point,
    /// Root mean square distance of the points from the centre.
    spread: f64,
    /// `spread` / the total contour length: unchanged by scale, turn and move.
    invariant: f64,
}

struct Entry {
    face: usize,
    gid: u32,
    ch: char,
    sig: Signature,
}

/// The glyphs of a set of faces, ready to match shapes against.
pub struct RetypeIndex {
    faces: Vec<Arc<FontFace>>,
    entries: Vec<Entry>,
    /// Entry indices by contour count, sorted by invariant.
    buckets: HashMap<usize, Vec<usize>>,
}

impl RetypeIndex {
    /// Index the outline glyphs of `faces` (the first [`MAX_FACES`], each face once).
    pub fn build(db: &FontDb, faces: impl IntoIterator<Item = Arc<FontFace>>) -> Self {
        let mut kept: Vec<Arc<FontFace>> = vec![];
        for f in faces {
            if kept.len() >= MAX_FACES {
                break;
            }
            if !kept.iter().any(|k| k.id() == f.id()) {
                kept.push(f);
            }
        }
        let mut entries = vec![];
        for (fi, face) in kept.iter().enumerate() {
            if entries.len() >= MAX_GLYPHS {
                break;
            }
            // One entry per glyph, under the lowest character that maps to it.
            let mut by_gid: HashMap<u32, char> = HashMap::new();
            for &(c, gid) in face.mapped_chars().iter().take(MAX_GLYPHS_PER_FACE) {
                if c.is_whitespace() || c.is_control() {
                    continue;
                }
                by_gid.entry(gid).and_modify(|e| *e = (*e).min(c)).or_insert(c);
            }
            let mut gids: Vec<(u32, char)> = by_gid.into_iter().collect();
            gids.sort_unstable();
            for (gid, ch) in gids {
                let outline = db.outline(face, gid);
                if let Some(sig) = signature(&outline) {
                    entries.push(Entry { face: fi, gid, ch, sig });
                }
            }
        }
        let mut buckets: HashMap<usize, Vec<usize>> = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            buckets.entry(e.sig.contours).or_default().push(i);
        }
        for list in buckets.values_mut() {
            list.sort_by(|a, b| {
                let (x, y) = (entries.get(*a).map_or(0.0, |e| e.sig.invariant), entries.get(*b).map_or(0.0, |e| e.sig.invariant));
                x.total_cmp(&y)
            });
        }
        RetypeIndex { faces: kept, entries, buckets }
    }

    /// The faces indexed.
    pub fn faces(&self) -> &[Arc<FontFace>] {
        &self.faces
    }

    /// How many glyphs are indexed.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The glyphs `shape` (in document space) is, the closest one per face, closest first; empty
    /// when it is no glyph of the indexed faces.
    pub fn matches(&self, shape: &BezPath) -> Vec<GlyphMatch> {
        let Some(sig) = signature(shape) else { return vec![] };
        let Some(list) = self.buckets.get(&sig.contours) else { return vec![] };
        let lo = sig.invariant * (1.0 - INVARIANT_SLACK);
        let hi = sig.invariant * (1.0 + INVARIANT_SLACK);
        let start = list.partition_point(|i| self.entries.get(*i).is_some_and(|e| e.sig.invariant < lo));
        let mut best: HashMap<usize, GlyphMatch> = HashMap::new();
        for i in list.iter().skip(start) {
            let Some(e) = self.entries.get(*i) else { continue };
            if e.sig.invariant > hi {
                break;
            }
            let Some((z, residual)) = fit(&e.sig, &sig) else { continue };
            if residual > TOLERANCE {
                continue;
            }
            let Some(face) = self.faces.get(e.face) else { continue };
            // The glyph's origin is (0, 0) in font units: it lands at centre − z·glyph centre.
            let gc = mul(z, e.sig.centre);
            let origin = Point::new(sig.centre.x - gc.x, sig.centre.y - gc.y);
            let m = GlyphMatch { face: face.clone(), gid: e.gid, ch: e.ch, scale: z.0.hypot(z.1), angle: z.1.atan2(z.0), origin, residual };
            match best.get(&e.face) {
                Some(b) if b.residual < m.residual || (b.residual == m.residual && b.ch <= m.ch) => {}
                _ => {
                    best.insert(e.face, m);
                }
            }
        }
        let mut out: Vec<(usize, GlyphMatch)> = best.into_iter().collect();
        out.sort_by(|a, b| a.1.residual.total_cmp(&b.1.residual).then(a.0.cmp(&b.0)));
        out.into_iter().map(|(_, m)| m).collect()
    }
}

/// Complex multiply: `z` (a scaled turn) applied to `p`.
fn mul(z: (f64, f64), p: Point) -> Point {
    Point::new(z.0 * p.x - z.1 * p.y, z.1 * p.x + z.0 * p.y)
}

/// The scaled turn `z` that best takes glyph `g`'s points onto shape `s`'s (both centred), and the
/// leftover distance relative to the shape's spread; `None` for degenerate shapes.
fn fit(g: &Signature, s: &Signature) -> Option<((f64, f64), f64)> {
    if g.pts.len() != s.pts.len() || g.pts.is_empty() {
        return None;
    }
    let (mut re, mut im, mut gg) = (0.0, 0.0, 0.0);
    for (a, b) in g.pts.iter().zip(&s.pts) {
        // Σ s · conj(g)
        re += b.x * a.x + b.y * a.y;
        im += b.y * a.x - b.x * a.y;
        gg += a.x * a.x + a.y * a.y;
    }
    if gg <= 0.0 || !gg.is_finite() {
        return None;
    }
    let z = (re / gg, im / gg);
    let mut res = 0.0;
    for (a, b) in g.pts.iter().zip(&s.pts) {
        let p = mul(z, *a);
        res += (p.x - b.x).powi(2) + (p.y - b.y).powi(2);
    }
    let n = s.pts.len() as f64;
    let rel = (res / n).sqrt() / s.spread;
    rel.is_finite().then_some((z, rel))
}

/// `path`'s contours as segments, the closing line included; `None` past the size caps.
fn contours(path: &BezPath) -> Option<Vec<Vec<PathSeg>>> {
    if path.elements().len() > MAX_ELEMENTS {
        return None;
    }
    let mut out: Vec<Vec<PathSeg>> = vec![];
    let mut cur: Vec<PathSeg> = vec![];
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let mut flush = |cur: &mut Vec<PathSeg>, start: Point, last: Point| {
        if last.distance(start) > 0.0 {
            cur.push(PathSeg::Line(Line::new(last, start)));
        }
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut cur, start, last);
                start = p;
                last = p;
            }
            PathEl::LineTo(p) => {
                cur.push(PathSeg::Line(Line::new(last, p)));
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                cur.push(PathSeg::Quad(QuadBez::new(last, a, p)));
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                cur.push(PathSeg::Cubic(CubicBez::new(last, a, b, p)));
                last = p;
            }
            PathEl::ClosePath => {
                flush(&mut cur, start, last);
                last = start;
            }
        }
    }
    flush(&mut cur, start, last);
    (out.len() <= MAX_CONTOURS).then_some(out)
}

fn signature(path: &BezPath) -> Option<Signature> {
    let bounds = path.bounding_box();
    let size = bounds.width().hypot(bounds.height());
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let accuracy = size * 1e-6;
    let mut pts = vec![];
    let mut total = 0.0;
    let mut contours_n = 0;
    for c in contours(path)? {
        let lens: Vec<f64> = c.iter().map(|s| s.arclen(accuracy)).collect();
        let len: f64 = lens.iter().sum();
        // A contour with no length (a lone move) draws nothing.
        if !(len > size * 1e-9) || !len.is_finite() {
            continue;
        }
        total += len;
        contours_n += 1;
        let (mut si, mut before) = (0usize, 0.0);
        for k in 0..SAMPLES {
            let at = len * k as f64 / SAMPLES as f64;
            while si + 1 < c.len() && before + lens.get(si).copied().unwrap_or(0.0) < at {
                before += lens.get(si).copied().unwrap_or(0.0);
                si += 1;
            }
            let (Some(seg), Some(l)) = (c.get(si), lens.get(si)) else { return None };
            let t = if *l > 0.0 { seg.inv_arclen((at - before).clamp(0.0, *l), accuracy) } else { 0.0 };
            pts.push(seg.eval(t));
        }
    }
    if pts.is_empty() {
        return None;
    }
    let n = pts.len() as f64;
    let centre = Point::new(pts.iter().map(|p| p.x).sum::<f64>() / n, pts.iter().map(|p| p.y).sum::<f64>() / n);
    for p in &mut pts {
        *p = Point::new(p.x - centre.x, p.y - centre.y);
    }
    let spread = (pts.iter().map(|p| p.x * p.x + p.y * p.y).sum::<f64>() / n).sqrt();
    if !(spread > 0.0) || !spread.is_finite() {
        return None;
    }
    Some(Signature { contours: contours_n, pts, centre, spread, invariant: spread / total })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Affine;

    fn letter() -> BezPath {
        // A hand-made "P"-like shape: an outer contour and a counter.
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((0.0, -700.0));
        p.curve_to((300.0, -700.0), (450.0, -600.0), (450.0, -480.0));
        p.quad_to((450.0, -300.0), (120.0, -300.0));
        p.line_to((120.0, 0.0));
        p.close_path();
        p.move_to((120.0, -600.0));
        p.line_to((120.0, -400.0));
        p.quad_to((330.0, -400.0), (330.0, -500.0));
        p.quad_to((330.0, -600.0), (120.0, -600.0));
        p.close_path();
        p
    }

    #[test]
    fn a_shape_matches_itself_scaled_turned_and_moved() {
        let g = signature(&letter()).unwrap();
        let xf = Affine::translate((300.0, 200.0)) * Affine::rotate(0.4) * Affine::scale(0.024);
        let s = signature(&(xf * letter())).unwrap();
        let (z, res) = fit(&g, &s).unwrap();
        assert!(res < 1e-6, "{res}");
        assert!((z.0.hypot(z.1) - 0.024).abs() < 1e-9);
        assert!((z.1.atan2(z.0) - 0.4).abs() < 1e-9);
        let origin = s.centre - mul(z, g.centre).to_vec2();
        assert!(origin.distance(Point::new(300.0, 200.0)) < 1e-6, "{origin:?}");
    }

    #[test]
    fn lines_written_as_curves_still_match() {
        // Paths store lines as cubics with handles on their anchors, or the closing line explicitly.
        let mut alt = BezPath::new();
        alt.move_to((0.0, 0.0));
        alt.curve_to((0.0, 0.0), (0.0, -700.0), (0.0, -700.0));
        alt.curve_to((300.0, -700.0), (450.0, -600.0), (450.0, -480.0));
        alt.extend([PathEl::QuadTo((450.0, -300.0).into(), (120.0, -300.0).into())]);
        alt.line_to((120.0, 0.0));
        alt.line_to((0.0, 0.0));
        alt.close_path();
        alt.move_to((120.0, -600.0));
        alt.line_to((120.0, -400.0));
        alt.quad_to((330.0, -400.0), (330.0, -500.0));
        alt.quad_to((330.0, -600.0), (120.0, -600.0));
        alt.close_path();
        let (_, res) = fit(&signature(&letter()).unwrap(), &signature(&alt).unwrap()).unwrap();
        assert!(res < 1e-6, "{res}");
    }

    #[test]
    fn another_shape_does_not_match() {
        let mut o = BezPath::new();
        o.extend(kurbo::Circle::new((200.0, -350.0), 300.0).path_elements(0.1));
        let mut box_ = BezPath::new();
        box_.extend(kurbo::Rect::new(0.0, -700.0, 450.0, 0.0).path_elements(0.1));
        let (a, b) = (signature(&o).unwrap(), signature(&box_).unwrap());
        assert!(fit(&a, &b).is_none_or(|(_, r)| r > TOLERANCE));
    }

    #[test]
    fn degenerate_and_hostile_shapes_give_none() {
        assert!(signature(&BezPath::new()).is_none());
        let mut dot = BezPath::new();
        dot.move_to((5.0, 5.0));
        dot.close_path();
        assert!(signature(&dot).is_none());
        let mut nan = BezPath::new();
        nan.move_to((f64::NAN, 0.0));
        nan.line_to((1.0, 1.0));
        assert!(signature(&nan).is_none());
        let mut huge = BezPath::new();
        huge.move_to((0.0, 0.0));
        for i in 0..(MAX_ELEMENTS + 1) {
            huge.line_to((i as f64, (i % 7) as f64));
        }
        assert!(signature(&huge).is_none());
    }
}
