//! Gouraud lighting represented by an ordinary vector linear gradient on each triangle.

use vectorcraft_geom::{Point, Vec2};

/// Document-space gradient placement and the brightness at its two ends. Clipping a
/// face must keep this placement unchanged, so newly cut edges retain their lighting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadeGradient {
    pub start: Point,
    pub end: Point,
    pub brightness: [f64; 2],
}

impl ShadeGradient {
    pub(crate) fn triangle(points: &[Point], values: &[f64]) -> Option<Self> {
        let [p0, p1, p2] = points else { return None };
        let [b0, b1, b2] = values else { return None };
        let u = *p1 - *p0;
        let v = *p2 - *p0;
        let det = u.cross(v);
        if det.abs() < 1e-10 {
            return None;
        }
        // Solve the brightness plane in screen space. Its perpendicular is the gradient
        // vector, so all three corner values and shared triangle edges agree exactly.
        let g = Vec2::new(((b1 - b0) * v.y - (b2 - b0) * u.y) / det, (u.x * (b2 - b0) - v.x * (b1 - b0)) / det);
        let lo = b0.min(*b1).min(*b2);
        let hi = b0.max(*b1).max(*b2);
        if hi - lo < 1e-10 || g.hypot2() < 1e-30 {
            return None;
        }
        let start = *p0 + g * ((lo - b0) / g.hypot2());
        let end = *p0 + g * ((hi - b0) / g.hypot2());
        [start.x, start.y, end.x, end.y].iter().all(|n| n.is_finite()).then_some(Self { start, end, brightness: [lo, hi] })
    }

    /// Brightness at a projected point, including a point introduced by visibility clipping.
    pub fn at(self, p: Point) -> f64 {
        let v = self.end - self.start;
        let t = if v.hypot2() > 0.0 { ((p - self.start).dot(v) / v.hypot2()).clamp(0.0, 1.0) } else { 0.0 };
        self.brightness[0] + (self.brightness[1] - self.brightness[0]) * t
    }
}

/// A directional-light ramp before channel clipping. Keep ambient light neutral and add
/// stops where individual RGB channels saturate, so colored highlights retain their hue.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightGradient {
    pub diffuse: ShadeGradient,
    pub ambient: f64,
    pub color: [f64; 3],
}

impl LightGradient {
    pub fn at(self, p: Point) -> [f64; 3] {
        self.factors(self.diffuse.at(p))
    }

    fn factors(self, direct: f64) -> [f64; 3] {
        self.color.map(|c| (self.ambient + direct * c).clamp(0.0, 1.0))
    }

    /// At most five ordinary linear-gradient stops, including channel clipping points.
    pub fn stops(self) -> Vec<(f32, [f64; 3])> {
        let [lo, hi] = self.diffuse.brightness;
        let span = hi - lo;
        let mut positions = vec![0.0f64, 1.0];
        for c in self.color {
            if c > 0.0 && span > 0.0 {
                let t = ((1.0 - self.ambient) / c - lo) / span;
                if t > 0.0 && t < 1.0 {
                    positions.push(t);
                }
            }
        }
        positions.sort_by(f64::total_cmp);
        positions.dedup_by(|a, b| (*a - *b).abs() < 1e-10);
        positions.into_iter().map(|t| (t as f32, self.factors(lo + span * t))).collect()
    }

    pub(crate) fn is_finite(self) -> bool {
        [
            self.diffuse.start.x,
            self.diffuse.start.y,
            self.diffuse.end.x,
            self.diffuse.end.y,
            self.diffuse.brightness[0],
            self.diffuse.brightness[1],
            self.ambient,
        ]
        .into_iter()
        .chain(self.color)
        .all(f64::is_finite)
    }
}

/// Independent affine RGB channels. Different light directions need different gradient axes;
/// interpolating a single luminance ramp would erase their color across a smooth surface.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RgbGradient {
    pub channels: [ShadeGradient; 3],
}

impl RgbGradient {
    pub(crate) fn triangle(points: &[Point], values: &[[f64; 3]]) -> Option<Self> {
        let [p, _, _] = points else { return None };
        let [first, _, _] = values else { return None };
        Some(Self {
            channels: std::array::from_fn(|channel| {
                let values: Vec<_> = values.iter().map(|v| v[channel]).collect();
                ShadeGradient::triangle(points, &values).unwrap_or(ShadeGradient { start: *p, end: *p, brightness: [first[channel]; 2] })
            }),
        })
    }

    pub fn at(self, p: Point) -> [f64; 3] {
        self.channels.map(|g| g.at(p).clamp(0.0, 1.0))
    }

    pub(crate) fn is_finite(self) -> bool {
        self.channels.iter().all(|g| [g.start.x, g.start.y, g.end.x, g.end.y, g.brightness[0], g.brightness[1]].into_iter().all(f64::is_finite))
    }
}
