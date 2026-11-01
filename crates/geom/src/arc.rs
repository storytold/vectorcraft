//! Arc-length parametrization of a path: the point and direction at a distance along it, and the
//! distance along it of the point nearest another (type on a path lays its glyphs out and the
//! selection tools drag its brackets by distance along the path).

use kurbo::{ParamCurve, ParamCurveArclen, ParamCurveNearest, PathSeg, Point, Vec2};

use crate::PathData;

const ACCURACY: f64 = 1e-4;

/// A path's segments with their lengths, measured once.
#[derive(Clone, Debug, Default)]
pub struct ArcPath {
    /// Segments of some length, each with the distance to its start and its length.
    segs: Vec<(PathSeg, f64, f64)>,
    len: f64,
    closed: bool,
}

impl ArcPath {
    pub fn new(path: &PathData) -> Self {
        let mut segs = Vec::new();
        let mut cum = 0.0;
        for s in path.to_bezpath().segments() {
            let l = crate::path::measured_arclen(&s, ACCURACY);
            if l.is_finite() && l > 1e-9 {
                segs.push((s, cum, l));
                cum += l;
            }
        }
        Self { segs, len: cum, closed: path.is_closed() }
    }

    /// The path's length.
    pub fn len(&self) -> f64 {
        self.len
    }

    /// No segment has any length.
    pub fn is_empty(&self) -> bool {
        self.segs.is_empty()
    }

    pub fn is_closed(&self) -> bool {
        self.closed
    }

    /// Point and unit tangent at distance `s` along the path: round and round a closed path, held
    /// at an open one's ends.
    pub fn at(&self, s: f64) -> (Point, Vec2) {
        let s = if self.closed && self.len > 0.0 { s.rem_euclid(self.len) } else { s.clamp(0.0, self.len) };
        let i = self.segs.partition_point(|(_, c, _)| *c <= s).saturating_sub(1);
        let Some(&(seg, c, l)) = self.segs.get(i) else { return (Point::ZERO, Vec2::new(1.0, 0.0)) };
        // (The solver's precision is relative to the segment's length: a long one needs a looser one.)
        let t = seg.inv_arclen((s - c).min(l), ACCURACY.max(l * 1e-9)).clamp(0.0, 1.0);
        let p = seg.eval(t);
        let (t0, t1) = ((t - 1e-4).max(0.0), (t + 1e-4).min(1.0));
        let d = seg.eval(t1) - seg.eval(t0);
        let len = d.hypot();
        (p, if len > 1e-12 { d / len } else { Vec2::new(1.0, 0.0) })
    }

    /// The distance along the path of its point nearest `p`.
    pub fn nearest(&self, p: Point) -> Option<f64> {
        self.segs
            .iter()
            .map(|(seg, c, l)| {
                let n = seg.nearest(p, ACCURACY);
                (n.distance_sq, c + seg.subsegment(0.0..n.t).arclen(ACCURACY).min(*l))
            })
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, s)| s)
    }

    /// [`Self::nearest`] as a fraction (0..1) of the path's length.
    pub fn fraction_at(&self, p: Point) -> Option<f64> {
        (self.len > 0.0).then(|| self.nearest(p)).flatten().map(|s| (s / self.len).clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shapes;
    use kurbo::{Rect, Shape};

    #[test]
    fn points_and_nearest_distances_along_a_path() {
        let line = PathData::from_bezpath(&kurbo::Line::new((0.0, 0.0), (100.0, 0.0)).to_path(0.1));
        let a = ArcPath::new(&line);
        assert!((a.len() - 100.0).abs() < 1e-6 && !a.is_closed());
        assert!(a.at(25.0).0.distance(Point::new(25.0, 0.0)) < 1e-6);
        // Held at an open path's ends.
        assert!(a.at(150.0).0.distance(Point::new(100.0, 0.0)) < 1e-6);
        assert!((a.nearest(Point::new(40.0, 30.0)).unwrap() - 40.0).abs() < 1e-6);
        // Round a closed one.
        let sq = ArcPath::new(&shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)));
        assert!(sq.is_closed() && (sq.len() - 40.0).abs() < 1e-6);
        assert!(sq.at(45.0).0.distance(sq.at(5.0).0) < 1e-6);
        assert!(ArcPath::new(&PathData::default()).is_empty());
    }
}
