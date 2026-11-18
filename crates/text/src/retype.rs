//! Retype's glyph matching (Type › Retype, #4934): which glyph of which loaded font an outlined
//! glyph's shape is, the same up to uniform scale, rotation and translation.
//!
//! Phase 1 is exact matching only. Each contour is read from a starting corner that does not
//! depend on where the path happens to start (the corner farthest from the shape's corners'
//! centre; every such corner is tried when several tie), with straight edges split in two read as
//! one. Points sampled at equal steps of arc length along the contours shortlist the glyphs whose
//! best similarity transform (a Procrustes fit) leaves a tiny leftover distance, and the whole
//! outline, turned onto the shape, must then lie on it. That holds for outlines made from a font
//! the app has (Create Outlines' output, and other apps' that keep each glyph's contours whole and
//! in the font's order). Near matches (other versions of a font, hinted or simplified outlines,
//! faux styles, glyphs merged into one path) are left unmatched; fuzzy identification is phase 2.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use kurbo::{Affine, BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveNearest, PathEl, PathSeg, Point, QuadBez, Shape};

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
/// The most readings of a shape (one per choice of tied starting corners) that are tried.
const MAX_VARIANTS: usize = 64;
/// The largest leftover distance (root mean square over the samples) for a match, as a fraction of
/// the shape's spread about its centre.
const TOLERANCE: f64 = 2e-3;
/// How far the whole outline, turned onto the shape, may lie from it (both ways), as a fraction of
/// the shape's spread.
const OUTLINE_TOLERANCE: f64 = 5e-3;
/// Points of a flattened outline past which the whole-outline check gives up (and so rejects).
const MAX_OUTLINE_POINTS: usize = 8192;
/// Points along an outline past which the whole-outline check gives up (and so rejects).
const MAX_CHECKED_POINTS: usize = 65_536;
/// The most glyphs one shape is reported to be ([`RetypeIndex::matches`]).
pub const MAX_CANDIDATES: usize = 16;
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
    /// How many contours the glyph has.
    pub contours: usize,
    /// Other poses the glyph fits the shape in equally (a glyph that looks the same turned).
    pub alts: Vec<Pose>,
}

/// Where and how a glyph lies: its size, turn and origin.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    /// Document units per font unit.
    pub scale: f64,
    /// Radians, as [`kurbo::Affine::rotate`] turns.
    pub angle: f64,
    /// Where the glyph's origin lies.
    pub origin: Point,
}

impl GlyphMatch {
    /// This match in each pose it fits: its own first, then [`Self::alts`].
    pub fn poses(&self) -> impl Iterator<Item = GlyphMatch> + '_ {
        let own = Pose { scale: self.scale, angle: self.angle, origin: self.origin };
        std::iter::once(own).chain(self.alts.iter().copied()).map(move |p| GlyphMatch {
            scale: p.scale,
            angle: p.angle,
            origin: p.origin,
            ..self.clone()
        })
    }
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
    /// The glyph's outline in font units, for the whole-outline check.
    outline: BezPath,
}

/// The glyphs of a set of faces, ready to match shapes against.
#[derive(Default)]
pub struct RetypeIndex {
    faces: Vec<Arc<FontFace>>,
    entries: Vec<Entry>,
    /// Entry indices by contour count, sorted by invariant.
    buckets: HashMap<usize, Vec<usize>>,
}

impl RetypeIndex {
    /// Index the outline glyphs of `faces` (the first [`MAX_FACES`], each face once) whose
    /// [`shape_key`] is in `keys` (the selected shapes' keys): the others could match no shape, so
    /// they are not sampled. Outlines are read past the shared outline cache, which keeps the
    /// canvas's glyphs.
    pub fn build(db: &FontDb, faces: impl IntoIterator<Item = Arc<FontFace>>, keys: &BTreeSet<u64>) -> Self {
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
        'faces: for (fi, face) in kept.iter().enumerate() {
            for &(gid, ch, key) in face_keys(db, face).iter() {
                // The budget holds inside a face too: no glyph is sampled past it.
                if entries.len() >= MAX_GLYPHS {
                    break 'faces;
                }
                if !keys.contains(&key) {
                    continue;
                }
                let outline = db.outline_uncached(face, gid);
                if let Some(sig) = signatures(&outline, false).into_iter().next() {
                    entries.push(Entry { face: fi, gid, ch, sig, outline });
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

    /// The glyphs `shape` (in document space) is: every glyph that fits it exactly, upright ones
    /// first, then the closest; empty when it is no glyph of the indexed faces. One shape can be
    /// several glyphs: an L, and a turned L (U+A780) turned half round; a u, and an n upside down.
    /// A glyph that looks the same turned (an I, an O, a hyphen) also fits in more than one pose:
    /// the upright-most comes first and the others are kept in [`GlyphMatch::alts`]. The line
    /// around the shape chooses among them. At most [`MAX_CANDIDATES`].
    pub fn matches(&self, shape: &BezPath) -> Vec<GlyphMatch> {
        let variants = signatures(shape, true);
        let Some(first) = variants.first() else { return vec![] };
        let Some(list) = self.buckets.get(&first.contours) else { return vec![] };
        let lo = first.invariant * (1.0 - INVARIANT_SLACK);
        let hi = first.invariant * (1.0 + INVARIANT_SLACK);
        let start = list.partition_point(|i| self.entries.get(*i).is_some_and(|e| e.sig.invariant < lo));
        let tol = first.spread * OUTLINE_TOLERANCE;
        // The shape flattened for the whole-outline check, once, when a glyph first gets that far.
        let mut shape_flat: Option<Option<Flat>> = None;
        let mut out: Vec<(usize, GlyphMatch)> = vec![];
        for i in list.iter().skip(start) {
            let Some(e) = self.entries.get(*i) else { continue };
            if e.sig.invariant > hi {
                break;
            }
            let Some(face) = self.faces.get(e.face) else { continue };
            // Each reading of the shape (its tied starting corners) that fits the glyph is a pose.
            let mut poses: Vec<(Pose, f64)> = vec![];
            for sig in &variants {
                let Some((z, residual)) = fit(&e.sig, sig) else { continue };
                if residual > TOLERANCE {
                    continue;
                }
                // The glyph's origin is (0, 0) in font units: it lands at centre − z·glyph centre.
                let gc = mul(z, e.sig.centre);
                let pose = Pose { scale: z.0.hypot(z.1), angle: z.1.atan2(z.0), origin: Point::new(sig.centre.x - gc.x, sig.centre.y - gc.y) };
                if poses.iter().any(|(p, _)| wrapped(p.angle - pose.angle).abs() < 1e-6) {
                    continue;
                }
                // The samples can step over a detail (a notch between two of them): the whole
                // outline, turned onto the shape, must lie on it.
                let flat = shape_flat.get_or_insert_with(|| Flat::new(shape, tol));
                let Some(flat) = flat.as_ref() else { break };
                let onto = Affine::new([z.0, z.1, -z.1, z.0, pose.origin.x, pose.origin.y]);
                if !Flat::new(&(onto * e.outline.clone()), tol).is_some_and(|g| g.agrees(flat)) {
                    continue;
                }
                poses.push((pose, residual));
            }
            // The upright-most pose first.
            poses.sort_by(|a, b| wrapped(a.0.angle).abs().total_cmp(&wrapped(b.0.angle).abs()));
            let Some(((pose, _), rest)) = poses.split_first() else { continue };
            let residual = poses.iter().map(|(_, r)| *r).fold(f64::INFINITY, f64::min);
            let m = GlyphMatch {
                face: face.clone(),
                gid: e.gid,
                ch: e.ch,
                scale: pose.scale,
                angle: pose.angle,
                origin: pose.origin,
                residual,
                contours: e.sig.contours,
                alts: rest.iter().map(|(p, _)| *p).collect(),
            };
            out.push((e.face, m));
        }
        let upright = |m: &GlyphMatch| wrapped(m.angle).abs() >= 1e-3;
        out.sort_by(|a, b| {
            upright(&a.1).cmp(&upright(&b.1)).then(a.1.residual.total_cmp(&b.1.residual)).then(a.0.cmp(&b.0)).then(a.1.ch.cmp(&b.1.ch))
        });
        out.truncate(MAX_CANDIDATES);
        out.into_iter().map(|(_, m)| m).collect()
    }
}

/// `a` wrapped into −π..π.
fn wrapped(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// A face's glyphs as the index looks them up: (glyph, character, [`shape_key`]), one per glyph
/// under the lowest character that maps to it.
type FaceKeys = Arc<Vec<(u32, char, u64)>>;

/// How many glyph keys [`face_keys`] keeps in all (16 bytes each).
const KEPT_KEYS: usize = 1 << 20;

/// The faces' keys [`face_keys`] keeps, for the database generation they were read in.
#[derive(Default)]
struct KeptKeys {
    generation: u64,
    glyphs: usize,
    faces: HashMap<(u64, u32), FaceKeys>,
}

/// [`FaceKeys`] of `face`, worked out once per face (each glyph drawn once, past the outline
/// cache) and kept: a later Retype samples only the glyphs whose key a selected shape has. Keys
/// already kept stay while new faces' keys would not fit, so a run over more faces than fit still
/// finds the same ones kept the next time; they are let go when fonts are added or rescanned.
fn face_keys(db: &FontDb, face: &FontFace) -> FaceKeys {
    static KEPT: std::sync::OnceLock<std::sync::Mutex<KeptKeys>> = std::sync::OnceLock::new();
    let kept = KEPT.get_or_init(Default::default);
    let at = (db.id(), face.id());
    let generation = db.generation();
    {
        let mut k = kept.lock().unwrap_or_else(|e| e.into_inner());
        if k.generation != generation {
            *k = KeptKeys { generation, ..Default::default() };
        }
        if let Some(keys) = k.faces.get(&at) {
            return keys.clone();
        }
    }
    let mut by_gid: HashMap<u32, char> = HashMap::new();
    for &(c, gid) in face.mapped_chars().iter().take(MAX_GLYPHS_PER_FACE) {
        if c.is_whitespace() || c.is_control() || is_shape_symbol(c) {
            continue;
        }
        by_gid.entry(gid).and_modify(|e| *e = (*e).min(c)).or_insert(c);
    }
    let mut gids: Vec<(u32, char)> = by_gid.into_iter().collect();
    gids.sort_unstable();
    let keys: FaceKeys = Arc::new(gids.into_iter().filter_map(|(gid, ch)| Some((gid, ch, shape_key(&db.outline_uncached(face, gid))?))).collect());
    let mut k = kept.lock().unwrap_or_else(|e| e.into_inner());
    if k.generation != generation {
        return keys;
    }
    // Another Retype may have worked them out meanwhile: keep and charge one list.
    if let Some(already) = k.faces.get(&at) {
        return already.clone();
    }
    if k.glyphs + keys.len() <= KEPT_KEYS {
        k.glyphs += keys.len();
        k.faces.insert(at, keys.clone());
    }
    keys
}

/// Characters whose glyphs are plain shapes (box drawing, blocks, geometric shapes, legacy
/// computing symbols) or a font's own icons (private use): a drawn rectangle or circle is the same
/// shape as one of them, so matching them would turn art into text. They are not matched.
fn is_shape_symbol(c: char) -> bool {
    matches!(
        u32::from(c),
        0x2500..=0x25FF | 0x2B00..=0x2BFF | 0x1F780..=0x1F7FF | 0x1FB00..=0x1FBFF | 0xE000..=0xF8FF | 0xF0000..=0x10FFFF
    )
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

/// An outline flattened for the whole-outline check: its segments, points along them (every
/// corner, and points twice `tol` apart between corners: a detail narrower than that has corners
/// of its own), and a uniform grid listing each segment under every cell its box, grown by `tol`,
/// touches, so whether a point lies within `tol` of the outline is asked of the few segments
/// around it.
struct Flat {
    segs: Vec<(Point, Point)>,
    pts: Vec<Point>,
    origin: Point,
    cell: f64,
    cols: usize,
    rows: usize,
    /// Segment indices per cell, row by row.
    cells: Vec<Vec<u32>>,
    tol: f64,
}

impl Flat {
    /// Cells along the longer side, at most.
    const MAX_SIDE: f64 = 48.0;
    /// Segment listings in all, at most: past it (or past [`MAX_OUTLINE_POINTS`] segments or
    /// [`MAX_CHECKED_POINTS`] points), `None`, and so no match.
    const MAX_LISTINGS: usize = 1 << 18;

    fn new(path: &BezPath, tol: f64) -> Option<Flat> {
        if !tol.is_finite() || tol <= 0.0 {
            return None;
        }
        let mut segs = vec![];
        let (mut start, mut last) = (Point::ZERO, Point::ZERO);
        let mut too_many = false;
        kurbo::flatten(path.iter(), tol / 2.0, |el| {
            if segs.len() > MAX_OUTLINE_POINTS {
                too_many = true;
                return;
            }
            match el {
                PathEl::MoveTo(q) => (start, last) = (q, q),
                PathEl::LineTo(q) => {
                    segs.push((last, q));
                    last = q;
                }
                PathEl::ClosePath => {
                    segs.push((last, start));
                    last = start;
                }
                _ => {}
            }
        });
        if too_many {
            return None;
        }
        let mut pts = vec![];
        for (s, e) in &segs {
            let steps = (s.distance(*e) / (2.0 * tol)).ceil();
            if !steps.is_finite() || pts.len() as f64 + steps > MAX_CHECKED_POINTS as f64 {
                return None;
            }
            let steps = (steps as usize).max(1);
            pts.extend((0..steps).map(|i| s.lerp(*e, i as f64 / steps as f64)));
        }
        let (first, _) = segs.first()?;
        let mut bb = kurbo::Rect::from_points(*first, *first);
        for (s, e) in &segs {
            bb = bb.union_pt(*s).union_pt(*e);
        }
        let bb = bb.inflate(tol, tol);
        let cell = (bb.width().max(bb.height()) / Self::MAX_SIDE).max(2.0 * tol);
        if !cell.is_finite() || cell <= 0.0 {
            return None;
        }
        let at = |v: f64, o: f64| (((v - o) / cell).floor().max(0.0) as usize).min(Self::MAX_SIDE as usize);
        let (cols, rows) = (at(bb.x1, bb.x0) + 1, at(bb.y1, bb.y0) + 1);
        let mut cells: Vec<Vec<u32>> = vec![vec![]; cols * rows];
        let mut listings = 0usize;
        for (i, (s, e)) in segs.iter().enumerate() {
            let r = kurbo::Rect::from_points(*s, *e).inflate(tol, tol);
            for cy in at(r.y0, bb.y0)..=at(r.y1, bb.y0).min(rows - 1) {
                for cx in at(r.x0, bb.x0)..=at(r.x1, bb.x0).min(cols - 1) {
                    listings += 1;
                    if listings > Self::MAX_LISTINGS {
                        return None;
                    }
                    if let Some(c) = cells.get_mut(cy * cols + cx) {
                        c.push(u32::try_from(i).ok()?);
                    }
                }
            }
        }
        Some(Flat { segs, pts, origin: Point::new(bb.x0, bb.y0), cell, cols, rows, cells, tol })
    }

    /// Does `p` lie within `tol` of a segment?
    fn near(&self, p: Point) -> bool {
        let (fx, fy) = ((p.x - self.origin.x) / self.cell, (p.y - self.origin.y) / self.cell);
        if !(fx >= 0.0 && fy >= 0.0) || fx >= self.cols as f64 || fy >= self.rows as f64 {
            return false;
        }
        let (cx, cy) = (fx as usize, fy as usize);
        let tol2 = self.tol * self.tol;
        self.cells.get(cy * self.cols + cx).is_some_and(|list| {
            list.iter().filter_map(|i| self.segs.get(*i as usize)).any(|(s, e)| Line::new(*s, *e).nearest(p, 1e-9).distance_sq <= tol2)
        })
    }

    /// Do the two outlines lie on each other: every point of each within `tol` of the other?
    fn agrees(&self, other: &Flat) -> bool {
        self.pts.iter().all(|p| other.near(*p)) && other.pts.iter().all(|p| self.near(*p))
    }
}

/// Do outlines `a` and `b` (in one space) lie on each other: every point of each, flattened, within
/// `tol` of the other? `false` when either is too large to check.
#[cfg(test)]
fn outlines_agree(a: &BezPath, b: &BezPath, tol: f64) -> bool {
    matches!((Flat::new(a, tol), Flat::new(b, tol)), (Some(fa), Some(fb)) if fa.agrees(&fb))
}

/// `seg` as a line when it is one (a curve with its handles on its ends or on its line), else as it is.
fn as_line(seg: PathSeg) -> PathSeg {
    let on_line = |a: Point, b: Point, p: Point| {
        let ab = b - a;
        let len = ab.hypot();
        len > 0.0 && (ab.cross(p - a)).abs() <= len * len * 1e-12 && (p - a).dot(ab) >= 0.0 && (p - b).dot(ab) <= 0.0
    };
    match seg {
        PathSeg::Quad(q) if q.p1 == q.p0 || q.p1 == q.p2 || on_line(q.p0, q.p2, q.p1) => PathSeg::Line(Line::new(q.p0, q.p2)),
        PathSeg::Cubic(c) if (c.p1 == c.p0 || on_line(c.p0, c.p3, c.p1)) && (c.p2 == c.p3 || on_line(c.p0, c.p3, c.p2)) => {
            PathSeg::Line(Line::new(c.p0, c.p3))
        }
        s => s,
    }
}

/// Lines `a` then `b` run on in one direction: one edge split in two.
fn runs_on(a: &PathSeg, b: &PathSeg) -> bool {
    let (PathSeg::Line(a), PathSeg::Line(b)) = (a, b) else { return false };
    let (u, v) = (a.p1 - a.p0, b.p1 - b.p0);
    let (lu, lv) = (u.hypot(), v.hypot());
    lu > 0.0 && lv > 0.0 && u.cross(v).abs() <= lu * lv * 1e-9 && u.dot(v) > 0.0
}

/// `path`'s contours as segments, the closing line included, read the same however the path was
/// written: segments that go nowhere are dropped, curves that are straight read as lines, and a
/// straight edge split in two (around the start too) reads as one. `None` past the size caps.
fn contours(path: &BezPath) -> Option<Vec<Vec<PathSeg>>> {
    if path.elements().len() > MAX_ELEMENTS {
        return None;
    }
    let mut out: Vec<Vec<PathSeg>> = vec![];
    let mut cur: Vec<PathSeg> = vec![];
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    let push = |cur: &mut Vec<PathSeg>, seg: PathSeg| {
        let seg = as_line(seg);
        if seg.start() == seg.end() && matches!(seg, PathSeg::Line(_)) {
            return;
        }
        match cur.last_mut() {
            Some(prev) if runs_on(prev, &seg) => *prev = PathSeg::Line(Line::new(prev.start(), seg.end())),
            _ => cur.push(seg),
        }
    };
    let flush = |cur: &mut Vec<PathSeg>, start: Point, last: Point, out: &mut Vec<Vec<PathSeg>>| {
        if last.distance(start) > 0.0 {
            push(cur, PathSeg::Line(Line::new(last, start)));
        }
        // An edge split at the contour's start: its two halves are the last and first segments.
        if cur.len() > 1
            && let (Some(l), Some(f)) = (cur.last().copied(), cur.first().copied())
            && runs_on(&l, &f)
        {
            cur.pop();
            if let Some(first) = cur.first_mut() {
                *first = PathSeg::Line(Line::new(l.start(), f.end()));
            }
        }
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for el in path.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                flush(&mut cur, start, last, &mut out);
                start = p;
                last = p;
            }
            PathEl::LineTo(p) => {
                push(&mut cur, PathSeg::Line(Line::new(last, p)));
                last = p;
            }
            PathEl::QuadTo(a, p) => {
                push(&mut cur, PathSeg::Quad(QuadBez::new(last, a, p)));
                last = p;
            }
            PathEl::CurveTo(a, b, p) => {
                push(&mut cur, PathSeg::Cubic(CubicBez::new(last, a, b, p)));
                last = p;
            }
            PathEl::ClosePath => {
                flush(&mut cur, start, last, &mut out);
                last = start;
            }
        }
        if out.len() > MAX_CONTOURS {
            return None;
        }
    }
    flush(&mut cur, start, last, &mut out);
    (out.len() <= MAX_CONTOURS).then_some(out)
}

/// A cheap key two shapes share when one is the other scaled, turned and moved: how many segments
/// each of its contours has, in order, as [`contours`] reads them, hashed. Glyphs whose key no
/// selected shape has are not sampled. `None` past the size caps or for an empty shape.
pub fn shape_key(path: &BezPath) -> Option<u64> {
    let cs = contours(path)?;
    if cs.is_empty() {
        return None;
    }
    // FNV-1a over the counts: equal keys for equal counts; a rare clash only samples a glyph more.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for n in std::iter::once(cs.len()).chain(cs.iter().map(Vec::len)) {
        for b in (n as u64).to_le_bytes() {
            h ^= u64::from(b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    Some(h)
}

/// `path` read for matching: one reading per choice of starting corners (the corner of each contour
/// farthest from the centre of all the shape's corners; every tied corner when `all_ties`, else the
/// first), at most [`MAX_VARIANTS`] tried. Empty for a shape with nothing to match.
fn signatures(path: &BezPath, all_ties: bool) -> Vec<Signature> {
    let bounds = path.bounding_box();
    let size = bounds.width().hypot(bounds.height());
    if !size.is_finite() || size <= 0.0 {
        return vec![];
    }
    let accuracy = size * 1e-6;
    let Some(cs) = contours(path) else { return vec![] };
    // Each contour with length, with its segments' lengths.
    let cs: Vec<(Vec<PathSeg>, Vec<f64>)> = cs
        .into_iter()
        .map(|c| {
            let lens: Vec<f64> = c.iter().map(|s| s.arclen(accuracy)).collect();
            (c, lens)
        })
        .filter(|(_, lens)| {
            let len: f64 = lens.iter().sum();
            len.is_finite() && len > size * 1e-9
        })
        .collect();
    let corners: Vec<Point> = cs.iter().flat_map(|(c, _)| c.iter().map(|s| s.start())).collect();
    if corners.is_empty() {
        return vec![];
    }
    let n = corners.len() as f64;
    let mid = Point::new(corners.iter().map(|p| p.x).sum::<f64>() / n, corners.iter().map(|p| p.y).sum::<f64>() / n);
    // Each contour's starting corners: those farthest from `mid`, ties within a hair.
    let starts: Vec<Vec<usize>> = cs
        .iter()
        .map(|(c, _)| {
            let d: Vec<f64> = c.iter().map(|s| s.start().distance(mid)).collect();
            let far = d.iter().copied().fold(0.0, f64::max);
            let tied: Vec<usize> = d.iter().enumerate().filter(|(_, v)| **v >= far - size * 1e-7).map(|(i, _)| i).collect();
            if all_ties { tied } else { tied.into_iter().take(1).collect() }
        })
        .collect();
    let mut out = vec![];
    let mut pick = vec![0usize; cs.len()];
    // Readings tried, kept or not: a shape whose readings all fail (its sums run past what f64
    // holds) tries no more than this either.
    let mut tried = 0usize;
    loop {
        if tried >= MAX_VARIANTS {
            break;
        }
        tried += 1;
        if let Some(sig) = sample(&cs, &starts, &pick, accuracy) {
            out.push(sig);
        }
        // The next choice of tied corners (a counter in mixed radix).
        let mut i = 0;
        loop {
            let Some((p, s)) = pick.get_mut(i).zip(starts.get(i)) else { return out };
            *p += 1;
            if *p < s.len() {
                break;
            }
            *p = 0;
            i += 1;
        }
    }
    out
}

/// One reading of the contours `cs`, contour `i` starting at segment `starts[i][pick[i]]`.
fn sample(cs: &[(Vec<PathSeg>, Vec<f64>)], starts: &[Vec<usize>], pick: &[usize], accuracy: f64) -> Option<Signature> {
    let mut pts = Vec::with_capacity(cs.len() * SAMPLES);
    let mut total = 0.0;
    for (((c, lens), s), p) in cs.iter().zip(starts).zip(pick) {
        let from = *s.get(*p)?;
        let m = c.len();
        let len: f64 = lens.iter().sum();
        total += len;
        let (mut k, mut before) = (0usize, 0.0);
        let at_k = |k: usize| (from + k) % m;
        for j in 0..SAMPLES {
            let at = len * j as f64 / SAMPLES as f64;
            while k + 1 < m && before + lens.get(at_k(k)).copied().unwrap_or(0.0) < at {
                before += lens.get(at_k(k)).copied().unwrap_or(0.0);
                k += 1;
            }
            let (Some(seg), Some(l)) = (c.get(at_k(k)), lens.get(at_k(k))) else { return None };
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
    if !spread.is_finite() || spread <= 0.0 || !total.is_finite() || total <= 0.0 {
        return None;
    }
    Some(Signature { contours: cs.len(), pts, centre, spread, invariant: spread / total })
}

/// The first reading of `path` (for tests).
#[cfg(test)]
fn signature(path: &BezPath) -> Option<Signature> {
    signatures(path, false).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn plain_shape_symbols_are_not_matched() {
        // A block element (U+1FB89) drew as a rectangle and took a drawn rectangle for it.
        for c in ['\u{1FB89}', '\u{25A0}', '\u{2588}', '\u{E000}'] {
            assert!(is_shape_symbol(c), "{c:?}");
        }
        for c in ['A', 'l', '-', '\u{6F22}', '\u{0915}'] {
            assert!(!is_shape_symbol(c), "{c:?}");
        }
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

    /// The best fit of `shape`'s readings onto `glyph`'s first one.
    fn best_fit(glyph: &BezPath, shape: &BezPath) -> f64 {
        let g = signature(glyph).unwrap();
        signatures(shape, true).iter().filter_map(|s| fit(&g, s)).map(|(_, r)| r).fold(f64::INFINITY, f64::min)
    }

    #[test]
    fn a_contour_started_at_another_corner_still_matches() {
        // The counter of letter(), started at its second corner instead of its first.
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((0.0, -700.0));
        p.curve_to((300.0, -700.0), (450.0, -600.0), (450.0, -480.0));
        p.quad_to((450.0, -300.0), (120.0, -300.0));
        p.line_to((120.0, 0.0));
        p.close_path();
        p.move_to((120.0, -400.0));
        p.quad_to((330.0, -400.0), (330.0, -500.0));
        p.quad_to((330.0, -600.0), (120.0, -600.0));
        p.line_to((120.0, -400.0));
        p.close_path();
        let shape = Affine::translate((40.0, 90.0)) * Affine::rotate(-1.1) * Affine::scale(0.03) * p;
        assert_eq!(shape_key(&shape), shape_key(&letter()));
        assert!(best_fit(&letter(), &shape) < 1e-6);
        // A triangle started at each of its corners.
        let tri = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| {
            let mut t = BezPath::new();
            t.move_to(a);
            t.line_to(b);
            t.line_to(c);
            t.close_path();
            t
        };
        let g = tri((0.0, 0.0), (300.0, 0.0), (0.0, -400.0));
        for s in [tri((300.0, 0.0), (0.0, -400.0), (0.0, 0.0)), tri((0.0, -400.0), (0.0, 0.0), (300.0, 0.0))] {
            assert!(best_fit(&g, &s) < 1e-6);
        }
    }

    #[test]
    fn an_edge_split_in_two_reads_as_one() {
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((0.0, -350.0));
        p.line_to((0.0, -700.0));
        p.curve_to((300.0, -700.0), (450.0, -600.0), (450.0, -480.0));
        p.quad_to((450.0, -300.0), (120.0, -300.0));
        p.line_to((120.0, 0.0));
        p.line_to((60.0, 0.0));
        p.close_path();
        p.move_to((120.0, -600.0));
        p.line_to((120.0, -400.0));
        p.quad_to((330.0, -400.0), (330.0, -500.0));
        p.quad_to((330.0, -600.0), (120.0, -600.0));
        p.close_path();
        assert_eq!(shape_key(&p), shape_key(&letter()), "split edges, the closing one too, read as one");
        assert!(best_fit(&letter(), &p) < 1e-6);
    }

    #[test]
    fn a_detail_between_samples_fails_the_whole_outline_check() {
        let bar = |notch: f64| {
            let mut p = BezPath::new();
            p.move_to((0.0, 0.0));
            p.line_to((1000.0, 0.0));
            p.line_to((1000.0, -100.0));
            // A notch 2 units wide, in or out, between two samples along the top edge.
            p.line_to((501.0, -100.0));
            p.line_to((501.0, -100.0 + notch));
            p.line_to((499.0, -100.0 + notch));
            p.line_to((499.0, -100.0));
            p.line_to((0.0, -100.0));
            p.close_path();
            p
        };
        let (out, inn) = (bar(-10.0), bar(10.0));
        assert_eq!(shape_key(&out), shape_key(&inn), "same key");
        let tol = signature(&inn).unwrap().spread * OUTLINE_TOLERANCE;
        assert!(!outlines_agree(&out, &inn, tol), "the whole outline does");
        assert!(outlines_agree(&out, &out, tol));
    }

    #[test]
    fn readings_that_all_fail_stop_at_the_cap() {
        // 48 concentric squares (four tied corners each: 4^48 choices) so large that every reading
        // overflows: the tries stop at the cap instead of running through the choices.
        let mut p = BezPath::new();
        for i in 1..=48 {
            let r = 1e153 * f64::from(i);
            p.move_to((-r, -r));
            p.line_to((r, -r));
            p.line_to((r, r));
            p.line_to((-r, r));
            p.close_path();
        }
        let t0 = std::time::Instant::now();
        assert!(signatures(&p, true).is_empty());
        assert!(t0.elapsed() < std::time::Duration::from_secs(5), "{:?}", t0.elapsed());
    }

    #[test]
    fn the_same_corners_joined_another_way_are_another_outline() {
        let poly = |pts: &[(f64, f64)]| {
            let mut p = BezPath::new();
            p.move_to(pts[0]);
            for q in &pts[1..] {
                p.line_to(*q);
            }
            p.close_path();
            p
        };
        let square = poly(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]);
        let bow = poly(&[(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)]);
        assert!(!outlines_agree(&square, &bow, 1.0), "every corner lies on the other, the edges do not");
        assert!(outlines_agree(&square, &square, 1.0));
    }
}
