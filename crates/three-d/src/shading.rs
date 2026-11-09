//! Gouraud lighting represented by an ordinary vector linear gradient on each triangle, and the
//! flat surface shading of the classic 3D effects.

use vectorcraft_geom::{Point, Vec2};

/// The surface of a classic 3D effect: how its faces are drawn and lit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ClassicSurface {
    /// Only the outline of every face, hidden ones included; the faces are transparent.
    Wireframe,
    /// Every face in the art's own paint, unlit.
    NoShading,
    /// Soft light: each face as bright as it faces the light.
    Diffuse,
    /// Diffuse light plus a glossy highlight where a face mirrors the light toward the viewer.
    #[default]
    Plastic,
}

impl ClassicSurface {
    pub const ALL: [ClassicSurface; 4] = [Self::Wireframe, Self::NoShading, Self::Diffuse, Self::Plastic];

    pub fn id(self) -> &'static str {
        match self {
            Self::Wireframe => "wireframe",
            Self::NoShading => "none",
            Self::Diffuse => "diffuse",
            Self::Plastic => "plastic",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id() == s)
    }
}

/// The surface settings of a classic 3D effect. Shares are 0..1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClassicShading {
    pub surface: ClassicSurface,
    /// Light every face gets, whichever way it faces.
    pub ambient: f64,
    /// How bright the highlight is at its center (Plastic only).
    pub highlight_intensity: f64,
    /// How wide the highlight spreads: 1 broad, 0 a pinpoint (Plastic only).
    pub highlight_size: f64,
    /// How many levels the shading and the highlight are rounded to, 1..256.
    pub blend_steps: u32,
}

impl Default for ClassicShading {
    fn default() -> Self {
        Self { surface: ClassicSurface::Plastic, ambient: 0.5, highlight_intensity: 0.6, highlight_size: 0.9, blend_steps: 25 }
    }
}

impl ClassicShading {
    /// The shine exponent of the highlight: 2 at the largest size, 256 at the smallest.
    fn shininess(self) -> f64 {
        2f64.powf(1.0 + 7.0 * (1.0 - self.highlight_size.clamp(0.0, 1.0)))
    }

    /// `x` (0..1) rounded to the nearest of the blend steps' levels.
    fn stepped(self, x: f64) -> f64 {
        let steps = f64::from(self.blend_steps.clamp(1, 256));
        ((x.clamp(0.0, 1.0) * steps).round() / steps).clamp(0.0, 1.0)
    }

    /// The tone (0 in full shadow, 1 fully lit) and the highlight (0 none, 1 white) of a face with
    /// view-space unit normal `normal`, seen from the front and lit by `lights`: each a unit
    /// direction toward the light and its intensity (0..1). The lights add up. Wireframe and No
    /// Shading are unlit: tone 1, no highlight.
    pub fn tone(self, normal: [f64; 3], lights: &[([f64; 3], f64)]) -> (f64, f64) {
        if matches!(self.surface, ClassicSurface::Wireframe | ClassicSurface::NoShading) {
            return (1.0, 0.0);
        }
        let dot = |a: [f64; 3], b: [f64; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        let (mut tone, mut spec) = (self.ambient.clamp(0.0, 1.0), 0.0);
        for &(light, intensity) in lights {
            let intensity = if intensity.is_finite() { intensity.clamp(0.0, 1.0) } else { 0.0 };
            let lit = dot(normal, light).max(0.0);
            tone += intensity * lit;
            // The halfway direction between the light and the eye in front of the page.
            let half = [light[0], light[1], light[2] + 1.0];
            let length = dot(half, half).sqrt();
            if self.surface == ClassicSurface::Plastic && length > 1e-12 && lit > 0.0 {
                spec += intensity * (dot(normal, half) / length).max(0.0).powf(self.shininess()) * self.highlight_intensity.clamp(0.0, 1.0);
            }
        }
        if !tone.is_finite() || !spec.is_finite() {
            return (1.0, 0.0);
        }
        (self.stepped(tone), self.stepped(spec))
    }
}

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
