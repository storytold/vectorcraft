//! Gradients: definitions (swatch-able) and their placement on an object.

use std::borrow::Cow;

use kurbo::{Affine, Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::Color;
use crate::freeform::Freeform;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    /// Freeform gradients: colour points and lines (see [`crate::freeform`]).
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GradientStop {
    /// 0..=1 along the gradient.
    pub offset: f32,
    pub color: Color,
    #[serde(default = "one")]
    pub opacity: f32,
    /// Midpoint to the next stop, 0.13..=0.87 (Illustrator's diamond), default 0.5.
    #[serde(default = "half")]
    pub midpoint: f32,
    /// The global or spot swatch this stop's colour is linked to (edits to the swatch recolour it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatch: Option<String>,
    /// Tint of the linked swatch (0..1; 1 without a link), as on a solid paint.
    #[serde(default = "crate::full_tint", skip_serializing_if = "crate::is_full_tint")]
    pub tint: f32,
}

impl GradientStop {
    /// An opaque, unlinked stop of `color` at `offset` with a centred midpoint.
    pub fn new(offset: f32, color: Color) -> Self {
        Self { offset, color, opacity: 1.0, midpoint: 0.5, swatch: None, tint: 1.0 }
    }
    /// Give the stop `color`, linked to `link` (a global swatch and tint) or unlinked.
    pub fn set_color(&mut self, color: Color, link: Option<(String, f32)>) {
        self.color = color;
        (self.swatch, self.tint) = match link {
            Some((n, t)) => (Some(n), t),
            None => (None, 1.0),
        };
    }
}

fn one() -> f32 {
    1.0
}
fn half() -> f32 {
    0.5
}

/// How a gradient mixes the colours between two stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GradientInterpolation {
    /// Straight through display RGB (the classic look).
    #[default]
    Linear,
    /// Through OKLab, a perceptually uniform space: even steps of lightness and no muddy middle.
    Perceptual,
}

impl GradientInterpolation {
    /// Parse a name (`linear`/`standard`/`classic`, `perceptual`; any case).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "linear" | "standard" | "classic" | "rgb" => Some(Self::Linear),
            "perceptual" | "oklab" => Some(Self::Perceptual),
            _ => None,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Perceptual => "Perceptual",
        }
    }
    fn is_linear(&self) -> bool {
        *self == Self::Linear
    }
}

/// Stops a perceptual gradient adds between two of its own stops for renderers and file formats
/// that only interpolate in RGB (enough that the difference is below one 8-bit level).
pub const PERCEPTUAL_SUBSTEPS: usize = 12;

/// Where a stop of [`Gradient::expanded`] comes from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ExpandedStop {
    /// One of the gradient's own stops.
    Own,
    /// The colour at a stop's midpoint (the midpoint location, 0..1).
    Midpoint(f32),
    /// A colour added so an RGB renderer follows perceptual interpolation.
    Interpolated,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Gradient {
    pub kind: GradientKind,
    pub stops: Vec<GradientStop>,
    /// How colours mix between stops.
    #[serde(default, skip_serializing_if = "GradientInterpolation::is_linear")]
    pub interpolation: GradientInterpolation,
    /// Dither the gradient on screen and in raster output to hide banding.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dither: bool,
}

impl Default for Gradient {
    /// Illustrator's default "White, Black" gradient.
    fn default() -> Self {
        Self::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)])
    }
}

impl Gradient {
    /// A gradient of `kind` through `stops`, interpolated linearly, not dithered.
    pub fn new(kind: GradientKind, stops: Vec<GradientStop>) -> Self {
        Self { kind, stops, interpolation: GradientInterpolation::Linear, dither: false }
    }
    /// Colour and opacity at `t` (honours midpoints and the interpolation).
    pub fn sample(&self, t: f32) -> (Color, f32) {
        match self.interpolation {
            GradientInterpolation::Linear => self.sample_with(t, Color::lerp),
            GradientInterpolation::Perceptual => self.sample_with(t, crate::libraries::mix_perceptual),
        }
    }
    /// [`Gradient::sample`] with neighbouring stop colours mixed by `mix` (e.g. in their own colour
    /// model rather than display RGB).
    pub fn sample_with(&self, t: f32, mix: impl Fn(&Color, &Color, f32) -> Color) -> (Color, f32) {
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
                return (mix(&a.color, &b.color, v), a.opacity + (b.opacity - a.opacity) * v);
            }
        }
        match stops.last() {
            Some(l) => (l.color, l.opacity),
            None => (Color::BLACK, 1.0),
        }
    }
    /// Stops expanded so that midpoints (and perceptual interpolation) are represented as
    /// explicit stops, for renderers that interpolate linearly in RGB between stops.
    pub fn expanded_stops(&self) -> Vec<(f32, Color, f32)> {
        self.expanded().map(|(t, c, o, _)| (t, c, o)).collect()
    }
    /// [`Self::expanded_stops`], each stop with where it comes from (see [`ExpandedStop`]).
    pub fn expanded(&self) -> impl Iterator<Item = (f32, Color, f32, ExpandedStop)> + '_ {
        let perceptual = self.interpolation == GradientInterpolation::Perceptual;
        self.stops.iter().enumerate().flat_map(move |(i, s)| {
            let next = self.stops.get(i + 1);
            let at = |t: f32, role: ExpandedStop| {
                let (c, o) = self.sample(t);
                (t, c, o, role)
            };
            let mid = next.filter(|_| (s.midpoint - 0.5).abs() > 1e-3).map(|n| s.offset + (n.offset - s.offset) * s.midpoint);
            let mut extra: Vec<(f32, Color, f32, ExpandedStop)> = Vec::new();
            if let Some(n) = next.filter(|n| perceptual && n.offset - s.offset > 1e-4) {
                let span = n.offset - s.offset;
                for k in 1..PERCEPTUAL_SUBSTEPS {
                    let t = s.offset + span * k as f32 / PERCEPTUAL_SUBSTEPS as f32;
                    if mid.is_some_and(|m| (m - t).abs() < span / (2.0 * PERCEPTUAL_SUBSTEPS as f32)) {
                        continue;
                    }
                    extra.push(at(t, ExpandedStop::Interpolated));
                }
            }
            if let Some(m) = mid {
                extra.push(at(m, ExpandedStop::Midpoint(s.midpoint)));
            }
            extra.sort_by(|a, b| a.0.total_cmp(&b.0));
            std::iter::once((s.offset, s.color, s.opacity, ExpandedStop::Own)).chain(extra)
        })
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
    /// The part of the gradient from `from` to `to` (0..1, either way round) as a linear gradient
    /// of its own running 0 → 1: the same colours (midpoints become stops, as in
    /// [`Self::expanded_stops`]). One colour throughout when `from == to`.
    pub fn span(&self, from: f32, to: f32) -> Gradient {
        if to < from {
            let mut g = self.span(to, from);
            g.reverse();
            return g;
        }
        let stop = |offset: f32, t: f32| {
            let (color, opacity) = self.sample(t);
            GradientStop { opacity, ..GradientStop::new(offset, color) }
        };
        let d = to - from;
        let mut stops = vec![stop(0.0, from)];
        if d > 1e-6 {
            stops.extend(
                self.expanded_stops()
                    .into_iter()
                    .filter(|(t, ..)| *t > from && *t < to)
                    .map(|(t, color, opacity)| GradientStop { opacity, ..GradientStop::new((t - from) / d, color) }),
            );
        }
        stops.push(stop(1.0, to));
        Gradient { dither: self.dither, ..Gradient::new(GradientKind::Linear, stops) }
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
    let i = place_stop(&mut stops, GradientStop { opacity, ..GradientStop::new(offset, color) });
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
    let Some(s) = v.get(i).cloned() else { return (v, i) };
    let ni = place_stop(&mut v, GradientStop { offset: offset.clamp(0.0, 1.0), ..s });
    (v, ni)
}

/// Swap the colours of stops `a` and `b` (Alt-dropping one stop on another), with their swatch
/// links and tints; offsets, opacities and midpoints stay. Out-of-range indices leave the stops
/// unchanged.
pub fn swap_stop_colors(stops: &[GradientStop], a: usize, b: usize) -> Vec<GradientStop> {
    let mut v = stops.to_vec();
    if a < v.len() && b < v.len() && a != b {
        let (lo, hi) = v.split_at_mut(a.max(b));
        let (x, y) = (&mut lo[a.min(b)], &mut hi[0]);
        std::mem::swap(&mut x.color, &mut y.color);
        std::mem::swap(&mut x.swatch, &mut y.swatch);
        std::mem::swap(&mut x.tint, &mut y.tint);
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
    /// A radial gradient's focal point, where its first stop sits (an off-centre radial), in the
    /// same space; inside the extent ellipse. None: the centre (`start`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focal: Option<Point>,
}

/// How far out a focal point may sit, as a fraction of the extent ellipse.
const FOCAL_MAX: f64 = 0.99;

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
            GradientKind::Radial => Self { start: c, end: c + d * (b.width().max(b.height()) / 2.0), aspect: 1.0, focal: None },
            _ => {
                // Project the box corners onto the direction to cover the whole box.
                let half = (b.width() * d.x.abs() + b.height() * d.y.abs()) / 2.0;
                Self { start: c - d * half, end: c + d * half, aspect: 1.0, focal: None }
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
        // The focal point is a point like any other.
        self.focal = self.focal.map(|f| a * f);
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
        self.start = rebase_point(self.start, from, to);
        self.end = rebase_point(self.end, from, to);
        self.focal = self.focal.map(|f| rebase_point(f, from, to));
    }

    /// The gradient parameter (0 at the start, 1 at the end) at document point `p`: the projection
    /// onto the vector for linear gradients, the elliptical radius (honouring `aspect`) for radial,
    /// measured from the focal point (each level is the ellipse scaled about the line from the
    /// focal point to the centre).
    pub fn param_at(&self, kind: GradientKind, p: Point) -> f64 {
        let u = self.end - self.start;
        if u.hypot2() < 1e-18 {
            return 0.0;
        }
        if kind != GradientKind::Radial {
            return (p - self.start).dot(u) / u.hypot2();
        }
        let Some(to_unit) = self.unit_frame().map(|m| m.inverse()) else { return 0.0 };
        let q = (to_unit * p).to_vec2();
        let Some(f) = self.focal.map(|f| (to_unit * f).to_vec2()) else { return q.hypot() };
        // The level t through q: |q - f(1 - t)| = t, the root at t >= 0 (|f| < 1).
        let d = q - f;
        let a = f.hypot2() - 1.0;
        if a.abs() < 1e-12 {
            return q.hypot();
        }
        let b = d.dot(f);
        (-b - (b * b - a * d.hypot2()).max(0.0).sqrt()) / a
    }

    /// The map from the unit circle onto a radial's extent ellipse: (1, 0) to the end, (0, 1) to
    /// `aspect` × the radius across the vector. None when the ellipse is flat (no inverse).
    pub fn unit_frame(&self) -> Option<Affine> {
        let u = self.end - self.start;
        (u.hypot2() * self.aspect.abs() > 1e-18).then(|| Affine::new([u.x, u.y, -u.y * self.aspect, u.x * self.aspect, self.start.x, self.start.y]))
    }

    /// The map that squashes a radial gradient's circle (centre `start`, radius the vector's
    /// length) into its extent ellipse: by `aspect` across the vector, about the centre.
    pub fn radial_squash(&self) -> Affine {
        let angle = (self.end - self.start).atan2();
        let c = self.start.to_vec2();
        Affine::translate(c)
            * Affine::rotate(angle)
            * Affine::scale_non_uniform(1.0, self.aspect.max(1e-3))
            * Affine::rotate(-angle)
            * Affine::translate(-c)
    }

    /// Where a radial gradient's first stop sits: its focal point, else its centre.
    pub fn focal_point(&self) -> Point {
        self.focal.unwrap_or(self.start)
    }

    /// Set the focal point, pulled inside the extent ellipse; one on the centre (or a gradient
    /// with no extent) has none.
    pub fn set_focal(&mut self, f: Option<Point>) {
        self.focal = f.and_then(|f| {
            let m = self.unit_frame()?;
            let q = (m.inverse() * f).to_vec2();
            let r = q.hypot();
            (r > 1e-6).then(|| if r > FOCAL_MAX { m * (q * (FOCAL_MAX / r)).to_point() } else { f })
        });
    }

    /// After a change of the vector or aspect from `before`, put the focal point back where it
    /// was relative to the extent ellipse.
    pub fn keep_focal_from(&mut self, before: &GradientGeom) {
        let rel = before.focal.zip(before.unit_frame()).map(|(f, m)| m.inverse() * f);
        self.focal = None;
        if let (Some(q), Some(m)) = (rel, self.unit_frame()) {
            self.set_focal(Some(m * q));
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

/// `p` moved from box `from` to box `to`, keeping its position relative to the box (along a side
/// `from` has no extent on, its offset from the centre).
fn rebase_point(p: Point, from: Rect, to: Rect) -> Point {
    let axis = |v: f64, a0: f64, a1: f64, b0: f64, b1: f64| {
        if (a1 - a0).abs() > 1e-12 { b0 + (v - a0) / (a1 - a0) * (b1 - b0) } else { v - (a0 + a1) / 2.0 + (b0 + b1) / 2.0 }
    };
    Point::new(axis(p.x, from.x0, from.x1, to.x0, to.x1), axis(p.y, from.y0, from.y1, to.y0, to.y1))
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
    /// A freeform gradient's points, in the same space as `geom` (None: placed automatically on
    /// the object's box each render, coloured along the stops).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freeform: Option<Freeform>,
}

impl GradientPaint {
    pub fn new(gradient: Gradient) -> Self {
        Self { gradient, geom: None, angle: 0.0, swatch: None, freeform: None }
    }
    /// The freeform gradient drawn on box `bounds`: its own points, else the automatic ones
    /// (fitted to the box: [`Freeform::auto`] without a shape to keep them inside).
    pub fn freeform_on(&self, bounds: Rect) -> Cow<'_, Freeform> {
        match &self.freeform {
            Some(f) if !f.points.is_empty() => Cow::Borrowed(f),
            f => Cow::Owned(Freeform {
                mode: f.as_ref().map_or_else(Default::default, |f| f.mode),
                ..Freeform::auto(bounds, &self.gradient, &|_| true)
            }),
        }
    }
    /// Does it carry its own freeform points (rather than placing them on each render)?
    pub fn has_freeform_points(&self) -> bool {
        self.freeform.as_ref().is_some_and(|f| !f.points.is_empty())
    }
    /// Give a freeform gradient without points its automatic ones on `bounds`, kept `inside` the
    /// shape (the Draw mode stays).
    pub fn seed_freeform(&mut self, bounds: Rect, inside: &dyn Fn(Point) -> bool) {
        if self.gradient.kind == GradientKind::Freeform && !self.has_freeform_points() {
            let mode = self.freeform.as_ref().map_or_else(Default::default, |f| f.mode);
            self.set_freeform(Freeform { mode, ..Freeform::auto(bounds, &self.gradient, inside) });
        }
    }
    /// Set the freeform points, with the stops following their colours (what swatch chips,
    /// exports without freeform shading and re-seeding on other art show).
    pub fn set_freeform(&mut self, f: Freeform) {
        self.gradient.stops = f.stops();
        self.freeform = Some(f);
    }
    pub fn resolve(&self, bounds: Rect) -> GradientGeom {
        self.geom.unwrap_or_else(|| GradientGeom::fit(self.gradient.kind, bounds, self.angle))
    }
    /// Is the gradient placed in its paint's space (its vector, and a freeform gradient's points),
    /// rather than fitted to the object's box on each render?
    pub fn is_placed(&self) -> bool {
        self.geom.is_some() && (self.gradient.kind != GradientKind::Freeform || self.has_freeform_points())
    }
    /// Fix an unplaced gradient (geom None) to its fit on `bounds`, so it can follow transforms
    /// that refitting wouldn't reproduce (rotation, shear, non-uniform scale, warps).
    /// Freeform gradients get their automatic points.
    pub fn pin(&mut self, bounds: Rect) {
        if self.geom.is_none() {
            self.geom = Some(self.resolve(bounds));
        }
        self.seed_freeform(bounds, &|_| true);
    }
    /// Move a placed gradient from box `from` to box `to` (see [`GradientGeom::rebase`]).
    pub fn rebase(&mut self, from: Rect, to: Rect) {
        if let Some(g) = &mut self.geom {
            g.rebase(from, to);
        }
        if let Some(f) = &mut self.freeform {
            f.map_points(|p| rebase_point(p, from, to));
        }
    }
    /// Map a placed gradient (and freeform points) through `a` (see [`GradientGeom::transform`]).
    pub fn transform(&mut self, a: Affine) {
        if let Some(g) = &mut self.geom {
            g.transform(a, self.gradient.kind);
        }
        if let Some(f) = &mut self.freeform {
            f.transform(a);
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
        GradientGeom { start: start.into(), end: end.into(), aspect: 1.0, focal: None }
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
        let mut g = GradientGeom { start: Point::new(10.0, 10.0), end: Point::new(20.0, 10.0), aspect: 1.0, focal: None };
        g.transform(Affine::scale_non_uniform(2.0, 1.0), GradientKind::Radial);
        assert!(close(g.start, Point::new(20.0, 10.0)) && close(g.end, Point::new(40.0, 10.0)), "{g:?}");
        assert!((g.aspect - 0.5).abs() < 1e-9);
        // A vertical vector keeps its axis: the ellipse is twice as wide as the vector is long.
        let mut v = GradientGeom { start: Point::ZERO, end: Point::new(0.0, -10.0), aspect: 1.0, focal: None };
        v.transform(Affine::scale_non_uniform(2.0, 1.0), GradientKind::Radial);
        assert!(close(v.end, Point::new(0.0, -10.0)) && (v.aspect - 2.0).abs() < 1e-9, "{v:?}");
    }

    #[test]
    fn samples_follow_shear_and_rotation() {
        let shear = Affine::new([1.0, 0.3, 0.7, 1.2, 15.0, -4.0]) * Affine::rotate(0.4);
        for kind in [GradientKind::Linear, GradientKind::Radial] {
            let g = GradientGeom { start: Point::new(30.0, 40.0), end: Point::new(80.0, 55.0), aspect: 0.6, focal: None };
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
        let e = GradientGeom { start: Point::ZERO, end: Point::new(10.0, 0.0), aspect: 0.5, focal: None };
        assert!((e.param_at(GradientKind::Radial, Point::new(0.0, 5.0)) - 1.0).abs() < 1e-12);
        assert!((e.param_at(GradientKind::Radial, Point::new(5.0, 0.0)) - 0.5).abs() < 1e-12);
        assert!((e.param_at(GradientKind::Linear, Point::new(5.0, 7.0)) - 0.5).abs() < 1e-12);
    }

    fn g3() -> Gradient {
        let mut g = Gradient::default();
        g.stops.insert(1, GradientStop::new(0.5, Color::rgb(1.0, 0.0, 0.0)));
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
        let mut g = GradientGeom { start: Point::new(110.0, 150.0), end: Point::new(190.0, 120.0), aspect: 0.5, focal: None };
        g.rebase(Rect::new(100.0, 100.0, 200.0, 200.0), Rect::new(500.0, 0.0, 700.0, 50.0));
        assert!(close(g.start, Point::new(520.0, 25.0)) && close(g.end, Point::new(680.0, 10.0)) && g.aspect == 0.5, "{g:?}");
        // A box without height (a horizontal line) keeps the offset from its centre line.
        let mut h = GradientGeom { start: Point::new(0.0, 12.0), end: Point::new(10.0, 10.0), aspect: 1.0, focal: None };
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

    /// A 100 pt circle at (200, 200) with its focal point 40 pt left of the centre.
    fn off_centre() -> GradientGeom {
        let mut g = GradientGeom { start: Point::new(200.0, 200.0), end: Point::new(300.0, 200.0), aspect: 1.0, focal: None };
        g.set_focal(Some(Point::new(160.0, 200.0)));
        g
    }

    #[test]
    fn focal_round_trips_through_serde_and_defaults_to_none() {
        let g = off_centre();
        let s = serde_json::to_string(&g).unwrap();
        assert!(s.contains("\"focal\""), "{s}");
        assert_eq!(serde_json::from_str::<GradientGeom>(&s).unwrap(), g);
        // Placements saved before focal points existed load centred, and centred ones write none.
        let old: GradientGeom = serde_json::from_str(r#"{"start":{"x":0.0,"y":0.0},"end":{"x":1.0,"y":0.0}}"#).unwrap();
        assert_eq!((old.focal, old.aspect), (None, 1.0));
        assert!(!serde_json::to_string(&old).unwrap().contains("focal"));
    }

    #[test]
    fn focal_follows_transforms_and_rebases() {
        let mut g = off_centre();
        let a = Affine::translate((10.0, -5.0)) * Affine::rotate(0.7) * Affine::scale_non_uniform(2.0, 0.5);
        let f = a * g.focal.unwrap();
        g.transform(a, GradientKind::Radial);
        assert!(close(g.focal.unwrap(), f), "{g:?}");
        // Still at the same parameter: the first stop.
        assert!(g.param_at(GradientKind::Radial, f).abs() < 1e-9);
        let mut h = off_centre();
        h.rebase(Rect::new(100.0, 100.0, 300.0, 300.0), Rect::new(0.0, 0.0, 100.0, 100.0));
        assert!(close(h.focal.unwrap(), Point::new(30.0, 50.0)) && close(h.start, Point::new(50.0, 50.0)), "{h:?}");
    }

    #[test]
    fn param_runs_from_the_focal_point_to_the_ellipse() {
        let g = off_centre();
        let t = |x: f64, y: f64| g.param_at(GradientKind::Radial, Point::new(x, y));
        assert!(t(160.0, 200.0).abs() < 1e-9);
        // The extent circle is the last stop all round.
        for a in [0.0f64, 1.0, 2.5, 4.0] {
            let p = Point::new(200.0 + 100.0 * a.cos(), 200.0 + 100.0 * a.sin());
            assert!((g.param_at(GradientKind::Radial, p) - 1.0).abs() < 1e-9, "{a}");
        }
        // The centre lies on the level-2/7 circle: centred 2/7 of the way back to the focal point
        // (0.4 of the radius off), with a radius of 2/7.
        assert!((t(200.0, 200.0) - 2.0 / 7.0).abs() < 1e-9, "{}", t(200.0, 200.0));
        assert!(t(180.0, 200.0) < t(220.0, 200.0), "nearer the focal side is earlier");
        assert_eq!(GradientGeom { focal: None, ..g }.param_at(GradientKind::Radial, Point::new(250.0, 200.0)), 0.5);
    }

    #[test]
    fn focal_is_kept_inside_the_ellipse_and_follows_its_reshaping() {
        let mut g = off_centre();
        g.set_focal(Some(Point::new(500.0, 200.0)));
        assert!(close(g.focal.unwrap(), Point::new(299.0, 200.0)), "pulled inside: {g:?}");
        g.set_focal(Some(Point::new(200.0, 200.0)));
        assert_eq!(g.focal, None, "on the centre: centred");
        // A new vector and aspect keep it in its place in the ellipse.
        let before = off_centre();
        let mut h = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(0.0, 50.0), aspect: 0.5, focal: None };
        h.keep_focal_from(&before);
        assert!(close(h.focal.unwrap(), Point::new(0.0, -20.0)), "{h:?}");
    }
}
