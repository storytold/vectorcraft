//! Where a path's strokes paint, from the path's geometry without building their outlines: the
//! visual bounds and the clicks of a stroke take in its alignment, its width profile, miter spikes
//! up to the miter limit, projecting caps and arrowheads. Butt caps end at the path's end points.

use vectorcraft_geom::bez::{CORNER_COS, has_length, kink, segments, subpaths, tangent, unit};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::kurbo::{ParamCurveArclen, ParamCurveExtrema, ParamCurveNearest};
use vectorcraft_geom::{BezPath, FillRule, ParamCurve, PathSeg, Point, Rect, Shape, Vec2};

use crate::appearance::HEAD_REACH;
use crate::{Appearance, ArrowAlign, Dash, LineCap, LineJoin, StrokeAlign, StrokeLayer, WidthProfile};

/// Arc-length accuracy for placing a point along a profile (document points).
const ARCLEN_ACCURACY: f64 = 1e-3;

/// The segments of a subpath that have length (zero-length ones, all of whose points coincide,
/// have no direction).
fn drawn(els: &[vectorcraft_geom::PathEl]) -> Vec<PathSeg> {
    segments(els).into_iter().filter(has_length).collect()
}

/// The open subpath ends of `segs`: (point, unit direction out of the path) at its start and end.
fn ends(segs: &[PathSeg]) -> Option<[(Point, Vec2); 2]> {
    let (first, last) = (segs.first()?, segs.last()?);
    Some([(first.start(), -tangent(first, 0.0)), (last.end(), tangent(last, 1.0))])
}

/// The joints between consecutive segments (and, `closed`, where the subpath closes): the point
/// and the unit tangents in and out.
fn joints(segs: &[PathSeg], closed: bool) -> impl Iterator<Item = (Point, Vec2, Vec2)> + '_ {
    let n = segs.len();
    let pairs = if closed && n > 1 { n } else { n.saturating_sub(1) };
    (0..pairs).map(move |i| {
        let (a, b) = (&segs[i], &segs[(i + 1) % n]);
        (a.end(), tangent(a, 1.0), tangent(b, 0.0))
    })
}

/// The miter spike at a corner with tangents `t_in`, `t_out` of a stroke reaching `side` from its
/// path: its tip and the corner's outer offset points, or `None` when the path doesn't turn there
/// or the miter limit bevels it.
fn miter(at: Point, t_in: Vec2, t_out: Vec2, side: f64, limit: f64) -> Option<[Point; 3]> {
    let cos = t_in.dot(t_out);
    // cos(half the turn) is sin(half the angle between the segments): the miter is side / it.
    let half = ((1.0 + cos) / 2.0).max(0.0).sqrt();
    if cos >= CORNER_COS || half * limit < 1.0 {
        return None;
    }
    let out = unit(t_in - t_out);
    // The offset of each segment on the outer side of the turn.
    let outer = |t: Vec2| {
        let n = Vec2::new(-t.y, t.x);
        if n.dot(out) >= 0.0 { n } else { -n }
    };
    Some([at + outer(t_in) * side, at + out * (side / half), at + outer(t_out) * side])
}

/// The heads' ends of `bp`: the start of its first subpath and the end of its last, when open (as
/// the stroke geometry places them), each with `true` for the end head.
fn head_ends(st: &StrokeLayer, bp: &BezPath) -> Vec<(Point, Vec2, bool)> {
    if st.start_arrow.is_none() && st.end_arrow.is_none() {
        return vec![];
    }
    let subs = subpaths(bp);
    let open_ends = |sub: Option<&(std::ops::Range<usize>, bool)>| sub.filter(|s| !s.1).and_then(|s| ends(&drawn(&bp.elements()[s.0.clone()])));
    let mut out = vec![];
    if st.start_arrow.is_some()
        && let Some([(at, dir), _]) = open_ends(subs.first())
    {
        out.push((at, dir, false));
    }
    if st.end_arrow.is_some()
        && let Some([_, (at, dir)]) = open_ends(subs.last())
    {
        out.push((at, dir, true));
    }
    out
}

/// (left, right) width factors where `p`'s nearest point on the subpath `segs` sits (segment `i`
/// at `t`): the profile at that fraction of the subpath's length, or wider where an end of the
/// segment is a corner. The stroke draws the sides there straight to the corner's join and inner
/// meeting point, which can lie outside the exact width all along the segment.
fn profile_at(profile: &WidthProfile, segs: &[PathSeg], closed: bool, i: usize, t: f64) -> (f64, f64) {
    let n = segs.len();
    let lens: Vec<f64> = segs.iter().map(|s| s.arclen(ARCLEN_ACCURACY)).collect();
    let total: f64 = lens.iter().sum::<f64>().max(1e-12);
    let before = lens[..i].iter().sum::<f64>();
    let mut w = profile.at((before + segs[i].subsegment(0.0..t).arclen(ARCLEN_ACCURACY)) / total);
    let corner = |a: usize, b: usize| (closed || a < b) && kink(&segs[a], &segs[b]);
    let starts = n > 1 && corner((i + n - 1) % n, i);
    let ends = n > 1 && corner(i, (i + 1) % n);
    // A closed subpath's last segment ends where it starts.
    let end = if i + 1 == n { 0.0 } else { before + lens[i] };
    for (is, at) in [(starts, before), (ends, end)] {
        if is {
            let (l, r) = profile.at(at / total);
            w = (w.0.max(l), w.1.max(r));
        }
    }
    w
}

/// Does the curvature of `seg` stay below one over `side` all along it? A curve's first derivative
/// is a mix of its three control differences, and its second a mix of their two differences, which
/// bound the curvature from above. Then the edges of a stroke reaching `side` from it do not fold
/// back, and those edges reach farthest at the ends of `seg` or where its tangent is square to an
/// axis.
fn gentle(seg: &PathSeg, side: f64) -> bool {
    let c = seg.to_cubic();
    let d = [c.p1 - c.p0, c.p2 - c.p1, c.p3 - c.p2].map(|v| v * 3.0);
    let along = unit(d[0] + d[1] + d[2]);
    let slowest = d.iter().map(|v| v.dot(along)).fold(f64::INFINITY, f64::min);
    let bend = 2.0 * (d[1] - d[0]).hypot().max((d[2] - d[1]).hypot());
    slowest > 0.0 && side * bend < slowest * slowest
}

/// The box a stroke reaching `side` from `seg` paints across it, from the directions its tangent
/// can take: the tangent is a mix of the nonzero control differences, so when their directions all
/// lie within a quarter turn of the directions' sum, every point across the stroke is in `seg`'s
/// box moved `side` along a normal of one of those directions. `None` when they do not. A handle on
/// its end point adds no direction, so a straight curve with one measures as a line.
fn turning_box(seg: &PathSeg, side: f64) -> Option<Rect> {
    let c = seg.to_cubic();
    let d = [c.p1 - c.p0, c.p2 - c.p1, c.p3 - c.p2];
    let moving = || d.iter().map(|v| (*v, v.hypot())).filter(|(_, h)| *h > 0.0).map(|(v, h)| v / h);
    let along = unit(moving().fold(Vec2::ZERO, |s, v| s + v));
    if moving().any(|v| v.dot(along) <= 0.0) {
        return None;
    }
    let angle = |v: &Vec2| along.cross(*v).atan2(along.dot(*v));
    let lo = moving().min_by(|a, b| angle(a).total_cmp(&angle(b)))?;
    let hi = moving().max_by(|a, b| angle(a).total_cmp(&angle(b)))?;
    let n = |t: Vec2| (Vec2::new(t.y, -t.x) * side).to_point();
    let one_side = round_join_box(Point::ZERO, lo, hi, side).unwrap_or_else(|| Rect::from_points(n(lo), n(hi)));
    let reach = one_side.union(Rect::new(-one_side.x1, -one_side.y1, -one_side.x0, -one_side.y0));
    let b = ParamCurveExtrema::bounding_box(seg);
    Some(Rect::new(b.x0 + reach.x0, b.y0 + reach.y0, b.x1 + reach.x1, b.y1 + reach.y1))
}

/// The box a stroke reaching `side` from `seg` paints when it stops square at both ends of `seg`:
/// the corners at the ends and the points across the stroke where the tangent is square to an axis.
/// A curve that bends tighter than `side` is halved up to `depth` times, and a piece still bending
/// that much is measured by [`turning_box`], or grown by `side` on every side when its tangent
/// turns too far.
fn swept_box(seg: &PathSeg, side: f64, depth: u32) -> Rect {
    if !matches!(seg, PathSeg::Line(_)) && !gentle(seg, side) {
        if depth == 0 {
            return turning_box(seg, side).unwrap_or_else(|| ParamCurveExtrema::bounding_box(seg).inflate(side, side));
        }
        return swept_box(&seg.subsegment(0.0..0.5), side, depth - 1).union(swept_box(&seg.subsegment(0.5..1.0), side, depth - 1));
    }
    let across = |t: f64| {
        let (p, tn) = (seg.eval(t), tangent(seg, t));
        let n = Vec2::new(-tn.y, tn.x) * side;
        [p + n, p - n]
    };
    let [a, b] = across(0.0);
    seg.extrema().into_iter().chain([1.0]).flat_map(across).fold(Rect::from_points(a, b), |r, p| r.union_pt(p))
}

/// The box of the arc a round join paints `side` from `at`, where the path turns from tangent
/// `t_in` to `t_out`: the arc runs on the outer side of the turn, and its box comes from its ends
/// and the points on it square to an axis. A joint where the path turns back on itself is given
/// the whole disc's box, and one where it does not turn paints no arc (`None`).
fn round_join_box(at: Point, t_in: Vec2, t_out: Vec2, side: f64) -> Option<Rect> {
    let (t_in, t_out) = (unit(t_in), unit(t_out));
    let turn = t_in.cross(t_out);
    if turn.abs() < 1e-12 {
        return (t_in.dot(t_out) < 0.0).then(|| Rect::from_center_size(at, (2.0 * side, 2.0 * side)));
    }
    let s = turn.signum();
    let outer = |t: Vec2| Vec2::new(t.y, -t.x) * s;
    let (a, b) = (outer(t_in), outer(t_out));
    let axes = [Vec2::new(1.0, 0.0), Vec2::new(-1.0, 0.0), Vec2::new(0.0, 1.0), Vec2::new(0.0, -1.0)];
    let arc = Rect::from_points(at + a * side, at + b * side);
    Some(axes.into_iter().filter(|d| a.cross(*d) * s >= 0.0 && d.cross(b) * s >= 0.0).fold(arc, |r, d| r.union_pt(at + d * side)))
}

/// Is `p` inside the convex polygon `pts` (either winding), or within `tol` of it?
fn in_convex(pts: &[Point], p: Point, tol: f64) -> bool {
    let n = pts.len();
    // Signed distances from each edge's line.
    let (lo, hi) = (0..n).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), i| {
        let e = pts[(i + 1) % n] - pts[i];
        let d = e.cross(p - pts[i]) / e.hypot().max(1e-12);
        (lo.min(d), hi.max(d))
    });
    lo >= -tol || hi <= tol
}

impl StrokeLayer {
    /// Does bounding this stroke need its path's shape: miter spikes, projecting caps, arrowheads,
    /// an alignment that depends on whether the path is closed, or butt caps, which stop at the
    /// ends of an open path? Else it reaches evenly round it.
    fn needs_shape(&self) -> bool {
        self.join == LineJoin::Miter
            || self.cap != LineCap::Round
            || self.start_arrow.is_some()
            || self.end_arrow.is_some()
            || self.align != StrokeAlign::Center
    }

    /// The box this stroke paints along `segs`, the segments with length of an open subpath, when
    /// its caps are butt caps: each segment's [`swept_box`] and, with round joins, the arc each
    /// joint between segments paints ([`round_join_box`]), a shallow one included.
    /// `None` when `segs` is empty. Butt dashes stop inside the same box. The heads and miter tips
    /// are measured apart. A segment in the middle is measured only when its own box grown by
    /// `side`, which holds everything it paints, is not inside the box so far.
    fn butt_body(&self, segs: &[PathSeg], side: f64) -> Option<Rect> {
        let (first, last) = (segs.first()?, segs.last()?);
        let ends = swept_box(first, side, 3).union(swept_box(last, side, 3));
        let body = segs.iter().skip(1).take(segs.len().saturating_sub(2)).fold(ends, |b, s| {
            if b.contains_rect(ParamCurveExtrema::bounding_box(s).inflate(side, side)) { b } else { b.union(swept_box(s, side, 3)) }
        });
        if self.join != LineJoin::Round {
            return Some(body);
        }
        Some(joints(segs, false).fold(body, |b, (at, t_in, t_out)| round_join_box(at, t_in, t_out, side).map_or(b, |j| b.union(j))))
    }

    /// The box this stroke paints along `bp` (geometric bounds `bounds`): an open subpath with butt
    /// caps by [`Self::butt_body`], every other subpath by the box of its segments grown by its
    /// reach, with `tips` and, when `measure_tips`, the miter spikes the limit keeps, the corners of
    /// projecting caps and the reach of the arrowheads round their end points.
    fn reach_along(&self, bp: &BezPath, bounds: Rect, tips: Option<Rect>, measure_tips: bool) -> Rect {
        let els = bp.elements();
        let subs = subpaths(bp);
        let mut out = tips.map_or(bounds, |t| bounds.union(t));
        let mut side_max: f64 = 0.0;
        // Every subpath reaches evenly unless one is open and butt-capped; only then is each
        // subpath's own box needed.
        let even_only = self.cap != LineCap::Butt || subs.iter().all(|s| s.1);
        let mut even = even_only.then_some(bounds);
        // Dashes put a cap on every dash: projecting ones reach a corner's diagonal anywhere.
        let capped_dashes = self.cap == LineCap::Square && self.dash.as_ref().is_some_and(Dash::is_dashed);
        for (r, closed) in subs {
            let side = self.side_reach(closed);
            let butt = self.cap == LineCap::Butt && !closed;
            if !butt {
                side_max = side_max.max(if capped_dashes { side * std::f64::consts::SQRT_2 } else { side });
            }
            if side <= 0.0 {
                continue;
            }
            let square = self.cap == LineCap::Square && !closed;
            let miters = measure_tips && self.join == LineJoin::Miter;
            if !(butt || square || miters || !even_only) {
                continue;
            }
            let segs = drawn(&els[r]);
            if miters {
                for (at, t_in, t_out) in joints(&segs, closed) {
                    if let Some([_, tip, _]) = miter(at, t_in, t_out, side, self.miter_limit) {
                        out = out.union_pt(tip);
                    }
                }
            }
            if butt {
                if let Some(body) = self.butt_body(&segs, side) {
                    out = out.union(body);
                }
                continue;
            }
            if !even_only {
                even = vectorcraft_geom::union_opt(even, segs.iter().map(ParamCurveExtrema::bounding_box).reduce(|a, b| a.union(b)));
            }
            if square {
                for (at, dir) in ends(&segs).into_iter().flatten() {
                    let n = Vec2::new(-dir.y, dir.x) * side;
                    out = out.union_pt(at + dir * side + n).union_pt(at + dir * side - n);
                }
            }
        }
        for (at, _, end) in head_ends(self, bp) {
            let r = self.head_reach(end);
            out = out.union(Rect::from_center_size(at, (2.0 * r, 2.0 * r)));
        }
        even.map_or(out, |e| out.union(e.inflate(side_max, side_max)))
    }

    /// The box this stroke paints along `bp`, whose geometric bounds are `bounds`: the bounds grown
    /// by the body's reach, with the tips of the miter spikes the limit keeps, the corners of
    /// projecting caps and the reach of the arrowheads round their end points. An open subpath
    /// with butt caps does not reach past its ends ([`Self::butt_body`]).
    pub fn bounds_on(&self, bp: &BezPath, bounds: Rect) -> Rect {
        self.reach_along(bp, bounds, None, true)
    }

    /// The tip of the miter spike this stroke paints where segment `b` follows segment `a` in a
    /// subpath of its path (`closed`: a closed one), both with length ([`has_length`]), or `None`
    /// when the join is not a miter, the path does not turn there or the miter limit bevels it.
    pub fn miter_tip(&self, closed: bool, a: &PathSeg, b: &PathSeg) -> Option<Point> {
        let side = self.side_reach(closed);
        if self.join != LineJoin::Miter || side <= 0.0 {
            return None;
        }
        miter(a.end(), tangent(a, 1.0), tangent(b, 0.0), side, self.miter_limit).map(|[_, tip, _]| tip)
    }

    /// [`Self::bounds_on`] with the miter spikes' tips taken from `tips`, the box of the tips of
    /// every joint of `bp` ([`Self::miter_tip`]) or `None` when there are none, instead of measured
    /// along `bp`. It measures the projecting caps and arrowheads as [`Self::bounds_on`] does.
    pub fn bounds_with_tips(&self, bp: &BezPath, bounds: Rect, tips: Option<Rect>) -> Rect {
        self.reach_along(bp, bounds, tips, false)
    }

    /// Does this stroke along `bp` (filled with `rule`) paint within `tol` of `p`? Inside and
    /// outside strokes of closed paths count on their own side only (and within `tol` of the
    /// path); a width profile counts at its width where `p` is nearest.
    pub fn hits(&self, bp: &BezPath, rule: FillRule, p: Point, tol: f64) -> bool {
        let reach = self.reach() + tol;
        if self.width.is_nan() || self.width <= 0.0 || !bp.bounding_box().inflate(reach, reach).contains(p) {
            return false;
        }
        if head_ends(self, bp).into_iter().any(|(at, out, end)| self.head_contains(at, out, end, p, tol)) {
            return true;
        }
        let els = bp.elements();
        // Aligned strokes paint on one side of the path only.
        let on_side = |region: Option<bool>, d: f64| region.is_none_or(|inside| d <= tol || fill_contains(bp, rule, p) == inside);
        // Every dash gets the caps, as wide as the stroke where the dash ends: round ones reach as
        // far as the widest point, projecting ones a corner's diagonal.
        let dash_caps = match self.cap {
            LineCap::Butt => None,
            _ if !self.dash.as_ref().is_some_and(Dash::is_dashed) => None,
            LineCap::Round => Some(1.0),
            LineCap::Square => Some(std::f64::consts::SQRT_2),
        };
        for (r, closed) in subpaths(bp) {
            let segs = drawn(&els[r]);
            let (half, region) = self.body(closed);
            let full = half * self.profile_max() * dash_caps.unwrap_or(1.0);
            // The nearest point on this subpath.
            let near = segs.iter().enumerate().map(|(i, s)| (i, s.nearest(p, 1e-9))).min_by(|a, b| a.1.distance_sq.total_cmp(&b.1.distance_sq));
            if let Some((i, nr)) = near
                && nr.distance_sq.sqrt() <= full + tol
            {
                let d = nr.distance_sq.sqrt();
                let factor = if dash_caps.is_some() {
                    full / half.max(1e-12)
                } else {
                    self.profile.as_ref().map_or(1.0, |pr| {
                        let (l, rt) = profile_at(pr, &segs, closed, i, nr.t);
                        // Nearest to a segment's end, `p` is beside a join or past a cap, which
                        // spans the widths of both sides.
                        let at_end = nr.t <= 1e-9 || nr.t >= 1.0 - 1e-9;
                        // Left of the direction of travel (y down) reads the left width.
                        let tn = tangent(&segs[i], nr.t);
                        let left = (p - segs[i].eval(nr.t)).dot(Vec2::new(tn.y, -tn.x)) >= 0.0;
                        if at_end {
                            l.max(rt)
                        } else if left {
                            l
                        } else {
                            rt
                        }
                    })
                };
                if d <= half * factor.max(0.0) + tol && on_side(region, d) {
                    return true;
                }
            }
            if full <= 0.0 {
                continue;
            }
            if self.join == LineJoin::Miter
                && joints(&segs, closed).any(|(at, t_in, t_out)| {
                    miter(at, t_in, t_out, full, self.miter_limit).is_some_and(|[a, tip, b]| in_convex(&[at, a, tip, b], p, tol))
                })
                && on_side(region, f64::INFINITY)
            {
                return true;
            }
            if self.cap == LineCap::Square
                && !closed
                && ends(&segs).into_iter().flatten().any(|(at, dir)| {
                    let n = Vec2::new(-dir.y, dir.x) * full;
                    in_convex(&[at + n, at + n + dir * full, at - n + dir * full, at - n], p, tol)
                })
            {
                return true;
            }
        }
        false
    }

    /// Is `p` within `tol` of where the start (`end == false`) or end arrowhead at `at` (the path
    /// leaving along `out`) can paint? Heads are at most about 4.6 weights long and 4 wide. With
    /// [`ArrowAlign::Extend`] a head points along `out` with its tip up to its length (or the cap)
    /// past the end; with [`ArrowAlign::Tip`] its tip is on the end and it lies back along the
    /// chord the path bends through under it, so anywhere within its length of the end.
    fn head_contains(&self, at: Point, out: Vec2, end: bool, p: Point, tol: f64) -> bool {
        let hw = self.arrow_weight(end);
        let v = p - at;
        match self.arrow_align {
            ArrowAlign::Tip => v.hypot() <= HEAD_REACH * hw + tol,
            ArrowAlign::Extend => {
                let along = v.dot(out);
                v.cross(out).abs() <= 2.0 * hw + tol && along <= 4.0 * hw + self.width / 2.0 + tol && along >= -HEAD_REACH * hw - tol
            }
        }
    }
}

impl Appearance {
    /// The visual bounds of a path with geometric bounds `bounds` painted with this appearance:
    /// the bounds with what each visible stroke paints round it ([`StrokeLayer::bounds_on`]).
    /// `bp` (the path) is only built when a stroke needs its shape.
    pub fn stroked_bounds(&self, bounds: Rect, bp: impl FnOnce() -> BezPath) -> Rect {
        let mut path = Some(bp);
        let mut shape = None;
        self.stroked_bounds_by(bounds, |s| {
            let bp: &BezPath = shape.get_or_insert_with(|| path.take().map(|f| f()).unwrap_or_default());
            s.bounds_on(bp, bounds)
        })
    }

    /// [`Self::stroked_bounds`] with the box of each visible stroke that needs the path's shape
    /// (miter spikes, projecting or butt caps, arrowheads or an alignment) given by `shaped`.
    pub fn stroked_bounds_by(&self, bounds: Rect, mut shaped: impl FnMut(&StrokeLayer) -> Rect) -> Rect {
        self.painted_strokes().fold(bounds, |acc, s| {
            if !s.needs_shape() {
                let o = s.side_reach(false);
                return acc.union(bounds.inflate(o, o));
            }
            acc.union(shaped(s))
        })
    }

    /// Does this appearance have a visible butt-capped stroke?
    pub fn has_butt_stroke(&self) -> bool {
        self.painted_strokes().any(|s| s.cap == LineCap::Butt)
    }

    /// Does a visible butt-capped stroke of this appearance run along an open subpath of `bp` with
    /// a segment that starts or ends at a slant, not square to the axes, once placed by `a`? The
    /// visual box's margin at the stroke's corners there depends on their direction, which a scale
    /// out of proportion changes.
    pub fn has_slanted_butt_stroke(&self, bp: &BezPath, a: vectorcraft_geom::Affine) -> bool {
        if !self.has_butt_stroke() {
            return false;
        }
        let slanted = |v: Vec2| {
            let d = unit(a * v.to_point() - a * Point::ZERO);
            (d.x * d.y).abs() > 1e-9
        };
        let els = bp.elements();
        subpaths(bp)
            .into_iter()
            .filter(|(_, closed)| !closed)
            .any(|(r, _)| drawn(&els[r]).iter().any(|s| slanted(tangent(s, 0.0)) || slanted(tangent(s, 1.0))))
    }

    /// Does a visible stroke of this appearance along `bp` (filled with `rule`) paint within
    /// `tol` of `p` ([`StrokeLayer::hits`])?
    pub fn stroke_hit(&self, bp: &BezPath, rule: FillRule, p: Point, tol: f64) -> bool {
        self.painted_strokes().any(|s| s.hits(bp, rule, p, tol))
    }
}

/// How many symbol instances [`crate::Node::has_slanted_butt_stroke`] looks into for one object
/// before it answers yes without looking further.
pub const SLANT_INSTANCES: usize = 4096;

impl crate::Node {
    /// Does a visible butt-capped stroke of this object, of an object in it or of an object in a
    /// symbol instance's art from `symbols`, placed by `a`, run along an open path with a segment
    /// at a slant ([`Appearance::has_slanted_butt_stroke`])? Past [`SLANT_INSTANCES`] instances it
    /// answers yes.
    pub fn has_slanted_butt_stroke(&self, symbols: &crate::SymbolBoxes, a: vectorcraft_geom::Affine) -> bool {
        self.slanted_butt(symbols, a, &std::cell::Cell::new(SLANT_INSTANCES))
    }

    /// [`Self::has_slanted_butt_stroke`] with `left` instances still to look into.
    fn slanted_butt(&self, symbols: &crate::SymbolBoxes, a: vectorcraft_geom::Affine, left: &std::cell::Cell<usize>) -> bool {
        if let crate::NodeKind::SymbolInstance { symbol, xf } = &self.kind {
            let Some(l) = left.get().checked_sub(1) else { return true };
            left.set(l);
            return symbols.measure_art(symbol, |art| art.slanted_butt(symbols, a * *xf, left)).unwrap_or(false);
        }
        (self.appearance.has_butt_stroke() && self.stroke_path().is_some_and(|bp| self.appearance.has_slanted_butt_stroke(&bp, a)))
            || self.children().is_some_and(|c| c.iter().any(|c| c.slanted_butt(symbols, a, left)))
    }
}

#[cfg(test)]
mod tests {
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_geom::{PathData, shapes};

    use super::*;
    use crate::{Arrowhead, Document, Node, StrokeAlign, hit};

    fn stroke(width: f64, f: impl FnOnce(&mut StrokeLayer)) -> StrokeLayer {
        let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
        f(&mut st);
        st
    }

    fn line() -> BezPath {
        shapes::line(Point::new(0.0, 0.0), Point::new(100.0, 0.0)).to_bezpath()
    }

    /// A thin triangle whose 22.6° apex at (100, 20) miters about 5 times half the weight out.
    fn triangle() -> BezPath {
        let mut bp = BezPath::new();
        bp.move_to((0.0, 0.0));
        bp.line_to((100.0, 20.0));
        bp.line_to((0.0, 40.0));
        bp.close_path();
        bp
    }

    fn square() -> BezPath {
        shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)).to_bezpath()
    }

    #[test]
    fn a_bigger_arrowhead_grows_the_bounds() {
        let l = line();
        let b = l.bounding_box();
        let at = |scale: f64| {
            stroke(2.0, |s| {
                s.join = LineJoin::Round;
                s.end_arrow = Some(Arrowhead::Triangle);
                s.arrow_scale = (100.0, scale);
            })
        };
        let (small, big) = (at(100.0).bounds_on(&l, b), at(300.0).bounds_on(&l, b));
        assert!(big.x1 > small.x1 + 15.0 && big.y1 > small.y1 + 7.0, "{small:?} → {big:?}");
        // A 300% head on a 2 pt stroke is 24 pt long and wide: its tip, past the end, is inside.
        assert!(big.x1 >= 100.0 + 24.0 && big.y1 >= 12.0 && big.x0 == 0.0, "{big:?}");
        // The start has no head.
        assert_eq!(at(300.0).head_reach(false), 0.0);
    }

    #[test]
    fn rectangles_keep_exact_bounds_and_sharp_corners_spike_up_to_the_limit() {
        let sq = square();
        let b = sq.bounding_box();
        let st = stroke(4.0, |_| {});
        assert_eq!(st.bounds_on(&sq, b), b.inflate(2.0, 2.0), "right-angle miters stay in the box");
        // A sharp spike: a thin triangle's apex miters far out unless the limit bevels it.
        let tri = triangle();
        let tb = tri.bounding_box();
        let spiked = st.bounds_on(&tri, tb);
        assert!(spiked.x1 > 100.0 + 10.0, "{spiked:?}");
        let mut low = st.clone();
        low.miter_limit = 2.0;
        assert!(low.bounds_on(&tri, tb).x1 <= 100.0 + 2.0 + 1e-9, "beveled");
        // Projecting caps reach past the ends; inside strokes stay in.
        let l = line();
        let cap = stroke(4.0, |s| s.cap = LineCap::Square).bounds_on(&l, l.bounding_box());
        assert_eq!((cap.x0, cap.x1, cap.y1), (-2.0, 102.0, 2.0));
        assert_eq!(stroke(4.0, |s| s.align = StrokeAlign::Inside).bounds_on(&sq, b), b);
        assert_eq!(stroke(4.0, |s| s.align = StrokeAlign::Outside).bounds_on(&sq, b), b.inflate(4.0, 4.0));
    }

    /// An open path through `points`.
    fn polyline(points: &[(f64, f64)]) -> BezPath {
        let mut bp = BezPath::new();
        for (i, p) in points.iter().enumerate() {
            if i == 0 { bp.move_to(*p) } else { bp.line_to(*p) }
        }
        bp
    }

    #[test]
    fn butt_caps_stop_at_the_ends_of_an_open_path() {
        let butt = |f: fn(&mut StrokeLayer)| {
            stroke(2.0, |s| {
                s.cap = LineCap::Butt;
                f(s);
            })
        };
        // A vertical line reaches sideways only.
        let v = polyline(&[(50.0, 10.0), (50.0, 60.0)]);
        assert_eq!(butt(|_| {}).bounds_on(&v, v.bounding_box()), Rect::new(49.0, 10.0, 51.0, 60.0));
        // A 3-4-5 diagonal: its corners reach 0.8 pt across x and 0.6 pt across y.
        let d = polyline(&[(0.0, 0.0), (30.0, 40.0)]);
        let b = butt(|_| {}).bounds_on(&d, d.bounding_box());
        for (got, want) in [(b.x0, -0.8), (b.y0, -0.6), (b.x1, 30.8), (b.y1, 40.6)] {
            assert!((got - want).abs() < 1e-9, "{b:?}");
        }
        // The dashes of a butt stroke stop inside the line as well, and an open path ignores the
        // alignment.
        let dashed = butt(|s| s.dash = Some(Dash { pattern: vec![4.0, 2.0], offset: 0.0, align_corners: false }));
        assert_eq!(dashed.bounds_on(&v, v.bounding_box()), Rect::new(49.0, 10.0, 51.0, 60.0));
        assert_eq!(butt(|s| s.align = StrokeAlign::Inside).bounds_on(&v, v.bounding_box()), Rect::new(49.0, 10.0, 51.0, 60.0));
        // A round cap reaches half the weight past each end, a square one the corners.
        let round = butt(|s| s.cap = LineCap::Round).bounds_on(&v, v.bounding_box());
        assert_eq!(round, Rect::new(49.0, 9.0, 51.0, 61.0));
        let square = butt(|s| s.cap = LineCap::Square).bounds_on(&v, v.bounding_box());
        assert_eq!(square, Rect::new(49.0, 9.0, 51.0, 61.0));
        // Two strokes union.
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
        ap.stroke_mut().unwrap().cap = LineCap::Butt;
        let wide = stroke(10.0, |s| s.cap = LineCap::Round);
        ap.items.push(crate::AppearanceItem::Stroke(wide));
        let vb = v.bounding_box();
        assert_eq!(ap.stroked_bounds(vb, || v.clone()), Rect::new(45.0, 5.0, 55.0, 65.0));
    }

    #[test]
    fn a_butt_paths_joins_and_curves_stay_inside_the_box() {
        // A corner in the middle: the round join reaches a full half weight straight down, outside
        // the turn, the ends do not, and a miter reaches its tip.
        let zig = polyline(&[(0.0, 0.0), (20.0, 20.0), (40.0, 0.0)]);
        let round = stroke(4.0, |s| {
            s.cap = LineCap::Butt;
            s.join = LineJoin::Round;
        })
        .bounds_on(&zig, zig.bounding_box());
        assert!((round.y1 - 22.0).abs() < 1e-9, "the round join's reach: {round:?}");
        assert!((round.x0 + 2.0_f64.sqrt()).abs() < 1e-9 && (round.x1 - 40.0 - 2.0_f64.sqrt()).abs() < 1e-9, "the ends' corners: {round:?}");
        // A round join paints its arc on the outer side of a turn too shallow to count as a
        // corner, and nothing where the path goes straight on.
        let round_butt = stroke(4.0, |s| {
            s.cap = LineCap::Butt;
            s.join = LineJoin::Round;
        });
        let shallow = polyline(&[(0.0, 0.25), (50.0, 0.0), (100.0, 0.25)]);
        let b = round_butt.bounds_on(&shallow, shallow.bounding_box());
        assert!((b.y0 + 2.0).abs() < 1e-12, "{b:?}");
        let straight = polyline(&[(0.0, 0.0), (0.1, 0.0), (100.0, 0.0)]);
        assert_eq!(round_butt.bounds_on(&straight, straight.bounding_box()), Rect::new(0.0, -2.0, 100.0, 2.0));
        let miter = stroke(4.0, |s| s.cap = LineCap::Butt).bounds_on(&zig, zig.bounding_box());
        assert!((miter.y1 - (20.0 + 2.0 * 2.0_f64.sqrt())).abs() < 1e-9, "the miter tip: {miter:?}");
        // An arc over the top whose ends leave the page straight down: the top reaches half the
        // weight past the path, and the bottom stops at the ends.
        let mut arc = BezPath::new();
        arc.move_to((0.0, 50.0));
        arc.curve_to((0.0, 22.4), (22.4, 0.0), (50.0, 0.0));
        arc.curve_to((77.6, 0.0), (100.0, 22.4), (100.0, 50.0));
        let b = stroke(4.0, |s| s.cap = LineCap::Butt).bounds_on(&arc, arc.bounding_box());
        assert!((b.y0 + 2.0).abs() < 1e-9 && (b.x0 + 2.0).abs() < 1e-9 && (b.x1 - 102.0).abs() < 1e-9, "{b:?}");
        assert!((b.y1 - 50.0).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn a_butt_curve_with_a_handle_on_its_end_point_stops_near_that_end() {
        let butt = stroke(2.0, |s| s.cap = LineCap::Butt);
        // A straight curve whose first or last handle sits on its end point measures like a line.
        let mut first = BezPath::new();
        first.move_to((50.0, 10.0));
        first.curve_to((50.0, 10.0), (50.0, 40.0), (50.0, 60.0));
        let mut last = BezPath::new();
        last.move_to((50.0, 10.0));
        last.curve_to((50.0, 20.0), (50.0, 60.0), (50.0, 60.0));
        for bp in [first, last] {
            assert_eq!(butt.bounds_on(&bp, bp.bounding_box()), Rect::new(49.0, 10.0, 51.0, 60.0));
        }
        // A bend that leaves its start straight up from a handle on the start point: the stroke
        // reaches only a little below the start, not half the weight.
        let mut bend = BezPath::new();
        bend.move_to((0.0, 50.0));
        bend.curve_to((0.0, 50.0), (0.0, 0.0), (50.0, 0.0));
        let b = stroke(4.0, |s| s.cap = LineCap::Butt).bounds_on(&bend, bend.bounding_box());
        assert!(b.y1 >= 50.0 && b.y1 < 50.2, "{b:?}");
        assert!((b.x0 + 2.0).abs() < 1e-9 && (b.y0 + 2.0).abs() < 1e-9 && (b.x1 - 50.0).abs() < 1e-9, "{b:?}");
        // Nearly a cusp: the curve turns back at its lowest point, where its tangent is level and
        // the stroke reaches the full half weight below it.
        let (a0, a1, a2) = (1e5 * 81.0 / 256.0, -1e5 * 9.0 / 8.0, 1e5);
        let x = |i: f64| 2e-9 * i / 3.0;
        let mut cusp = BezPath::new();
        cusp.move_to((x(0.0), a0));
        cusp.curve_to((x(1.0), a0 + a1 / 3.0), (x(2.0), a0 + 2.0 * a1 / 3.0 + a2 / 3.0), (x(3.0), a0 + a1 + a2));
        let b = butt.bounds_on(&cusp, cusp.bounding_box());
        assert!(b.y0 <= -1.0 + 1e-6, "{b:?}");
        // A curve far shorter than the weight still reaches half the weight above where its
        // tangent is level.
        let mut tiny = BezPath::new();
        tiny.move_to((0.0, 0.9e-12));
        tiny.curve_to((0.3e-12, -0.5e-12), (1.7e-12, -0.2e-12), (2e-12, 1e-12));
        let b = butt.bounds_on(&tiny, tiny.bounding_box());
        assert!(b.y0 <= -1.0 + 1e-9, "{b:?}");
    }

    #[test]
    fn a_butt_stroke_is_slanted_where_a_segment_starts_or_ends_at_a_slant() {
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
        ap.stroke_mut().unwrap().cap = LineCap::Butt;
        let id = vectorcraft_geom::Affine::IDENTITY;
        assert!(!ap.has_slanted_butt_stroke(&polyline(&[(0.0, 0.0), (0.0, 50.0), (30.0, 50.0)]), id));
        // Square ends, slanted segments in the middle.
        let apex = polyline(&[(0.0, 50.0), (0.0, 40.0), (10.0, 0.0), (20.0, 40.0), (20.0, 50.0)]);
        assert!(ap.has_slanted_butt_stroke(&apex, id));
        // A rotation slants a vertical line; a round cap is never slanted.
        let v = polyline(&[(0.0, 0.0), (0.0, 50.0)]);
        assert!(ap.has_slanted_butt_stroke(&v, vectorcraft_geom::Affine::rotate(0.3)));
        ap.stroke_mut().unwrap().cap = LineCap::Round;
        assert!(!ap.has_slanted_butt_stroke(&apex, id));
    }

    #[test]
    fn the_slant_check_looks_into_at_most_4096_symbol_instances() {
        // S0 is a vertical butt line. Each of S1 to S12 holds two instances of the symbol before
        // it, so an instance of S12 holds 8,191 instances and none of them is slanted. Past 4,096
        // the check answers yes.
        let mut d = Document::new(400.0, 400.0);
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
        ap.stroke_mut().unwrap().cap = LineCap::Butt;
        let line = Node::path(d.alloc_id(), PathData::from_bezpath(&polyline(&[(0.0, 0.0), (0.0, 10.0)])), ap);
        d.symbols.push(crate::Symbol { dynamic: false, name: "S0".into(), art: std::sync::Arc::new(line) });
        for k in 1..=12 {
            let mut instance = |dx: f64| {
                let xf = vectorcraft_geom::Affine::translate((dx, 0.0));
                std::sync::Arc::new(Node::new(d.alloc_id(), crate::NodeKind::SymbolInstance { symbol: format!("S{}", k - 1), xf }))
            };
            let pair = vec![instance(0.0), instance(20.0)];
            let art = Node::group(d.alloc_id(), pair);
            d.symbols.push(crate::Symbol { dynamic: false, name: format!("S{k}"), art: std::sync::Arc::new(art) });
        }
        let top = Node::new(d.alloc_id(), crate::NodeKind::SymbolInstance { symbol: "S12".into(), xf: vectorcraft_geom::Affine::IDENTITY });
        let symbols = d.symbol_boxes();
        assert!(top.has_slanted_butt_stroke(&symbols, vectorcraft_geom::Affine::IDENTITY));
        assert!(symbols.art_measured() <= SLANT_INSTANCES, "{}", symbols.art_measured());
        // One instance of S1 holds three instances, all looked into.
        let one = Node::new(d.alloc_id(), crate::NodeKind::SymbolInstance { symbol: "S1".into(), xf: vectorcraft_geom::Affine::IDENTITY });
        assert!(!one.has_slanted_butt_stroke(&d.symbol_boxes(), vectorcraft_geom::Affine::IDENTITY));
    }

    #[test]
    fn a_compound_path_measures_each_of_its_subpaths_by_its_own_ends() {
        // A closed square at the top and an open line below it: only the square reaches past its
        // own edges.
        let mut bp = BezPath::new();
        bp.extend(square().elements().iter().copied());
        bp.move_to((50.0, 150.0));
        bp.line_to((50.0, 200.0));
        let st = stroke(4.0, |s| s.cap = LineCap::Butt);
        assert_eq!(st.bounds_on(&bp, bp.bounding_box()), Rect::new(-2.0, -2.0, 102.0, 200.0));
    }

    #[test]
    fn a_middle_segment_can_set_the_box_of_a_butt_path() {
        // The flat middle segment reaches a full half weight above the path, and the ends' corners
        // reach 10/51 of it below.
        let hat = polyline(&[(0.0, 50.0), (10.0, 0.0), (20.0, 0.0), (30.0, 50.0)]);
        let b = stroke(2.0, |s| {
            s.cap = LineCap::Butt;
            s.join = LineJoin::Bevel;
        })
        .bounds_on(&hat, hat.bounding_box());
        let corner = 10.0 / (10.0_f64 * 10.0 + 50.0 * 50.0).sqrt();
        assert!((b.y0 + 1.0).abs() < 1e-9 && (b.y1 - 50.0 - corner).abs() < 1e-9, "{b:?}");
    }

    #[test]
    fn bounds_with_tips_measures_a_butt_line_as_bounds_on_does() {
        let v = polyline(&[(50.0, 10.0), (50.0, 60.0)]);
        let st = stroke(2.0, |s| s.cap = LineCap::Butt);
        assert_eq!(st.bounds_with_tips(&v, v.bounding_box(), None), Rect::new(49.0, 10.0, 51.0, 60.0));
    }

    #[test]
    fn clicks_hit_outside_strokes_on_their_side_only() {
        let sq = square();
        let w = 10.0;
        let st = stroke(w, |s| s.align = StrokeAlign::Outside);
        assert!(st.hits(&sq, FillRule::NonZero, Point::new(-0.8 * w, 50.0), 0.5), "0.8·w outside");
        assert!(!st.hits(&sq, FillRule::NonZero, Point::new(0.8 * w, 50.0), 0.5), "nothing painted inside");
        assert!(st.hits(&sq, FillRule::NonZero, Point::new(0.3, 50.0), 0.5), "on the path");
        let inside = stroke(w, |s| s.align = StrokeAlign::Inside);
        assert!(inside.hits(&sq, FillRule::NonZero, Point::new(0.8 * w, 50.0), 0.5));
        assert!(!inside.hits(&sq, FillRule::NonZero, Point::new(-0.8 * w, 50.0), 0.5));
        // A centred stroke reaches half its weight each side.
        let centre = stroke(w, |_| {});
        assert!(centre.hits(&sq, FillRule::NonZero, Point::new(-4.0, 50.0), 0.5));
        assert!(!centre.hits(&sq, FillRule::NonZero, Point::new(-8.0, 50.0), 0.5));
    }

    #[test]
    fn clicks_follow_the_width_profile() {
        let l = line();
        let lens = stroke(10.0, |s| s.profile = Some(WidthProfile::lens()));
        assert!(lens.hits(&l, FillRule::NonZero, Point::new(50.0, 4.8), 0.1), "the lens's widest point");
        assert!(!lens.hits(&l, FillRule::NonZero, Point::new(5.0, 4.8), 0.1), "its thin end");
        // A profile wider than the weight on one side.
        let wide = stroke(10.0, |s| s.profile = Some(WidthProfile { points: vec![(0.0, 3.0, 1.0), (1.0, 3.0, 1.0)] }));
        // Left of travel (+x, y down) is −y.
        assert!(wide.hits(&l, FillRule::NonZero, Point::new(50.0, -14.0), 0.1));
        assert!(!wide.hits(&l, FillRule::NonZero, Point::new(50.0, 14.0), 0.1));
        assert!(wide.bounds_on(&l, l.bounding_box()).y0 <= -15.0);
    }

    #[test]
    fn clicks_hit_heads_caps_and_spikes() {
        let l = line();
        let head = stroke(2.0, |s| {
            s.end_arrow = Some(Arrowhead::Triangle);
            s.arrow_scale = (100.0, 300.0);
        });
        // The 24 pt head sits past the end in extend mode, 12 pt either side of the line.
        assert!(head.hits(&l, FillRule::NonZero, Point::new(110.0, 6.0), 0.1));
        assert!(!head.hits(&l, FillRule::NonZero, Point::new(50.0, 6.0), 0.1));
        let mut tip = head.clone();
        tip.arrow_align = ArrowAlign::Tip;
        assert!(tip.hits(&l, FillRule::NonZero, Point::new(90.0, 6.0), 0.1) && !tip.hits(&l, FillRule::NonZero, Point::new(130.0, 0.0), 0.1));
        let cap = stroke(10.0, |s| s.cap = LineCap::Square);
        assert!(cap.hits(&l, FillRule::NonZero, Point::new(104.0, 4.5), 0.1), "the cap's corner");
        assert!(!stroke(10.0, |_| {}).hits(&l, FillRule::NonZero, Point::new(104.0, 4.5), 0.1), "a butt end");
        let tri = triangle();
        let spike = stroke(4.0, |_| {});
        assert!(spike.hits(&tri, FillRule::NonZero, Point::new(106.0, 20.0), 0.1), "in the miter spike");
        assert!(!spike.hits(&tri, FillRule::NonZero, Point::new(106.0, 23.0), 0.1));
    }

    #[test]
    fn document_hit_testing_and_visual_bounds_use_the_whole_stroke() {
        let mut d = Document::new(300.0, 300.0);
        let (l, id) = (d.layers[0].id, d.alloc_id());
        let mut ap = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 10.0);
        ap.stroke_mut().unwrap().align = StrokeAlign::Outside;
        let sq = PathData::from_bezpath(&shapes::rectangle(Rect::new(100.0, 100.0, 200.0, 200.0)).to_bezpath());
        d.insert(Some(l), 0, Node::path(id, sq, ap)).unwrap();
        let opt = hit::HitOptions { tol: 1.0, ..Default::default() };
        let h = hit::hit_test(&d, Point::new(92.0, 150.0), opt).expect("0.8·w outside the path");
        assert_eq!((h.leaf, h.kind), (id, hit::HitKind::Stroke));
        assert!(hit::hit_test(&d, Point::new(108.0, 150.0), opt).is_none(), "an unfilled interior");
        assert_eq!(d.node(id).unwrap().visual_bounds(), Some(Rect::new(90.0, 90.0, 210.0, 210.0)));
    }
}
