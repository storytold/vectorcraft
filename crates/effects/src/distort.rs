//! Distort & Transform effects.

use serde_json::Value;
use vectorcraft_geom::kurbo::ParamCurveArclen;
use vectorcraft_geom::{Affine, Anchor, ParamCurve, PathData, Point, Rect, SubPath, Vec2};

use crate::util::{catmull_rom, fit_piece, flag, map_nonlinear_within, mean_size, noise, normal_at, num, pieces, seed, seg_cubic, smooth_points};

fn diag(b: Rect) -> f64 {
    b.width().hypot(b.height()).max(1e-6)
}

/// Free Distort: bilinear map of the bounding box onto four new corners (unit box coordinates),
/// making at most `max` anchors ([`map_nonlinear_within`]).
pub fn free_distort(path: &PathData, b: Rect, p: &Value, max: usize) -> PathData {
    let mut c = [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)];
    if let Some(arr) = p.get("corners").and_then(Value::as_array) {
        for (i, v) in arr.iter().take(4).enumerate() {
            if let Some(a) = v.as_array()
                && let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64))
                && x.is_finite()
                && y.is_finite()
            {
                c[i] = Point::new(x, y);
            }
        }
    }
    let (w, h) = (b.width().max(1e-9), b.height().max(1e-9));
    let to_doc = |q: Point| Point::new(b.x0 + q.x * w, b.y0 + q.y * h);
    let c = c.map(to_doc);
    let f = |q: Point| {
        let u = (q.x - b.x0) / w;
        let v = (q.y - b.y0) / h;
        let top = c[0].lerp(c[1], u);
        let bot = c[3].lerp(c[2], u);
        top.lerp(bot, v)
    };
    map_nonlinear_within(path, diag(b) / 16.0, max, f)
}

/// Pucker & Bloat: anchors move toward (bloat) or away from (pucker) the centre, handles the
/// opposite way.
pub fn pucker_bloat(path: &PathData, b: Rect, amount: f64) -> PathData {
    let a = amount.clamp(-200.0, 200.0) / 100.0;
    if a == 0.0 {
        return path.clone();
    }
    let c = b.center();
    // Lines have retracted handles: give them handles at 1/3 before distorting so they bulge.
    {
        let mut res = path.clone();
        for (si, sp) in res.subpaths.iter_mut().enumerate() {
            let n = sp.anchors.len();
            let segs = sp.segment_count();
            let orig = &path.subpaths[si];
            for i in 0..segs {
                let cub = seg_cubic(orig, i);
                let j = (i + 1) % n;
                sp.anchors[i].h_out = cub.p1;
                sp.anchors[j].h_in = cub.p2;
            }
            for an in &mut sp.anchors {
                let p = an.p;
                *an = Anchor::with_handles(p.lerp(c, a * 0.5), an.h_in.lerp(c, -a * 0.5), an.h_out.lerp(c, -a * 0.5));
            }
        }
        res
    }
}

/// Most points Roughen resamples one segment into.
const MAX_ROUGHEN_POINTS: usize = 2000;

/// Roughen: resample at `detail` points per inch and jitter each point by up to `size`. Where
/// that would make more than `max` anchors, the points are spaced further apart until it doesn't
/// (stacked Roughens multiply their points, #3578).
pub fn roughen(path: &PathData, b: Rect, p: &Value, max: usize) -> PathData {
    let size = num(p, "size", 5.0).clamp(0.0, 1e4);
    let amp = if flag(p, "relative", true) { size / 100.0 * mean_size(b) } else { size };
    let detail = num(p, "detail", 10.0).clamp(0.0, 100.0);
    let spacing = if detail > 0.0 { 72.0 / detail } else { f64::INFINITY };
    // A segment of k points adds k - 1 anchors to the path.
    let lens: Vec<f64> = path.subpaths.iter().flat_map(|sp| (0..sp.segment_count()).map(|i| sp.segment(i).arclen(0.1))).collect();
    let spacing = fit_piece(&lens, spacing, MAX_ROUGHEN_POINTS, max.saturating_sub(path.anchor_count()));
    let mut lens = lens.iter();
    let smooth = smooth_points(p);
    let seed = seed(p);
    let mut subs = vec![];
    for (si, sp) in path.subpaths.iter().enumerate() {
        let mut pts = vec![];
        for (i, &len) in (0..sp.segment_count()).zip(&mut lens) {
            let c = sp.segment(i);
            let k = pieces(len, spacing, MAX_ROUGHEN_POINTS);
            for j in 0..k {
                pts.push(c.eval(j as f64 / k as f64));
            }
        }
        if !sp.closed
            && let Some(l) = sp.anchors.last()
        {
            pts.push(l.p);
        }
        if pts.is_empty() {
            pts.extend(sp.anchors.iter().map(|a| a.p));
        }
        for (k, q) in pts.iter_mut().enumerate() {
            q.x += amp * noise(seed, si as u64, k as u64, 1);
            q.y += amp * noise(seed, si as u64, k as u64, 2);
        }
        subs.push(if smooth { catmull_rom(&pts, sp.closed, 1.0) } else { SubPath::polyline(&pts, sp.closed) });
    }
    PathData::new(subs)
}

/// The move and the number of copies of a Transform effect with params `p` that only moves what
/// it applies to, as [`transform_map`] reads them; `None` when it also scales, rotates or reflects,
/// or moves by a random share, or moves pattern paints along (Transform Patterns), or scales
/// strokes while leaving the geometry (Scale Strokes & Effects with Transform Objects off).
pub(crate) fn transform_move(p: &Value) -> Option<(Vec2, usize)> {
    let (m, copies) = transform_map(Rect::ZERO, p, 0);
    let [a, b, c, d, x, y] = m.as_coeffs();
    // Transform Patterns moves the patterns along; with Transform Objects off, Scale Strokes &
    // Effects still scales the strokes by the map the geometry doesn't take.
    let o = TransformOptions::of(p);
    let plain = !flag(p, "random", false) && !o.patterns && (o.objects || !o.strokes);
    (plain && [a, b, c, d] == [1.0, 0.0, 0.0, 1.0]).then_some((Vec2::new(x, y), copies))
}

/// The offsets at which Transform effects with params `ps`, applied in order, place `path`, the
/// original's place first, when each of them only moves it ([`transform_move`]): each effect moves
/// every place it is given, or with copies keeps them and adds each copy's. `None` when one of
/// them does more than move, or when [`transform`] stops its copies of `path` early.
pub(crate) fn transform_offsets(ps: &[Value], path: &PathData) -> Option<Vec<Vec2>> {
    let mut offsets = vec![Vec2::ZERO];
    let mut all = path.clone();
    for p in ps {
        let (v, copies) = transform_move(p)?;
        let next = transform(&all, Rect::ZERO, p, 0);
        if next.anchor_count() != all.anchor_count().saturating_mul(copies + 1) {
            return None;
        }
        all = next;
        if copies == 0 {
            offsets.iter_mut().for_each(|o| *o += v);
            continue;
        }
        let from = offsets.clone();
        offsets.extend((1..=copies).flat_map(|k| from.iter().map(move |o| *o + v * k as f64)));
    }
    Some(offsets)
}

/// Largest scale (%) and move (pt) the Transform effect takes.
const MAX_SCALE: f64 = 100_000.0;
const MAX_MOVE: f64 = 1.0e7;
/// The Transform effect's copies stop before one reaches further than this (pt) from the origin.
const MAX_REACH: f64 = 1.0e9;
/// Most anchors a Transform effect's copies make of a path, the original included: 1000 copies of
/// a 2000-anchor path. Stacked Transform effects multiply their copies (three of 1000 copies make
/// a thousand million of one square), so the limit is on the result, not on each effect's count.
pub(crate) const MAX_COPY_ANCHORS: usize = 2_000_000;
/// Most image and symbol instance nodes a Transform effect's copies make of an art, the originals
/// included: 1000 copies of a 2000-image art. Stacked Transform effects multiply their copies
/// (three of 1000 copies make a thousand million of one image), so the limit is on the result,
/// not on each effect's count.
pub(crate) const MAX_COPY_OBJECTS: usize = 2_000_000;

/// Does `r` stay within [`MAX_REACH`] of the origin? A Transform effect's copies stop before one
/// doesn't.
pub(crate) fn within_reach(r: Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.abs() <= MAX_REACH)
}

/// Transform effect: scale, rotate and reflect about reference point `reference` of the bounds
/// (the 9-point grid, 4 = centre), then move; `copies` more times, each from the last. With
/// `random`, each scale goes a random share of the way from 100 % to its value and each move and
/// the angle a share of theirs, drawn from `seed` (the object's), so an object keeps its result
/// on every redraw while each object varies its own way. With Transform Objects off the path stays
/// as it is; Transform Patterns and Scale Strokes & Effects reach the appearance through the
/// object's art ([`crate::has_transform_art`]).
pub fn transform(path: &PathData, b: Rect, p: &Value, seed: u64) -> PathData {
    transform_within(path, b, p, seed, MAX_COPY_ANCHORS)
}

/// [`transform`] whose copies make at most `max` anchors of the path, the original included.
pub(crate) fn transform_within(path: &PathData, b: Rect, p: &Value, seed: u64, max: usize) -> PathData {
    let (m, copies) = transform_map(b, p, seed);
    if copies == 0 {
        return path.transformed(m);
    }
    // Each copy adds the path once more, and a Transform after this one copies the lot again:
    // copies stop where the result would pass [`MAX_COPY_ANCHORS`] (#1705).
    let each = path.anchor_count();
    let mut anchors = each;
    let mut out = path.clone();
    let mut cur = path.clone();
    for _ in 0..copies {
        anchors = anchors.saturating_add(each);
        if anchors > max {
            break;
        }
        cur = cur.transformed(m);
        if !cur.bounds().is_some_and(within_reach) {
            break;
        }
        out.subpaths.extend(cur.subpaths.iter().cloned());
    }
    out
}

/// The Transform effect's options beyond its geometry (#885).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct TransformOptions {
    /// Transform Objects (default on): the geometry transforms and is copied. Off, it stays where
    /// it is and only the two options below apply, once.
    pub objects: bool,
    /// Transform Patterns (default off): pattern fills and strokes transform along.
    pub patterns: bool,
    /// Scale Strokes & Effects (default off): stroke weights and dashes, and the distances of the
    /// effects on the fills and strokes, scale by the map's mean scale (`√|det|`).
    pub strokes: bool,
}

impl TransformOptions {
    /// The options of a Transform effect with params `p`. Missing keys take their defaults, so
    /// effects saved before the options existed draw as they did.
    pub fn of(p: &Value) -> Self {
        Self { objects: flag(p, "objects", true), patterns: flag(p, "patterns", false), strokes: flag(p, "strokes", false) }
    }

    /// Do they reach past the geometry (Transform Patterns or Scale Strokes & Effects)?
    pub fn reach_appearance(self) -> bool {
        self.patterns || self.strokes
    }
}

/// The Transform effect's map and how many copies of what it is applied to it makes, for bounds
/// `b` (the reference point lies on them), params `p` and the object's `seed` for Random: the map
/// applies to the object itself when `copies` is 0, and once more to each of `copies` copies
/// otherwise (see [`transform_within`], and images, whose copies count nodes against
/// [`MAX_COPY_OBJECTS`]). With Transform Objects off the geometry stays: the identity and no
/// copies ([`transform_affine`] is the map its other options still use).
pub(crate) fn transform_map(b: Rect, p: &Value, seed: u64) -> (Affine, usize) {
    if !TransformOptions::of(p).objects {
        return (Affine::IDENTITY, 0);
    }
    (transform_affine(b, p, seed), num(p, "copies", 0.0).clamp(0.0, 1000.0) as usize)
}

/// The one transformation of the Transform effect with params `p` on an object of bounds `b`
/// (the reference point lies on them; `seed` for Random), whatever Transform Objects says. Each
/// copy applies it once more to the last.
pub(crate) fn transform_affine(b: Rect, p: &Value, seed: u64) -> Affine {
    let random = flag(p, "random", false);
    // The share (0..1) of value `k` applied.
    let share = |k: u64| if random { (noise(seed, k, 0, 0) + 1.0) / 2.0 } else { 1.0 };
    let scale = |key: &str, k: u64| 1.0 + (num(p, key, 100.0).clamp(-MAX_SCALE, MAX_SCALE) / 100.0 - 1.0) * share(k);
    let moved = |key: &str, k: u64| num(p, key, 0.0).clamp(-MAX_MOVE, MAX_MOVE) * share(k);
    let reflect = |key: &str| if flag(p, key, false) { -1.0 } else { 1.0 };
    let o = vectorcraft_geom::reference_point(b, num(p, "reference", 4.0).clamp(0.0, 8.0) as usize).to_vec2();
    // Reflect X flips the x coordinates (left to right), Reflect Y the y coordinates.
    Affine::translate(Vec2::new(moved("moveH", 2), moved("moveV", 3)) + o)
        * Affine::rotate(-(num(p, "rotate", 0.0) * share(4)).to_radians())
        * Affine::scale_non_uniform(scale("scaleH", 0) * reflect("reflectX"), scale("scaleV", 1) * reflect("reflectY"))
        * Affine::translate(-o)
}

/// Tweak: random displacement of anchors and/or control points.
pub fn tweak(path: &PathData, b: Rect, p: &Value) -> PathData {
    let rel = flag(p, "relative", true);
    let h = num(p, "h", 10.0);
    let v = num(p, "v", 10.0);
    let (ah, av) = if rel { (h / 100.0 * b.width().abs(), v / 100.0 * b.height().abs()) } else { (h, v) };
    let (anchors, ins, outs) = (flag(p, "anchors", true), flag(p, "in", true), flag(p, "out", true));
    let seed = seed(p);
    let mut out = path.clone();
    for (si, sp) in out.subpaths.iter_mut().enumerate() {
        for (ai, an) in sp.anchors.iter_mut().enumerate() {
            let key = ((si as u64) << 32) | ai as u64;
            let d = |c: u64| Vec2::new(ah * noise(seed, key, c, 1), av * noise(seed, key, c, 2));
            let (mut pp, mut hi, mut ho) = (an.p, an.h_in, an.h_out);
            if anchors {
                let dp = d(0);
                pp += dp;
                hi += dp;
                ho += dp;
            }
            if ins {
                hi += d(1);
            }
            if outs {
                ho += d(2);
            }
            *an = Anchor::with_handles(pp, hi, ho);
        }
    }
    out
}

/// Twist: rotation that is strongest at the centre and fades to zero at the bounding circle,
/// making at most `max` anchors ([`map_nonlinear_within`]).
pub fn twist(path: &PathData, b: Rect, angle: f64, max: usize) -> PathData {
    let ang = angle.clamp(-3600.0, 3600.0).to_radians();
    if ang == 0.0 {
        return path.clone();
    }
    let c = b.center();
    let r = diag(b) / 2.0;
    let f = move |q: Point| {
        let d = q - c;
        let t = (1.0 - d.hypot() / r).max(0.0);
        let a = -ang * t;
        let (s, co) = a.sin_cos();
        c + Vec2::new(d.x * co - d.y * s, d.x * s + d.y * co)
    };
    let piece = (r / 8.0).min(r / (1.0 + ang.abs() * 2.0));
    map_nonlinear_within(path, piece, max, f)
}

const ZIGZAG_ARCLEN_ACCURACY: f64 = 1e-4;

/// A Zig Zag ridge: where it sits, the path's unit direction there, and the length along the
/// path to the ridges before and after it (zero at an open path's ends).
struct Ridge {
    p: Point,
    dir: Vec2,
    gap_in: f64,
    gap_out: f64,
}

/// Zig Zag: `ridges` extra points per segment, evenly spaced along its length and alternately
/// displaced along the normal. Corner points join them with straight lines. Smooth points make a
/// wave: every peak and valley is a smooth point whose handles run along the path, a third of the
/// way to the next ridge, so each span between them follows a half cosine (an S through the path)
/// whatever the size of the ridges (#2343). Where `ridges` would make more than `max` anchors,
/// each segment gets as many as fit (stacked Zig Zags multiply their points, #3578).
pub fn zig_zag(path: &PathData, b: Rect, p: &Value, max: usize) -> PathData {
    let size = num(p, "size", 10.0).clamp(-1e4, 1e4);
    let amp = if flag(p, "relative", false) { size / 100.0 * mean_size(b) } else { size };
    // Each segment makes `ridges + 1` points, and an open path's end one more (all its anchors
    // when it has no segment). With no ridges, that is at most the path's own anchors.
    let segs = path.subpaths.iter().map(SubPath::segment_count).fold(0, usize::saturating_add);
    let ends: usize = path.subpaths.iter().filter(|sp| !sp.closed).map(|sp| if sp.segment_count() > 0 { 1 } else { sp.anchors.len() }).sum();
    let fit = (max.saturating_sub(ends) / segs.max(1)).saturating_sub(1);
    let ridges = (num(p, "ridges", 4.0).clamp(0.0, 100.0) as usize).min(fit);
    let smooth = smooth_points(p);
    let mut subs = vec![];
    for sp in &path.subpaths {
        let segs = sp.segment_count();
        let gap = |i: usize| sp.segment(i).arclen(ZIGZAG_ARCLEN_ACCURACY) / (ridges + 1) as f64;
        let mut pts: Vec<Ridge> = vec![];
        let mut k = 0usize;
        for i in 0..segs {
            let c = sp.segment(i);
            // Ridges are spaced by length, not by `t`: a line's retracted handles bunch `t` up
            // near the anchors, and so does any uneven curve.
            let len = c.arclen(ZIGZAG_ARCLEN_ACCURACY);
            let step = len / (ridges + 1) as f64;
            for j in 0..=ridges {
                let s = j as f64 / (ridges + 1) as f64;
                let t = if j > 0 && len > 1e-9 { c.inv_arclen(s * len, ZIGZAG_ARCLEN_ACCURACY).clamp(0.0, 1.0) } else { s };
                let mut gap_in = step;
                let nrm = if j == 0 {
                    // Average the normals of the segments meeting at the anchor.
                    let prev = if i > 0 {
                        Some(i - 1)
                    } else if sp.closed {
                        Some(segs - 1)
                    } else {
                        None
                    };
                    gap_in = prev.map_or(0.0, gap);
                    let n1 = normal_at(&c, 0.0);
                    match prev {
                        Some(pi) => {
                            let s = n1 + normal_at(&sp.segment(pi), 1.0);
                            if s.hypot() > 1e-9 { s / s.hypot() } else { n1 }
                        }
                        None => n1,
                    }
                } else {
                    normal_at(&c, t)
                };
                let sign = if k.is_multiple_of(2) { 1.0 } else { -1.0 };
                pts.push(Ridge { p: c.eval(t) + nrm * (amp * sign), dir: Vec2::new(-nrm.y, nrm.x), gap_in, gap_out: step });
                k += 1;
            }
        }
        if !sp.closed {
            if segs > 0 {
                let c = sp.segment(segs - 1);
                let sign = if k.is_multiple_of(2) { 1.0 } else { -1.0 };
                let nrm = normal_at(&c, 1.0);
                pts.push(Ridge { p: c.p3 + nrm * (amp * sign), dir: Vec2::new(-nrm.y, nrm.x), gap_in: gap(segs - 1), gap_out: 0.0 });
            } else {
                pts.extend(sp.anchors.iter().map(|a| Ridge { p: a.p, dir: Vec2::ZERO, gap_in: 0.0, gap_out: 0.0 }));
            }
        }
        subs.push(if smooth {
            let anchors = pts.iter().map(|r| Anchor::with_handles(r.p, r.p - r.dir * (r.gap_in / 3.0), r.p + r.dir * (r.gap_out / 3.0))).collect();
            SubPath::new(anchors, sp.closed)
        } else {
            SubPath::polyline(&pts.iter().map(|r| r.p).collect::<Vec<_>>(), sp.closed)
        });
    }
    PathData::new(subs)
}
