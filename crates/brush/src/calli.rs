//! Calligraphic and bristle brushes: an elliptical nib swept along the path.
//!
//! The swept area of an ellipse moving along a curve is bounded by the ellipse's support points
//! in the directions normal to the path. For nib axes `u`, `v` with semi-axes `a`, `b` the
//! support point in direction `d` is `(a²(u·d)u + b²(v·d)v) / √(a²(u·d)² + b²(v·d)²)`, so the
//! stroke's local width across direction `n` is `2·√(a²(u·n)² + b²(v·n)²)`.

use vectorcraft_color::Paint;
use vectorcraft_doc::Node;
use vectorcraft_geom::{BezPath, PathData, Point, SubPath, Vec2};

use crate::track::{Rng, Track, normal, seed_of, tracks};
use crate::{Bristle, BristleShape, Calligraphic, filled, tolerance};

/// An elliptical nib.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Nib {
    u: Vec2,
    v: Vec2,
    a: f64,
    b: f64,
}

impl Nib {
    /// `angle` in degrees counter-clockwise on the page (y-down document space), `size` the
    /// major diameter, `roundness` the minor/major ratio (0..1].
    pub fn new(angle_deg: f64, roundness: f64, size: f64) -> Self {
        let t = angle_deg.to_radians();
        let u = Vec2::new(t.cos(), -t.sin());
        let v = Vec2::new(-u.y, u.x);
        let a = (size / 2.0).max(1e-4);
        Nib { u, v, a, b: (a * roundness.clamp(0.01, 1.0)).max(1e-4) }
    }
    pub fn support(&self, d: Vec2) -> Vec2 {
        let (du, dv) = (self.u.dot(d), self.v.dot(d));
        let den = ((self.a * du).powi(2) + (self.b * dv).powi(2)).sqrt().max(1e-12);
        (self.u * (self.a * self.a * du) + self.v * (self.b * self.b * dv)) / den
    }
}

/// Unit vector at `angle` radians from `base` towards its left normal.
fn rot(base: Vec2, angle: f64) -> Vec2 {
    base * angle.cos() + normal(base) * angle.sin()
}

/// Samples (point, tangent) along a track, with extra tangents fanned in at corners so the
/// outer edge of the sweep follows the nib around the corner.
fn samples(t: &Track) -> Vec<(Point, Vec2)> {
    let mut out = vec![];
    let n = t.segments();
    for i in 0..=n {
        let p = t.verts[i];
        if t.corner[i] {
            let (a, b) = t.in_out(i);
            let turn = a.cross(b).atan2(a.dot(b));
            let steps = (turn.abs() / 10f64.to_radians()).ceil().max(1.0) as usize;
            for k in 0..=steps {
                out.push((p, rot(a, turn * k as f64 / steps as f64)));
            }
        } else {
            out.push((p, t.tangents[i]));
        }
    }
    out
}

/// The swept outline of `nib` along `t` as closed subpaths.
pub(crate) fn sweep(t: &Track, nib: &Nib) -> Vec<SubPath> {
    let s = samples(t);
    let left: Vec<Point> = s.iter().map(|(p, tg)| *p + nib.support(normal(*tg))).collect();
    let right: Vec<Point> = s.iter().map(|(p, tg)| *p + nib.support(-normal(*tg))).collect();
    if t.closed {
        let mut r = right;
        r.reverse();
        return vec![SubPath::polyline(&left, true), SubPath::polyline(&r, true)];
    }
    let cap = |p: Point, tg: Vec2, from: f64, to: f64, out: &mut Vec<Point>| {
        let steps = 16;
        for k in 1..steps {
            let ang = from + (to - from) * k as f64 / steps as f64;
            out.push(p + nib.support(rot(tg, ang)));
        }
    };
    let (Some(&(ps, ts)), Some(&(pe, te))) = (s.first(), s.last()) else {
        return vec![];
    };
    let half = std::f64::consts::FRAC_PI_2;
    let mut poly = left.clone();
    cap(pe, te, half, -half, &mut poly);
    poly.extend(right.iter().rev());
    cap(ps, ts, -half, -3.0 * half, &mut poly);
    vec![SubPath::polyline(&poly, true)]
}

fn randomized(c: &Calligraphic, rng: &mut Rng) -> (f64, f64, f64) {
    let [va, vr, vs] = c.variation;
    let angle = c.angle + rng.range(-va, va);
    let round = (c.roundness + rng.range(-vr, vr)).clamp(1.0, 100.0);
    let size = (c.size + rng.range(-vs, vs)).max(0.05);
    (angle, round, size)
}

pub(crate) fn calligraphic(c: &Calligraphic, bp: &BezPath, weight: f64, paint: &Paint, name: &str) -> Vec<Node> {
    let mut rng = Rng::new(seed_of(bp, name));
    let (angle, round, size) = randomized(c, &mut rng);
    let size = size * weight;
    let nib = Nib::new(angle, round / 100.0, size);
    let subs: Vec<SubPath> = tracks(bp, tolerance(size)).iter().flat_map(|t| sweep(t, &nib)).collect();
    if subs.is_empty() {
        return vec![];
    }
    vec![filled(PathData::new(subs), paint)]
}

/// Offset a track sideways by `d` (a polyline following it).
fn offset_track(t: &Track, d: f64, trim0: f64, trim1: f64) -> Option<BezPath> {
    let l = t.len();
    let (s0, s1) = (trim0.clamp(0.0, l), (l - trim1).clamp(0.0, l));
    if s1 - s0 < 1e-6 {
        return None;
    }
    let mut bp = BezPath::new();
    let mut first = true;
    let mut push = |p: Point, bp: &mut BezPath| {
        if first {
            bp.move_to(p);
            first = false;
        } else {
            bp.line_to(p);
        }
    };
    push(t.map(s0, d), &mut bp);
    for (i, c) in t.cum.iter().enumerate() {
        if *c > s0 && *c < s1 {
            push(t.verts[i] + normal(t.tangents[i]) * d, &mut bp);
        }
    }
    push(t.map(s1, d), &mut bp);
    if t.closed && trim0 <= 0.0 && trim1 <= 0.0 {
        bp.close_path();
    }
    Some(bp)
}

pub(crate) fn bristle(b: &Bristle, bp: &BezPath, weight: f64, paint: &Paint, name: &str) -> Vec<Node> {
    let mut rng = Rng::new(seed_of(bp, name));
    let size = b.size.max(0.1) * weight;
    let strands = (3.0 + b.density.clamp(1.0, 100.0) / 100.0 * 13.0).round() as usize;
    let thick = (size * b.thickness.clamp(1.0, 100.0) / 100.0 * 0.45).max(0.15);
    let flat = matches!(
        b.shape,
        BristleShape::FlatPoint | BristleShape::FlatBlunt | BristleShape::FlatCurve | BristleShape::FlatAngle | BristleShape::FlatFan
    );
    let wobble = (100.0 - b.stiffness.clamp(1.0, 100.0)) / 100.0;
    let ragged = b.length.clamp(25.0, 300.0) / 100.0;
    let opacity = (b.opacity.clamp(1.0, 100.0) / 100.0) as f32;
    let mut out = vec![];
    for t in tracks(bp, tolerance(size)) {
        let l = t.len();
        for k in 0..strands {
            // Round tips concentrate strands near the centre; flat tips spread them evenly.
            let lane = (k as f64 + 0.5) / strands as f64 * 2.0 - 1.0;
            let off = if flat { lane } else { lane.signum() * lane.abs().powf(1.4) } * size / 2.0 + rng.range(-wobble, wobble) * size * 0.08;
            let edge = 1.0 - lane.abs();
            let trim = |r: &mut Rng| r.unit() * ragged * size * (1.2 - edge) * 0.8;
            let (t0, t1) = (trim(&mut rng), trim(&mut rng));
            let Some(sp) = offset_track(&t, off, t0.min(l * 0.3), t1.min(l * 0.3)) else { continue };
            let w = thick * (0.6 + 0.4 * edge) * rng.range(0.8, 1.2);
            let nib = Nib::new(0.0, 1.0, w);
            let subs: Vec<SubPath> = tracks(&sp, tolerance(w)).iter().flat_map(|tt| sweep(tt, &nib)).collect();
            if subs.is_empty() {
                continue;
            }
            let mut n = filled(PathData::new(subs), paint);
            n.opacity = (opacity * rng.range(0.7, 1.0) as f32).clamp(0.0, 1.0);
            out.push(n);
        }
    }
    out
}
