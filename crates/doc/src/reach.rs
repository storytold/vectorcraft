//! Where a path's strokes paint, from the path's geometry without building their outlines: the
//! visual bounds and the clicks of a stroke take in its alignment, its width profile, miter spikes
//! up to the miter limit, projecting caps and arrowheads.

use vectorcraft_geom::bez::{CORNER_COS, kink, segments, subpaths, tangent, unit};
use vectorcraft_geom::hit::fill_contains;
use vectorcraft_geom::kurbo::{ParamCurveArclen, ParamCurveNearest};
use vectorcraft_geom::{BezPath, FillRule, ParamCurve, PathSeg, Point, Rect, Shape, Vec2};

use crate::appearance::HEAD_REACH;
use crate::{Appearance, ArrowAlign, Dash, LineCap, LineJoin, StrokeAlign, StrokeLayer, WidthProfile};

/// Arc-length accuracy for placing a point along a profile (document points).
const ARCLEN_ACCURACY: f64 = 1e-3;

/// The segments of a subpath that have length (zero-length ones, all of whose points coincide,
/// have no direction).
fn drawn(els: &[vectorcraft_geom::PathEl]) -> Vec<PathSeg> {
    let moves = |s: &PathSeg| match s {
        PathSeg::Line(l) => l.p1 != l.p0,
        PathSeg::Quad(q) => q.p1 != q.p0 || q.p2 != q.p0,
        PathSeg::Cubic(c) => c.p1 != c.p0 || c.p2 != c.p0 || c.p3 != c.p0,
    };
    segments(els).into_iter().filter(moves).collect()
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
fn profile_at(profile: &WidthProfile, segs: &[PathSeg], lens: &[f64], total: f64, closed: bool, i: usize, t: f64) -> (f64, f64) {
    let n = segs.len();
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
    /// or an alignment that depends on whether the path is closed? Else it reaches evenly round it.
    fn needs_shape(&self) -> bool {
        self.join == LineJoin::Miter
            || self.cap == LineCap::Square
            || self.start_arrow.is_some()
            || self.end_arrow.is_some()
            || self.align != StrokeAlign::Center
    }

    /// The box this stroke paints along `bp`, whose geometric bounds are `bounds`: the bounds grown
    /// by the body's reach, with the tips of the miter spikes the limit keeps, the corners of
    /// projecting caps and the reach of the arrowheads round their end points.
    pub fn bounds_on(&self, bp: &BezPath, bounds: Rect) -> Rect {
        let els = bp.elements();
        let mut out = bounds;
        let mut side_max: f64 = 0.0;
        // Dashes put a cap on every dash: projecting ones reach a corner's diagonal anywhere.
        let capped_dashes = self.cap == LineCap::Square && self.dash.as_ref().is_some_and(Dash::is_dashed);
        for (r, closed) in subpaths(bp) {
            let side = self.side_reach(closed);
            side_max = side_max.max(if capped_dashes { side * std::f64::consts::SQRT_2 } else { side });
            if side <= 0.0 {
                continue;
            }
            let segs = drawn(&els[r]);
            if self.join == LineJoin::Miter {
                for (at, t_in, t_out) in joints(&segs, closed) {
                    if let Some([_, tip, _]) = miter(at, t_in, t_out, side, self.miter_limit) {
                        out = out.union_pt(tip);
                    }
                }
            }
            if self.cap == LineCap::Square && !closed {
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
        out.union(bounds.inflate(side_max, side_max))
    }

    /// Does this stroke along `bp` (filled with `rule`) paint within `tol` of `p`? Inside and
    /// outside strokes of closed paths count on their own side only (and within `tol` of the
    /// path); a width profile counts at the width of every segment that can reach `p`, as the
    /// wider side of a farther one can paint past the nearest.
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
            // Every segment near enough to reach `p`: with a width profile the wider side of a
            // farther segment can paint past the nearest one, so the nearest alone is not enough.
            let near = segs.iter().enumerate().filter_map(|(i, s)| {
                let nr = s.nearest(p, 1e-9);
                (nr.distance_sq.sqrt() <= full + tol).then_some((i, nr))
            });
            // The lengths are only needed when a profiled candidate reaches `p`; compute them
            // once for the whole subpath rather than once per candidate.
            let mut lengths: Option<(Vec<f64>, f64)> = None;
            for (i, nr) in near {
                let d = nr.distance_sq.sqrt();
                let factor = if dash_caps.is_some() {
                    full / half.max(1e-12)
                } else {
                    self.profile.as_ref().map_or(1.0, |pr| {
                        let (lens, total) = lengths.get_or_insert_with(|| {
                            let lens: Vec<f64> = segs.iter().map(|s| s.arclen(ARCLEN_ACCURACY)).collect();
                            let total = lens.iter().sum::<f64>().max(1e-12);
                            (lens, total)
                        });
                        let (l, rt) = profile_at(pr, &segs, lens, *total, closed, i, nr.t);
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
        self.painted_strokes().fold(bounds, |acc, s| {
            if !s.needs_shape() {
                let o = s.side_reach(false);
                return acc.union(bounds.inflate(o, o));
            }
            let bp: &BezPath = shape.get_or_insert_with(|| path.take().map(|f| f()).unwrap_or_default());
            acc.union(s.bounds_on(bp, bounds))
        })
    }

    /// Does a visible stroke of this appearance along `bp` (filled with `rule`) paint within
    /// `tol` of `p` ([`StrokeLayer::hits`])?
    pub fn stroke_hit(&self, bp: &BezPath, rule: FillRule, p: Point, tol: f64) -> bool {
        self.painted_strokes().any(|s| s.hits(bp, rule, p, tol))
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
        assert!(big.x1 >= 100.0 + 24.0 && big.y1 >= 12.0 && big.x0 == -1.0, "{big:?}");
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
