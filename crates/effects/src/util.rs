//! Parameter parsing, deterministic noise and path-mapping helpers.

use serde_json::Value;
use vectorcraft_geom::{Anchor, CubicBez, ParamCurve, PathData, Point, Rect, SubPath, Vec2};

/// Number param (also accepts numeric strings such as `"10 pt"`); non-finite → default.
pub fn num(p: &Value, key: &str, default: f64) -> f64 {
    match p.get(key) {
        Some(Value::Number(n)) => n.as_f64(),
        Some(Value::String(s)) => {
            let t: String = s.trim().chars().take_while(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+' | 'e' | 'E')).collect();
            t.parse().ok()
        }
        Some(Value::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
    .filter(|v| v.is_finite())
    .unwrap_or(default)
}

pub fn flag(p: &Value, key: &str, default: bool) -> bool {
    match p.get(key) {
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|v| v != 0.0),
        _ => default,
    }
}

pub fn text<'a>(p: &'a Value, key: &str, default: &'a str) -> &'a str {
    p.get(key).and_then(Value::as_str).unwrap_or(default)
}

pub fn join(p: &Value, key: &str) -> vectorcraft_pathops::Join {
    match text(p, key, "miter").to_ascii_lowercase().as_str() {
        "round" => vectorcraft_pathops::Join::Round,
        "bevel" => vectorcraft_pathops::Join::Bevel,
        _ => vectorcraft_pathops::Join::Miter,
    }
}

/// "smooth" (default) vs "corner" points.
pub fn smooth_points(p: &Value) -> bool {
    !text(p, "points", "smooth").eq_ignore_ascii_case("corner")
}

/// Deterministic hash noise in [-1, 1] for (seed, a, b, c).
pub fn noise(seed: u64, a: u64, b: u64, c: u64) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ a.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ b.wrapping_mul(0x94D0_49BB_1331_11EB) ^ c;
    // splitmix64 finaliser
    x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^= x >> 31;
    (x >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
}

pub fn seed(p: &Value) -> u64 {
    num(p, "seed", 0.0).abs() as u64
}

/// Mean side of a box (the reference for "relative" sizes).
pub fn mean_size(b: Rect) -> f64 {
    (b.width().abs() + b.height().abs()) / 2.0
}

/// Segment as a cubic; straight lines get handles at 1/3 and 2/3 so a non-linear map bends them.
pub fn seg_cubic(sp: &SubPath, i: usize) -> CubicBez {
    let c = sp.segment(i);
    if sp.segment_is_line(i) { CubicBez::new(c.p0, c.p0.lerp(c.p3, 1.0 / 3.0), c.p0.lerp(c.p3, 2.0 / 3.0), c.p3) } else { c }
}

/// Non-linear path mapping (shared with live envelopes in `vectorcraft-doc`), and the split of
/// segments into pieces within an anchor budget.
pub use vectorcraft_doc::live::{fit_piece, map_nonlinear_within, pieces};

/// About how many lines the renderer flattens `path` into at 1 px/pt: one for a straight
/// segment, and for a curve the count Wang's formula gives at vello's tolerance of 0.25 px,
/// `sqrt(0.75 × d / 0.25)` for the largest second difference `d` of its control points.
pub fn line_estimate(path: &PathData) -> usize {
    let curve = |c: CubicBez| {
        let d =
            (c.p0.to_vec2() - c.p1.to_vec2() * 2.0 + c.p2.to_vec2()).hypot().max((c.p1.to_vec2() - c.p2.to_vec2() * 2.0 + c.p3.to_vec2()).hypot());
        // A NaN casts to 0 and an infinity to the largest count.
        ((3.0 * d).sqrt().ceil() as usize).max(1)
    };
    path.subpaths
        .iter()
        .flat_map(|sp| (0..sp.segment_count()).map(move |i| if sp.segment_is_line(i) { 1 } else { curve(sp.segment(i)) }))
        .fold(0, usize::saturating_add)
}

/// Smooth subpath through `pts` (Catmull-Rom tangents, see [`turn_tangent`] for sharp turns).
pub fn catmull_rom(pts: &[Point], closed: bool, tension: f64) -> SubPath {
    let n = pts.len();
    if n < 3 {
        return SubPath::polyline(pts, closed);
    }
    let anchors = (0..n)
        .map(|i| {
            let prev = if i == 0 { if closed { pts[n - 1] } else { pts[0] } } else { pts[i - 1] };
            let next = if i == n - 1 { if closed { pts[0] } else { pts[n - 1] } } else { pts[i + 1] };
            let t: Vec2 = turn_tangent(prev, pts[i], next) * (tension / 6.0);
            Anchor::with_handles(pts[i], pts[i] - t, pts[i] + t)
        })
        .collect();
    SubPath::new(anchors, closed)
}

/// The Catmull-Rom tangent `next − prev` at `p`. Where the path turns back sharply, `next`
/// nearly meets `prev` and that tangent shrinks to nothing: the anchor would be a cusp, which
/// PDF viewers draw with a miter spike (#1643). So it is kept at least half as long as the
/// longer chord, and there it follows the bisector of the two chords (the perpendicular of the
/// incoming one when the path reverses exactly): the turn becomes a round hairpin. Turns of
/// less than about 150° keep the plain Catmull-Rom tangent.
fn turn_tangent(prev: Point, p: Point, next: Point) -> Vec2 {
    let t = next - prev;
    let (a, b) = (p - prev, next - p);
    let floor = 0.5 * a.hypot().max(b.hypot());
    if t.hypot() >= floor {
        return t;
    }
    let unit = |v: Vec2| {
        let l = v.hypot();
        if l > 1e-12 { v / l } else { Vec2::ZERO }
    };
    let (u, v) = (unit(a), unit(b));
    let mut d = unit(u + v);
    if d == Vec2::ZERO {
        d = Vec2::new(-u.y, u.x);
    }
    if d == Vec2::ZERO { t } else { d * floor }
}

/// Unit normal (left of travel direction in y-down coordinates) of cubic `c` at `t`.
pub fn normal_at(c: &CubicBez, t: f64) -> Vec2 {
    let eps = 1e-4;
    let a = c.eval((t - eps).max(0.0));
    let b = c.eval((t + eps).min(1.0));
    let mut d = b - a;
    if d.hypot() < 1e-12 {
        d = c.p3 - c.p0;
    }
    let len = d.hypot();
    if len < 1e-12 { Vec2::ZERO } else { Vec2::new(d.y / len, -d.x / len) }
}
