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
//! in the font's order).
//!
//! Near matches ([`RetypeIndex::near_matches`]) take a tolerance: the shape and the glyph are
//! both read at equal steps of length from any start and in either direction, their contours paired
//! by length, so outlines built another way (a simplified outline, a curve split in two, contours
//! in another order or direction) still fit, and [`GlyphMatch::score`] says how closely.
//! [`clusters`] splits a path holding several glyphs (a word outlined as one compound path) into
//! the contour groups that can each be a glyph. Glyphs fused into one contour are not split.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use kurbo::{
    Affine, BezPath, CubicBez, Line, ParamCurve, ParamCurveArclen, ParamCurveArea, ParamCurveNearest, PathEl, PathSeg, Point, QuadBez, Shape,
};

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
/// The most glyphs one shape is reported to be ([`RetypeIndex::matches`]), and the most of one face
/// [`RetypeIndex::near_matches`] reports.
pub const MAX_CANDIDATES: usize = 16;
/// How far apart two shapes' spread-to-length ratios may be and still be compared.
const INVARIANT_SLACK: f64 = 0.02;
/// The largest tolerance [`RetypeIndex::near_matches`] takes, and the leftover distance at which
/// [`GlyphMatch::score`] reaches 0.
pub const MAX_TOLERANCE: f64 = 0.05;
/// Points sampled along each contour of a shape for near matching, per point of a glyph's.
const NEAR_OVERSAMPLE: usize = 4;
/// Shapes (and glyphs) with more contours than this are not near matched.
const MAX_NEAR_CONTOURS: usize = 8;
/// How far apart two shapes' spread-to-length ratios may be and still be near matched.
const NEAR_INVARIANT_SLACK: f64 = 0.1;
/// How far apart two shapes' area-to-squared-length ratios may be and still be near matched.
const NEAR_COMPACT_SLACK: f64 = 0.15;
/// Glyphs drawn to check whether they could near match a shape, at most.
const MAX_NEAR_CHECKS: usize = 262_144;
/// Glyphs indexed for near matching, at most.
const MAX_NEAR_GLYPHS: usize = 32_768;
/// Near keys [`RetypeIndex::build_near`] looks glyphs up by, at most: past them, the rest are left
/// out.
pub const MAX_NEAR_KEYS: usize = 1024;
/// Point comparisons near matching makes in all, over every shape of one index: past them, shapes
/// are not near matched.
const NEAR_WORK: u64 = 2_000_000_000;
/// Point comparisons near matching makes for one shape, at most.
const NEAR_SHAPE_WORK: u64 = 50_000_000;
/// Glyphs one shape is near matched against, at most: those closest to it in proportion.
const NEAR_FITS: usize = 256;
/// Contours [`clusters`] reads, at most: past it, none.
const MAX_CLUSTER_CONTOURS: usize = 1024;

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
    /// Whether the shape is exactly the glyph ([`RetypeIndex::matches`]) rather than nearly
    /// ([`RetypeIndex::near_matches`]).
    pub exact: bool,
    /// How many contours the glyph has.
    pub contours: usize,
    /// Other poses the glyph fits the shape in equally (a glyph that looks the same turned).
    pub alts: Vec<Pose>,
    /// The leftover distance of each of [`Self::alts`], in the same order.
    pub alt_residuals: Vec<f64>,
    /// The leftover distance of the glyph's own pose: [`Self::residual`] is the least over all its
    /// poses, which orders the matches.
    pub pose_residual: f64,
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
    /// How closely the glyph fits the shape: 1 for an exact match, and for a near one from 1 down
    /// to 0 at a leftover distance of [`MAX_TOLERANCE`] or more.
    pub fn score(&self) -> f64 {
        if self.exact {
            return 1.0;
        }
        if !self.residual.is_finite() {
            return 0.0;
        }
        (1.0 - self.residual / MAX_TOLERANCE).clamp(0.0, 1.0)
    }

    /// This match in each pose it fits: its own first, then [`Self::alts`].
    pub fn poses(&self) -> impl Iterator<Item = GlyphMatch> + '_ {
        let own = (Pose { scale: self.scale, angle: self.angle, origin: self.origin }, self.pose_residual);
        let alts = self.alts.iter().copied().zip(self.alt_residuals.iter().copied().chain(std::iter::repeat(self.residual)));
        std::iter::once(own).chain(alts).map(move |(p, residual)| GlyphMatch {
            scale: p.scale,
            angle: p.angle,
            origin: p.origin,
            residual,
            pose_residual: residual,
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
    /// Each contour's length.
    lens: Vec<f64>,
}

struct Entry {
    face: usize,
    gid: u32,
    ch: char,
    sig: Signature,
    /// The glyph's outline in font units, for the whole-outline check.
    outline: BezPath,
    /// Whether the glyph was indexed for its exact key: only these take part in exact matching,
    /// and the ones indexed only for near matching ([`RetypeIndex::near_matches`]) do not.
    exact: bool,
}

/// The glyphs of a set of faces, ready to match shapes against.
#[derive(Default)]
pub struct RetypeIndex {
    faces: Vec<Arc<FontFace>>,
    entries: Vec<Entry>,
    /// Entry indices by contour count, sorted by invariant.
    buckets: HashMap<usize, Vec<usize>>,
    /// Entry indices by face id and glyph id.
    by_glyph: HashMap<(u32, u32), usize>,
    /// The point comparisons [`Self::near_matches`] may still make, over every shape.
    near_work: AtomicU64,
}

impl RetypeIndex {
    /// Index the outline glyphs of `faces` (the first [`MAX_FACES`], each face once) whose
    /// [`shape_key`] is in `keys` (the selected shapes' keys): the others could match no shape, so
    /// they are not sampled. Outlines are read past the shared outline cache, which keeps the
    /// canvas's glyphs.
    pub fn build(db: &FontDb, faces: impl IntoIterator<Item = Arc<FontFace>>, keys: &BTreeSet<u64>) -> Self {
        Self::build_near(db, faces, keys, &[])
    }

    /// [`Self::build`], also indexing for [`Self::near_matches`] the glyphs that could near match
    /// a shape with one of the [`near_key`]s in `near` (the first [`MAX_NEAR_KEYS`]): the same
    /// number of contours (at most 8) and a close area-to-length ratio. At most 262,144 glyphs are
    /// drawn to check that, and at most 32,768 indexed for it, each face taking an equal share of
    /// both; of a face's glyphs, those closest in that ratio are indexed.
    pub fn build_near(db: &FontDb, faces: impl IntoIterator<Item = Arc<FontFace>>, keys: &BTreeSet<u64>, near: &[NearKey]) -> Self {
        Self::build_near_within(db, faces, keys, near, MAX_NEAR_GLYPHS)
    }

    /// [`Self::build_near`] indexing at most `near_glyphs` glyphs for near matching.
    fn build_near_within(
        db: &FontDb,
        faces: impl IntoIterator<Item = Arc<FontFace>>,
        keys: &BTreeSet<u64>,
        near: &[NearKey],
        near_glyphs: usize,
    ) -> Self {
        let near: Vec<NearKey> = near
            .iter()
            .copied()
            .filter(|k| k.contours <= MAX_NEAR_CONTOURS && k.compact.is_finite() && k.compact > 0.0)
            .take(MAX_NEAR_KEYS)
            .collect();
        let mut kept: Vec<Arc<FontFace>> = vec![];
        for f in faces {
            if kept.len() >= MAX_FACES {
                break;
            }
            if !kept.iter().any(|k| k.id() == f.id()) {
                kept.push(f);
            }
        }
        // Each face gets an equal share of the near checks and entries, so a face with many glyphs
        // (a CJK face) does not use them up before the faces after it.
        let share = |cap: usize| cap / kept.len().max(1);
        let mut entries = vec![];
        'faces: for (fi, face) in kept.iter().enumerate() {
            let mut face_checks = 0usize;
            // The face's glyphs that could near match, with how far their key is from the closest
            // wanted one: the closest share of them are indexed, whatever their glyph order.
            let keep = share(near_glyphs);
            let mut close: Vec<(f64, u32, char, BezPath)> = vec![];
            let trim = |close: &mut Vec<(f64, u32, char, BezPath)>| {
                close.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
                close.truncate(keep);
            };
            for &(gid, ch, key, count) in face_keys(db, face).iter() {
                // The budget holds inside a face too: no glyph is sampled past it.
                if entries.len() >= MAX_GLYPHS {
                    break 'faces;
                }
                if keys.contains(&key) {
                    let outline = db.outline_uncached(face, gid);
                    if let Some(sig) = signatures(&outline, false).into_iter().next() {
                        entries.push(Entry { face: fi, gid, ch, sig, outline, exact: true });
                    }
                    continue;
                }
                let count = usize::from(count);
                if keep == 0 || face_checks >= share(MAX_NEAR_CHECKS) || !near.iter().any(|k| k.contours == count) {
                    continue;
                }
                face_checks += 1;
                let outline = db.outline_uncached(face, gid);
                let Some(g) = near_key(&outline) else { continue };
                let Some(d) = near.iter().filter(|k| k.close_to(&g)).map(|k| k.apart(&g)).min_by(f64::total_cmp) else { continue };
                close.push((d, gid, ch, outline));
                if close.len() >= keep.saturating_mul(2) {
                    trim(&mut close);
                }
            }
            trim(&mut close);
            for (_, gid, ch, outline) in close {
                if entries.len() >= MAX_GLYPHS {
                    break 'faces;
                }
                if let Some(sig) = signatures(&outline, false).into_iter().next() {
                    entries.push(Entry { face: fi, gid, ch, sig, outline, exact: false });
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
        let by_glyph = entries.iter().enumerate().filter_map(|(i, e)| Some(((kept.get(e.face)?.id(), e.gid), i))).collect();
        RetypeIndex { faces: kept, entries, buckets, by_glyph, near_work: AtomicU64::new(NEAR_WORK) }
    }

    /// `m` kept to the poses in which the glyph paints the area `shape` paints, the shape filled
    /// by the nonzero rule when `nonzero` is set and by the even-odd rule otherwise, the glyph
    /// filled by the nonzero rule. Two outlines on each other can paint different areas: a ring
    /// whose inner contour runs the same way as its outer one, filled by the nonzero rule, has no
    /// hole. Probes lie on both sides of the shape's outline, left out where either outline is
    /// within reach of them. `tolerance` is the one `m` was found with (0 for an exact match). The
    /// first pose that agrees comes first; `None` when none does.
    pub fn painted_agrees(&self, m: &GlyphMatch, shape: &BezPath, nonzero: bool, tolerance: f64) -> Option<GlyphMatch> {
        let Some(e) = self.by_glyph.get(&(m.face.id(), m.gid)).and_then(|i| self.entries.get(*i)) else { return Some(m.clone()) };
        let bb = shape.bounding_box();
        let tol = bb.width().hypot(bb.height()) * 0.5 * OUTLINE_TOLERANCE.max(3.0 * tolerance.clamp(0.0, MAX_TOLERANCE));
        let mut agreeing = m.poses().filter(|p| {
            let onto = Affine::new([
                p.scale * p.angle.cos(),
                p.scale * p.angle.sin(),
                -p.scale * p.angle.sin(),
                p.scale * p.angle.cos(),
                p.origin.x,
                p.origin.y,
            ]);
            paints_alike(shape, nonzero, &(onto * e.outline.clone()), tol)
        });
        let first = agreeing.next()?;
        let rest: Vec<GlyphMatch> = agreeing.collect();
        Some(GlyphMatch {
            alts: rest.iter().map(|p| Pose { scale: p.scale, angle: p.angle, origin: p.origin }).collect(),
            alt_residuals: rest.iter().map(|p| p.pose_residual).collect(),
            ..first
        })
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
        let mut out = self.all_matches(shape);
        out.truncate(MAX_CANDIDATES);
        out
    }

    /// Every glyph `shape` fits exactly, in every face, in the order of [`RetypeIndex::matches`],
    /// which returns the first [`MAX_CANDIDATES`] of them.
    pub fn all_matches(&self, shape: &BezPath) -> Vec<GlyphMatch> {
        self.matches_read(shape, shape)
    }

    /// [`Self::all_matches`], and when the shape fits no glyph as it is, its contours read the
    /// other way round: for a part of a compound path, which Make Compound Path may have turned so
    /// it cuts a hole in the paths behind it. When neither reading fits, the contours are read the
    /// way their depth gives, outer contours one way and their counters the other, both ways round:
    /// an even-odd compound paints the same whichever way each contour runs, so an o's counter may
    /// run the same way as its outer contour.
    pub fn matches_either_way(&self, shape: &BezPath) -> Vec<GlyphMatch> {
        let found = self.matches_read(shape, shape);
        if !found.is_empty() {
            return found;
        }
        let found = self.matches_read(shape, &shape.reverse_subpaths());
        if !found.is_empty() {
            return found;
        }
        let Some(nested) = nested_reading(shape) else { return vec![] };
        let found = self.matches_read(shape, &nested);
        if !found.is_empty() {
            return found;
        }
        self.matches_read(shape, &nested.reverse_subpaths())
    }

    /// [`Self::all_matches`] of `shape`, its contours read as in `reading` (the same outline).
    fn matches_read(&self, shape: &BezPath, reading: &BezPath) -> Vec<GlyphMatch> {
        let variants = signatures(reading, true);
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
            if !e.exact {
                continue;
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
            let Some(((pose, own_residual), rest)) = poses.split_first() else { continue };
            let residual = poses.iter().map(|(_, r)| *r).fold(f64::INFINITY, f64::min);
            let m = GlyphMatch {
                face: face.clone(),
                gid: e.gid,
                ch: e.ch,
                scale: pose.scale,
                angle: pose.angle,
                origin: pose.origin,
                residual,
                exact: true,
                contours: e.sig.contours,
                alts: rest.iter().map(|(p, _)| *p).collect(),
                alt_residuals: rest.iter().map(|(_, r)| *r).collect(),
                pose_residual: *own_residual,
            };
            out.push((e.face, m));
        }
        let upright = |m: &GlyphMatch| wrapped(m.angle).abs() >= 1e-3;
        out.sort_by(|a, b| {
            upright(&a.1).cmp(&upright(&b.1)).then(a.1.residual.total_cmp(&b.1.residual)).then(a.0.cmp(&b.0)).then(a.1.ch.cmp(&b.1.ch))
        });
        out.into_iter().map(|(_, m)| m).collect()
    }

    /// The glyphs `shape` (in document space) nearly is: every glyph of the index with as many
    /// contours whose best fit leaves a leftover distance of at most `tolerance` (relative to the
    /// shape's spread, at most [`MAX_TOLERANCE`]) and whose whole outline, turned onto the shape,
    /// lies within three times the tolerance of it (at least 0.5%). The shape and the glyph are
    /// read from any start, in either direction, with their contours paired by length, so the
    /// shape need not be built as the glyph is. Upright ones first, then the closest; the other
    /// turns a glyph fits in (at most 7) are kept in [`GlyphMatch::alts`]; at most
    /// [`MAX_CANDIDATES`] for each face. Empty for a tolerance of 0 or less, and for shapes of more
    /// than 8 contours. A shape is compared with at most the 256 glyphs closest to it in proportion
    /// (the spread-to-length ratio, and the share of the length each contour has), closest first,
    /// in at most 50 million point comparisons, and the index's calls make at most 2 billion in
    /// all (ranking a glyph by proportion counts as one per contour and one more); once they are
    /// spent, the glyphs not yet ranked or compared are left out.
    pub fn near_matches(&self, shape: &BezPath, tolerance: f64) -> Vec<GlyphMatch> {
        // NaN too.
        if tolerance.is_nan() || tolerance <= 0.0 {
            return vec![];
        }
        let tolerance = tolerance.min(MAX_TOLERANCE);
        // With no comparisons left, no shape is sampled or ranked.
        if self.near_work.load(Ordering::Acquire) == 0 {
            return vec![];
        }
        let Some(s) = near_signature(shape) else { return vec![] };
        if !self.buckets.contains_key(&s.contours) {
            return vec![];
        }
        let tol = s.spread * OUTLINE_TOLERANCE.max(3.0 * tolerance);
        let mut shape_flat: Option<Option<Flat>> = None;
        let mut out: Vec<(usize, GlyphMatch)> = vec![];
        // This call's allowance comes out of the index's budget in one step, so calls that overlap
        // never spend the same comparisons; what it leaves unspent goes back afterward.
        let mut seen = self.near_work.load(Ordering::Acquire);
        let mut work = loop {
            let take = seen.min(NEAR_SHAPE_WORK);
            match self.near_work.compare_exchange_weak(seen, seen - take, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => break take,
                Err(now) => seen = now,
            }
        };
        let near = self.ranked_near(&s, &mut work);
        for (_, e) in near {
            if work == 0 {
                break;
            }
            let Some(face) = self.faces.get(e.face) else { continue };
            let fits = near_fit(&e.sig, &s, tolerance, &mut work);
            let flat = shape_flat.get_or_insert_with(|| Flat::new(shape, tol));
            let Some(flat) = flat.as_ref() else { break };
            // Every turn the glyph fits the shape at, nearest upright first: the first is its
            // pose, the others its alternatives (a glyph that looks alike turned).
            let mut poses =
                fits.into_iter().filter(|f| f.1 <= tolerance).filter_map(|(z, r)| Some((near_pose(e, &s, z, flat, tol)?, r))).take(MAX_NEAR_POSES);
            let Some((pose, residual)) = poses.next() else { continue };
            let rest: Vec<(Pose, f64)> = poses.collect();
            let m = GlyphMatch {
                face: face.clone(),
                gid: e.gid,
                ch: e.ch,
                scale: pose.scale,
                angle: pose.angle,
                origin: pose.origin,
                residual,
                exact: false,
                contours: e.sig.contours,
                alts: rest.iter().map(|(p, _)| *p).collect(),
                alt_residuals: rest.iter().map(|(_, r)| *r).collect(),
                pose_residual: residual,
            };
            out.push((e.face, m));
        }
        self.near_work.fetch_add(work, Ordering::AcqRel);
        let upright = |m: &GlyphMatch| wrapped(m.angle).abs() >= 1e-3;
        out.sort_by(|a, b| {
            upright(&a.1).cmp(&upright(&b.1)).then(a.1.residual.total_cmp(&b.1.residual)).then(a.0.cmp(&b.0)).then(a.1.ch.cmp(&b.1.ch))
        });
        keep_per_face(&mut out);
        out.into_iter().map(|(_, m)| m).collect()
    }

    /// The glyphs of as many contours as near-matching shape `s` that are closest to it in
    /// proportion, at most [`NEAR_FITS`], closest first: their spread-to-length ratio, and the
    /// share of the length each contour has, longest first. Ranking a glyph takes one comparison
    /// per contour, and one more, from `work`; once it is spent, the glyphs not yet ranked are
    /// left out.
    fn ranked_near(&self, s: &Signature, work: &mut u64) -> Vec<(f64, &Entry)> {
        let Some(list) = self.buckets.get(&s.contours) else { return vec![] };
        let lo = s.invariant * (1.0 - NEAR_INVARIANT_SLACK);
        let hi = s.invariant * (1.0 + NEAR_INVARIANT_SLACK);
        let start = list.partition_point(|i| self.entries.get(*i).is_some_and(|e| e.sig.invariant < lo));
        // Each contour's share of the length, longest first (a near-matched shape has at most
        // MAX_NEAR_CONTOURS contours, and the glyphs ranked as many).
        let shares = |sig: &Signature| {
            let mut v = [0.0f64; MAX_NEAR_CONTOURS];
            let total: f64 = sig.lens.iter().sum();
            for (o, l) in v.iter_mut().zip(&sig.lens) {
                *o = l / total;
            }
            v.sort_by(|a, b| b.total_cmp(a));
            v
        };
        let own = shares(s);
        let apart = |e: &Entry| {
            let d = (e.sig.invariant - s.invariant).abs() / s.invariant + shares(&e.sig).iter().zip(&own).map(|(a, b)| (a - b).abs()).sum::<f64>();
            if d.is_finite() { d } else { f64::INFINITY }
        };
        let cost = s.contours as u64 + 1;
        let mut near: Vec<(f64, &Entry)> = vec![];
        for e in list.iter().skip(start).filter_map(|i| self.entries.get(*i)).take_while(|e| e.sig.invariant <= hi) {
            let Some(left) = work.checked_sub(cost) else { break };
            *work = left;
            near.push((apart(e), e));
        }
        if near.len() > NEAR_FITS {
            near.select_nth_unstable_by(NEAR_FITS, |a, b| a.0.total_cmp(&b.0));
            near.truncate(NEAR_FITS);
        }
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        near
    }
}

/// Turns of one glyph [`RetypeIndex::near_matches`] keeps, at most: its pose and its alternatives.
const MAX_NEAR_POSES: usize = 8;

/// `out` (each match with its face's index) without the matches past the first
/// [`MAX_CANDIDATES`] of each face, so faces drawn alike (copies of one font under other names)
/// do not crowd out the glyphs of the faces after them.
fn keep_per_face(out: &mut Vec<(usize, GlyphMatch)>) {
    let mut seen: HashMap<usize, usize> = HashMap::new();
    out.retain(|(face, _)| {
        let n = seen.entry(*face).or_default();
        *n += 1;
        *n <= MAX_CANDIDATES
    });
}

/// Entry `e`'s pose on near-matching shape `s` at scaled turn `z`, when its whole outline, so
/// placed, lies within `tol` of the shape's (`flat`).
fn near_pose(e: &Entry, s: &Signature, z: (f64, f64), flat: &Flat, tol: f64) -> Option<Pose> {
    let gc = mul(z, e.sig.centre);
    let origin = Point::new(s.centre.x - gc.x, s.centre.y - gc.y);
    let onto = Affine::new([z.0, z.1, -z.1, z.0, origin.x, origin.y]);
    Flat::new(&(onto * e.outline.clone()), tol).is_some_and(|g| g.agrees(flat)).then(|| Pose { scale: z.0.hypot(z.1), angle: z.1.atan2(z.0), origin })
}

/// What near matching compares first: a shape's number of contours and its area over its squared
/// length, both unchanged by scale, turn and move, and close for outlines drawn alike.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NearKey {
    pub contours: usize,
    pub compact: f64,
}

impl NearKey {
    /// Could shapes with these keys near match?
    fn close_to(&self, other: &NearKey) -> bool {
        self.contours == other.contours && self.apart(other) <= NEAR_COMPACT_SLACK
    }

    /// How far apart the two keys' area-to-squared-length ratios are, relative to the larger.
    fn apart(&self, other: &NearKey) -> f64 {
        (self.compact - other.compact).abs() / self.compact.max(other.compact)
    }
}

/// `path`'s [`NearKey`]; `None` past the size caps, for an empty shape or one of more than 8
/// contours.
pub fn near_key(path: &BezPath) -> Option<NearKey> {
    let (cs, _, _) = measured(path)?;
    if cs.is_empty() || cs.len() > MAX_NEAR_CONTOURS {
        return None;
    }
    let (mut area, mut len) = (0.0, 0.0);
    for (c, lens) in &cs {
        area += c.iter().map(|s| s.signed_area()).sum::<f64>().abs();
        len += lens.iter().sum::<f64>();
    }
    let compact = area / (len * len);
    (compact.is_finite() && compact > 0.0).then_some(NearKey { contours: cs.len(), compact })
}

/// `path` read for near matching: points at equal steps of length along each contour from where
/// it starts, [`NEAR_OVERSAMPLE`] times as many as a glyph's. `None` for more than 8 contours.
fn near_signature(path: &BezPath) -> Option<Signature> {
    let (cs, _, accuracy) = measured(path)?;
    if cs.is_empty() || cs.len() > MAX_NEAR_CONTOURS {
        return None;
    }
    let starts = vec![vec![0usize]; cs.len()];
    let pick = vec![0usize; cs.len()];
    sample(&cs, &starts, &pick, accuracy, SAMPLES * NEAR_OVERSAMPLE)
}

/// The scaled turn that best takes glyph `g`'s points onto near-matching shape `s`'s (read by
/// [`near_signature`]), whatever contour each starts at, in either direction and order, and the
/// leftover distance relative to the shape's spread. The longest contours are paired first and give
/// the turns to try (spread over the circle when that pair fits about as well at many). Under each
/// turn, every glyph contour takes the shape contour, start and direction closest to it, and the
/// turn is fitted again over them all, for up to [`REFITS`] rounds. The turns that fit about
/// equally well (a glyph that looks alike turned) and within `limit` are kept, the one nearest
/// upright first ([`fits_within`]). Each point comparison is taken from `work`; none when it runs
/// out before the first turn is fitted, and the turns fitted so far when it runs out after.
fn near_fit(g: &Signature, s: &Signature, limit: f64, work: &mut u64) -> Vec<((f64, f64), f64)> {
    fits_within(near_turns(g, s, work).unwrap_or_default(), limit)
}

/// The turns [`near_fit`] fits, each with its leftover distance.
fn near_turns(g: &Signature, s: &Signature, work: &mut u64) -> Option<Vec<((f64, f64), f64)>> {
    let nc = g.contours;
    let m = SAMPLES * NEAR_OVERSAMPLE;
    if nc == 0 || s.contours != nc || g.pts.len() != nc * SAMPLES || s.pts.len() != nc * m || g.lens.len() != nc || s.lens.len() != nc {
        return None;
    }
    // Comparisons the longest pair's every start and direction take, and one round of pairing
    // every contour and fitting the turn.
    let first_cost = (4 * m * SAMPLES) as u64;
    let round_cost = ((nc * (nc + 1) * m + 20 * nc) * SAMPLES) as u64;
    let Some(left) = work.checked_sub(first_cost) else {
        *work = 0;
        return None;
    };
    *work = left;
    let by_len = |lens: &[f64]| {
        let mut o: Vec<usize> = (0..lens.len()).collect();
        o.sort_by(|a, b| lens.get(*b).copied().unwrap_or(0.0).total_cmp(&lens.get(*a).copied().unwrap_or(0.0)));
        o
    };
    let (go, so) = (by_len(&g.lens), by_len(&s.lens));
    let (&ga, &sa) = (go.first()?, so.first()?);
    let gpts = |c: usize| g.pts.get(c * SAMPLES..(c + 1) * SAMPLES);
    let spts = |c: usize| s.pts.get(c * m..(c + 1) * m);
    // The point of shape contour `b` matched to glyph sample j, from start k (in samples, and
    // between two samples read on the line joining them), forward or back.
    let at = |b: &[Point], k: f64, j: usize, back: bool| {
        let step = (j * NEAR_OVERSAMPLE) as f64;
        let x = (if back { k - step } else { k + step }).rem_euclid(m as f64);
        let i = (x.floor() as usize).min(m - 1);
        let (p, q) = (b.get(i).copied().unwrap_or(Point::ZERO), b.get((i + 1) % m).copied().unwrap_or(Point::ZERO));
        p.lerp(q, x - i as f64)
    };
    // The fit of glyph contour `a` onto shape contour `b` from start k: Σ b·conj(a) and Σ|a|².
    let sums = |a: &[Point], b: &[Point], k: f64, back: bool| {
        let (mut re, mut im, mut aa) = (0.0, 0.0, 0.0);
        for (j, p) in a.iter().enumerate() {
            let q = at(b, k, j, back);
            re += q.x * p.x + q.y * p.y;
            im += q.y * p.x - q.x * p.y;
            aa += p.x * p.x + p.y * p.y;
        }
        (re, im, aa)
    };
    let sse = |a: &[Point], b: &[Point], k: f64, back: bool, z: (f64, f64)| {
        a.iter().enumerate().map(|(j, p)| at(b, k, j, back).distance_squared(mul(z, *p))).sum::<f64>()
    };
    // Every start and direction of the longest pair, with its own best turn.
    let (a0, b0) = (gpts(ga)?, spts(sa)?);
    let mut firsts: Vec<(f64, (f64, f64))> = vec![];
    for back in [false, true] {
        for k in 0..m {
            let (re, im, aa) = sums(a0, b0, k as f64, back);
            if aa <= 0.0 || !aa.is_finite() {
                return None;
            }
            let z = (re / aa, im / aa);
            firsts.push((sse(a0, b0, k as f64, back, z), z));
        }
    }
    firsts.sort_by(|a, b| a.0.total_cmp(&b.0));
    let best = firsts.first()?.0;
    // The turns that fit the longest pair about as well as the best. A longest contour that is
    // nearly round fits at every angle, and only the other contours tell the turns apart, so
    // besides the turns nearest upright, the best in each of TURN_BINS equal ranges of angle is
    // tried, at most MAX_TURNS of them, evenly by angle.
    const TURN_BINS: usize = 64;
    const MAX_TURNS: usize = 24;
    let window = best * 1.5 + 1e-12 * s.spread * s.spread;
    let mut bins: Vec<Option<(f64, (f64, f64))>> = vec![None; TURN_BINS];
    for &(e, z) in firsts.iter().take_while(|(e, _)| *e <= window) {
        let ang = z.1.atan2(z.0);
        if !ang.is_finite() {
            continue;
        }
        let b = (((ang + std::f64::consts::PI) / std::f64::consts::TAU * TURN_BINS as f64) as usize).min(TURN_BINS - 1);
        if let Some(slot) = bins.get_mut(b)
            && slot.is_none_or(|(old, _)| e < old)
        {
            *slot = Some((e, z));
        }
    }
    // First the best turn and up to five others nearest upright, told apart by their angle, then
    // the spread ones.
    let mut turns: Vec<(f64, f64)> = firsts.first().map(|f| f.1).into_iter().collect();
    let mut upright: Vec<(f64, f64)> = firsts.iter().take_while(|(e, _)| *e <= window).map(|(_, z)| *z).collect();
    upright.sort_by(|a, b| wrapped(a.1.atan2(a.0)).abs().total_cmp(&wrapped(b.1.atan2(b.0)).abs()));
    for z in upright {
        if turns.len() >= 6 {
            break;
        }
        let ang = z.1.atan2(z.0);
        if turns.iter().all(|t| wrapped(t.1.atan2(t.0) - ang).abs() > 0.05) {
            turns.push(z);
        }
    }
    // One contour: every start of it was tried above, and its own fit is final.
    if nc > 1 {
        let spread: Vec<(f64, f64)> = bins.into_iter().flatten().map(|(_, z)| z).collect();
        let step = spread.len().div_ceil(MAX_TURNS).max(1);
        turns.extend(spread.into_iter().step_by(step));
    }
    let rounds = if nc > 1 { REFITS } else { 1 };
    let n = (nc * SAMPLES) as f64;
    // Each glyph contour, longest first, takes the shape contour, start and direction that fit it
    // best under turn `z`; then the turn is fitted over every pair. The new turn and the leftover
    // distance; `None` inside for a fit with nothing to go on, and `None` for malformed readings.
    let pair_and_fit = |z: (f64, f64)| -> Option<Option<((f64, f64), f64)>> {
        let mut free: Vec<usize> = so.clone();
        let mut pairs: Vec<(usize, usize, f64, bool)> = vec![];
        for &gc in &go {
            let a = gpts(gc)?;
            let mut pick: Option<(f64, usize, f64, bool)> = None;
            for (fi, &sc) in free.iter().enumerate() {
                let b = spts(sc)?;
                for back in [false, true] {
                    for k in 0..m {
                        let e = sse(a, b, k as f64, back, z);
                        if pick.is_none_or(|p| e < p.0) {
                            pick = Some((e, fi, k as f64, back));
                        }
                    }
                }
            }
            let (mut e, fi, mut k, back) = pick?;
            let sc = free.remove(fi);
            // The start between samples: the shape's samples need not fall where the glyph's do.
            let b = spts(sc)?;
            let k0 = k;
            for d in 1..16 {
                let kd = k0 - 0.5 + d as f64 / 16.0;
                let ed = sse(a, b, kd, back, z);
                if ed < e {
                    (e, k) = (ed, kd);
                }
            }
            pairs.push((gc, sc, k, back));
        }
        let (mut re, mut im, mut aa) = (0.0, 0.0, 0.0);
        for &(gc, sc, k, back) in &pairs {
            let (r, i, a) = sums(gpts(gc)?, spts(sc)?, k, back);
            re += r;
            im += i;
            aa += a;
        }
        if aa <= 0.0 || !aa.is_finite() {
            return Some(None);
        }
        let z = (re / aa, im / aa);
        let mut total = 0.0;
        for &(gc, sc, k, back) in &pairs {
            total += sse(gpts(gc)?, spts(sc)?, k, back, z);
        }
        let rel = (total / n).sqrt() / s.spread;
        Some(rel.is_finite().then_some((z, rel)))
    };
    let mut results: Vec<((f64, f64), f64)> = vec![];
    'turns: for z0 in turns {
        // Each glyph contour is paired under the turn, the turn is fitted again over the pairs,
        // and the contours are paired again under the new turn, until it settles: a turn that
        // starts a little off (a round longest contour) moves onto the one the other contours set.
        let mut z = z0;
        let mut fitted: Option<((f64, f64), f64)> = None;
        for _ in 0..rounds {
            let Some(left) = work.checked_sub(round_cost) else {
                *work = 0;
                results.extend(fitted);
                break 'turns;
            };
            *work = left;
            let Some((nz, rel)) = pair_and_fit(z)? else { break };
            let settled = (nz.0 - z.0).hypot(nz.1 - z.1) <= 1e-6 * z.0.hypot(z.1);
            if fitted.is_none_or(|f| rel <= f.1) {
                fitted = Some((nz, rel));
            }
            z = nz;
            if settled {
                break;
            }
        }
        results.extend(fitted);
    }
    Some(results)
}

/// Of fitted turns and their leftover distances, those that fit about as well as the best (within
/// 1.5 times its distance) and within `limit`, nearest upright first, one per turn (turns less
/// than 0.001 apart are one); the best one alone when none is within `limit`.
fn fits_within(results: Vec<((f64, f64), f64)>, limit: f64) -> Vec<((f64, f64), f64)> {
    let best = results.iter().map(|r| r.1).fold(f64::INFINITY, f64::min);
    let cap = (best * 1.5 + 1e-9).min(limit.max(best));
    let turn = |r: &((f64, f64), f64)| r.0.1.atan2(r.0.0);
    let mut kept: Vec<((f64, f64), f64)> = results.into_iter().filter(|r| r.1 <= cap).collect();
    kept.sort_by(|a, b| wrapped(turn(a)).abs().total_cmp(&wrapped(turn(b)).abs()).then(a.1.total_cmp(&b.1)));
    let mut out: Vec<((f64, f64), f64)> = vec![];
    for r in kept {
        if out.iter().all(|o| wrapped(turn(o) - turn(&r)).abs() >= 1e-3) {
            out.push(r);
        }
    }
    out
}

/// Rounds of pairing and fitting [`near_fit`] gives each turn, at most.
const REFITS: usize = 4;

/// Anchor points of a contour [`clusters`] tests against a candidate container, at most.
const MAX_CONTAINMENT_PROBES: usize = 8;

/// `path`'s contours in groups that can each be one glyph: a contour with the contours that lie
/// inside it (a letter's counters, and what lies inside those), in the order the outer contours
/// come. A path of several glyphs (a word outlined as one compound path) gives one group per glyph
/// part (an i gives two: its stem and its dot). Empty past 1,024 contours.
pub fn clusters(path: &BezPath) -> Vec<BezPath> {
    let mut subs: Vec<BezPath> = vec![];
    for el in path.elements() {
        match el {
            PathEl::MoveTo(_) => {
                if subs.len() >= MAX_CLUSTER_CONTOURS {
                    return vec![];
                }
                subs.push(BezPath::new());
                if let Some(s) = subs.last_mut() {
                    s.push(*el);
                }
            }
            _ => match subs.last_mut() {
                Some(s) => s.push(*el),
                None => return vec![],
            },
        }
    }
    let parent = containers(&subs);
    let root = |mut i: usize| {
        // Containers are strictly larger, so the chain ends; the bound keeps it so.
        for _ in 0..subs.len() {
            match parent.get(i).copied().flatten() {
                Some(p) => i = p,
                None => break,
            }
        }
        i
    };
    let mut groups: Vec<(usize, BezPath)> = vec![];
    for (i, s) in subs.iter().enumerate() {
        let r = root(i);
        match groups.iter_mut().find(|(g, _)| *g == r) {
            Some((_, p)) => p.extend(s.iter()),
            None => groups.push((r, s.clone())),
        }
    }
    groups.sort_by_key(|(r, _)| *r);
    groups.into_iter().map(|(_, p)| p).collect()
}

/// Each contour's container among `subs` (one contour each): the smallest other contour that holds
/// it, `None` for an outer contour.
fn containers(subs: &[BezPath]) -> Vec<Option<usize>> {
    let boxes: Vec<kurbo::Rect> = subs.iter().map(Shape::bounding_box).collect();
    // Up to MAX_CONTAINMENT_PROBES of each contour's anchor points, evenly spread: a contour lies in
    // another only when every one of them does.
    let probes: Vec<Vec<Point>> = subs
        .iter()
        .map(|s| {
            let anchors: Vec<Point> = s.elements().iter().filter_map(|e| e.end_point()).collect();
            let step = anchors.len().div_ceil(MAX_CONTAINMENT_PROBES).max(1);
            anchors.into_iter().step_by(step).collect()
        })
        .collect();
    // Each contour's container: the smallest contour around it.
    (0..subs.len())
        .map(|i| {
            let (Some(bi), Some(pi)) = (boxes.get(i), probes.get(i)) else { return None };
            if pi.is_empty() {
                return None;
            }
            (0..subs.len())
                .filter(|&j| j != i)
                .filter_map(|j| Some((j, boxes.get(j)?, subs.get(j)?)))
                .filter(|(_, bj, sj)| {
                    bj.area() > bi.area()
                        && bj.x0 <= bi.x0
                        && bj.y0 <= bi.y0
                        && bj.x1 >= bi.x1
                        && bj.y1 >= bi.y1
                        && pi.iter().all(|p| sj.winding(*p) != 0)
                })
                .min_by(|a, b| a.1.area().total_cmp(&b.1.area()))
                .map(|(j, _, _)| j)
        })
        .collect()
}

/// `path` with each contour turned to run the way its depth gives (an outer contour one way, its
/// counters the other, what lies in those the first way again), the outer contours keeping the way
/// the first of them runs. `None` when that turns no contour (the path read as it is), for a
/// contour that encloses no area, and for more than [`MAX_CONTOURS`] contours.
fn nested_reading(path: &BezPath) -> Option<BezPath> {
    let mut subs: Vec<BezPath> = vec![];
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            if subs.len() >= MAX_CONTOURS {
                return None;
            }
            subs.push(BezPath::new());
        }
        subs.last_mut()?.push(*el);
    }
    let parent = containers(&subs);
    let depth = |mut i: usize| {
        let mut d = 0usize;
        // Containers are strictly larger, so the chain ends; the bound keeps it so.
        for _ in 0..subs.len() {
            match parent.get(i).copied().flatten() {
                Some(p) => {
                    i = p;
                    d += 1;
                }
                None => break,
            }
        }
        d
    };
    let areas: Vec<f64> = subs.iter().map(Shape::area).collect();
    if areas.iter().any(|a| !a.is_normal()) {
        return None;
    }
    let outer = (0..subs.len()).find(|&i| depth(i) == 0).and_then(|i| areas.get(i)).map(|a| a.signum())?;
    let turned: Vec<bool> = areas.iter().enumerate().map(|(i, a)| a.signum() != if depth(i) % 2 == 0 { outer } else { -outer }).collect();
    if turned.iter().all(|t| !*t) {
        return None;
    }
    let mut out = BezPath::new();
    for (s, t) in subs.iter().zip(&turned) {
        out.extend(if *t { s.reverse_subpaths() } else { s.clone() }.iter());
    }
    Some(out)
}

/// `a` wrapped into −π..π.
fn wrapped(a: f64) -> f64 {
    (a + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

/// A face's glyphs as the index looks them up: (glyph, character, [`shape_key`], contours), one per
/// glyph under the lowest character that maps to it.
type FaceKeys = Arc<Vec<(u32, char, u64, u16)>>;

/// How many glyph keys [`face_keys`] keeps in all (24 bytes each).
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
    let keys: FaceKeys = Arc::new(
        gids.into_iter()
            .filter_map(|(gid, ch)| {
                let (key, count) = key_and_count(&db.outline_uncached(face, gid))?;
                Some((gid, ch, key, u16::try_from(count).ok()?))
            })
            .collect(),
    );
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

/// Shape segments [`paints_alike`] probes beside, at most.
const MAX_FILL_PROBES: usize = 512;

/// Do `shape` (filled by the nonzero rule when `nonzero` is set, else by the even-odd rule) and
/// `glyph` (filled by the nonzero rule), whose outlines lie within `tol` of each other, paint the
/// same area? Each probe lies `1.5 tol` or `3 tol` to a side of the middle of a segment of the shape's
/// outline; one within `tol` of either outline is skipped, so a region narrower than that is not
/// probed. `false` when a counter of the shape (a contour inside another) is narrower than `3 tol`
/// across in some direction, however it is turned, since the probes cannot then tell whether the
/// area inside it is painted, and `false` when an outline is too large to probe.
fn paints_alike(shape: &BezPath, nonzero: bool, glyph: &BezPath, tol: f64) -> bool {
    if narrow_contour(shape, 3.0 * tol) {
        return false;
    }
    let (Some(fs), Some(fg)) = (Flat::new(shape, tol), Flat::new(glyph, tol)) else { return false };
    let step = fs.segs.len().div_ceil(MAX_FILL_PROBES).max(1);
    for (s, e) in fs.segs.iter().step_by(step) {
        let d = *e - *s;
        let len = d.hypot();
        if len <= 0.0 || !len.is_finite() {
            continue;
        }
        let mid = s.midpoint(*e);
        // The nearer probes reach counters and gaps too narrow for the farther ones.
        for reach in [1.5, 3.0] {
            let side = kurbo::Vec2::new(-d.y, d.x) * (reach * tol / len);
            for q in [mid + side, mid - side] {
                if fs.near(q) || fg.near(q) {
                    continue;
                }
                let w = shape.winding(q);
                let painted = if nonzero { w != 0 } else { w.rem_euclid(2) == 1 };
                if painted != (glyph.winding(q) != 0) {
                    return false;
                }
            }
        }
    }
    true
}

/// The least width of the convex hull of `path` over every direction it could be measured in, its
/// points flattened to within `tol`; 0 when the hull has fewer than three corners and when the
/// path flattens to more than [`MAX_OUTLINE_POINTS`] points.
fn hull_width(path: &BezPath, tol: f64) -> f64 {
    let mut pts: Vec<Point> = vec![];
    let mut too_many = false;
    kurbo::flatten(path.iter(), tol, |el| {
        if let PathEl::MoveTo(q) | PathEl::LineTo(q) = el {
            if pts.len() >= MAX_OUTLINE_POINTS {
                too_many = true;
            } else {
                pts.push(q);
            }
        }
    });
    if too_many {
        return 0.0;
    }
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    // Points a thousandth of `tol` apart are one point: their chord has no direction to speak of.
    let eps = tol * 1e-3;
    pts.dedup_by(|b, a| a.distance(*b) <= eps);
    let cross = |o: Point, a: Point, b: Point| (a - o).cross(b - o);
    let mut hull: Vec<Point> = vec![];
    for pass in 0..2 {
        let from = hull.len();
        let ordered: Vec<Point> = if pass == 0 { pts.clone() } else { pts.iter().rev().copied().collect() };
        for p in ordered {
            while hull.len() >= from + 2 && hull.get(hull.len() - 2).zip(hull.last()).is_some_and(|(a, b)| cross(*a, *b, p) <= 0.0) {
                hull.pop();
            }
            hull.push(p);
        }
        hull.pop();
    }
    if hull.len() < 3 {
        return 0.0;
    }
    // Rotating calipers: the point farthest from each edge only moves forward from one edge to the
    // next, so the whole walk visits each hull point a bounded number of times.
    let n = hull.len();
    let mut least = f64::INFINITY;
    let mut j = 1;
    for (i, a) in hull.iter().enumerate() {
        let Some(b) = hull.get((i + 1) % n) else { continue };
        let edge = *b - *a;
        let len = edge.hypot();
        if len <= 0.0 || !len.is_finite() {
            continue;
        }
        let away = |k: usize| hull.get(k % n).map_or(0.0, |p| (edge.cross(*p - *a) / len).abs());
        let mut steps = 0;
        while steps < n && away(j + 1) >= away(j) {
            j += 1;
            steps += 1;
        }
        least = least.min(away(j));
        j %= n;
    }
    if least.is_finite() { least } else { 0.0 }
}

/// Does `path` have a contour inside another contour (a counter) whose convex hull is no wider
/// than `width` in some direction?
fn narrow_contour(path: &BezPath, width: f64) -> bool {
    if !width.is_finite() || width <= 0.0 {
        return false;
    }
    let mut all: Vec<BezPath> = vec![];
    for el in path.elements() {
        if matches!(el, PathEl::MoveTo(_)) {
            all.push(BezPath::new());
        }
        if let Some(cur) = all.last_mut() {
            cur.push(*el);
        }
    }
    all.iter().enumerate().any(|(i, c)| {
        let Some(PathEl::MoveTo(at)) = c.elements().first().copied() else { return false };
        all.iter().enumerate().any(|(j, o)| j != i && o.winding(at) != 0) && hull_width(c, width / 30.0) <= width
    })
}

/// Do `parts` (the runs of contours a compound path `shape` is split into, each already checked
/// by [`RetypeIndex::painted_agrees`] on its own) paint together the area `shape` paints, the
/// shape filled by the nonzero rule when `nonzero` is set and by the even-odd rule otherwise? Each
/// part is read by the same rule, and the glyphs that replace them are set one beside another, so
/// an overlap that the shape's rule cancels would come out painted. Probes lie on both sides of
/// every part's outline, as in [`RetypeIndex::painted_agrees`]; `tolerance` is the one the parts
/// were found with (0 for exact matches). `false` when an outline is too large to probe, and when
/// two parts' boxes overlap in a strip no wider than `3 tol`, which the probes cannot see into.
pub fn parts_paint_alike(shape: &BezPath, parts: &[BezPath], nonzero: bool, tolerance: f64) -> bool {
    let bb = shape.bounding_box();
    let tol = bb.width().hypot(bb.height()) * 0.5 * OUTLINE_TOLERANCE.max(3.0 * tolerance.clamp(0.0, MAX_TOLERANCE));
    let painted = |p: &BezPath, q: Point| {
        let w = p.winding(q);
        if nonzero { w != 0 } else { w.rem_euclid(2) == 1 }
    };
    let mut flats = vec![];
    for p in parts.iter().chain(std::iter::once(shape)) {
        let Some(f) = Flat::new(p, tol) else { return false };
        flats.push(f);
    }
    // Two parts whose boxes overlap in a strip too thin to probe cannot be told from parts that
    // overlap in their paint.
    let boxes: Vec<kurbo::Rect> = parts.iter().map(Shape::bounding_box).collect();
    for (i, a) in boxes.iter().enumerate() {
        for b in boxes.iter().skip(i + 1) {
            let both = a.intersect(*b);
            if both.area() > 0.0 && both.width().min(both.height()) <= 3.0 * tol {
                return false;
            }
        }
    }
    for own in flats.iter().take(parts.len()) {
        let step = own.segs.len().div_ceil(MAX_FILL_PROBES).max(1);
        for (s, e) in own.segs.iter().step_by(step) {
            let d = *e - *s;
            let len = d.hypot();
            if len <= 0.0 || !len.is_finite() {
                continue;
            }
            let mid = s.midpoint(*e);
            for reach in [1.5, 3.0] {
                let side = kurbo::Vec2::new(-d.y, d.x) * (reach * tol / len);
                for q in [mid + side, mid - side] {
                    if flats.iter().any(|f| f.near(q)) {
                        continue;
                    }
                    if painted(shape, q) != parts.iter().any(|o| painted(o, q)) {
                        return false;
                    }
                }
            }
        }
    }
    true
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
    key_and_count(path).map(|(k, _)| k)
}

/// [`shape_key`] and the number of contours it counts.
fn key_and_count(path: &BezPath) -> Option<(u64, usize)> {
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
    Some((h, cs.len()))
}

/// `path` read for matching: one reading per choice of starting corners (the corner of each contour
/// farthest from the centre of all the shape's corners; every tied corner when `all_ties`, else the
/// first), at most [`MAX_VARIANTS`] tried. Empty for a shape with nothing to match.
fn signatures(path: &BezPath, all_ties: bool) -> Vec<Signature> {
    let Some((cs, size, accuracy)) = measured(path) else { return vec![] };
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
        if let Some(sig) = sample(&cs, &starts, &pick, accuracy, SAMPLES) {
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

/// `path`'s contours as [`contours`] reads them, each with its segments' lengths, leaving out
/// contours of no length, the shape's size (its bounding box's diagonal) and the accuracy lengths
/// are measured to (a millionth of the size). `None` past the size caps and for a shape of no size.
#[allow(clippy::type_complexity)]
fn measured(path: &BezPath) -> Option<(Vec<(Vec<PathSeg>, Vec<f64>)>, f64, f64)> {
    let bounds = path.bounding_box();
    let size = bounds.width().hypot(bounds.height());
    if !size.is_finite() || size <= 0.0 {
        return None;
    }
    let accuracy = size * 1e-6;
    let cs = contours(path)?
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
    Some((cs, size, accuracy))
}

/// One reading of the contours `cs`, contour `i` starting at segment `starts[i][pick[i]]`, `n`
/// points along each.
fn sample(cs: &[(Vec<PathSeg>, Vec<f64>)], starts: &[Vec<usize>], pick: &[usize], accuracy: f64, n: usize) -> Option<Signature> {
    let mut pts = Vec::with_capacity(cs.len() * n);
    let mut lens_out = Vec::with_capacity(cs.len());
    let mut total = 0.0;
    for (((c, lens), s), p) in cs.iter().zip(starts).zip(pick) {
        let from = *s.get(*p)?;
        let m = c.len();
        let len: f64 = lens.iter().sum();
        total += len;
        lens_out.push(len);
        let (mut k, mut before) = (0usize, 0.0);
        let at_k = |k: usize| (from + k) % m;
        for j in 0..n {
            let at = len * j as f64 / n as f64;
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
    Some(Signature { contours: cs.len(), pts, centre, spread, invariant: spread / total, lens: lens_out })
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

    /// `p` flattened into lines `tol` from its curves, each contour started at its second point and
    /// run backwards: a simplified outline built nothing like the original.
    fn simplified(p: &BezPath, tol: f64) -> BezPath {
        let mut contours: Vec<Vec<Point>> = vec![];
        kurbo::flatten(p.iter(), tol, |el| match el {
            PathEl::MoveTo(q) => contours.push(vec![q]),
            PathEl::LineTo(q) => {
                if let Some(c) = contours.last_mut() {
                    c.push(q);
                }
            }
            _ => {}
        });
        let mut out = BezPath::new();
        for mut c in contours {
            c.rotate_left(1);
            c.reverse();
            out.move_to(c[0]);
            for q in &c[1..] {
                out.line_to(*q);
            }
            out.close_path();
        }
        out
    }

    fn near_residual(glyph: &BezPath, shape: &BezPath) -> f64 {
        near_fit_all(&signature(glyph).unwrap(), &near_signature(shape).unwrap()).map_or(f64::INFINITY, |(_, r)| r)
    }

    /// [`near_fit`]'s first fit, with no limit on its work.
    fn near_fit_all(g: &Signature, s: &Signature) -> Option<((f64, f64), f64)> {
        let mut work = u64::MAX;
        near_fit(g, s, MAX_TOLERANCE, &mut work).into_iter().next()
    }

    /// [`fits_within`]'s first fit.
    fn nearest_upright(results: Vec<((f64, f64), f64)>, limit: f64) -> Option<((f64, f64), f64)> {
        fits_within(results, limit).into_iter().next()
    }

    #[test]
    fn a_simplified_outline_near_matches_and_another_shape_does_not() {
        let xf = Affine::translate((300.0, 200.0)) * Affine::rotate(0.4) * Affine::scale(0.024);
        let shape = xf * simplified(&letter(), 2.0);
        assert_ne!(shape_key(&shape), shape_key(&letter()), "built another way: the exact index never samples it");
        assert!(best_fit(&letter(), &shape) > TOLERANCE, "no exact match");
        let r = near_residual(&letter(), &shape);
        assert!(r < 0.005, "near match {r}");
        let (z, _) = near_fit_all(&signature(&letter()).unwrap(), &near_signature(&shape).unwrap()).unwrap();
        // With too little work left, no fit, and nothing left after.
        let mut work = 1000;
        assert!(near_fit(&signature(&letter()).unwrap(), &near_signature(&shape).unwrap(), MAX_TOLERANCE, &mut work).is_empty());
        assert_eq!(work, 0);
        assert!((z.0.hypot(z.1) - 0.024).abs() < 0.024 * 0.01 && (z.1.atan2(z.0) - 0.4).abs() < 0.01, "{z:?}");
        let (a, b) = (near_key(&letter()).unwrap(), near_key(&shape).unwrap());
        assert!(a.close_to(&b), "{a:?} {b:?}");
        // The exact reading of the letter itself near matches it too, with next to nothing left over.
        assert!(near_residual(&letter(), &letter()) < 1e-3);
        // A box is not near a circle.
        let mut o = BezPath::new();
        o.extend(kurbo::Circle::new((200.0, -350.0), 300.0).path_elements(0.1));
        let mut box_ = BezPath::new();
        box_.extend(kurbo::Rect::new(0.0, -700.0, 600.0, 0.0).path_elements(0.1));
        assert!(near_residual(&o, &box_) > MAX_TOLERANCE);
        assert!(!near_key(&o).unwrap().close_to(&near_key(&box_).unwrap()));
    }

    #[test]
    fn a_turned_round_glyph_near_matches_upright() {
        // A circle fits at every turn: the turn kept is the upright one.
        let mut o = BezPath::new();
        o.extend(kurbo::Circle::new((0.0, 0.0), 300.0).path_elements(0.1));
        let (z, r) = near_fit_all(&signature(&o).unwrap(), &near_signature(&simplified(&o, 1.0)).unwrap()).unwrap();
        assert!(r < 0.005, "{r}");
        assert!(z.1.atan2(z.0).abs() < 0.06, "{z:?}");
    }

    #[test]
    fn the_upright_turn_is_kept_only_within_the_limit() {
        let turned = |a: f64, r: f64| ((a.cos(), a.sin()), r);
        // The best fit is turned a quarter round; an upright one fits nearly as well.
        let fits = || vec![turned(std::f64::consts::FRAC_PI_2, 0.010), turned(0.0, 0.014)];
        let angle = |f: Option<((f64, f64), f64)>| f.map(|((c, s), _)| s.atan2(c));
        assert_eq!(angle(nearest_upright(fits(), 0.02)), Some(0.0), "both within the limit: the upright one");
        let kept = nearest_upright(fits(), 0.012).unwrap();
        assert!((kept.1 - 0.010).abs() < 1e-12, "only the best is within the limit: the best {kept:?}");
        let kept = nearest_upright(fits(), 0.005).unwrap();
        assert!((kept.1 - 0.010).abs() < 1e-12, "none within the limit: the best {kept:?}");
        assert!(nearest_upright(vec![], 0.05).is_none());
        // Every turn within the limit is kept, upright first, once each.
        let all = fits_within(
            vec![turned(std::f64::consts::PI, 0.011), turned(std::f64::consts::FRAC_PI_2, 0.010), turned(0.0, 0.014), turned(1e-4, 0.0141)],
            0.02,
        );
        let angles: Vec<f64> = all.iter().map(|((c, s), _)| s.atan2(*c)).collect();
        assert_eq!(angles.len(), 3, "{angles:?}");
        assert!(angles[0] == 0.0 && (angles[1] - std::f64::consts::FRAC_PI_2).abs() < 1e-12, "{angles:?}");
    }

    #[test]
    fn a_curve_split_in_two_still_near_matches() {
        // letter() with its cubic split at its middle: exactly another shape, nearly the same.
        let (a, b) = kurbo::CubicBez::new((0.0, -700.0), (300.0, -700.0), (450.0, -600.0), (450.0, -480.0)).subdivide();
        let mut p = BezPath::new();
        p.move_to((0.0, 0.0));
        p.line_to((0.0, -700.0));
        p.curve_to(a.p1, a.p2, a.p3);
        p.curve_to(b.p1, b.p2, b.p3);
        p.quad_to((450.0, -300.0), (120.0, -300.0));
        p.line_to((120.0, 0.0));
        p.close_path();
        p.move_to((120.0, -600.0));
        p.line_to((120.0, -400.0));
        p.quad_to((330.0, -400.0), (330.0, -500.0));
        p.quad_to((330.0, -600.0), (120.0, -600.0));
        p.close_path();
        assert_ne!(shape_key(&p), shape_key(&letter()));
        let shape = Affine::translate((40.0, 90.0)) * Affine::scale(0.03) * p;
        assert!(near_residual(&letter(), &shape) < 0.005);
    }

    #[test]
    fn contours_in_another_order_still_near_match() {
        // letter() with its counter first, simplified.
        let l = letter();
        let els = l.elements();
        let second = els.iter().rposition(|e| matches!(e, PathEl::MoveTo(_))).unwrap();
        let mut back = BezPath::new();
        back.extend(els[second..].iter().copied());
        back.extend(els[..second].iter().copied());
        let shape = Affine::translate((40.0, 90.0)) * Affine::rotate(0.3) * Affine::scale(0.03) * simplified(&back, 2.0);
        assert!(near_residual(&letter(), &shape) < 0.005);
    }

    #[test]
    fn copies_of_a_font_do_not_crowd_out_the_faces_after_them() {
        use crate::test_fonts::{HEIGHT_FAMILY, HEIGHT_OUTSIDE_FAMILY, HEIGHTLESS_FAMILY, NUMBERLESS_FAMILY};
        // Source Sans 3 Regular and four copies of it under other names: each face has several
        // glyphs drawn as the period (. · ∙ ․), more than MAX_CANDIDATES in all.
        let db = FontDb::with_font_dirs(vec![]);
        for bytes in [
            crate::test_fonts::height_font(true),
            crate::test_fonts::height_font(false),
            crate::test_fonts::height_outside_font(),
            crate::test_fonts::numberless_font(),
        ] {
            db.add_font(bytes.unwrap());
        }
        let faces: Vec<Arc<FontFace>> = ["Source Sans 3", HEIGHT_FAMILY, HEIGHTLESS_FAMILY, HEIGHT_OUTSIDE_FAMILY, NUMBERLESS_FAMILY]
            .iter()
            .map(|f| db.face(f, "Regular").unwrap())
            .collect();
        let gid = faces[0].mapped_chars().iter().find(|(c, _)| *c == '.').map(|(_, g)| *g).unwrap();
        let period = Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * db.outline_uncached(&faces[0], gid);
        let has_every_period = |found: &[GlyphMatch]| faces.iter().all(|f| found.iter().any(|m| m.ch == '.' && m.face.id() == f.id()));
        let keys: BTreeSet<u64> = shape_key(&period).into_iter().collect();
        let index = RetypeIndex::build(&db, faces.clone(), &keys);
        // The parts of a split compound path are looked up in every face, as whole shapes are.
        let found = index.matches_either_way(&period);
        assert!(found.len() > MAX_CANDIDATES && has_every_period(&found), "exact: {} matches", found.len());
        let simple = simplified(&period, 0.05);
        let index = RetypeIndex::build_near(&db, faces.clone(), &BTreeSet::new(), &[near_key(&simple).unwrap()]);
        let found = index.near_matches(&simple, 0.02);
        assert!(found.len() > MAX_CANDIDATES && has_every_period(&found), "near: {} matches", found.len());
    }

    #[test]
    fn a_face_past_its_near_share_keeps_the_glyphs_closest_to_the_shapes() {
        // One glyph of the face is indexed for near matching: the one closest to a simplified o,
        // whatever comes before the o in glyph order.
        let db = FontDb::global();
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let gid = face.mapped_chars().iter().find(|(c, _)| *c == 'o').map(|(_, g)| *g).unwrap();
        let shape = Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * simplified(&db.outline_uncached(&face, gid), 4.0);
        let key = near_key(&shape).unwrap();
        let index = RetypeIndex::build_near_within(db, [face], &BTreeSet::new(), &[key], 1);
        assert_eq!(index.len(), 1);
        let found: Vec<char> = index.near_matches(&shape, 0.02).iter().map(|m| m.ch).collect();
        // The Latin o or the Greek omicron, drawn the same.
        assert!(found.iter().any(|c| matches!(c, 'o' | 'ο')), "{found:?}");
    }

    /// `p` with its first curve split in two at its middle: the same outline with another key.
    fn first_curve_split(p: &BezPath) -> BezPath {
        let mut out = BezPath::new();
        let (mut last, mut done) = (Point::ZERO, false);
        for el in p.elements() {
            match *el {
                PathEl::CurveTo(a, b, c) if !done => {
                    let (x, y) = kurbo::CubicBez::new(last, a, b, c).subdivide();
                    out.curve_to(x.p1, x.p2, x.p3);
                    out.curve_to(y.p1, y.p2, y.p3);
                    done = true;
                    last = c;
                }
                PathEl::QuadTo(a, c) if !done => {
                    let (x, y) = kurbo::QuadBez::new(last, a, c).subdivide();
                    out.quad_to(x.p1, x.p2);
                    out.quad_to(y.p1, y.p2);
                    done = true;
                    last = c;
                }
                e => {
                    out.push(e);
                    if let PathEl::MoveTo(q) | PathEl::LineTo(q) | PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q) = e {
                        last = q;
                    }
                }
            }
        }
        out
    }

    #[test]
    fn a_glyph_indexed_only_for_near_matching_is_no_exact_match() {
        let db = FontDb::global();
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let gid = face.mapped_chars().iter().find(|(c, _)| *c == 'o').map(|(_, g)| *g).unwrap();
        let split = first_curve_split(&db.outline_uncached(&face, gid));
        let shape = Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * split;
        assert_ne!(shape_key(&shape), shape_key(&(Affine::scale(0.04) * db.outline_uncached(&face, gid))), "another key");
        let keys: BTreeSet<u64> = shape_key(&shape).into_iter().collect();
        let index = RetypeIndex::build_near(db, [face], &keys, &[near_key(&shape).unwrap()]);
        assert!(index.all_matches(&shape).is_empty(), "exact matching sees only the glyphs of the exact keys");
        assert!(index.matches_either_way(&shape).is_empty());
        assert!(index.near_matches(&shape, 0.02).iter().any(|m| m.ch == 'o'));
    }

    #[test]
    fn each_pose_of_a_match_reports_its_own_residual() {
        let db = FontDb::global();
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let turn = std::f64::consts::FRAC_PI_2;
        let m = GlyphMatch {
            face,
            gid: 1,
            ch: 'o',
            scale: 1.0,
            angle: 0.0,
            origin: Point::ZERO,
            residual: 0.010,
            exact: false,
            contours: 1,
            alts: vec![Pose { scale: 1.0, angle: turn, origin: Point::ZERO }],
            alt_residuals: vec![0.010],
            pose_residual: 0.014,
        };
        let seen: Vec<(f64, f64)> = m.poses().map(|p| (p.angle, p.residual)).collect();
        assert_eq!(seen, vec![(0.0, 0.014), (turn, 0.010)]);
    }

    #[test]
    fn a_ring_whose_contours_run_one_way_paints_no_hole() {
        let db = FontDb::global();
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let gid = face.mapped_chars().iter().find(|(c, _)| *c == 'o').map(|(_, g)| *g).unwrap();
        let ring = db.outline_uncached(&face, gid);
        let els = ring.elements();
        let second = els.iter().rposition(|e| matches!(e, PathEl::MoveTo(_))).unwrap();
        let (mut outer, mut inner) = (BezPath::new(), BezPath::new());
        outer.extend(els[..second].iter().copied());
        inner.extend(els[second..].iter().copied());
        let mut same_way = outer.clone();
        same_way.extend(inner.reverse_subpaths().iter());
        let (ring, same_way) =
            (Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * ring, Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * same_way);
        let keys: BTreeSet<u64> = shape_key(&ring).into_iter().collect();
        let index = RetypeIndex::build_near(db, [face], &keys, &[near_key(&same_way).unwrap()]);
        // Near matching reads each contour in either direction, so the outline matches the glyph's:
        // only what it paints tells them apart.
        let found = index.near_matches(&same_way, 0.02);
        assert!(found.iter().any(|m| m.ch == 'o'), "the outline matches");
        assert!(found.iter().all(|m| index.painted_agrees(m, &same_way, true, 0.02).is_none()), "nonzero paints the counter");
        assert!(found.iter().any(|m| index.painted_agrees(m, &same_way, false, 0.02).is_some()), "even-odd leaves it empty");
        let found = index.all_matches(&ring);
        assert!(found.iter().any(|m| index.painted_agrees(m, &ring, true, 0.0).is_some()), "the glyph's own windings paint a ring");
    }

    #[test]
    fn a_counter_too_narrow_to_probe_does_not_count_as_agreeing() {
        let square = |x0: f64, y0: f64, x1: f64, y1: f64, back: bool| {
            let p = kurbo::Rect::new(x0, y0, x1, y1).to_path(0.1);
            if back { p.reverse_subpaths() } else { p }
        };
        // A 100 by 100 square with a 10 by 10 counter: the shape's counter runs the way its outer
        // contour does (filled by the nonzero rule), the glyph's runs the other way (a hole).
        let mut filled = square(0.0, 0.0, 100.0, 100.0, false);
        filled.extend(square(45.0, 45.0, 55.0, 55.0, false).iter());
        let mut holed = square(0.0, 0.0, 100.0, 100.0, false);
        holed.extend(square(45.0, 45.0, 55.0, 55.0, true).iter());
        let tol = 100.0_f64.hypot(100.0) * 0.5 * 3.0 * MAX_TOLERANCE;
        assert!(!paints_alike(&filled, true, &holed, tol), "no probe can see the counter");
        // A counter wide enough to probe is told apart as before.
        let mut wide = square(0.0, 0.0, 100.0, 100.0, false);
        wide.extend(square(10.0, 10.0, 90.0, 90.0, false).iter());
        let mut wide_hole = square(0.0, 0.0, 100.0, 100.0, false);
        wide_hole.extend(square(10.0, 10.0, 90.0, 90.0, true).iter());
        assert!(!paints_alike(&wide, true, &wide_hole, 1.0));
        assert!(paints_alike(&wide_hole, true, &wide_hole, 1.0));
    }

    #[test]
    fn a_counter_turned_too_narrow_to_probe_does_not_count_as_agreeing() {
        let square = |x0: f64, y0: f64, x1: f64, y1: f64, back: bool| {
            let p = kurbo::Rect::new(x0, y0, x1, y1).to_path(0.1);
            if back { p.reverse_subpaths() } else { p }
        };
        // A 60 by 10 counter turned 45 degrees: its box is about 49.5 square, wider than the
        // threshold of about 31.8, but it is 10 across.
        let strip = |back: bool| {
            let turn = Affine::translate((50.0, 50.0)) * Affine::rotate(std::f64::consts::FRAC_PI_4);
            let mut p = kurbo::Rect::new(-30.0, -5.0, 30.0, 5.0).to_path(0.1);
            if back {
                p = p.reverse_subpaths();
            }
            turn * p
        };
        let mut filled = square(0.0, 0.0, 100.0, 100.0, false);
        filled.extend(strip(false).iter());
        let mut holed = square(0.0, 0.0, 100.0, 100.0, false);
        holed.extend(strip(true).iter());
        let tol = 100.0_f64.hypot(100.0) * 0.5 * 3.0 * MAX_TOLERANCE;
        assert!(narrow_contour(&filled, 3.0 * tol));
        assert!(!paints_alike(&filled, true, &holed, tol));
        // The same strip with its sides along the axes is told apart as before.
        let mut level = square(0.0, 0.0, 100.0, 100.0, false);
        level.extend(square(20.0, 45.0, 80.0, 55.0, false).iter());
        assert!(narrow_contour(&level, 3.0 * tol));
        // A counter wider than the threshold in every direction is not narrow.
        let mut wide = square(0.0, 0.0, 100.0, 100.0, false);
        wide.extend(square(10.0, 10.0, 90.0, 90.0, false).iter());
        assert!(!narrow_contour(&wide, 3.0 * tol));
    }

    #[test]
    fn a_contour_reaching_outside_a_concave_one_is_no_counter_of_it() {
        // An L, and a triangle that starts inside the L and inside its box but reaches outside it.
        let mut l = BezPath::new();
        for (i, (x, y)) in [(0.0, 0.0), (100.0, 0.0), (100.0, 20.0), (20.0, 20.0), (20.0, 100.0), (0.0, 100.0)].into_iter().enumerate() {
            if i == 0 { l.move_to((x, y)) } else { l.line_to((x, y)) }
        }
        l.close_path();
        let mut tri = BezPath::new();
        tri.move_to((5.0, 5.0));
        tri.line_to((90.0, 90.0));
        tri.line_to((5.0, 50.0));
        tri.close_path();
        let mut both = l.clone();
        both.extend(tri.iter());
        assert_eq!(clusters(&both).len(), 2);
        // A contour wholly inside stays with its container.
        let mut inside = l.clone();
        inside.extend(kurbo::Rect::new(5.0, 5.0, 15.0, 15.0).to_path(0.1).iter());
        assert_eq!(clusters(&inside).len(), 1);
    }

    #[test]
    fn the_hull_width_is_the_least_width_in_any_direction() {
        let strip = kurbo::Rect::new(-30.0, -5.0, 30.0, 5.0).to_path(0.1);
        for deg in [0.0_f64, 17.0, 45.0, 90.0, 133.0] {
            let w = hull_width(&(Affine::rotate(deg.to_radians()) * strip.clone()), 0.01);
            assert!((w - 10.0).abs() < 0.05, "{deg} degrees: {w}");
        }
        let circle = kurbo::Circle::new((5.0, 7.0), 50.0).to_path(0.1);
        assert!((hull_width(&circle, 0.01) - 100.0).abs() < 0.5, "{}", hull_width(&circle, 0.01));
        // A triangle's least width is its shortest height.
        let mut tri = BezPath::new();
        tri.move_to((0.0, 0.0));
        tri.line_to((40.0, 0.0));
        tri.line_to((0.0, 30.0));
        tri.close_path();
        assert!((hull_width(&tri, 0.01) - 24.0).abs() < 1e-9);
        assert_eq!(hull_width(&BezPath::new(), 0.01), 0.0);
    }

    #[test]
    fn an_outline_too_large_to_probe_does_not_count_as_agreeing() {
        let n = MAX_OUTLINE_POINTS + 64;
        let mut many = BezPath::new();
        for i in 0..n {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            let p = Point::new(50.0 + 50.0 * a.cos(), 50.0 + 50.0 * a.sin());
            if i == 0 { many.move_to(p) } else { many.line_to(p) }
        }
        many.close_path();
        assert!(Flat::new(&many, 0.5).is_none(), "too many segments to flatten");
        assert!(!paints_alike(&many, true, &many, 0.5));
        assert!(paints_alike(&kurbo::Circle::new((50.0, 50.0), 50.0).to_path(0.1), true, &kurbo::Circle::new((50.0, 50.0), 50.0).to_path(0.1), 0.5));
    }

    #[test]
    fn parts_that_overlap_under_the_even_odd_rule_do_not_paint_the_shape() {
        let square = |x0: f64, y0: f64, x1: f64, y1: f64| kurbo::Rect::new(x0, y0, x1, y1).to_path(0.1);
        let (a, b) = (square(0.0, 0.0, 60.0, 60.0), square(40.0, 0.0, 100.0, 60.0));
        let mut both = a.clone();
        both.extend(b.iter());
        assert!(!parts_paint_alike(&both, &[a.clone(), b.clone()], false, 0.0), "the overlap is a hole in the shape");
        assert!(parts_paint_alike(&both, &[a.clone(), b.clone()], true, 0.0), "the nonzero rule paints it");
        // Windings that cancel under the nonzero rule leave the overlap unpainted too.
        let mut turned = a.clone();
        turned.extend(b.reverse_subpaths().iter());
        assert!(!parts_paint_alike(&turned, &[a.clone(), b.reverse_subpaths()], true, 0.0));
        // An overlap too thin for the probes to land in is not taken for none.
        let thin = square(59.9, 0.0, 100.0, 60.0);
        let mut strip = a.clone();
        strip.extend(thin.iter());
        assert!(!parts_paint_alike(&strip, &[a.clone(), thin], false, 0.0));
        let apart = square(80.0, 0.0, 140.0, 60.0);
        let mut aside = a.clone();
        aside.extend(apart.iter());
        assert!(parts_paint_alike(&aside, &[a, apart], false, 0.0));
    }

    #[test]
    fn near_matching_with_no_work_left_returns_nothing() {
        let db = FontDb::global();
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let gid = face.mapped_chars().iter().find(|(c, _)| *c == 'o').map(|(_, g)| *g).unwrap();
        let shape = Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * simplified(&db.outline_uncached(&face, gid), 4.0);
        let index = RetypeIndex::build_near(db, [face], &BTreeSet::new(), &[near_key(&shape).unwrap()]);
        assert!(!index.near_matches(&shape, 0.02).is_empty());
        let after = index.near_work.load(Ordering::Acquire);
        assert!((NEAR_WORK - NEAR_SHAPE_WORK..NEAR_WORK).contains(&after), "one allowance at most: {after}");
        index.near_work.store(0, Ordering::Release);
        assert!(index.near_matches(&shape, 0.02).is_empty());
        assert_eq!(index.near_work.load(Ordering::Acquire), 0);
    }

    #[test]
    fn ranking_near_match_candidates_takes_from_the_work_budget() {
        let db = FontDb::global();
        let face = db.face("Source Sans 3", "Regular").unwrap();
        let gid = face.mapped_chars().iter().find(|(c, _)| *c == 'o').map(|(_, g)| *g).unwrap();
        let shape = Affine::translate((100.0, 200.0)) * Affine::scale(0.04) * simplified(&db.outline_uncached(&face, gid), 4.0);
        let index = RetypeIndex::build_near(db, [face], &BTreeSet::new(), &[near_key(&shape).unwrap()]);
        let s = near_signature(&shape).unwrap();
        let mut all = u64::MAX;
        let ranked = index.ranked_near(&s, &mut all).len();
        assert!(ranked >= 1, "the o is ranked");
        assert_eq!(u64::MAX - all, ranked as u64 * 3, "each glyph of two contours takes three");
        // Enough to rank one glyph of two contours and nothing more.
        let mut work = 4;
        assert_eq!(index.ranked_near(&s, &mut work).len(), 1);
        assert_eq!(work, 1);
    }

    #[test]
    fn a_round_glyph_with_an_off_centre_counter_near_matches_at_its_turn() {
        // A round outer contour fits the shape at every turn; only the counter, off the centre,
        // tells the turn, which is far from upright.
        let polygon = |cx: f64, cy: f64, r: f64, n: usize, back: bool| {
            let mut p = BezPath::new();
            for i in 0..n {
                let a = std::f64::consts::TAU * i as f64 / n as f64 * if back { -1.0 } else { 1.0 };
                let q = Point::new(cx + r * a.cos(), cy + r * a.sin());
                if i == 0 { p.move_to(q) } else { p.line_to(q) }
            }
            p.close_path();
            p
        };
        // The outer contour has 720 corners: as round as the samples can tell.
        let mut g = polygon(0.0, 0.0, 300.0, 720, false);
        g.extend(polygon(120.0, 40.0, 80.0, 24, true).iter());
        let xf = Affine::translate((500.0, 400.0)) * Affine::rotate(2.0) * Affine::scale(0.05);
        let shape = xf * simplified(&g, 1.0);
        let (z, r) = near_fit_all(&signature(&g).unwrap(), &near_signature(&shape).unwrap()).unwrap();
        assert!(r < 0.01, "{r}");
        assert!(wrapped(z.1.atan2(z.0) - 2.0).abs() < 0.02, "{z:?}");
    }

    #[test]
    fn clusters_split_a_word_into_its_glyph_parts() {
        let mut word = letter();
        word.extend((Affine::translate((600.0, 0.0)) * letter()).iter());
        // An i: a stem and a dot beside it.
        word.extend(kurbo::Rect::new(1200.0, -500.0, 1260.0, 0.0).path_elements(0.1));
        word.extend(kurbo::Circle::new((1230.0, -600.0), 40.0).path_elements(0.1));
        let parts = clusters(&word);
        assert_eq!(parts.len(), 4, "two letters (each with its counter), a stem and a dot");
        assert_eq!(shape_key(&parts[0]), shape_key(&letter()));
        assert_eq!(shape_key(&parts[1]), shape_key(&letter()));
        assert_eq!(parts[2].elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count(), 1);
        // A counter written before its outer contour still goes with it, in the outer's place.
        let mut back = BezPath::new();
        let l = letter();
        let els = l.elements();
        let second = els.iter().rposition(|e| matches!(e, PathEl::MoveTo(_))).unwrap();
        back.extend(els[second..].iter().copied());
        back.extend(els[..second].iter().copied());
        assert_eq!(clusters(&back).len(), 1);
        assert!(clusters(&BezPath::new()).is_empty());
    }

    #[test]
    fn the_nested_reading_turns_counters_against_their_outer_contour() {
        let split = |p: &BezPath| -> Vec<BezPath> {
            let mut out: Vec<BezPath> = vec![];
            for el in p.elements() {
                if matches!(el, PathEl::MoveTo(_)) {
                    out.push(BezPath::new());
                }
                out.last_mut().unwrap().push(*el);
            }
            out
        };
        let join = |cs: &[BezPath]| {
            let mut p = BezPath::new();
            for c in cs {
                p.extend(c.iter());
            }
            p
        };
        let cs = split(&letter());
        let (outer, counter) = (cs[0].clone(), cs[1].clone());
        let (same, opposite) = if outer.area().signum() == counter.area().signum() {
            (counter.clone(), counter.reverse_subpaths())
        } else {
            (counter.reverse_subpaths(), counter.clone())
        };
        // A counter running the same way as its outer contour is turned; the outer one is kept.
        let read = nested_reading(&join(&[outer.clone(), same.clone()])).unwrap();
        let rs = split(&read);
        assert_eq!(rs[0], outer);
        assert_eq!(rs[1].area().signum(), opposite.area().signum());
        // So is the outer contour's way when it comes second.
        let read = nested_reading(&join(&[opposite.clone(), outer.reverse_subpaths()])).unwrap();
        let rs = split(&read);
        assert_eq!(rs[1], outer.reverse_subpaths());
        assert_eq!(rs[0].area().signum(), outer.area().signum());
        // Already nested so: nothing to turn.
        assert!(nested_reading(&join(&[outer.clone(), opposite])).is_none());
        // A contour enclosing no area gives none.
        let mut flat = BezPath::new();
        flat.move_to((0.0, 0.0));
        flat.line_to((10.0, 0.0));
        flat.close_path();
        assert!(nested_reading(&join(&[outer, flat])).is_none());
    }

    #[test]
    fn a_part_with_some_contours_turned_still_matches_exactly() {
        let db = FontDb::with_font_dirs(vec![]);
        assert_eq!(db.add_font(crate::test_fonts::named_copy(false, ("Turntest", "Regular", "Version 1.000", "Turntest-Regular")).unwrap()), 1);
        let face = db.face("Turntest", "Regular").unwrap();
        let o = Affine::translate((40.0, 90.0)) * Affine::scale(0.03) * db.outline_uncached(&face, face.glyph_for('o'));
        let mut cs: Vec<BezPath> = vec![];
        for el in o.elements() {
            if matches!(el, PathEl::MoveTo(_)) {
                cs.push(BezPath::new());
            }
            cs.last_mut().unwrap().push(*el);
        }
        assert_eq!(cs.len(), 2, "an o is two contours");
        let keys = std::iter::once(shape_key(&o).unwrap()).collect();
        let index = RetypeIndex::build(&db, vec![face], &keys);
        assert!(!index.all_matches(&o).is_empty());
        for turned in 0..2 {
            let mut shape = BezPath::new();
            for (i, c) in cs.iter().enumerate() {
                shape.extend(if i == turned { c.reverse_subpaths() } else { c.clone() }.iter());
            }
            assert!(index.all_matches(&shape).is_empty(), "contour {turned} turned is no glyph as it is");
            assert!(index.all_matches(&shape.reverse_subpaths()).is_empty(), "nor read the other way round");
            let found = index.matches_either_way(&shape);
            assert!(found.iter().any(|m| m.ch == 'o'), "contour {turned} turned still matches the o");
        }
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

    /// A glyph that more than [`MAX_CANDIDATES`] faces share: `matches` gives the first
    /// [`MAX_CANDIDATES`] fits, and `all_matches` gives the fits in every face.
    #[test]
    fn all_matches_lists_every_face_a_shape_fits() {
        let db = FontDb::with_font_dirs(vec![]);
        let copies = MAX_CANDIDATES + 4;
        let mut faces = vec![];
        for i in 0..copies {
            let (family, postscript) = (format!("Copytest {i:02}"), format!("Copytest{i:02}-Regular"));
            assert_eq!(db.add_font(crate::test_fonts::named_copy(false, (&family, "Regular", "Version 1.000", &postscript)).unwrap()), 1);
            faces.push(db.face(&family, "Regular").unwrap());
        }
        let face = faces[0].clone();
        let shape = Affine::scale(0.03) * db.outline_uncached(&face, face.glyph_for('o'));
        let keys = std::iter::once(shape_key(&shape).unwrap()).collect();
        let index = RetypeIndex::build(&db, faces, &keys);
        let ids = |ms: &[GlyphMatch]| ms.iter().map(|m| m.face.id()).collect::<std::collections::BTreeSet<_>>().len();
        assert_eq!(index.matches(&shape).len(), MAX_CANDIDATES);
        assert!(ids(&index.matches(&shape)) < copies);
        let all = index.all_matches(&shape);
        assert_eq!(ids(&all), copies, "every copy of the font fits the o");
        assert_eq!(
            all[..MAX_CANDIDATES].iter().map(|m| m.face.id()).collect::<Vec<_>>(),
            index.matches(&shape).iter().map(|m| m.face.id()).collect::<Vec<_>>()
        );
    }
}
