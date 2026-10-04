//! Gradients: definitions (swatch-able) and their placement on an object.

use kurbo::{Affine, Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    /// Freeform gradients (points/lines) — rendered via a mesh approximation.
    Freeform,
}

impl GradientKind {
    /// Parse a kind name (`linear`, `radial`, `freeform`; any case).
    pub fn parse(s: &str) -> Option<Self> {
        [GradientKind::Linear, GradientKind::Radial, GradientKind::Freeform].into_iter().find(|k| k.label().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            GradientKind::Linear => "Linear",
            GradientKind::Radial => "Radial",
            GradientKind::Freeform => "Freeform",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// 0..=1 along the gradient.
    pub offset: f32,
    pub color: Color,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Midpoint to the next stop, 0.13..=0.87 (Illustrator's diamond), default 0.5.
    #[serde(default = "half")]
    pub midpoint: f32,
}

fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
}

impl Default for Gradient {
    /// Illustrator's default "White, Black" gradient.
    fn default() -> Self {
        Self {
            kind: GradientKind::Linear,
            stops: vec![
                GradientStop { offset: 0.0, color: Color::WHITE, opacity: 1.0, midpoint: 0.5 },
                GradientStop { offset: 1.0, color: Color::BLACK, opacity: 1.0, midpoint: 0.5 },
            ],
        }
    }
}

impl Gradient {
    /// Colour and opacity at `t` (honours midpoints).
    pub fn sample(&self, t: f32) -> (Color, f32) {
        let stops = &self.stops;
        if stops.is_empty() {
            return (Color::BLACK, 1.0);
        }
        if t <= stops[0].offset {
            return (stops[0].color, stops[0].opacity);
        }
        for w in stops.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            if t <= b.offset {
                let span = (b.offset - a.offset).max(1e-6);
                let u = (t - a.offset) / span;
                // Map through the midpoint: u=mid → 0.5.
                let m = a.midpoint.clamp(0.01, 0.99);
                let v = if u < m { 0.5 * u / m } else { 0.5 + 0.5 * (u - m) / (1.0 - m) };
                return (a.color.lerp(&b.color, v), a.opacity + (b.opacity - a.opacity) * v);
            }
        }
        stops.last().map_or((Color::BLACK, 1.0), |l| (l.color, l.opacity))
    }
    /// Stops expanded so that midpoints are represented as explicit stops (for renderers without midpoints).
    pub fn expanded_stops(&self) -> Vec<(f32, Color, f32)> {
        let mut out = Vec::new();
        for (i, s) in self.stops.iter().enumerate() {
            out.push((s.offset, s.color, s.opacity));
            if let Some(n) = self.stops.get(i + 1)
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                let t = s.offset + (n.offset - s.offset) * s.midpoint;
                let (c, o) = self.sample(t);
                out.push((t, c, o));
            }
        }
        out
    }
    pub fn reverse(&mut self) {
        self.stops.reverse();
        for s in &mut self.stops {
            s.offset = 1.0 - s.offset;
        }
    }
    pub fn sort(&mut self) {
        self.stops.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    }
}

// ---------- stop editing (the Gradient panel, the annotator and agents share these) ----------

/// Fewest stops a gradient keeps: deleting a stop never goes below this.
pub const MIN_STOPS: usize = 2;

/// Put `s` among `stops` (sorted by offset, after stops at the same offset). Returns its index.
fn place_stop(stops: &mut Vec<GradientStop>, s: GradientStop) -> usize {
    let i = stops.iter().position(|o| o.offset > s.offset).unwrap_or(stops.len());
    stops.insert(i, s);
    i
}

/// Insert a stop at `offset`, coloured by sampling the gradient there. Returns the new stops and
/// the new stop's index.
pub fn insert_stop(g: &Gradient, offset: f32) -> (Vec<GradientStop>, usize) {
    let offset = offset.clamp(0.0, 1.0);
    let (color, opacity) = g.sample(offset);
    let mut stops = g.stops.clone();
    let i = place_stop(&mut stops, GradientStop { offset, color, opacity, midpoint: 0.5 });
    (stops, i)
}

/// Remove stop `i`; `None` when that would leave fewer than [`MIN_STOPS`].
pub fn remove_stop(stops: &[GradientStop], i: usize) -> Option<Vec<GradientStop>> {
    if stops.len() <= MIN_STOPS || i >= stops.len() {
        return None;
    }
    let mut v = stops.to_vec();
    v.remove(i);
    Some(v)
}

/// Move stop `i` to `offset`, keeping the list sorted. Returns the stops and the stop's new index.
pub fn move_stop(stops: &[GradientStop], i: usize, offset: f32) -> (Vec<GradientStop>, usize) {
    let mut v = stops.to_vec();
    if i >= v.len() {
        return (v, i);
    }
    let mut s = v.remove(i);
    s.offset = offset.clamp(0.0, 1.0);
    let ni = place_stop(&mut v, s);
    (v, ni)
}

/// Add a copy of stop `i` at `offset` (Alt-drag). Returns the stops and the copy's index.
pub fn duplicate_stop(stops: &[GradientStop], i: usize, offset: f32) -> (Vec<GradientStop>, usize) {
    let mut v = stops.to_vec();
    let Some(&s) = v.get(i) else { return (v, i) };
    let ni = place_stop(&mut v, GradientStop { offset: offset.clamp(0.0, 1.0), ..s });
    (v, ni)
}

/// Swap the colours of stops `a` and `b` (Alt-dropping one stop on another); offsets, opacities
/// and midpoints stay. Out-of-range indices leave the stops unchanged.
pub fn swap_stop_colors(stops: &[GradientStop], a: usize, b: usize) -> Vec<GradientStop> {
    let mut v = stops.to_vec();
    if a < v.len() && b < v.len() {
        let c = v[a].color;
        v[a].color = v[b].color;
        v[b].color = c;
    }
    v
}

/// Set the midpoint between stop `i` and `i + 1` (clamped to the diamond's 13–87 %).
pub fn set_midpoint(stops: &[GradientStop], i: usize, m: f32) -> Vec<GradientStop> {
    let mut v = stops.to_vec();
    if let Some(s) = v.get_mut(i) {
        s.midpoint = m.clamp(0.13, 0.87);
    }
    v
}

/// Absolute position (0..1) of the midpoint diamond after stop `i`.
pub fn midpoint_pos(stops: &[GradientStop], i: usize) -> Option<f32> {
    let (a, b) = (stops.get(i)?, stops.get(i + 1)?);
    Some(a.offset + (b.offset - a.offset) * a.midpoint)
}

/// Inverse of [`midpoint_pos`]: the relative midpoint for an absolute position.
pub fn midpoint_from_pos(stops: &[GradientStop], i: usize, pos: f32) -> Option<f32> {
    let (a, b) = (stops.get(i)?, stops.get(i + 1)?);
    let span = (b.offset - a.offset).max(1e-6);
    Some(((pos - a.offset) / span).clamp(0.13, 0.87))
}

/// Where the gradient sits on an object, in document coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientGeom {
    pub start: Point,
    pub end: Point,
    /// Radial aspect ratio (height / width), 1 = circle.
    #[serde(default = "one64")]
    pub aspect: f64,
}

fn one64() -> f64 {
    1.0
}

impl GradientGeom {
    /// Default placement for a bounding box: linear spans the box through the centre at `angle_deg`,
    /// radial is centred with radius = half the larger dimension and its vector at `angle_deg`.
    pub fn fit(kind: GradientKind, b: Rect, angle_deg: f64) -> Self {
        let c = b.center();
        let a = angle_deg.to_radians();
        let d = Vec2::new(a.cos(), -a.sin());
        match kind {
            GradientKind::Radial => Self { start: c, end: c + d * (b.width().max(b.height()) / 2.0), aspect: 1.0 },
            _ => {
                // Project the box corners onto the direction to cover the whole box.
                let half = (b.width() * d.x.abs() + b.height() * d.y.abs()) / 2.0;
                Self { start: c - d * half, end: c + d * half, aspect: 1.0 }
            }
        }
    }

    /// Map the placement through `a` so the gradient follows its object exactly.
    ///
    /// Linear (and freeform) gradients keep their isolines: the new vector is the normal to the
    /// mapped isolines, reaching the line the old end maps onto. Radial gradients map their ellipse
    /// (the radius along the vector, `aspect` × the radius across it); its principal axes give the
    /// new end (on the axis nearest the mapped vector) and aspect.
    pub fn transform(&mut self, a: Affine, kind: GradientKind) {
        let [m0, m1, m2, m3, _, _] = a.as_coeffs();
        let lin = |v: Vec2| Vec2::new(m0 * v.x + m2 * v.y, m1 * v.x + m3 * v.y);
        let perp = |v: Vec2| Vec2::new(-v.y, v.x);
        let start = a * self.start;
        let u = self.end - self.start;
        if u.hypot2() < 1e-18 {
            self.start = start;
            self.end = start;
            return;
        }
        if kind != GradientKind::Radial {
            let far = a * self.end;
            let nrm = perp(lin(perp(u)));
            self.start = start;
            self.end = if nrm.hypot2() > 1e-18 { start + nrm * ((far - start).dot(nrm) / nrm.hypot2()) } else { far };
            return;
        }
        let (lu, lw) = (lin(u), lin(perp(u) * self.aspect));
        // Eigen-decomposition of M·Mᵀ with M = [lu lw]: the mapped ellipse's axes and squared radii.
        let p = lu.x * lu.x + lw.x * lw.x;
        let q = lu.y * lu.y + lw.y * lw.y;
        let r = lu.x * lu.y + lw.x * lw.y;
        let disc = ((p - q).powi(2) + 4.0 * r * r).sqrt();
        let s1 = ((p + q + disc) / 2.0).max(0.0).sqrt();
        let s2 = ((p + q - disc) / 2.0).max(0.0).sqrt();
        let (axis, len, other) = if disc <= 1e-12 * (p + q) {
            // A circle: every direction is an axis, so keep the mapped vector's.
            (lu / lu.hypot().max(1e-300), s1, s2)
        } else {
            let th = 0.5 * (2.0 * r).atan2(p - q);
            let e1 = Vec2::new(th.cos(), th.sin());
            let e2 = perp(e1);
            if e1.dot(lu).abs() >= e2.dot(lu).abs() { (e1, s1, s2) } else { (e2, s2, s1) }
        };
        let axis = if axis.dot(lu) < 0.0 { -axis } else { axis };
        self.start = start;
        self.end = start + axis * len;
        self.aspect = if len > 1e-12 { other / len } else { 1.0 };
    }

    /// Move the placement from box `from` to box `to`: the start and end keep their position
    /// relative to the box (along a side `from` has no extent on, their offset from its centre).
    /// The aspect ratio is kept.
    pub fn rebase(&mut self, from: Rect, to: Rect) {
        let axis = |v: f64, a0: f64, a1: f64, b0: f64, b1: f64| {
            if (a1 - a0).abs() > 1e-12 { b0 + (v - a0) / (a1 - a0) * (b1 - b0) } else { v - (a0 + a1) / 2.0 + (b0 + b1) / 2.0 }
        };
        let map = |p: Point| Point::new(axis(p.x, from.x0, from.x1, to.x0, to.x1), axis(p.y, from.y0, from.y1, to.y0, to.y1));
        self.start = map(self.start);
        self.end = map(self.end);
    }

    /// The gradient parameter (0 at the start, 1 at the end) at document point `p`: the projection
    /// onto the vector for linear gradients, the elliptical radius (honouring `aspect`) for radial.
    pub fn param_at(&self, kind: GradientKind, p: Point) -> f64 {
        let u = self.end - self.start;
        let l2 = u.hypot2();
        if l2 < 1e-18 {
            return 0.0;
        }
        let d = p - self.start;
        let along = d.dot(u) / l2;
        match kind {
            GradientKind::Radial => along.hypot(d.cross(u) / (l2 * self.aspect.max(1e-9))),
            _ => along,
        }
    }
    pub fn angle_deg(&self) -> f64 {
        let v = self.end - self.start;
        (-v.y).atan2(v.x).to_degrees()
    }
    pub fn length(&self) -> f64 {
        (self.end - self.start).hypot()
    }
}

/// A gradient applied to a fill or stroke.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientPaint {
    pub gradient: Gradient,
    /// None = fit to the object's bounds at `angle` each render (fresh gradients behave like this).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub geom: Option<GradientGeom>,
    #[serde(default)]
    pub angle: f64,
    /// Name of the gradient swatch, if linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatch: Option<String>,
}

impl GradientPaint {
    pub fn new(gradient: Gradient) -> Self {
        Self { gradient, geom: None, angle: 0.0, swatch: None }
    }
    pub fn resolve(&self, bounds: Rect) -> GradientGeom {
        self.geom.unwrap_or_else(|| GradientGeom::fit(self.gradient.kind, bounds, self.angle))
    }
    /// Fix an unplaced gradient (geom None) to its fit on `bounds`, so it can follow transforms
    /// that refitting wouldn't reproduce (rotation, shear, non-uniform scale, warps).
    pub fn pin(&mut self, bounds: Rect) {
        if self.geom.is_none() {
            self.geom = Some(self.resolve(bounds));
        }
    }
    /// Move a placed gradient from box `from` to box `to` (see [`GradientGeom::rebase`]).
    pub fn rebase(&mut self, from: Rect, to: Rect) {
        if let Some(g) = &mut self.geom {
            g.rebase(from, to);
        }
    }
    /// Map a placed gradient through `a` (see [`GradientGeom::transform`]).
    pub fn transform(&mut self, a: Affine) {
        if let Some(g) = &mut self.geom {
            g.transform(a, self.gradient.kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_endpoints_and_mid() {
        let g = Gradient::default();
        assert_eq!(g.sample(0.0).0.to_hex(), "#ffffff");
        assert_eq!(g.sample(1.0).0.to_hex(), "#000000");
        let mid = g.sample(0.5).0.to_rgb()[0];
        assert!((mid - 0.5).abs() < 1e-5);
    }

    #[test]
    fn midpoint_shifts() {
        let mut g = Gradient::default();
        g.stops[0].midpoint = 0.25;
        // At t = 0.25 we should be half way.
        assert!((g.sample(0.25).0.to_rgb()[0] - 0.5).abs() < 1e-5);
        assert_eq!(g.expanded_stops().len(), 3);
    }

    #[test]
    fn fit_linear_horizontal() {
        let g = GradientGeom::fit(GradientKind::Linear, Rect::new(0.0, 0.0, 100.0, 50.0), 0.0);
        assert_eq!(g.start, Point::new(0.0, 25.0));
        assert_eq!(g.end, Point::new(100.0, 25.0));
        assert!((g.angle_deg()).abs() < 1e-9);
        let v = GradientGeom::fit(GradientKind::Linear, Rect::new(0.0, 0.0, 100.0, 50.0), 90.0);
        assert!((v.angle_deg() - 90.0).abs() < 1e-9);
        assert!((v.length() - 50.0).abs() < 1e-9);
    }

    #[test]
    fn reverse_stops() {
        let mut g = Gradient::default();
        g.reverse();
        assert_eq!(g.stops[0].color.to_hex(), "#000000");
        assert_eq!(g.stops[0].offset, 0.0);
    }

    fn close(a: Point, b: Point) -> bool {
        a.distance(b) < 1e-9
    }

    fn linear(start: (f64, f64), end: (f64, f64)) -> GradientGeom {
        GradientGeom { start: start.into(), end: end.into(), aspect: 1.0 }
    }

    #[test]
    fn rotate_90_gives_vertical_vector() {
        let mut g = linear((0.0, 0.0), (100.0, 0.0));
        g.transform(Affine::rotate(std::f64::consts::FRAC_PI_2), GradientKind::Linear);
        assert!(close(g.start, Point::ZERO) && close(g.end, Point::new(0.0, 100.0)), "{g:?}");
        assert!((g.angle_deg() + 90.0).abs() < 1e-9);
    }

    #[test]
    fn reflect_swaps_start_and_end() {
        let mut g = linear((0.0, 25.0), (100.0, 25.0));
        // Reflect across the vertical axis through x = 50.
        let flip = Affine::translate((50.0, 0.0)) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate((-50.0, 0.0));
        g.transform(flip, GradientKind::Linear);
        assert!(close(g.start, Point::new(100.0, 25.0)) && close(g.end, Point::new(0.0, 25.0)), "{g:?}");
    }

    #[test]
    fn non_uniform_scale_turns_a_circle_into_an_ellipse() {
        let mut g = GradientGeom { start: Point::new(10.0, 10.0), end: Point::new(20.0, 10.0), aspect: 1.0 };
        g.transform(Affine::scale_non_uniform(2.0, 1.0), GradientKind::Radial);
        assert!(close(g.start, Point::new(20.0, 10.0)) && close(g.end, Point::new(40.0, 10.0)), "{g:?}");
        assert!((g.aspect - 0.5).abs() < 1e-9);
        // A vertical vector keeps its axis: the ellipse is twice as wide as the vector is long.
        let mut v = GradientGeom { start: Point::ZERO, end: Point::new(0.0, -10.0), aspect: 1.0 };
        v.transform(Affine::scale_non_uniform(2.0, 1.0), GradientKind::Radial);
        assert!(close(v.end, Point::new(0.0, -10.0)) && (v.aspect - 2.0).abs() < 1e-9, "{v:?}");
    }

    #[test]
    fn samples_follow_shear_and_rotation() {
        let shear = Affine::new([1.0, 0.3, 0.7, 1.2, 15.0, -4.0]) * Affine::rotate(0.4);
        for kind in [GradientKind::Linear, GradientKind::Radial] {
            let g = GradientGeom { start: Point::new(30.0, 40.0), end: Point::new(80.0, 55.0), aspect: 0.6 };
            let mut m = g;
            m.transform(shear, kind);
            for p in [Point::new(30.0, 40.0), Point::new(55.0, 70.0), Point::new(90.0, 10.0), Point::new(-20.0, 48.0)] {
                let (a, b) = (g.param_at(kind, p), m.param_at(kind, shear * p));
                assert!((a - b).abs() < 1e-9, "{kind:?} at {p:?}: {a} vs {b}");
            }
        }
    }

    #[test]
    fn radial_fit_uses_the_angle_and_param_honours_aspect() {
        let b = Rect::new(0.0, 0.0, 100.0, 60.0);
        let g = GradientGeom::fit(GradientKind::Radial, b, 90.0);
        assert!(close(g.start, Point::new(50.0, 30.0)) && close(g.end, Point::new(50.0, -20.0)), "end above the centre: {g:?}");
        let e = GradientGeom { start: Point::ZERO, end: Point::new(10.0, 0.0), aspect: 0.5 };
        assert!((e.param_at(GradientKind::Radial, Point::new(0.0, 5.0)) - 1.0).abs() < 1e-12);
        assert!((e.param_at(GradientKind::Radial, Point::new(5.0, 0.0)) - 0.5).abs() < 1e-12);
        assert!((e.param_at(GradientKind::Linear, Point::new(5.0, 7.0)) - 0.5).abs() < 1e-12);
    }

    fn g3() -> Gradient {
        let mut g = Gradient::default();
        g.stops.insert(1, GradientStop { offset: 0.5, color: Color::rgb(1.0, 0.0, 0.0), opacity: 1.0, midpoint: 0.5 });
        g
    }

    #[test]
    fn insert_samples_color_and_sorts() {
        let g = Gradient::default();
        let (stops, i) = insert_stop(&g, 0.25);
        assert_eq!(stops.len(), 3);
        assert_eq!(i, 1);
        let r = stops[1].color.to_rgb()[0];
        assert!((r - 0.75).abs() < 1e-4, "sampled {r}");
        let (stops, i) = insert_stop(&g, 2.0);
        assert_eq!(i, 2);
        assert_eq!(stops[2].offset, 1.0);
    }

    #[test]
    fn remove_keeps_two() {
        let g = g3();
        let v = remove_stop(&g.stops, 1).unwrap();
        assert_eq!(v.len(), 2);
        assert!(remove_stop(&v, 0).is_none());
        assert!(remove_stop(&g.stops, 9).is_none());
    }

    #[test]
    fn move_reorders_and_tracks_index() {
        let g = g3();
        let (v, i) = move_stop(&g.stops, 0, 0.8);
        assert_eq!(i, 1);
        assert_eq!(v[1].color.to_hex(), "#ffffff");
        assert!(v.windows(2).all(|w| w[0].offset <= w[1].offset));
        let (v, i) = move_stop(&g.stops, 1, -3.0);
        // Clamped to 0 and placed after the existing stop at 0.
        assert_eq!((i, v[i].offset, v[i].color.to_hex()), (1, 0.0, "#ff0000".to_string()));
    }

    #[test]
    fn duplicate_copies_the_stop_to_a_new_offset() {
        let g = g3();
        let (v, i) = duplicate_stop(&g.stops, 1, 0.9);
        assert_eq!((v.len(), i, v[i].offset, v[i].color.to_hex()), (4, 2, 0.9, "#ff0000".to_string()));
        assert_eq!(v[1], g.stops[1], "the original stays");
        assert_eq!(duplicate_stop(&g.stops, 7, 0.5).0, g.stops);
    }

    #[test]
    fn swap_exchanges_only_the_colours() {
        let mut g = g3();
        g.stops[0].opacity = 0.25;
        let v = swap_stop_colors(&g.stops, 0, 1);
        assert_eq!((v[0].color.to_hex(), v[1].color.to_hex()), ("#ff0000".to_string(), "#ffffff".to_string()));
        assert_eq!((v[0].offset, v[0].opacity, v[1].offset, v[1].opacity), (0.0, 0.25, 0.5, 1.0));
        assert_eq!(swap_stop_colors(&g.stops, 0, 9), g.stops);
    }

    #[test]
    fn midpoint_math() {
        let g = g3();
        assert_eq!(midpoint_pos(&g.stops, 0), Some(0.25));
        assert_eq!(midpoint_pos(&g.stops, 2), None);
        let m = midpoint_from_pos(&g.stops, 1, 0.6).unwrap();
        assert!((m - 0.2).abs() < 1e-5);
        assert_eq!(midpoint_from_pos(&g.stops, 1, 0.51).unwrap(), 0.13);
        let v = set_midpoint(&g.stops, 0, 0.99);
        assert_eq!(v[0].midpoint, 0.87);
    }

    #[test]
    fn rebase_keeps_relative_positions() {
        let mut g = GradientGeom { start: Point::new(110.0, 150.0), end: Point::new(190.0, 120.0), aspect: 0.5 };
        g.rebase(Rect::new(100.0, 100.0, 200.0, 200.0), Rect::new(500.0, 0.0, 700.0, 50.0));
        assert!(close(g.start, Point::new(520.0, 25.0)) && close(g.end, Point::new(680.0, 10.0)) && g.aspect == 0.5, "{g:?}");
        // A box without height (a horizontal line) keeps the offset from its centre line.
        let mut h = GradientGeom { start: Point::new(0.0, 12.0), end: Point::new(10.0, 10.0), aspect: 1.0 };
        h.rebase(Rect::new(0.0, 10.0, 10.0, 10.0), Rect::new(0.0, 0.0, 20.0, 40.0));
        assert!(close(h.start, Point::new(0.0, 22.0)) && close(h.end, Point::new(20.0, 20.0)), "{h:?}");
        // Unplaced paints have nothing to move.
        let mut p = GradientPaint::new(Gradient::default());
        p.rebase(Rect::new(0.0, 0.0, 1.0, 1.0), Rect::new(5.0, 5.0, 9.0, 9.0));
        assert_eq!(p.geom, None);
    }

    #[test]
    fn pin_fixes_the_fit_once() {
        let mut p = GradientPaint::new(Gradient::default());
        p.angle = 90.0;
        let b = Rect::new(0.0, 0.0, 100.0, 50.0);
        p.pin(b);
        assert_eq!(p.geom, Some(GradientGeom::fit(GradientKind::Linear, b, 90.0)));
        p.pin(Rect::new(0.0, 0.0, 1.0, 1.0));
        assert_eq!(p.geom, Some(GradientGeom::fit(GradientKind::Linear, b, 90.0)));
    }
}
