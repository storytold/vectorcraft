use super::envelope::{EnvelopeKind, H_DOWN, H_LEFT, H_RIGHT, H_UP, envelope_grid, mesh_at};
use super::warp::{WarpNet, WarpStyle, flatten_subpath, resample, uses_net, warp_point};
use super::{
    Affine, Anchor, Arc, Color, CubicBez, Deserialize, Node, ParamCurve, PathData, Point, Rect, Serialize, Shape, SubPath, Vec2, fifty, lerp_color,
    one,
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeshPoint {
    pub p: Point,
    pub color: Color,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Tangent handles as offsets from `p` (see [`H_RIGHT`] …).
    #[serde(default)]
    pub handles: [Vec2; 4],
}

/// A gradient mesh: `(rows+1)×(cols+1)` points, row-major.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientMesh {
    pub rows: u32,
    pub cols: u32,
    pub points: Vec<MeshPoint>,
}

/// One tessellated piece of a mesh: a small quad, its colour at the centre, and its colour and
/// opacity at each corner (in `pts` order), which a renderer shades between so neighbouring quads
/// meet without a step.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshQuad {
    pub pts: [Point; 4],
    pub color: Color,
    pub opacity: f32,
    pub corners: [(Color, f32); 4],
}

/// One flat-coloured piece of a mesh ([`GradientMesh::pieces`]): a closed outline through
/// tessellation points, its colour and opacity.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshPiece {
    pub pts: Vec<Point>,
    pub color: Color,
    pub opacity: f32,
}

/// Create Gradient Mesh → Appearance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MeshAppearance {
    #[default]
    Flat,
    ToCenter,
    ToEdge,
}

impl MeshAppearance {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "flat" => Some(Self::Flat),
            "center" | "tocenter" | "to center" => Some(Self::ToCenter),
            "edge" | "toedge" | "to edge" => Some(Self::ToEdge),
            _ => None,
        }
    }
}

/// Default envelope fidelity.
pub fn default_fidelity() -> f64 {
    fifty()
}

const SIDE: usize = 65;

/// Four boundary curves (top TL→TR, right TR→BR, bottom BL→BR, left TL→BL), each resampled to
/// [`SIDE`] points, from the outline of a closed path split at the points nearest its bounding
/// box corners. [`Coons::from_top_object`] samples its sides at the places `knots` gives instead.
#[derive(Clone, Debug)]
pub struct Coons {
    top: Vec<Point>,
    right: Vec<Point>,
    bottom: Vec<Point>,
    left: Vec<Point>,
    /// Where along its side (0 to 1, rising) each point of the top, right, bottom and left side
    /// lies; `None` when the points are evenly spaced.
    knots: Option<Box<[Vec<f64>; 4]>>,
}

impl Coons {
    /// A rectangle.
    pub fn rect(r: Rect) -> Self {
        let line = |a: Point, b: Point| (0..SIDE).map(|i| a.lerp(b, i as f64 / (SIDE - 1) as f64)).collect::<Vec<_>>();
        let (tl, tr, br, bl) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
        Self { top: line(tl, tr), right: line(tr, br), bottom: line(bl, br), left: line(tl, bl), knots: None }
    }

    /// The patch of `path`'s outline, each side measured by its length: equal steps in `u` or `v`
    /// cover equal lengths of the side.
    pub fn from_path(path: &PathData) -> Option<Self> {
        // The largest subpath is the outline.
        let sp = path.subpaths.iter().filter(|s| s.anchors.len() >= 2).max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))?;
        let r = path.bounds()?;
        let tol = (r.width() + r.height()).max(1e-6) / 2000.0;
        let mut poly = flatten_subpath(sp, tol);
        if poly.len() > 1
            && let (Some(a), Some(b)) = (poly.first(), poly.last())
            && a.distance(*b) < 1e-9
        {
            poly.pop();
        }
        if poly.len() < 3 {
            return Some(Self::rect(r));
        }
        // Orient clockwise in y-down (TL → TR → BR → BL).
        let area: f64 = (0..poly.len())
            .map(|i| {
                let (p, q) = (poly[i], poly[(i + 1) % poly.len()]);
                p.x * q.y - q.x * p.y
            })
            .sum();
        if area < 0.0 {
            poly.reverse();
        }
        let nearest = |c: Point| (0..poly.len()).min_by(|i, j| poly[*i].distance_squared(c).total_cmp(&poly[*j].distance_squared(c))).unwrap_or(0);
        let tl = nearest(Point::new(r.x0, r.y0));
        let tr = nearest(Point::new(r.x1, r.y0));
        let br = nearest(Point::new(r.x1, r.y1));
        let bl = nearest(Point::new(r.x0, r.y1));
        let n = poly.len();
        let arc = |i: usize, j: usize| -> Vec<Point> {
            let mut v = vec![poly[i]];
            let mut k = i;
            while k != j {
                k = (k + 1) % n;
                v.push(poly[k]);
            }
            v
        };
        let top = resample(&arc(tl, tr), SIDE);
        let right = resample(&arc(tr, br), SIDE);
        let mut bottom = resample(&arc(br, bl), SIDE);
        bottom.reverse();
        let mut left = resample(&arc(bl, tl), SIDE);
        left.reverse();
        Some(Self { top, right, bottom, left, knots: None })
    }

    /// The patch of a top object's outline (the largest subpath of `path`), split into four sides
    /// at its corners. Each piece of a side between two anchors or corners takes a share of `u` or
    /// `v` in proportion to the length of its control polygon, spread along the piece by its own
    /// curve parameter. A segment without handles counts as a cubic with its handles a third of
    /// the way along, so the content spreads evenly along it. `corners` are the anchors (indices
    /// into the outline, in its own order) at the top left, top right, bottom right and bottom
    /// left, as a saved file gives them. They stay on those anchors when they run the way the
    /// outline does or the opposite way, so a reflected outline keeps them; without them, or when
    /// they don't fit the outline, each corner is found from the outline ([`top_object_corner`]).
    /// A straight segment closes an open subpath, and segments of no length are left out. A top
    /// object's envelope maps onto this patch.
    pub fn from_top_object(path: &PathData, corners: Option<[u32; 4]>) -> Option<Self> {
        let sp = path.subpaths.iter().filter(|s| s.anchors.len() >= 2).max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))?;
        let mut sp = sp.clone();
        let count = sp.anchors.len();
        // Stored corners count only when they are distinct anchors met in order around the outline.
        let fits = |c: &[usize; 4]| {
            if !c.iter().all(|i| *i < count) {
                return false;
            }
            let gaps: Vec<usize> = (0..4).map(|k| (c[(k + 1) % 4] + count - c[k]) % count).collect();
            gaps.iter().all(|g| *g > 0) && gaps.iter().sum::<usize>() == count
        };
        let given = corners.map(|c| c.map(|i| i as usize));
        // Stored corners met in the outline's own order keep it as drawn, even counter-clockwise
        // (a reflected outline). Otherwise orient it clockwise in y-down (TL → TR → BR → BL).
        let mut corners = given.filter(fits);
        let reversed = corners.is_none() && sp.signed_area() < 0.0;
        if reversed {
            sp.reverse();
            corners = given.map(|c| c.map(|i| count.saturating_sub(1).saturating_sub(i))).filter(fits);
        }
        // Each segment, and whether it is straight: a straight one's parameter runs evenly along it.
        let line = |a: Point, b: Point| (CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b), true);
        let mut segs: Vec<(usize, (CubicBez, bool))> = (0..sp.segment_count())
            .map(|i| {
                let c = sp.segment(i);
                (i, if sp.segment_is_line(i) { line(c.p0, c.p3) } else { (c, false) })
            })
            .collect();
        if !sp.closed
            && let (Some(a), Some(b)) = (sp.anchors.last(), sp.anchors.first())
        {
            segs.push((count.saturating_sub(1), line(a.p, b.p)));
        }
        segs.retain(|(_, (c, _))| [c.p1, c.p2, c.p3].iter().any(|p| p.distance(c.p0) > 1e-9));
        // The kept segment that anchor `k` starts (a segment of no length starts where the next begins).
        let starts: Vec<usize> = segs.iter().map(|(i, _)| *i).collect();
        // Where each kept segment and the anchor it starts at come in the outline as drawn: a tie
        // between corners goes to the one met first there.
        let order: Vec<(usize, usize)> = starts
            .iter()
            .map(|&i| if reversed { (count.saturating_sub(1).saturating_sub(i), (2 * count).saturating_sub(2 + i) % count.max(1)) } else { (i, i) })
            .collect();
        let segs: Vec<(CubicBez, bool)> = segs.into_iter().map(|(_, s)| s).collect();
        let r = segs.iter().map(|(c, _)| c.bounding_box()).reduce(|a, b| a.union(b)).or_else(|| path.bounds())?;
        // An outline of fewer than three points (a straight line there and back) maps onto its
        // bounding box, as in `from_path`.
        if segs.len() < 2 || (segs.len() == 2 && segs.iter().all(|(_, straight)| *straight)) {
            return Some(Self::rect(r));
        }
        let n = segs.len() as f64;
        let at = |s: f64| {
            let s = s.rem_euclid(n);
            let i = (s.floor() as usize).min(segs.len() - 1);
            segs.get(i).map_or(Point::ZERO, |(c, _)| c.eval((s - i as f64).clamp(0.0, 1.0)))
        };
        // The side from `a` to `b`: its points, and where along it (0 to 1) each lies. It runs
        // through pieces of segments, split at `a` and `b`. Each piece takes its share of the side
        // by the length of its control polygon and holds points at even steps of its parameter
        // (at least 16, and at least `SIDE` points on the side), its ends included. A side of
        // many pieces takes fewer points in each, so it holds at most `MAX_SIDE_POINTS` points,
        // or one for each piece when it has more pieces than that.
        let side = |a: f64, b: f64| {
            let span = (b - a).rem_euclid(n);
            if !(span.is_finite() && span > 0.0) {
                return (vec![at(a), at(a)], vec![0.0, 1.0]);
            }
            let mut pieces: Vec<CubicBez> = vec![];
            let (mut s, end) = (a, a + span);
            while s < end - 1e-12 && pieces.len() <= segs.len() {
                let i = s.floor();
                let next = (i + 1.0).min(end);
                if let Some((c, _)) = segs.get((i as usize) % segs.len()) {
                    pieces.push(c.subsegment((s - i).clamp(0.0, 1.0)..(next - i).clamp(0.0, 1.0)));
                }
                s = next;
            }
            let polygon = |c: &CubicBez| c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3);
            let mut weights: Vec<f64> = pieces.iter().map(polygon).collect();
            let mut total: f64 = weights.iter().sum();
            if !(total.is_finite() && total > 0.0) {
                weights.iter_mut().for_each(|w| *w = 1.0);
                total = weights.len().max(1) as f64;
            }
            let count = pieces.len().max(1);
            let per = 16usize.max((SIDE - 1).div_ceil(count));
            let per = if count * per < MAX_SIDE_POINTS { per } else { (MAX_SIDE_POINTS / count).max(1) };
            let mut pts = vec![pieces.first().map_or_else(|| at(a), |c| c.p0)];
            let mut knots = vec![0.0];
            let mut before = 0.0;
            for (c, w) in pieces.iter().zip(&weights) {
                for k in 1..=per {
                    let f = k as f64 / per as f64;
                    pts.push(c.eval(f));
                    knots.push((before + w * f) / total);
                }
                before += w;
            }
            if let Some(last) = knots.last_mut() {
                *last = 1.0;
            }
            (pts, knots)
        };
        let (tl, tr, br, bl) = match corners {
            // Anchor `k` sits at the start of the first kept segment from it on.
            Some(c) => {
                let place = |k: usize| starts.iter().position(|i| *i >= k).unwrap_or(0) as f64;
                (place(c[0]), place(c[1]), place(c[2]), place(c[3]))
            }
            None => {
                let corner = |q: Point| top_object_corner(&segs, &order, r, q);
                (corner(Point::new(r.x0, r.y0)), corner(Point::new(r.x1, r.y0)), corner(Point::new(r.x1, r.y1)), corner(Point::new(r.x0, r.y1)))
            }
        };
        let flip = |(mut p, mut u): (Vec<Point>, Vec<f64>)| {
            p.reverse();
            u.reverse();
            u.iter_mut().for_each(|x| *x = 1.0 - *x);
            (p, u)
        };
        let (top, top_u) = side(tl, tr);
        let (right, right_u) = side(tr, br);
        let (bottom, bottom_u) = flip(side(br, bl));
        let (left, left_u) = flip(side(bl, tl));
        Some(Self { top, right, bottom, left, knots: Some(Box::new([top_u, right_u, bottom_u, left_u])) })
    }

    /// The point at `u` along a side; past its ends the end segments go on straight (art an
    /// envelope bends can reach past the content's box: outlined strokes).
    fn sample(side: &[Point], u: f64) -> Point {
        let s = u * (side.len() - 1) as f64;
        let i = (s.floor().max(0.0) as usize).min(side.len() - 2);
        side[i].lerp(side[i + 1], s - i as f64)
    }

    /// The point at `u` along a side whose points lie at `knots` (rising, 0 to 1); past its ends
    /// the end segments go on straight, as in [`Self::sample`].
    fn sample_at(side: &[Point], knots: &[f64], u: f64) -> Point {
        if side.len() < 2 || side.len() != knots.len() {
            return side.first().copied().unwrap_or_default();
        }
        let i = knots.partition_point(|k| *k <= u).clamp(1, side.len() - 1) - 1;
        let (k0, k1) = (knots[i], knots[i + 1]);
        let f = if k1 > k0 { (u - k0) / (k1 - k0) } else { 0.0 };
        side[i].lerp(side[i + 1], f)
    }

    /// The point at `u` along side `which` (0 top, 1 right, 2 bottom, 3 left).
    fn side_at(&self, which: usize, u: f64) -> Point {
        let side = match which {
            0 => &self.top,
            1 => &self.right,
            2 => &self.bottom,
            _ => &self.left,
        };
        match self.knots.as_deref().and_then(|k| k.get(which)) {
            Some(knots) => Self::sample_at(side, knots, u),
            None => Self::sample(side, u),
        }
    }

    /// The surface point at (u, v) ∈ [0,1]².
    pub fn eval(&self, u: f64, v: f64) -> Point {
        let (t, b, l, r) = (self.side_at(0, u), self.side_at(2, u), self.side_at(3, v), self.side_at(1, v));
        let [p00, p10, p11, p01] = self.corners();
        let c = (1.0 - v) * t.to_vec2() + v * b.to_vec2() + (1.0 - u) * l.to_vec2() + u * r.to_vec2()
            - ((1.0 - u) * (1.0 - v) * p00.to_vec2() + u * (1.0 - v) * p10.to_vec2() + (1.0 - u) * v * p01.to_vec2() + u * v * p11.to_vec2());
        c.to_point()
    }

    /// The four corners TL, TR, BR, BL.
    pub fn corners(&self) -> [Point; 4] {
        let end = |side: &[Point]| side.last().copied().unwrap_or_default();
        [self.top[0], end(&self.top), end(&self.bottom), self.bottom[0]]
    }
}

/// The most points one side of [`Coons::from_top_object`] holds while it adds points between its
/// anchors.
const MAX_SIDE_POINTS: usize = 1 << 16;

/// How far into the outline's bounding box, as a share of its width and of its height, a corner
/// of a top object looks for an anchor.
const CORNER_REACH: f64 = 0.475;

/// A handle this long (in points) or shorter makes its anchor a corner of a top object.
const SHORT_HANDLE: f64 = 0.01;

/// An anchor whose tangent directions, as unit vectors, differ by less than this in x and in y is
/// smooth, and is not a corner of a top object.
const SMOOTH_TURN: f64 = 0.01;

/// A top object's corner this close (in curve parameter) to either end of its segment moves onto
/// the anchor there.
const CORNER_END_SNAP: f64 = 0.005;

/// Is the anchor at `a`, reached by handle `h_in` and left by handle `h_out` on an outline that
/// runs clockwise on screen, a corner of a top object? A handle of [`SHORT_HANDLE`] or shorter
/// makes it one. Otherwise its tangent directions must differ by [`SMOOTH_TURN`] or more and turn
/// clockwise, so it points out of the outline. A straight segment's handles here are the ones
/// [`Coons::from_top_object`] gives it, a third of the way along, so an anchor between two
/// straight segments longer than three times [`SHORT_HANDLE`] is a corner only when it turns out
/// of the outline.
fn top_object_corner_anchor(h_in: Point, a: Point, h_out: Point) -> bool {
    let (vin, vout) = (a - h_in, h_out - a);
    let (len_in, len_out) = (vin.hypot(), vout.hypot());
    if len_in <= SHORT_HANDLE || len_out <= SHORT_HANDLE {
        return true;
    }
    let (ui, uo) = (vin / len_in, vout / len_out);
    if (ui.x - uo.x).abs() < SMOOTH_TURN && (ui.y - uo.y).abs() < SMOOTH_TURN {
        return false;
    }
    ui.cross(uo) >= 0.0
}

/// Where along `segs` (segment index + parameter, in [0, n)) the corner of a top object nearest
/// corner `q` of its bounding box `r` lies, for an outline that runs clockwise on screen. It is
/// the nearest anchor to `q` that is a corner ([`top_object_corner_anchor`]) and lies within
/// [`CORNER_REACH`] of the box's width and height from `q`. Without one, it is the nearest point
/// of the outline to `q`, moved onto its segment's anchor when within [`CORNER_END_SNAP`] of
/// either end. `order` gives where each segment's first anchor and the segment itself come in the
/// outline as drawn, which may run the other way; on a tie the one met first there wins.
fn top_object_corner(segs: &[(CubicBez, bool)], order: &[(usize, usize)], r: Rect, q: Point) -> f64 {
    use vectorcraft_geom::kurbo::ParamCurveNearest;
    let n = segs.len();
    if n == 0 {
        return 0.0;
    }
    let rank = |i: usize| order.get(i).copied().unwrap_or((i, i));
    let (reach_x, reach_y) = (r.width() * CORNER_REACH, r.height() * CORNER_REACH);
    let mut best: Option<(f64, usize, usize)> = None;
    for i in 0..n {
        let (Some((prev, _)), Some((cur, _))) = (segs.get((i + n - 1) % n), segs.get(i)) else { continue };
        let a = cur.p0;
        if (a.x - q.x).abs() > reach_x || (a.y - q.y).abs() > reach_y || !top_object_corner_anchor(prev.p2, a, cur.p1) {
            continue;
        }
        let (d, k) = (a.distance_squared(q), rank(i).0);
        if best.is_none_or(|(b, bk, _)| d < b || (d == b && k < bk)) {
            best = Some((d, k, i));
        }
    }
    if let Some((_, _, i)) = best {
        return i as f64;
    }
    let tol = (r.width() + r.height()).max(1e-6) * 1e-9;
    let nearest = segs
        .iter()
        .enumerate()
        .map(|(i, (c, _))| (c.nearest(q, tol), i))
        .min_by(|a, b| a.0.distance_sq.total_cmp(&b.0.distance_sq).then(rank(a.1).1.cmp(&rank(b.1).1)));
    let Some((near, i)) = nearest else { return 0.0 };
    if near.t > CORNER_END_SNAP && near.t < 1.0 - CORNER_END_SNAP {
        i as f64 + near.t
    } else if near.t < 0.5 {
        i as f64
    } else {
        ((i + 1) % n) as f64
    }
}

// =====================================================================================
// Gradient meshes
// =====================================================================================

/// The handles, as offsets from `ends`, of the cubic from `ends.0` to `ends.1` that comes closest
/// (least squares) to `side` between `span.0` and `span.1`; `None` when they can't be computed.
/// Each point of the side is matched to the curve's nearest point, refined over a few rounds.
fn fit_edge(side: &[Point], span: (f64, f64), ends: (Point, Point)) -> Option<(Vec2, Vec2)> {
    const N: usize = 32;
    const ROUNDS: usize = 6;
    let (p0, p3) = (ends.0, ends.1);
    let qs: Vec<Point> = (1..N).map(|i| Coons::sample(side, span.0 + (span.1 - span.0) * i as f64 / N as f64)).collect();
    let mut ts: Vec<f64> = (1..N).map(|i| i as f64 / N as f64).collect();
    let mut fit = None;
    for _ in 0..ROUNDS {
        let (mut c11, mut c12, mut c22) = (0.0, 0.0, 0.0);
        let (mut x1, mut x2) = (Vec2::ZERO, Vec2::ZERO);
        for (q, &t) in qs.iter().zip(&ts) {
            let s = 1.0 - t;
            let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
            let r = q.to_vec2() - b0 * p0.to_vec2() - b3 * p3.to_vec2();
            c11 += b1 * b1;
            c12 += b1 * b2;
            c22 += b2 * b2;
            x1 += b1 * r;
            x2 += b2 * r;
        }
        let det = c11 * c22 - c12 * c12;
        if det.abs() < 1e-12 {
            break;
        }
        let (p1, p2) = (((c22 * x1 - c12 * x2) / det).to_point(), ((c11 * x2 - c12 * x1) / det).to_point());
        if !(p1.is_finite() && p2.is_finite()) {
            break;
        }
        fit = Some((p1 - p0, p2 - p3));
        // One Newton step towards the nearest point of the curve for each point of the side.
        let c = CubicBez::new(p0, p1, p2, p3);
        let (a, b, k) = (p1 - p0, p2 - p1, p3 - p2);
        for (q, t) in qs.iter().zip(ts.iter_mut()) {
            let s = 1.0 - *t;
            let v = 3.0 * (s * s * a + 2.0 * s * *t * b + *t * *t * k);
            let acc = 6.0 * (s * (b - a) + *t * (k - b));
            let e = c.eval(*t) - *q;
            let den = v.hypot2() + e.dot(acc);
            if den.abs() > 1e-12 {
                *t = (*t - e.dot(v) / den).clamp(0.0, 1.0);
            }
        }
    }
    fit
}

/// The cubic from `p0` to `p3` leaving `p0` along `d0` and reaching `p3` against `d1` (unit
/// vectors; `d1` points from `p3` back into the curve) that comes closest (least squares) to the
/// points `qs`, which lie along the way in order; `None` when no such cubic has handles of positive
/// length. Each point is matched to the curve's nearest point, refined over a few rounds, as in
/// [`fit_edge`].
fn fit_tangents(qs: &[Point], (p0, d0): (Point, Vec2), (p3, d1): (Point, Vec2)) -> Option<CubicBez> {
    const ROUNDS: usize = 6;
    let n = qs.len() + 1;
    let mut ts: Vec<f64> = (1..n).map(|i| i as f64 / n as f64).collect();
    let mut fit = None;
    for _ in 0..ROUNDS {
        let (mut c11, mut c12, mut c22, mut x1, mut x2) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (q, &t) in qs.iter().zip(&ts) {
            let s = 1.0 - t;
            let (b0, b1, b2, b3) = (s * s * s, 3.0 * s * s * t, 3.0 * s * t * t, t * t * t);
            let (a1, a2) = (d0 * b1, d1 * b2);
            let r = q.to_vec2() - (b0 + b1) * p0.to_vec2() - (b2 + b3) * p3.to_vec2();
            c11 += a1.dot(a1);
            c12 += a1.dot(a2);
            c22 += a2.dot(a2);
            x1 += r.dot(a1);
            x2 += r.dot(a2);
        }
        let det = c11 * c22 - c12 * c12;
        if det.abs() < 1e-12 {
            break;
        }
        let (l1, l2) = ((c22 * x1 - c12 * x2) / det, (c11 * x2 - c12 * x1) / det);
        if !(l1.is_finite() && l2.is_finite() && l1 > 0.0 && l2 > 0.0) {
            break;
        }
        let c = CubicBez::new(p0, p0 + d0 * l1, p3 + d1 * l2, p3);
        fit = Some(c);
        let (a, b, k) = (c.p1 - c.p0, c.p2 - c.p1, c.p3 - c.p2);
        for (q, t) in qs.iter().zip(ts.iter_mut()) {
            let s = 1.0 - *t;
            let v = 3.0 * (s * s * a + 2.0 * s * *t * b + *t * *t * k);
            let acc = 6.0 * (s * (b - a) + *t * (k - b));
            let e = c.eval(*t) - *q;
            let den = v.hypot2() + e.dot(acc);
            if den.abs() > 1e-12 {
                *t = (*t - e.dot(v) / den).clamp(0.0, 1.0);
            }
        }
    }
    fit
}

/// How far (in points) a border edge of a mesh made from a path may stray from the path's outline
/// before the mesh takes a line through an anchor of the outline there; a thousandth of the outline's
/// size when that is less.
const BORDER_TOLERANCE: f64 = 0.1;

/// The most lines each way a mesh made from a path takes to keep its border on the outline
/// ([`BORDER_TOLERANCE`]); past that, each border edge keeps its closest fit.
pub(super) const MAX_BORDER_LINES: usize = 256;

/// The most rounds of new lines a mesh made from a path takes to keep its border on the outline.
const MAX_BORDER_ROUNDS: usize = 64;

/// A point this close (in points) to a mesh's outline is on it, for [`GradientMesh::locate_within`]
/// with any tolerance.
const ON_OUTLINE: f64 = 1e-6;

/// An anchor whose directions in and out differ by more than this (radians) is a corner: a border
/// edge that can't follow the outline takes a mesh line through its sharpest corner first.
const SHARP_TURN: f64 = 0.01;

/// Steps of each segment's parameter at which [`Outline`] measures the segment's length.
const LENGTH_STEPS: usize = 32;

/// Points of the outline each border edge of a mesh made from a path is fitted to
/// ([`Outline::fit`]).
const FIT_POINTS: usize = 32;

/// Points of the outline, and of the fitted edge, at which [`Outline::fit`] measures how far the
/// two stray from each other.
const CHECK_POINTS: usize = 128;

/// The outline a gradient mesh is made from: the largest subpath of a path, running clockwise on
/// screen, an open one closed by a straight segment, without segments of no length. A place on it
/// is a length from its first anchor, measured along a polyline through even steps of each
/// segment's parameter; the same length always gives the same point.
struct Outline {
    segs: Vec<CubicBez>,
    /// Where each segment starts, and the whole length last.
    starts: Vec<f64>,
    /// For each segment, the length from its start at each of [`LENGTH_STEPS`] + 1 even steps of
    /// its parameter.
    table: Vec<[f64; LENGTH_STEPS + 1]>,
    /// Lengths closer than this are the same place.
    eps: f64,
}

/// One side of the patch a mesh made from a path spans: the stretch of the outline from `start`,
/// `len` long, run backwards when `reversed` (the bottom and left sides run against the outline).
#[derive(Clone, Copy)]
struct Side {
    start: f64,
    len: f64,
    reversed: bool,
}

impl Side {
    /// The place on the outline at `f` (0 to 1) along the side.
    fn at(&self, f: f64) -> f64 {
        self.start + if self.reversed { (1.0 - f) * self.len } else { f * self.len }
    }
}

/// The direction a cubic leaves its start in (`false`) or reaches its end in (`true`), from the
/// first handle or point that is not at the end; zero when all its points are together.
fn cubic_tangent(c: &CubicBez, end: bool) -> Vec2 {
    let near = 1e-12 * (c.p0.to_vec2().hypot() + c.p3.to_vec2().hypot() + 1.0);
    let options = if end { [c.p3 - c.p2, c.p3 - c.p1, c.p3 - c.p0] } else { [c.p1 - c.p0, c.p2 - c.p0, c.p3 - c.p0] };
    options.into_iter().find(|v| v.hypot() > near).unwrap_or(Vec2::ZERO)
}

impl Outline {
    /// The outline of `path`; `None` when it has no subpath of two anchors or more, a point that
    /// isn't finite, or all its points on one line (a line there and back).
    fn new(path: &PathData) -> Option<Self> {
        let sp = path.subpaths.iter().filter(|s| s.anchors.len() >= 2).max_by(|a, b| a.area().abs().total_cmp(&b.area().abs()))?;
        if !sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return None;
        }
        let mut sp = sp.clone();
        if sp.signed_area() < 0.0 {
            sp.reverse();
        }
        // A straight segment's parameter runs evenly along it.
        let line = |a: Point, b: Point| CubicBez::new(a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b);
        let mut segs: Vec<CubicBez> =
            (0..sp.segment_count()).map(|i| if sp.segment_is_line(i) { line(sp.segment(i).p0, sp.segment(i).p3) } else { sp.segment(i) }).collect();
        if !sp.closed
            && let (Some(a), Some(b)) = (sp.anchors.last(), sp.anchors.first())
        {
            segs.push(line(a.p, b.p));
        }
        segs.retain(|c| [c.p1, c.p2, c.p3].iter().any(|p| p.distance(c.p0) > 1e-9));
        if segs.len() < 2 {
            return None;
        }
        let mut starts = Vec::with_capacity(segs.len() + 1);
        let mut table = Vec::with_capacity(segs.len());
        let mut total = 0.0;
        for c in &segs {
            let mut lens = [0.0; LENGTH_STEPS + 1];
            let mut prev = c.p0;
            for (k, len) in lens.iter_mut().enumerate().skip(1) {
                let p = c.eval(k as f64 / LENGTH_STEPS as f64);
                *len = prev.distance(p);
                prev = p;
            }
            for k in 1..=LENGTH_STEPS {
                lens[k] += lens[k - 1];
            }
            starts.push(total);
            total += lens[LENGTH_STEPS];
            table.push(lens);
        }
        starts.push(total);
        if !(total.is_finite() && total > 0.0) {
            return None;
        }
        // An outline whose points all lie on one line encloses nothing; one that crosses itself
        // (a bow tie) may have areas that cancel, and is still followed.
        let pts: Vec<Point> = segs.iter().flat_map(|c| [c.p0, c.p1, c.p2, c.p3]).collect();
        let first = pts.first().copied()?;
        let far = pts.iter().copied().max_by(|a, b| a.distance_squared(first).total_cmp(&b.distance_squared(first)))?;
        let dir = far - first;
        let width = pts.iter().map(|p| ((*p - first).cross(dir) / dir.hypot()).abs()).fold(0.0, f64::max);
        if !(dir.hypot() > 0.0 && width > 1e-9 * total) {
            return None;
        }
        Some(Self { segs, starts, table, eps: total * 1e-9 })
    }

    fn total(&self) -> f64 {
        self.starts.last().copied().unwrap_or_default()
    }

    /// The bounding box of the outline.
    fn bounds(&self) -> Option<Rect> {
        self.segs.iter().map(|c| c.bounding_box()).reduce(|a, b| a.union(b))
    }

    /// The parameter of segment `i` `local` along it (clamped to the segment).
    fn seg_t(&self, i: usize, local: f64) -> f64 {
        let Some(lens) = self.table.get(i) else { return 0.0 };
        let k = lens.partition_point(|x| *x <= local).clamp(1, LENGTH_STEPS) - 1;
        let (l0, l1) = (lens[k], lens[k + 1]);
        let f = if l1 > l0 { ((local - l0) / (l1 - l0)).clamp(0.0, 1.0) } else { 0.0 };
        (k as f64 + f) / LENGTH_STEPS as f64
    }

    /// The segment and its parameter at place `s` (wrapping around); a place at the very end of a
    /// segment is the start of the next.
    fn locate(&self, s: f64) -> (usize, f64) {
        let n = self.segs.len();
        let s = s.rem_euclid(self.total());
        let mut i = self.starts.partition_point(|x| *x <= s).clamp(1, n) - 1;
        if self.starts.get(i + 1).is_some_and(|end| end - s <= self.eps) {
            i = (i + 1) % n;
            return (i, 0.0);
        }
        (i, self.seg_t(i, s - self.starts[i]))
    }

    fn point(&self, s: f64) -> Point {
        let (i, t) = self.locate(s);
        self.segs.get(i).map_or(Point::ZERO, |c| c.eval(t))
    }

    /// The anchors strictly inside the stretch from `a`, `len` long: each one's distance from `a`
    /// and the segment it starts.
    fn anchors_in(&self, a: f64, len: f64) -> Vec<(f64, usize)> {
        let (total, n) = (self.total(), self.segs.len());
        let a = a.rem_euclid(total);
        let (lo, hi) = (a + self.eps, a + len - self.eps);
        let mut out = vec![];
        for lap in [0.0, total] {
            let from = self.starts[..n].partition_point(|x| x + lap <= lo);
            for (i, start) in self.starts[..n].iter().enumerate().skip(from) {
                let p = start + lap;
                if p >= hi {
                    break;
                }
                out.push((p - a, i));
            }
        }
        out
    }

    /// The piece of the outline from `a`, `len` long, which no anchor lies inside.
    fn piece(&self, a: f64, len: f64) -> CubicBez {
        let (i, t0) = self.locate(a);
        let Some(c) = self.segs.get(i) else { return CubicBez::new(Point::ZERO, Point::ZERO, Point::ZERO, Point::ZERO) };
        // At a segment's start (a place at the end of the one before counts as that), none of it is behind.
        let behind = if t0 == 0.0 { 0.0 } else { a.rem_euclid(self.total()) - self.starts[i] };
        let t1 = self.seg_t(i, behind + len);
        c.subsegment(t0..t1.max(t0))
    }

    /// The direction the outline leaves place `s` in (`into` false) or reaches it in (`into` true).
    fn tangent(&self, s: f64, into: bool) -> Vec2 {
        let n = self.segs.len();
        let (i, t) = self.locate(s);
        let c = match (into, t <= 0.0) {
            (true, true) => self.segs[(i + n - 1) % n],
            (true, false) => self.segs[i].subsegment(0.0..t),
            (false, _) => self.segs[i].subsegment(t..1.0),
        };
        cubic_tangent(&c, into)
    }

    /// How sharply the outline turns at the anchor starting segment `i` (radians, 0 to π).
    fn turn(&self, i: usize) -> f64 {
        let s = self.starts[i];
        let (din, dout) = (self.tangent(s, true), self.tangent(s, false));
        din.cross(dout).atan2(din.dot(dout)).abs()
    }

    /// The place on the outline nearest `q`.
    fn nearest(&self, q: Point) -> f64 {
        use vectorcraft_geom::kurbo::ParamCurveNearest;
        let mut best = (f64::INFINITY, 0.0);
        for (i, c) in self.segs.iter().enumerate() {
            let near = c.nearest(q, 1e-9);
            if near.distance_sq < best.0 {
                // The length at that parameter, from the table.
                let lens = &self.table[i];
                let x = near.t * LENGTH_STEPS as f64;
                let k = (x.floor() as usize).min(LENGTH_STEPS - 1);
                let local = lens[k] + (lens[k + 1] - lens[k]) * (x - k as f64);
                best = (near.distance_sq, self.starts[i] + local);
            }
        }
        best.1
    }

    /// The cubic standing for the stretch from `a`, `len` long, and how far it strays from it: the
    /// piece of the segment when no anchor lies inside, otherwise a cubic fitted to the outline with
    /// its ends along the outline's own directions there.
    fn fit(&self, a: f64, len: f64) -> (CubicBez, f64) {
        let (p0, p3) = (self.point(a), self.point(a + len));
        if len <= self.eps {
            return (CubicBez::new(p0, p0, p0, p0), 0.0);
        }
        let anchors = self.anchors_in(a, len);
        if anchors.is_empty() {
            return (self.piece(a, len), 0.0);
        }
        let qs: Vec<Point> = (1..FIT_POINTS).map(|i| self.point(a + len * i as f64 / FIT_POINTS as f64)).collect();
        let (d0, d1) = (self.tangent(a, false), -self.tangent(a + len, true));
        let unit = |v: Vec2| if v.hypot() > 0.0 { Some(v / v.hypot()) } else { None };
        let straight = CubicBez::new(p0, p0.lerp(p3, 1.0 / 3.0), p0.lerp(p3, 2.0 / 3.0), p3);
        let c = match (unit(d0), unit(d1)) {
            (Some(d0), Some(d1)) => fit_tangents(&qs, (p0, d0), (p3, d1)),
            _ => None,
        }
        .unwrap_or(straight);
        // Both ways: the outline's points from the cubic, and the cubic's points from the outline.
        // The outline is checked at even steps and at every anchor inside, so a spike or a notch
        // narrower than a step is not missed.
        use vectorcraft_geom::kurbo::ParamCurveNearest;
        let mut at: Vec<f64> = (0..=CHECK_POINTS).map(|i| len * i as f64 / CHECK_POINTS as f64).chain(anchors.iter().map(|(o, _)| *o)).collect();
        at.sort_by(f64::total_cmp);
        let line: Vec<Point> = at.iter().map(|o| self.point(a + o)).collect();
        let off_outline = line.iter().map(|q| c.nearest(*q, 1e-9).distance_sq.sqrt()).fold(0.0, f64::max);
        let off_cubic = (0..=CHECK_POINTS)
            .map(|i| {
                let p = c.eval(i as f64 / CHECK_POINTS as f64);
                line.windows(2)
                    .map(|w| {
                        let (u, v) = (w[0], w[1]);
                        let d = v - u;
                        let t = if d.hypot2() > 0.0 { ((p - u).dot(d) / d.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
                        p.distance(u + d * t)
                    })
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max);
        (c, off_outline.max(off_cubic))
    }

    /// The border edge of `side` from `f0` to `f1` (0 to 1 along it, rising), run the way the side
    /// runs, and when it strays from the outline by more than `tol`, where along the side (0 to 1)
    /// a mesh line through an anchor would help: the sharpest corner inside it, or the anchor
    /// nearest its middle when none is sharp.
    fn edge(&self, side: &Side, f0: f64, f1: f64, tol: f64) -> (CubicBez, Option<f64>) {
        let len = (f1 - f0) * side.len;
        let a = side.at(if side.reversed { f1 } else { f0 });
        let (c, off) = self.fit(a, len);
        let c = if side.reversed { CubicBez::new(c.p3, c.p2, c.p1, c.p0) } else { c };
        if off <= tol || side.len <= 0.0 {
            return (c, None);
        }
        let anchors = self.anchors_in(a, len);
        let sharpest = anchors.iter().map(|&(o, i)| (self.turn(i), o)).max_by(|x, y| x.0.total_cmp(&y.0));
        let at = match sharpest {
            Some((turn, o)) if turn > SHARP_TURN => Some(o),
            _ => anchors.iter().map(|&(o, _)| o).min_by(|x, y| (x - len / 2.0).abs().total_cmp(&(y - len / 2.0).abs())),
        };
        let split = at.map(|o| if side.reversed { f1 - o / side.len } else { f0 + o / side.len });
        (c, split.filter(|f| *f > f0 && *f < f1))
    }

    /// The cuts (0 to 1) of the lines across sides `one` and `other`: `even`, and as many more as
    /// keep every border edge along them within `tol` of the outline (see [`Self::edge`]), at most
    /// [`MAX_BORDER_LINES`] in all.
    fn cuts(&self, one: &Side, other: &Side, even: Vec<f64>, tol: f64) -> Vec<f64> {
        let mut cuts = even;
        for _ in 0..MAX_BORDER_ROUNDS {
            let mut add: Vec<f64> =
                [one, other].iter().flat_map(|s| cuts.windows(2).filter_map(|w| self.edge(s, w[0], w[1], tol).1).collect::<Vec<_>>()).collect();
            add.retain(|f| cuts.iter().all(|c| (c - f).abs() > 1e-9));
            add.truncate((MAX_BORDER_LINES + 1).saturating_sub(cuts.len()));
            if add.is_empty() {
                break;
            }
            cuts.extend(add);
            cuts.sort_by(f64::total_cmp);
            cuts.dedup_by(|x, y| (*x - *y).abs() <= 1e-9);
        }
        cuts
    }

    /// The points along `side` at `knots` (rising, 0 to 1).
    fn side_points(&self, side: &Side, knots: &[f64]) -> Vec<Point> {
        knots.iter().map(|f| self.point(side.at(*f))).collect()
    }
}

/// The sorted union of `cuts` and `SIDE` even steps.
fn side_knots(cuts: &[f64]) -> Vec<f64> {
    let mut k: Vec<f64> = (0..SIDE).map(|i| i as f64 / (SIDE - 1) as f64).chain(cuts.iter().copied()).collect();
    k.sort_by(f64::total_cmp);
    k.dedup_by(|x, y| (*x - *y).abs() <= 1e-9);
    k
}

/// The handles of one cubic standing for two edges that meet at point `mid.0`, whose handles
/// towards them are `mid.1` (towards `a`) and `mid.2` (towards `b`), as offsets from their outer
/// ends: each outer handle grown back by the share of the curve its edge had, which undoes the
/// split [`GradientMesh::insert_row`] and [`GradientMesh::insert_col`] make, so deleting a mesh
/// line keeps the curve of the border it crossed (#3187). The share is the split parameter, read
/// from the lengths of the middle handles. When the edges no longer fit one cubic (a point or a
/// handle was moved since the split), the cubic grown back would not pass through the middle
/// point, and the merged edge is straight.
fn merged_handles(a: (Point, Vec2), mid: (Point, Vec2, Vec2), b: (Point, Vec2)) -> (Vec2, Vec2) {
    let straight = ((b.0 - a.0) / 3.0, (a.0 - b.0) / 3.0);
    let (la, lb) = (mid.1.hypot(), mid.2.hypot());
    let t = la / (la + lb);
    if !(0.01..=0.99).contains(&t) {
        return straight;
    }
    let (ha, hb) = (a.1 / t, b.1 / (1.0 - t));
    let through = CubicBez::new(a.0, a.0 + ha, b.0 + hb, b.0).eval(t);
    let tol = 1e-3 * (a.0.distance(mid.0) + mid.0.distance(b.0)).max(1e-9);
    if ha.is_finite() && hb.is_finite() && through.distance(mid.0) <= tol { (ha, hb) } else { straight }
}

/// `0, 1/n, …, 1` for `n` (clamped to 1..=200) even steps.
fn even_steps(n: u32) -> Vec<f64> {
    let n = n.clamp(1, 200);
    (0..=n).map(|i| i as f64 / n as f64).collect()
}

/// The number of points of a `(rows+1)×(cols+1)` grid, `None` past `usize`: counts come from
/// files, and u32::MAX rows and columns (65,535 on 32-bit targets) would wrap to an empty grid.
pub(super) fn grid_len(rows: u32, cols: u32) -> Option<usize> {
    (rows as usize).checked_add(1)?.checked_mul((cols as usize).checked_add(1)?)
}

impl GradientMesh {
    /// The most quads one tessellation ([`Self::quads`]) makes, and so the most patches a mesh can
    /// have: about 170 bytes a quad, so about 170 MB. A file's mesh can have any number of patches
    /// and the view any zoom; the subdivision per patch gives way before memory does, and a mesh
    /// with more patches than that is malformed, like a point count that doesn't fit (#3592).
    pub const MAX_QUADS: usize = 1 << 20;

    pub fn idx(&self, r: usize, c: usize) -> usize {
        r * (self.cols as usize + 1) + c
    }
    pub fn is_valid(&self) -> bool {
        self.rows >= 1 && self.cols >= 1 && grid_len(self.rows, self.cols) == Some(self.points.len()) && self.patches() <= Self::MAX_QUADS
    }

    /// The number of patches (rows times columns), saturating.
    fn patches(&self) -> usize {
        (self.rows as usize).saturating_mul(self.cols as usize)
    }

    /// A 1×1 mesh from its four corners (bottom-left, top-left, top-right, bottom-right in u/v
    /// order: row 0 is v = 0, columns follow u), each with its handles ([`H_RIGHT`] …).
    fn patch(corners: [(Point, [Vec2; 4], Color); 4]) -> Self {
        let point = |(p, handles, color): (Point, [Vec2; 4], Color)| MeshPoint { p, color, opacity: 1.0, handles };
        let [a, b, c, d] = corners;
        Self { rows: 1, cols: 1, points: vec![point(a), point(d), point(b), point(c)] }
    }

    /// A 1×1 mesh from a Coons patch's twelve boundary points (in the order PDF and PostScript
    /// shadings list them: v runs cp0 → cp3 along cp1 and cp2, u runs cp0 → cp9 along cp11 and
    /// cp10) and its corner colours (at cp0, cp3, cp6, cp9); `None` if a point isn't finite.
    pub fn coons(cp: &[Point; 12], colors: [Color; 4]) -> Option<Self> {
        if !cp.iter().all(|p| p.is_finite()) {
            return None;
        }
        let h = |a: usize, b: usize| cp[b] - cp[a];
        let handles = |right: Vec2, left: Vec2, down: Vec2, up: Vec2| {
            let mut h = [Vec2::ZERO; 4];
            (h[H_RIGHT], h[H_LEFT], h[H_DOWN], h[H_UP]) = (right, left, down, up);
            h
        };
        let [c0, c1, c2, c3] = colors;
        Some(Self::patch([
            (cp[0], handles(h(0, 11), Vec2::ZERO, h(0, 1), Vec2::ZERO), c0),
            (cp[3], handles(h(3, 4), Vec2::ZERO, Vec2::ZERO, h(3, 2)), c1),
            (cp[6], handles(Vec2::ZERO, h(6, 5), Vec2::ZERO, h(6, 7)), c2),
            (cp[9], handles(Vec2::ZERO, h(9, 10), h(9, 8), Vec2::ZERO), c3),
        ]))
    }

    /// A 1×1 mesh of a Gouraud-shaded triangle (a patch with two corners together); `None` if a
    /// point isn't finite.
    pub fn triangle(corners: [(Point, Color); 3]) -> Option<Self> {
        if !corners.iter().all(|(p, _)| p.is_finite()) {
            return None;
        }
        let [a, b, c] = corners.map(|(p, color)| (p, [Vec2::ZERO; 4], color));
        Some(Self::patch([a, b, c, c]))
    }

    /// A mesh over surface `s` (u, v ∈ [0,1]) with handles from its partial derivatives.
    pub fn from_surface(rows: u32, cols: u32, s: &dyn Fn(f64, f64) -> Point, color: &dyn Fn(f64, f64) -> Color) -> Self {
        let (us, vs) = (even_steps(cols), even_steps(rows));
        Self::from_grid(&us, &vs, s, &|r, c, _| (color(us[c], vs[r]), 1.0))
    }

    /// A mesh over surface `s` (u, v ∈ [0,1]) whose column lines sit at `us` and row lines at `vs`
    /// (ascending, at least two each; equal neighbours make a patch of no width, a sharp colour
    /// change), with handles from its partial derivatives. `color(row, col, point)` gives each
    /// point its colour and opacity.
    pub fn from_grid(us: &[f64], vs: &[f64], s: &dyn Fn(f64, f64) -> Point, color: &dyn Fn(usize, usize, Point) -> (Color, f32)) -> Self {
        let (rows, cols) = (vs.len().saturating_sub(1), us.len().saturating_sub(1));
        let eps = 1e-3;
        let mut points = Vec::with_capacity(us.len() * vs.len());
        for (r, &v) in vs.iter().enumerate() {
            for (c, &u) in us.iter().enumerate() {
                let p = s(u, v);
                let d_u = (s((u + eps).min(1.0), v) - s((u - eps).max(0.0), v)) / ((u + eps).min(1.0) - (u - eps).max(0.0));
                let d_v = (s(u, (v + eps).min(1.0)) - s(u, (v - eps).max(0.0))) / ((v + eps).min(1.0) - (v - eps).max(0.0));
                // A third of the way to the neighbouring line along the tangent; outward handles on
                // the border are unused, so they stay at the point.
                let h = |d: Vec2, from: f64, to: Option<&f64>| to.map_or(Vec2::ZERO, |to| d * ((to - from) / 3.0));
                let handles = [
                    h(d_u, u, us.get(c + 1)),
                    h(d_u, u, c.checked_sub(1).map(|i| &us[i])),
                    h(d_v, v, vs.get(r + 1)),
                    h(d_v, v, r.checked_sub(1).map(|i| &vs[i])),
                ];
                let (color, opacity) = color(r, c, p);
                points.push(MeshPoint { p, color, opacity, handles });
            }
        }
        Self { rows: rows as u32, cols: cols as u32, points }
    }

    /// Create Gradient Mesh for a path: the outline split at its corners, coloured per `appearance`.
    pub fn for_path(path: &PathData, rows: u32, cols: u32, base: Color, appearance: MeshAppearance, highlight: f64) -> Option<Self> {
        Self::for_path_shaded(path, rows, cols, &|_| (base, 1.0), appearance, highlight)
    }

    /// As [`Self::for_path_with`], then lightened per `appearance` ([`Self::highlight`]), measured
    /// by where each line crosses the patch.
    pub fn for_path_shaded(
        path: &PathData,
        rows: u32,
        cols: u32,
        color_at: &dyn Fn(Point) -> (Color, f32),
        appearance: MeshAppearance,
        highlight: f64,
    ) -> Option<Self> {
        let (mut m, us, vs) = Self::build_for_path(path, rows, cols, color_at)?;
        m.highlight_at(appearance, highlight, &us, &vs);
        Some(m)
    }

    /// Create Gradient Mesh for a path: the outline split at its corners, each point taking the
    /// colour and opacity `color_at` gives at its position (a gradient fill's, say).
    pub fn for_path_with(path: &PathData, rows: u32, cols: u32, color_at: &dyn Fn(Point) -> (Color, f32)) -> Option<Self> {
        Self::build_for_path(path, rows, cols, color_at).map(|(m, ..)| m)
    }

    /// The mesh of [`Self::for_path_with`] and where its column and row lines cross the patch (0
    /// to 1). Its border lies on the outline of the path's largest subpath (#2752): the patch's
    /// corners are the outline's places nearest the corners of its bounding box (when those don't
    /// come in order round the outline, the top left one and the places a quarter, a half and three
    /// quarters of the way round from it), its points on the
    /// border sit on the outline, and each border edge is the piece of the outline between its
    /// points, a single cubic. Where the outline bends through an anchor between two of the `rows`
    /// × `cols` lines and no single cubic follows it within [`BORDER_TOLERANCE`], the mesh takes a
    /// line through that anchor (its sharpest corner first), as many as that needs up to
    /// [`MAX_BORDER_LINES`] each way. `None` when the path has no outline or a point that isn't
    /// finite; an outline whose points all lie on one line gives the mesh of its bounding box.
    fn build_for_path(path: &PathData, rows: u32, cols: u32, color_at: &dyn Fn(Point) -> (Color, f32)) -> Option<(Self, Vec<f64>, Vec<f64>)> {
        if !path.subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite()) {
            return None;
        }
        let (us, vs) = (even_steps(cols), even_steps(rows));
        let Some((o, sides)) = Outline::new(path).and_then(|o| {
            let r = o.bounds()?;
            let [tl, mut tr, mut br, mut bl] = [(r.x0, r.y0), (r.x1, r.y0), (r.x1, r.y1), (r.x0, r.y1)].map(|(x, y)| o.nearest(Point::new(x, y)));
            let span = |a: f64, b: f64| (b - a).rem_euclid(o.total());
            // Corners met out of order around the outline would make sides that overlap: the
            // other three then go a quarter of the way round from the top left each.
            let around = span(tl, tr) + span(tr, br) + span(br, bl) + span(bl, tl);
            if (around - o.total()).abs() > 1e-6 * o.total() {
                let quarter = o.total() / 4.0;
                (tr, br, bl) = (tl + quarter, tl + 2.0 * quarter, tl + 3.0 * quarter);
            }
            let sides = [
                Side { start: tl, len: span(tl, tr), reversed: false },
                Side { start: tr, len: span(tr, br), reversed: false },
                Side { start: br, len: span(br, bl), reversed: true },
                Side { start: bl, len: span(bl, tl), reversed: true },
            ];
            Some((o, sides))
        }) else {
            // No outline to follow: the patch of the flattened outline (its bounding box when it
            // encloses nothing), with each border edge fitted.
            let coons = Coons::from_path(path)?;
            let mut m = Self::from_grid(&us, &vs, &|u, v| coons.eval(u, v), &|_, _, p| color_at(p));
            m.fit_border(&coons, &us, &vs);
            return Some((m, us, vs));
        };
        let [top, right, bottom, left] = sides;
        let size = o.bounds().map_or(0.0, |r| r.width() + r.height());
        let tol = BORDER_TOLERANCE.min(size * 1e-3).max(o.eps);
        let us = o.cuts(&top, &bottom, us, tol);
        let vs = o.cuts(&left, &right, vs, tol);
        let (ku, kv) = (side_knots(&us), side_knots(&vs));
        let coons = Coons {
            top: o.side_points(&top, &ku),
            right: o.side_points(&right, &kv),
            bottom: o.side_points(&bottom, &ku),
            left: o.side_points(&left, &kv),
            knots: Some(Box::new([ku.clone(), kv.clone(), ku, kv])),
        };
        let mut m = Self::from_grid(&us, &vs, &|u, v| coons.eval(u, v), &|_, _, p| color_at(p));
        let (rows, cols) = (m.rows as usize, m.cols as usize);
        let edges = [(0, &top), (rows, &bottom)]
            .map(|(r, side)| (r, side, true))
            .into_iter()
            .chain([(0, &left), (cols, &right)].map(|(c, side)| (c, side, false)));
        for (line, side, along_u) in edges {
            let steps = if along_u { &us } else { &vs };
            for (k, w) in steps.windows(2).enumerate() {
                let (a, b, out, back) =
                    if along_u { (m.idx(line, k), m.idx(line, k + 1), H_RIGHT, H_LEFT) } else { (m.idx(k, line), m.idx(k + 1, line), H_DOWN, H_UP) };
                let (c, _) = o.edge(side, w[0], w[1], tol);
                m.points[a].p = c.p0;
                m.points[b].p = c.p3;
                m.points[a].handles[out] = c.p1 - c.p0;
                m.points[b].handles[back] = c.p2 - c.p3;
            }
        }
        Some((m, us, vs))
    }

    /// Give each edge on the mesh's border the handles that make it follow the outline it stands
    /// for, in place of the handles [`Self::from_grid`] estimates from the surface's derivatives
    /// (#3187). Where two border edges meet along a side, their handles are made collinear, so the
    /// border stays smooth there; the four corners keep the handles their own edges were fitted with.
    fn fit_border(&mut self, coons: &Coons, us: &[f64], vs: &[f64]) {
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        if !self.is_valid() || us.len() != cols + 1 || vs.len() != rows + 1 {
            return;
        }
        let edges = [(0, &coons.top), (rows, &coons.bottom)]
            .map(|(r, side)| (r, side, true))
            .into_iter()
            .chain([(0, &coons.left), (cols, &coons.right)].map(|(c, side)| (c, side, false)));
        for (line, side, along_u) in edges {
            let (n, steps) = if along_u { (cols, us) } else { (rows, vs) };
            for k in 0..n {
                let (a, b, out, back) = if along_u {
                    (self.idx(line, k), self.idx(line, k + 1), H_RIGHT, H_LEFT)
                } else {
                    (self.idx(k, line), self.idx(k + 1, line), H_DOWN, H_UP)
                };
                if let Some((h1, h2)) = fit_edge(side, (steps[k], steps[k + 1]), (self.points[a].p, self.points[b].p)) {
                    self.points[a].handles[out] = h1;
                    self.points[b].handles[back] = h2;
                }
            }
            for k in 1..n {
                let (i, out, back) = if along_u { (self.idx(line, k), H_RIGHT, H_LEFT) } else { (self.idx(k, line), H_DOWN, H_UP) };
                let h = &mut self.points[i].handles;
                let (fwd, bwd) = (h[out], h[back]);
                let dir = (fwd.normalize() - bwd.normalize()).normalize();
                if fwd.hypot2() > 0.0 && bwd.hypot2() > 0.0 && dir.is_finite() {
                    h[out] = dir * fwd.hypot();
                    h[back] = -dir * bwd.hypot();
                }
            }
        }
    }

    /// Create Gradient Mesh → Appearance: lighten the points towards white, to the centre or to
    /// the edges, by at most `highlight` %. Each point lightens towards white in its own colour
    /// model (paper white, no ink, for CMYK), so a CMYK or grey mesh keeps its inks (#3232).
    pub fn highlight(&mut self, appearance: MeshAppearance, highlight: f64) {
        let even = |n: u32| (0..=n).map(|i| f64::from(i) / f64::from(n)).collect::<Vec<_>>();
        let (us, vs) = (even(self.cols), even(self.rows));
        self.highlight_at(appearance, highlight, &us, &vs);
    }

    /// As [`Self::highlight`], with the column lines at `us` and the row lines at `vs` (0 to 1
    /// across the patch); a mesh whose lines don't match them is left as it is.
    fn highlight_at(&mut self, appearance: MeshAppearance, highlight: f64, us: &[f64], vs: &[f64]) {
        let h = (highlight / 100.0).clamp(0.0, 1.0) as f32;
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        if !self.is_valid() || us.len() != cols + 1 || vs.len() != rows + 1 {
            return;
        }
        for (r, &v) in vs.iter().enumerate() {
            for (c, &u) in us.iter().enumerate() {
                let d = ((u - 0.5).abs().max((v - 0.5).abs()) * 2.0) as f32; // 0 centre … 1 edge
                let w = match appearance {
                    MeshAppearance::Flat => 0.0,
                    MeshAppearance::ToCenter => (1.0 - d) * h,
                    MeshAppearance::ToEdge => d * h,
                };
                if w > 0.0 {
                    let i = self.idx(r, c);
                    let color = self.points[i].color;
                    self.points[i].color = lerp_color(&color, &Color::WHITE.in_model(color.model()), w);
                }
            }
        }
    }

    /// Horizontal edge from (r, c) to (r, c+1).
    pub fn edge_u(&self, r: usize, c: usize) -> CubicBez {
        let (a, b) = (&self.points[self.idx(r, c)], &self.points[self.idx(r, c + 1)]);
        CubicBez::new(a.p, a.p + a.handles[H_RIGHT], b.p + b.handles[H_LEFT], b.p)
    }
    /// Vertical edge from (r, c) to (r+1, c).
    pub fn edge_v(&self, r: usize, c: usize) -> CubicBez {
        let (a, b) = (&self.points[self.idx(r, c)], &self.points[self.idx(r + 1, c)]);
        CubicBez::new(a.p, a.p + a.handles[H_DOWN], b.p + b.handles[H_UP], b.p)
    }

    /// Coons patch point of patch (r, c) at local (u, v).
    pub fn eval(&self, r: usize, c: usize, u: f64, v: f64) -> Point {
        let (t, b) = (self.edge_u(r, c).eval(u), self.edge_u(r + 1, c).eval(u));
        let (l, rr) = (self.edge_v(r, c).eval(v), self.edge_v(r, c + 1).eval(v));
        let p00 = self.points[self.idx(r, c)].p.to_vec2();
        let p10 = self.points[self.idx(r, c + 1)].p.to_vec2();
        let p01 = self.points[self.idx(r + 1, c)].p.to_vec2();
        let p11 = self.points[self.idx(r + 1, c + 1)].p.to_vec2();
        ((1.0 - v) * t.to_vec2() + v * b.to_vec2() + (1.0 - u) * l.to_vec2() + u * rr.to_vec2()
            - ((1.0 - u) * (1.0 - v) * p00 + u * (1.0 - v) * p10 + (1.0 - u) * v * p01 + u * v * p11))
            .to_point()
    }

    /// Bilinear colour and opacity of patch (r, c) at local (u, v). Corners all in CMYK (or all
    /// grey) mix in that space, as a PDF shading mixes in its own colour space: the inks come out
    /// exact, and a CMYK document separates no colour to draw them. Other corners mix in RGB.
    pub fn color_at(&self, r: usize, c: usize, u: f64, v: f64) -> (Color, f32) {
        let q =
            [&self.points[self.idx(r, c)], &self.points[self.idx(r, c + 1)], &self.points[self.idx(r + 1, c)], &self.points[self.idx(r + 1, c + 1)]];
        let w = [(1.0 - u) * (1.0 - v), u * (1.0 - v), (1.0 - u) * v, u * v];
        let op = q.iter().zip(w).map(|(p, w)| p.opacity * w as f32).sum();
        let cmyk: Option<Vec<[f32; 4]>> =
            q.iter().map(|p| if let Color::Cmyk { c, m, y, k } = p.color { Some([c, m, y, k]) } else { None }).collect();
        if let Some(inks) = cmyk {
            let mut mix = [0.0f32; 4];
            for (ink, w) in inks.iter().zip(w) {
                for (m, x) in mix.iter_mut().zip(ink) {
                    *m += x * w as f32;
                }
            }
            let [c, m, y, k] = mix;
            return (Color::Cmyk { c, m, y, k }, op);
        }
        let grey: Option<Vec<f32>> = q.iter().map(|p| if let Color::Gray { k } = p.color { Some(k) } else { None }).collect();
        if let Some(ks) = grey {
            return (Color::Gray { k: ks.iter().zip(w).map(|(k, w)| k * w as f32).sum() }, op);
        }
        let mut rgb = [0.0f32; 3];
        for (p, w) in q.iter().zip(w) {
            for (ch, x) in rgb.iter_mut().zip(p.color.to_rgb()) {
                *ch += x * w as f32;
            }
        }
        (Color::rgb(rgb[0], rgb[1], rgb[2]), op)
    }

    /// The quads per patch side [`Self::quads`] cuts at when `n` is asked: 1 to 64, and coarser
    /// where the patches at `n` would pass [`Self::MAX_QUADS`] (a quad per patch is the least).
    fn subdivision(&self, n: usize) -> usize {
        n.clamp(1, 64).min(((Self::MAX_QUADS / self.patches().max(1)) as f64).sqrt() as usize).max(1)
    }

    /// Tessellate into `n`×`n` quads per patch ([`Self::subdivision`]), each coloured at its centre
    /// and its corners.
    pub fn quads(&self, n: usize) -> Vec<MeshQuad> {
        if !self.is_valid() {
            return vec![];
        }
        let n = self.subdivision(n);
        let mut out = Vec::with_capacity(self.patches() * n * n);
        let mut grid = vec![Point::ZERO; (n + 1) * (n + 1)];
        let mut colors = vec![(Color::default(), 0.0f32); (n + 1) * (n + 1)];
        for r in 0..self.rows as usize {
            for c in 0..self.cols as usize {
                for j in 0..=n {
                    for i in 0..=n {
                        let (u, v) = (i as f64 / n as f64, j as f64 / n as f64);
                        grid[j * (n + 1) + i] = self.eval(r, c, u, v);
                        colors[j * (n + 1) + i] = self.color_at(r, c, u, v);
                    }
                }
                for j in 0..n {
                    for i in 0..n {
                        let (color, opacity) = self.color_at(r, c, (i as f64 + 0.5) / n as f64, (j as f64 + 0.5) / n as f64);
                        let g = |a: usize, b: usize| grid[b * (n + 1) + a];
                        let k = |a: usize, b: usize| colors[b * (n + 1) + a];
                        out.push(MeshQuad {
                            pts: [g(i, j), g(i + 1, j), g(i + 1, j + 1), g(i, j + 1)],
                            color,
                            opacity,
                            corners: [k(i, j), k(i + 1, j), k(i + 1, j + 1), k(i, j + 1)],
                        });
                    }
                }
            }
        }
        out
    }

    /// Flat-coloured pieces covering the mesh, for outputs that can't shade a mesh smoothly (SVG,
    /// Expand): each patch cut into `n`×`n` quads as by [`Self::quads`], then neighbouring quads
    /// joined into square blocks as large as keeps the colour across each block within about one
    /// 8-bit step. A patch of one colour is one piece; a patch whose colour changes a lot keeps
    /// all its quads. A piece's outline runs through its quads' outer corners, so it covers what
    /// they covered, and it takes the colour at its centre.
    pub fn pieces(&self, n: usize) -> Vec<MeshPiece> {
        let n = self.subdivision(n);
        let quads = self.quads(n);
        let cols = (self.cols as usize).max(1);
        let mut out = Vec::with_capacity(quads.len());
        for (k, patch) in quads.chunks_exact(n * n).enumerate() {
            let (r, c) = (k / cols, k % cols);
            let side = self.block_side(r, c, n, patch);
            if side == 1 {
                out.extend(patch.iter().map(|q| MeshPiece { pts: q.pts.to_vec(), color: q.color, opacity: q.opacity }));
                continue;
            }
            // Tessellation point (i, j) of the patch (0..=n each), from a quad it is a corner of.
            let at = |i: usize, j: usize| -> Option<Point> {
                let (qi, qj) = (i.min(n - 1), j.min(n - 1));
                let q = patch.get(qj * n + qi)?;
                q.pts
                    .get(match (i > qi, j > qj) {
                        (false, false) => 0,
                        (true, false) => 1,
                        (true, true) => 2,
                        (false, true) => 3,
                    })
                    .copied()
            };
            for j0 in (0..n).step_by(side) {
                for i0 in (0..n).step_by(side) {
                    let (i1, j1) = (i0 + side, j0 + side);
                    // Around the block, in the quads' order: along its top, down its right side,
                    // back along its bottom and up its left side.
                    let ring = (i0..i1)
                        .map(|i| (i, j0))
                        .chain((j0..j1).map(|j| (i1, j)))
                        .chain((i0 + 1..=i1).rev().map(|i| (i, j1)))
                        .chain((j0 + 1..=j1).rev().map(|j| (i0, j)));
                    let Some(pts) = ring.map(|(i, j)| at(i, j)).collect::<Option<Vec<Point>>>() else { continue };
                    let half = side as f64 / 2.0;
                    let (color, opacity) = self.color_at(r, c, (i0 as f64 + half) / n as f64, (j0 as f64 + half) / n as f64);
                    out.push(MeshPiece { pts, color, opacity });
                }
            }
        }
        out
    }

    /// The side, in quads, of the blocks [`Self::pieces`] joins patch (r, c) into, cut into
    /// `n`×`n` `quads`: the largest divisor of `n` that keeps the colour changing by at most one
    /// 8-bit step across a block. 1 (no joining) where the patch folds over itself, as the outline
    /// of a block across the fold wouldn't fill it as its quads do.
    fn block_side(&self, r: usize, c: usize, n: usize, quads: &[MeshQuad]) -> usize {
        let corner = |dr: usize, dc: usize| self.points.get(self.idx(r + dr, c + dc));
        let (Some(p00), Some(p01), Some(p10), Some(p11)) = (corner(0, 0), corner(0, 1), corner(1, 0), corner(1, 1)) else {
            return 1;
        };
        // Corners all in CMYK mix in their inks ([`Self::color_at`]); the RGB they show is checked too.
        let inks = [p00, p01, p10, p11].iter().all(|p| matches!(p.color, Color::Cmyk { .. }));
        let channels = |p: &MeshPoint| -> Vec<f32> {
            let mut v = p.color.to_rgb().to_vec();
            if inks && let Color::Cmyk { c, m, y, k } = p.color {
                v.extend([c, m, y, k]);
            }
            v.push(p.opacity);
            v
        };
        let [a, b, cc, d] = [p00, p01, p10, p11].map(channels);
        // A bilinear patch changes across a block of side h (0..1 of the patch) by at most
        // h × (du + dv) in each channel, du and dv its largest change along each parameter.
        let mut steps = 0.0f32;
        for k in 0..a.len() {
            let ch = |v: &[f32]| v.get(k).copied().unwrap_or_default();
            let du = (ch(&a) - ch(&b)).abs().max((ch(&cc) - ch(&d)).abs());
            let dv = (ch(&a) - ch(&cc)).abs().max((ch(&b) - ch(&d)).abs());
            steps = steps.max((du + dv) * 255.0);
        }
        if !steps.is_finite() {
            return 1;
        }
        let side = (1..=n).rev().find(|&s| n.is_multiple_of(s) && (n / s) as f32 >= steps).unwrap_or(1);
        if side > 1 {
            // Every quad turning the same way: the patch doesn't fold, so a block's outline
            // encloses just what its quads cover.
            let turn = |q: &MeshQuad| {
                let [a, b, c, d] = q.pts;
                (b - a).cross(c - a) + (c - a).cross(d - a)
            };
            let first = quads.first().map(turn).unwrap_or_default();
            if !quads.iter().all(|q| turn(q) * first > 0.0) {
                return 1;
            }
        }
        side
    }

    /// The mesh's outer boundary as a closed path.
    pub fn outline(&self) -> PathData {
        if !self.is_valid() {
            return PathData::default();
        }
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let mut segs: Vec<CubicBez> = vec![];
        for c in 0..cols {
            segs.push(self.edge_u(0, c));
        }
        for r in 0..rows {
            segs.push(self.edge_v(r, cols));
        }
        for c in (0..cols).rev() {
            let e = self.edge_u(rows, c);
            segs.push(CubicBez::new(e.p3, e.p2, e.p1, e.p0));
        }
        for r in (0..rows).rev() {
            let e = self.edge_v(r, 0);
            segs.push(CubicBez::new(e.p3, e.p2, e.p1, e.p0));
        }
        let mut anchors: Vec<Anchor> = Vec::with_capacity(segs.len());
        for (i, s) in segs.iter().enumerate() {
            let prev = &segs[(i + segs.len() - 1) % segs.len()];
            anchors.push(Anchor::with_handles(s.p0, prev.p2, s.p1));
        }
        PathData::single(SubPath::new(anchors, true))
    }

    /// Mesh lines (all row and column edges) as open paths, for outline view and editing overlays.
    pub fn lines(&self) -> PathData {
        if !self.is_valid() {
            return PathData::default();
        }
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let mut subs = vec![];
        let cub = |e: CubicBez| SubPath::new(vec![Anchor::with_handles(e.p0, e.p0, e.p1), Anchor::with_handles(e.p3, e.p2, e.p3)], false);
        for r in 0..=rows {
            for c in 0..cols {
                subs.push(cub(self.edge_u(r, c)));
            }
        }
        for c in 0..=cols {
            for r in 0..rows {
                subs.push(cub(self.edge_v(r, c)));
            }
        }
        PathData::new(subs)
    }

    /// Control bounds (points and handles).
    pub fn bounds(&self) -> Option<Rect> {
        let mut it = self.points.iter().flat_map(|p| std::iter::once(p.p).chain(p.handles.iter().map(move |h| p.p + *h)));
        let first = it.next()?;
        Some(it.fold(Rect::from_points(first, first), |r, q| r.union_pt(q)))
    }

    pub fn transform(&mut self, a: Affine) {
        let lin = Affine::new({
            let c = a.as_coeffs();
            [c[0], c[1], c[2], c[3], 0.0, 0.0]
        });
        for p in &mut self.points {
            p.p = a * p.p;
            for h in &mut p.handles {
                *h = (lin * h.to_point()).to_vec2();
            }
        }
    }

    /// Map points (and handle ends) through a non-linear function.
    pub fn map(&mut self, f: &dyn Fn(Point) -> Point) {
        for p in &mut self.points {
            let q = f(p.p);
            for h in &mut p.handles {
                *h = f(p.p + *h) - q;
            }
            p.p = q;
        }
    }

    /// Which patch and local (u, v) contains `p` (sampled search; `None` outside the mesh).
    pub fn locate(&self, p: Point) -> Option<(usize, usize, f64, f64)> {
        self.locate_within(p, 0.0)
    }

    /// As [`Self::locate`], and for a point outside the mesh within `tol` of its outline, the
    /// patch and local (u, v) of the nearest point of the mesh's edge. A point on the outline
    /// (within [`ON_OUTLINE`]) locates on its edge on every side, with any `tol`.
    pub fn locate_within(&self, p: Point, tol: f64) -> Option<(usize, usize, f64, f64)> {
        if !self.is_valid() || !p.is_finite() {
            return None;
        }
        // A point on the outline takes the nearest point of the edge on every side: the nonzero
        // winding number counts a point on some edges as outside (a rectangle's bottom and right
        // edges) and on the others as inside.
        if let Some((_, r, c, u, v)) = self.nearest_edge(p, ON_OUTLINE).filter(|e| e.0 <= ON_OUTLINE * ON_OUTLINE) {
            return Some((r, c, u, v));
        }
        if self.outline().to_bezpath().winding(p) == 0 {
            if tol.is_nan() || tol <= 0.0 {
                return None;
            }
            return self.nearest_edge(p, tol).filter(|e| e.0 <= tol * tol).map(|(_, r, c, u, v)| (r, c, u, v));
        }
        const N: usize = 16;
        let mut best = (f64::INFINITY, 0, 0, 0.0, 0.0);
        for r in 0..self.rows as usize {
            for c in 0..self.cols as usize {
                for j in 0..=N {
                    for i in 0..=N {
                        let (u, v) = (i as f64 / N as f64, j as f64 / N as f64);
                        let d = self.eval(r, c, u, v).distance_squared(p);
                        if d < best.0 {
                            best = (d, r, c, u, v);
                        }
                    }
                }
            }
        }
        // Refine with a few Newton-free local searches.
        let (_, r, c, mut u, mut v) = best;
        let mut step = 0.5 / N as f64;
        for _ in 0..12 {
            let mut improved = false;
            for (du, dv) in [(step, 0.0), (-step, 0.0), (0.0, step), (0.0, -step)] {
                let (nu, nv) = ((u + du).clamp(0.0, 1.0), (v + dv).clamp(0.0, 1.0));
                if self.eval(r, c, nu, nv).distance_squared(p) < self.eval(r, c, u, v).distance_squared(p) {
                    u = nu;
                    v = nv;
                    improved = true;
                }
            }
            if !improved {
                step /= 2.0;
            }
        }
        Some((r, c, u, v))
    }

    /// The point of the mesh's outer edge nearest `p`: its squared distance from `p`, and its
    /// boundary patch and local (u, v). Edges whose control points all lie farther than `within`
    /// from `p` are skipped, so `None` means no edge is within `within`.
    fn nearest_edge(&self, p: Point, within: f64) -> Option<(f64, usize, usize, f64, f64)> {
        use vectorcraft_geom::kurbo::ParamCurveNearest;
        let (rows, cols) = (self.rows as usize, self.cols as usize);
        let mut best: Option<(f64, usize, usize, f64, f64)> = None;
        let mut take = |e: CubicBez, at: (usize, usize, Option<f64>, Option<f64>)| {
            // The edge lies inside its control points' box, so a box farther than `within` holds
            // no point of the edge within it.
            let b = Rect::from_points(e.p0, e.p1).union_pt(e.p2).union_pt(e.p3);
            let (dx, dy) = ((b.x0 - p.x).max(p.x - b.x1).max(0.0), (b.y0 - p.y).max(p.y - b.y1).max(0.0));
            if dx * dx + dy * dy > within * within {
                return;
            }
            let n = e.nearest(p, 1e-9);
            if best.is_none_or(|b| n.distance_sq < b.0) {
                let (r, c, u, v) = at;
                best = Some((n.distance_sq, r, c, u.unwrap_or(n.t), v.unwrap_or(n.t)));
            }
        };
        for c in 0..cols {
            take(self.edge_u(0, c), (0, c, None, Some(0.0)));
            take(self.edge_u(rows, c), (rows - 1, c, None, Some(1.0)));
        }
        for r in 0..rows {
            take(self.edge_v(r, 0), (r, 0, Some(0.0), None));
            take(self.edge_v(r, cols), (r, cols - 1, Some(1.0), None));
        }
        best
    }

    /// Index of the mesh point within `tol` of `p`.
    pub fn point_near(&self, p: Point, tol: f64) -> Option<usize> {
        self.points
            .iter()
            .enumerate()
            .filter(|(_, q)| q.p.distance(p) <= tol)
            .min_by(|a, b| a.1.p.distance(p).total_cmp(&b.1.p.distance(p)))
            .map(|(i, _)| i)
    }

    fn surface_du(&self, r: usize, c: usize, u: f64, v: f64) -> Vec2 {
        let e = 1e-3;
        let (a, b) = ((u - e).max(0.0), (u + e).min(1.0));
        (self.eval(r, c, b, v) - self.eval(r, c, a, v)) / (b - a)
    }
    fn surface_dv(&self, r: usize, c: usize, u: f64, v: f64) -> Vec2 {
        let e = 1e-3;
        let (a, b) = ((v - e).max(0.0), (v + e).min(1.0));
        (self.eval(r, c, u, b) - self.eval(r, c, u, a)) / (b - a)
    }

    /// Insert a mesh row through patch row `r` at local `v`.
    pub fn insert_row(&mut self, r: usize, v: f64) {
        let cols = self.cols as usize;
        let v = v.clamp(0.01, 0.99);
        let mut row = Vec::with_capacity(cols + 1);
        // Split the vertical edges.
        let mut ups = vec![];
        for c in 0..=cols {
            let e = self.edge_v(r, c);
            let (l, rr) = (e.subsegment(0.0..v), e.subsegment(v..1.0));
            ups.push((l.p1 - l.p0, rr.p2 - rr.p3));
            let pc = c.min(cols - 1);
            let u = if c == cols { 1.0 } else { 0.0 };
            let (color, opacity) = self.color_at(r, pc, u, v);
            // Horizontal handles: isocurve derivatives on either side.
            let right = if c < cols { self.surface_du(r, c, 0.0, v) / (3.0) } else { Vec2::ZERO };
            let left = if c > 0 { -self.surface_du(r, c - 1, 1.0, v) / 3.0 } else { Vec2::ZERO };
            row.push(MeshPoint { p: l.p3, color, opacity, handles: [right, left, rr.p1 - rr.p0, l.p2 - l.p3] });
        }
        for (c, (down, up)) in ups.into_iter().enumerate() {
            let i0 = self.idx(r, c);
            let i1 = self.idx(r + 1, c);
            self.points[i0].handles[H_DOWN] = down;
            self.points[i1].handles[H_UP] = up;
        }
        let at = self.idx(r + 1, 0);
        self.points.splice(at..at, row);
        self.rows += 1;
    }

    /// Insert a mesh column through patch column `c` at local `u`.
    pub fn insert_col(&mut self, c: usize, u: f64) {
        let rows = self.rows as usize;
        let u = u.clamp(0.01, 0.99);
        let mut col = Vec::with_capacity(rows + 1);
        for r in 0..=rows {
            let e = self.edge_u(r, c);
            let (l, rr) = (e.subsegment(0.0..u), e.subsegment(u..1.0));
            let pr = r.min(rows - 1);
            let v = if r == rows { 1.0 } else { 0.0 };
            let (color, opacity) = self.color_at(pr, c, u, v);
            let down = if r < rows { self.surface_dv(r, c, u, 0.0) / 3.0 } else { Vec2::ZERO };
            let up = if r > 0 { -self.surface_dv(r - 1, c, u, 1.0) / 3.0 } else { Vec2::ZERO };
            col.push((MeshPoint { p: l.p3, color, opacity, handles: [rr.p1 - rr.p0, l.p2 - l.p3, down, up] }, l.p1 - l.p0, rr.p2 - rr.p3));
        }
        let cols = self.cols as usize;
        let mut pts = Vec::with_capacity((rows + 1) * (cols + 2));
        for (r, (mp, right, left)) in col.into_iter().enumerate() {
            for cc in 0..=cols {
                let mut q = self.points[r * (cols + 1) + cc].clone();
                if cc == c {
                    q.handles[H_RIGHT] = right;
                }
                if cc == c + 1 {
                    q.handles[H_LEFT] = left;
                }
                pts.push(q);
                if cc == c {
                    pts.push(mp.clone());
                }
            }
        }
        self.points = pts;
        self.cols += 1;
    }

    /// Mesh tool click: add a row and a column through `p` (inside the mesh, or outside within
    /// `tol` of its edge, through the nearest point of the edge). Returns the new point index.
    pub fn add_lines_at(&mut self, p: Point, tol: f64) -> Option<usize> {
        let (r, c, u, v) = self.locate_within(p, tol)?;
        let mut rr = r;
        let mut cc = c;
        let row_new = v > 0.02 && v < 0.98;
        let col_new = u > 0.02 && u < 0.98;
        if row_new {
            self.insert_row(r, v);
            rr = r + 1;
        } else if v >= 0.98 {
            rr = r + 1;
        }
        if col_new {
            self.insert_col(c, u);
            cc = c + 1;
        } else if u >= 0.98 {
            cc = c + 1;
        }
        Some(self.idx(rr, cc))
    }

    /// Mesh tool Alt-click: delete the mesh lines through point `index` (interior lines only).
    pub fn remove_point_lines(&mut self, index: usize) -> bool {
        // A mesh from a file may not have the points its rows and columns need: it is indexed below.
        if !self.is_valid() || index >= self.points.len() {
            return false;
        }
        let w = self.cols as usize + 1;
        let (r, c) = (index / w, index % w);
        let mut changed = false;
        if r > 0 && r < self.rows as usize {
            // Merge the edges above and below: keep the outer handles.
            for cc in 0..w {
                let above = self.idx(r - 1, cc);
                let below = self.idx(r + 1, cc);
                let mid = self.idx(r, cc);
                let (pa, pb, m) = (self.points[above].p, self.points[below].p, &self.points[mid]);
                let hm = (m.p, m.handles[H_UP], m.handles[H_DOWN]);
                let (down, up) = merged_handles((pa, self.points[above].handles[H_DOWN]), hm, (pb, self.points[below].handles[H_UP]));
                self.points[above].handles[H_DOWN] = down;
                self.points[below].handles[H_UP] = up;
            }
            self.points.drain(r * w..(r + 1) * w);
            self.rows -= 1;
            changed = true;
        }
        let w = self.cols as usize + 1;
        if c > 0 && c < self.cols as usize {
            let rows = self.rows as usize;
            for rr in 0..=rows {
                let (l, rt) = (rr * w + c - 1, rr * w + c + 1);
                let (pl, pr, m) = (self.points[l].p, self.points[rt].p, &self.points[rr * w + c]);
                let hm = (m.p, m.handles[H_LEFT], m.handles[H_RIGHT]);
                let (right, left) = merged_handles((pl, self.points[l].handles[H_RIGHT]), hm, (pr, self.points[rt].handles[H_LEFT]));
                self.points[l].handles[H_RIGHT] = right;
                self.points[rt].handles[H_LEFT] = left;
            }
            let mut k = 0;
            self.points.retain(|_| {
                let keep = k % w != c;
                k += 1;
                keep
            });
            self.cols -= 1;
            changed = true;
        }
        changed
    }
}

// =====================================================================================
// Envelopes
// =====================================================================================

/// Union of the geometric bounds of `nodes`.
pub fn nodes_bounds(nodes: &[Arc<Node>]) -> Option<Rect> {
    nodes.iter().fold(None, |acc, c| vectorcraft_geom::union_opt(acc, c.geometric_bounds()))
}

/// A `(rows+1)×(cols+1)` grid over `r` (row-major).
pub fn grid_points(r: Rect, rows: u32, cols: u32) -> Vec<Point> {
    let mut v = Vec::with_capacity(((rows + 1) * (cols + 1)) as usize);
    for i in 0..=rows {
        for j in 0..=cols {
            v.push(Point::new(r.x0 + r.width() * j as f64 / cols.max(1) as f64, r.y0 + r.height() * i as f64 / rows.max(1) as f64));
        }
    }
    v
}

fn catmull(p0: Vec2, p1: Vec2, p2: Vec2, p3: Vec2, t: f64) -> Vec2 {
    let t2 = t * t;
    let t3 = t2 * t;
    0.5 * (2.0 * p1 + (p2 - p0) * t + (2.0 * p0 - 5.0 * p1 + 4.0 * p2 - p3) * t2 + (3.0 * p1 - p0 - 3.0 * p2 + p3) * t3)
}

/// Point map from the unit square onto a point grid (tensor-product Catmull-Rom; linear
/// extrapolation at the borders so an undistorted grid is the identity, also past the square).
pub(super) fn grid_eval(rows: usize, cols: usize, pts: &[Point], u: f64, v: f64) -> Point {
    let at = |r: i64, c: i64| -> Vec2 {
        let rr = r.clamp(0, rows as i64);
        let cc = c.clamp(0, cols as i64);
        let base = pts[rr as usize * (cols + 1) + cc as usize].to_vec2();
        // Linear extrapolation outside the grid.
        let er = r - rr;
        let ec = c - cc;
        let mut p = base;
        if er != 0 && rows >= 1 {
            let inner = (rr - er.signum()).clamp(0, rows as i64);
            p += (base - pts[inner as usize * (cols + 1) + cc as usize].to_vec2()) * er.abs() as f64;
        }
        if ec != 0 && cols >= 1 {
            let inner = (cc - ec.signum()).clamp(0, cols as i64);
            p += (base - pts[rr as usize * (cols + 1) + inner as usize].to_vec2()) * ec.abs() as f64;
        }
        p
    };
    // Past the grid's edges the outer patches go on (art can reach past the content's box).
    let (su, sv) = (u * cols as f64, v * rows as f64);
    let (ci, ri) = ((su.floor() as i64).clamp(0, cols as i64 - 1), (sv.floor() as i64).clamp(0, rows as i64 - 1));
    let (fu, fv) = (su - ci as f64, sv - ri as f64);
    let mut rowsv = [Vec2::ZERO; 4];
    for (k, rv) in rowsv.iter_mut().enumerate() {
        let r = ri - 1 + k as i64;
        *rv = catmull(at(r, ci - 1), at(r, ci), at(r, ci + 1), at(r, ci + 2), fu);
    }
    catmull(rowsv[0], rowsv[1], rowsv[2], rowsv[3], fv).to_point()
}

/// The surface (u, v ∈ [0,1]) an envelope of `kind` maps content onto, for content whose bounds in
/// the envelope's own `frame` (its axes → the document) are `src`: a warp bends along the frame's
/// axes; a mesh's points and a top object's path are in the document already.
pub(super) fn surface_of<'a>(kind: &'a EnvelopeKind, src: Rect, frame: Affine) -> Box<dyn Fn(f64, f64) -> Point + 'a> {
    let flat = move |u: f64, v: f64| frame * Point::new(src.x0 + u * src.width(), src.y0 + v * src.height());
    match kind {
        EnvelopeKind::Warp { style, bend, h: dh, v: dv, horizontal, version } => {
            let st = WarpStyle::from_id(style).unwrap_or(WarpStyle::Arc);
            let b = bend.clamp(-100.0, 100.0) / 100.0;
            let (dh, dv) = (dh.clamp(-100.0, 100.0) / 100.0, dv.clamp(-100.0, 100.0) / 100.0);
            if uses_net(*version) {
                let net = WarpNet::new(st, b, dh, dv, src.width(), src.height(), !*horizontal);
                // Content with no width or no height takes the middle of the patch along that side.
                let (flat_u, flat_v) = (src.width() <= 0.0, src.height() <= 0.0);
                return Box::new(move |u: f64, v: f64| {
                    let (u, v) = (if flat_u { 0.5 } else { u }, if flat_v { 0.5 } else { v });
                    frame * (src.origin() + net.at(u, v).to_vec2())
                });
            }
            let c = src.center();
            let (hw, hh) = (src.width() / 2.0, src.height() / 2.0);
            let horizontal = *horizontal;
            Box::new(move |u: f64, v: f64| {
                let (x, y) = (2.0 * u - 1.0, 2.0 * v - 1.0);
                let (x2, y2) = if horizontal {
                    warp_point(st, b, dh, dv, hh / hw.max(1e-9), x, y)
                } else {
                    let (a, b2) = warp_point(st, b, dh, dv, hw / hh.max(1e-9), y, x);
                    (b2, a)
                };
                frame * Point::new(c.x + x2 * hw, c.y + y2 * hh)
            })
        }
        EnvelopeKind::Mesh { rows, cols, points, handles } => {
            if !handles.is_empty()
                && let Some(m) = envelope_grid(*rows, *cols, points, handles)
            {
                return Box::new(move |u: f64, v: f64| mesh_at(&m, u, v));
            }
            if *rows == 0 || *cols == 0 || grid_len(*rows, *cols) != Some(points.len()) {
                return Box::new(flat);
            }
            let (rows, cols) = (*rows as usize, *cols as usize);
            Box::new(move |u: f64, v: f64| grid_eval(rows, cols, points, u, v))
        }
        EnvelopeKind::TopObject { path, corners } => match Coons::from_top_object(path, *corners) {
            Some(co) => Box::new(move |u: f64, v: f64| co.eval(u, v)),
            None => Box::new(flat),
        },
    }
}
