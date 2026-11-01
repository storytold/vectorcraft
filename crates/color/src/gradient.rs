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

/// How a gradient mixes the colours between its stops (the Gradient panel's Method).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientMethod {
    /// Mixed in display RGB, as gradients always were.
    #[default]
    Classic,
    /// Mixed in a perceptual colour space (OKLab), so the steps look even to the eye: blue to
    /// yellow passes through a light blue-grey rather than a dull mid grey.
    Perceptual,
}

impl GradientMethod {
    /// Parse a method name (`classic`, `perceptual`; any case).
    pub fn parse(s: &str) -> Option<Self> {
        [Self::Classic, Self::Perceptual].into_iter().find(|m| m.label().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Perceptual => "Perceptual",
        }
    }
    pub fn is_classic(&self) -> bool {
        *self == Self::Classic
    }
    /// `a` and `b` mixed at `t` (0..1): in display RGB (Classic), or in OKLab (Perceptual: an
    /// RGB colour between the two).
    pub fn mix(self, a: &Color, b: &Color, t: f32) -> Color {
        match self {
            Self::Classic => Color::lerp(a, b, t),
            Self::Perceptual => {
                use crate::libraries::{oklab_to_srgb, srgb_to_oklab};
                let (p, q, t) = (srgb_to_oklab(a.to_rgb()), srgb_to_oklab(b.to_rgb()), f64::from(t));
                let [r, g, bl] = oklab_to_srgb(p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t, p[2] + (q[2] - p[2]) * t);
                let c = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) as f32 } else { 0.0 };
                Color::rgb(c(r), c(g), c(bl))
            }
        }
    }
}

/// How far (display RGB, 0..1) the straight line between a Perceptual gradient's neighbouring
/// expanded stops may stray from the perceptual mix: under half a colour level.
const PERCEPTUAL_TOLERANCE: f32 = 0.4 / 255.0;
/// Most times each half of a Perceptual gradient's segment (either side of its midpoint) is
/// halved to keep within [`PERCEPTUAL_TOLERANCE`].
const PERCEPTUAL_DEPTH: u32 = 10;
/// How many times [`Gradient::ink_tints`] halves the range of tints it searches: to within
/// 1/65536 of a tint, finer than the 1/255 steps PDF writes.
const TINT_STEPS: u32 = 16;
/// The least OKLab lightness by which an ink at 100% must differ from paper white for
/// [`Gradient::ink_tints`] to tell its tints apart by lightness.
const MIN_INK_LIGHTNESS_SPAN: f64 = 1.0 / 255.0;
/// Most stops on a gradient's curve that [`Gradient::ink_tints`] finds tints for.
const MAX_INK_TINTS: usize = 4096;

/// The mix positions (0..1, after `v0`, before `v1`) between which display RGB mixed linearly
/// stays within [`PERCEPTUAL_TOLERANCE`] of the perceptual mix of `a` and `b`: the span halved
/// where a straight line strays, at most [`PERCEPTUAL_DEPTH`] times, in order.
fn perceptual_steps(a: &Color, b: &Color, (v0, v1): (f32, f32), depth: u32, out: &mut Vec<f32>) {
    let at = |v: f32| GradientMethod::Perceptual.mix(a, b, v).to_rgb();
    let (c0, c1) = (at(v0), at(v1));
    let stray = [0.25, 0.5, 0.75]
        .into_iter()
        .flat_map(|u: f32| {
            let c = at(v0 + (v1 - v0) * u);
            (0..3).map(move |k| (c0[k] + (c1[k] - c0[k]) * u - c[k]).abs())
        })
        .fold(0.0, f32::max);
    if depth < PERCEPTUAL_DEPTH && stray > PERCEPTUAL_TOLERANCE {
        let m = (v0 + v1) / 2.0;
        perceptual_steps(a, b, (v0, m), depth + 1, out);
        out.push(m);
        perceptual_steps(a, b, (m, v1), depth + 1, out);
    }
}

/// The mix positions (`v`, 0..1) of the stops a Perceptual gradient's curve has between stops `a`
/// and `b`, in order: [`perceptual_steps`] on either side of the midpoint, and the midpoint (½).
fn perceptual_mixes(a: &GradientStop, b: &GradientStop) -> Vec<f32> {
    let mut vs = Vec::new();
    perceptual_steps(&a.color, &b.color, (0.0, 0.5), 1, &mut vs);
    vs.push(0.5);
    perceptual_steps(&a.color, &b.color, (0.5, 1.0), 1, &mut vs);
    vs
}

/// The offset of mix position `v` between stops `a` and `b`, placed through `a`'s midpoint
/// (`v` = ½ at the midpoint) as [`Gradient::sample`] maps it.
fn curve_offset(a: &GradientStop, b: &GradientStop, v: f32) -> f32 {
    let m = a.midpoint.clamp(0.01, 0.99);
    let u = if v == 0.5 {
        a.midpoint
    } else if v < 0.5 {
        2.0 * v * m
    } else {
        m + (v - 0.5) * 2.0 * (1.0 - m)
    };
    a.offset + (b.offset - a.offset) * u
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

/// The colour and opacity at `t` between stops `a` and `b`, through `a`'s midpoint (`u` = midpoint
/// maps to `v` = ½), mixed by `mix`.
fn between(a: &GradientStop, b: &GradientStop, t: f32, mix: &impl Fn(&Color, &Color, f32) -> Color) -> (Color, f32) {
    let span = (b.offset - a.offset).max(1e-6);
    let u = (t - a.offset) / span;
    let m = a.midpoint.clamp(0.01, 0.99);
    let v = if u < m { 0.5 * u / m } else { 0.5 + 0.5 * (u - m) / (1.0 - m) };
    (mix(&a.color, &b.color, v), a.opacity + (b.opacity - a.opacity) * v)
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
    /// Dither: a fill of this gradient is drawn as pixels with fine noise of up to a colour level
    /// added, which hides the steps (banding) between the levels 8-bit colour has.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dither: bool,
    /// How the colours between the stops are mixed (Classic unless set; written only when not).
    #[serde(default, skip_serializing_if = "GradientMethod::is_classic")]
    pub method: GradientMethod,
}

impl Default for Gradient {
    /// Illustrator's default "White, Black" gradient.
    fn default() -> Self {
        Self::new(GradientKind::Linear, vec![GradientStop::new(0.0, Color::WHITE), GradientStop::new(1.0, Color::BLACK)])
    }
}

impl Gradient {
    /// A gradient of `kind` through `stops`, with the default options.
    pub fn new(kind: GradientKind, stops: Vec<GradientStop>) -> Self {
        Self { kind, stops, dither: false, method: GradientMethod::Classic }
    }
    /// Colour and opacity at `t` (honours midpoints and the method).
    pub fn sample(&self, t: f32) -> (Color, f32) {
        self.sample_with(t, |a, b, v| self.method.mix(a, b, v))
    }
    /// [`Self::sample`], a Classic gradient's neighbouring stop colours of one model mixed in it
    /// (CMYK stays CMYK; [`Color::lerp_keep_model`]) rather than in display RGB, a Perceptual
    /// gradient's as [`Self::sample`] mixes them.
    pub fn sample_keep_model(&self, t: f32) -> (Color, f32) {
        match self.method {
            GradientMethod::Classic => self.sample_with(t, Color::lerp_keep_model),
            m => self.sample_with(t, |a, b, v| m.mix(a, b, v)),
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
                return between(a, b, t, &mix);
            }
        }
        match stops.last() {
            Some(l) => (l.color, l.opacity),
            None => (Color::BLACK, 1.0),
        }
    }
    /// [`Self::sample_with`] at each of `ts`, which must not decrease. Each search starts where
    /// the one before it stopped, so the work is the samples plus the stops, not their product.
    pub fn sample_increasing_with(&self, ts: impl IntoIterator<Item = f32>, mix: impl Fn(&Color, &Color, f32) -> Color) -> Vec<(Color, f32)> {
        let stops = &self.stops;
        // The shared index assumes ordered stops; others (a file's, unsorted) sample one by one.
        if stops.windows(2).any(|w| w[1].offset < w[0].offset) {
            return ts.into_iter().map(|t| self.sample_with(t, &mix)).collect();
        }
        let mut j = 0;
        ts.into_iter()
            .map(|t| {
                let Some(first) = stops.first() else { return (Color::BLACK, 1.0) };
                if t <= first.offset {
                    return (first.color, first.opacity);
                }
                // The first pair whose end `t` doesn't pass, as `sample_with` finds it: one that a
                // smaller `t` passed, this one passes too.
                while j + 1 < stops.len() && t > stops[j + 1].offset {
                    j += 1;
                }
                match (stops.get(j), stops.get(j + 1)) {
                    (Some(a), Some(b)) => between(a, b, t, &mix),
                    _ => stops.last().map_or((Color::BLACK, 1.0), |l| (l.color, l.opacity)),
                }
            })
            .collect()
    }
    /// Stops expanded so that midpoints are represented as explicit stops (for renderers without
    /// midpoints), and a Perceptual gradient's segments as stops on its curve (for renderers that
    /// mix linearly; within [`PERCEPTUAL_TOLERANCE`] of it).
    pub fn expanded_stops(&self) -> Vec<(f32, Color, f32)> {
        self.expanded().map(|(t, c, o, _)| (t, c, o)).collect()
    }
    /// [`Self::expanded_stops`], each stop that stands for a midpoint with that midpoint (`None`
    /// on the gradient's own stops and a Perceptual gradient's other in-between stops).
    pub fn expanded(&self) -> impl Iterator<Item = (f32, Color, f32, Option<f32>)> + '_ {
        // Direct interpolation matches `sample` only for strictly increasing offsets.
        let increasing = self.stops.windows(2).all(|w| w[0].offset < w[1].offset);
        self.stops.iter().enumerate().flat_map(move |(i, s)| {
            let next = self.stops.get(i + 1);
            let off_centre = (s.midpoint - 0.5).abs() > 1e-3;
            // Classic: a stop at the midpoint, when it is off-centre.
            let classic = next.filter(|_| off_centre && self.method.is_classic()).map(|n| {
                let t = s.offset + (n.offset - s.offset) * s.midpoint;
                // Mixed from the two stops it lies between, as `sample` would: searching all the
                // stops again for each midpoint made many stops cost their square.
                let mix = |a: &Color, b: &Color, v| self.method.mix(a, b, v);
                let (c, o) = if increasing { between(s, n, t, &mix) } else { self.sample_with(t, mix) };
                (t, c, o, Some(s.midpoint))
            });
            // Perceptual: stops along the mix (`v`) where a straight line would stray from it, the
            // midpoint (v = ½) among them, placed through the midpoint as [`Self::sample`] maps them.
            let perceptual = next.filter(|_| !self.method.is_classic()).map(|n| {
                perceptual_mixes(s, n).into_iter().map(move |v| {
                    let mid = v == 0.5;
                    let t = curve_offset(s, n, v);
                    let o = s.opacity + (n.opacity - s.opacity) * v;
                    (t, self.method.mix(&s.color, &n.color, v), o, (mid && off_centre).then_some(s.midpoint))
                })
            });
            std::iter::once((s.offset, s.color, s.opacity, None)).chain(classic).chain(perceptual.into_iter().flatten())
        })
    }
    /// A gradient of tints of one ink `ink` (its color at 100%; paper white is 0% of it) as
    /// (offset, tint, opacity) at each of [`Self::expanded_stops`]: the tint (0..1) whose OKLab
    /// lightness is the lightness of the color there, found to [`TINT_STEPS`] halvings. Mixed
    /// linearly between them, the tints follow a Perceptual gradient's curve. `None` when the
    /// ink's lightness is within [`MIN_INK_LIGHTNESS_SPAN`] of paper white's (a white or nearly
    /// white ink), so lightness does not tell its tints apart, and when the curve has more than
    /// [`MAX_INK_TINTS`] stops ([`Self::fits_ink_tints`]).
    pub fn ink_tints(&self, ink: &Color) -> Option<Vec<(f32, f32, f32)>> {
        use crate::libraries::srgb_to_oklab;
        let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
        if lightness(&ink.tinted(0.0)) - lightness(ink) < MIN_INK_LIGHTNESS_SPAN {
            return None;
        }
        let curve: Vec<_> = self.expanded().take(MAX_INK_TINTS + 1).collect();
        if curve.len() > MAX_INK_TINTS {
            return None;
        }
        let tint = |c: &Color| {
            let want = lightness(c);
            let (mut lo, mut hi) = (0.0f32, 1.0f32);
            for _ in 0..TINT_STEPS {
                let mid = (lo + hi) / 2.0;
                if lightness(&ink.tinted(mid)) > want {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            (lo + hi) / 2.0
        };
        Some(curve.into_iter().map(|(t, c, o, _)| (t, tint(&c), o)).collect())
    }
    /// Does the gradient's curve ([`Self::expanded`]) have at most [`MAX_INK_TINTS`] stops, so
    /// [`Self::ink_tints`] finds tints for it?
    pub fn fits_ink_tints(&self) -> bool {
        self.expanded().nth(MAX_INK_TINTS).is_none()
    }
    /// A Perceptual gradient's curve ([`Self::expanded_stops`]) as mixes of its stops' inks:
    /// (offset, `i`, weight, opacity) at each stop on it, where the inks there are those of stop
    /// `i` and stop `i + 1` mixed linearly at the weight (0..1; 0 on the gradient's own stops).
    /// The weight is the one at which the two stops' colors mixed in display RGB have the OKLab
    /// lightness of the color on the curve there, found to [`TINT_STEPS`] halvings, or the mix
    /// position itself when the two colors' lightness differs by less than
    /// [`MIN_INK_LIGHTNESS_SPAN`]. `None` for a Classic gradient, and when the curve has more than
    /// [`MAX_INK_TINTS`] stops ([`Self::fits_ink_tints`]).
    pub fn ink_mixes(&self) -> Option<Vec<(f32, usize, f32, f32)>> {
        use crate::libraries::srgb_to_oklab;
        if self.method.is_classic() || !self.fits_ink_tints() {
            return None;
        }
        let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
        let mut out = Vec::new();
        for (i, s) in self.stops.iter().enumerate() {
            out.push((s.offset, i, 0.0, s.opacity));
            let Some(n) = self.stops.get(i + 1) else { continue };
            let (l0, l1) = (lightness(&s.color), lightness(&n.color));
            for v in perceptual_mixes(s, n) {
                let weight = if (l1 - l0).abs() < MIN_INK_LIGHTNESS_SPAN {
                    v
                } else {
                    let want = lightness(&self.method.mix(&s.color, &n.color, v));
                    let (mut lo, mut hi) = (0.0f32, 1.0f32);
                    for _ in 0..TINT_STEPS {
                        let mid = (lo + hi) / 2.0;
                        // The mix at `mid` has not yet gone from stop `i`'s lightness to the wanted one.
                        if (lightness(&GradientMethod::Classic.mix(&s.color, &n.color, mid)) - want) * (l1 - l0) < 0.0 {
                            lo = mid;
                        } else {
                            hi = mid;
                        }
                    }
                    (lo + hi) / 2.0
                };
                out.push((curve_offset(s, n, v), i, weight, s.opacity + (n.opacity - s.opacity) * v));
            }
        }
        Some(out)
    }
    /// The same ramp the other way round: the stops in reverse order at mirrored offsets, each
    /// segment's midpoint mirrored with it.
    pub fn reverse(&mut self) {
        self.stops.reverse();
        for i in 0..self.stops.len() {
            // A midpoint belongs to the segment after its stop: reversed, stop i's segment is the
            // one the next stop (not yet mirrored) had, measured from the other end.
            let mid = self.stops.get(i + 1).map_or(0.5, |n| 1.0 - n.midpoint);
            if let Some(s) = self.stops.get_mut(i) {
                s.offset = 1.0 - s.offset;
                s.midpoint = mid;
            }
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
        Gradient::new(GradientKind::Linear, stops)
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
    /// Is this gradient drawn as pixels rather than as a vector gradient when it paints a fill
    /// (`fill`) or a stroke (SVG and PDF draw such gradients as images): a freeform gradient, a
    /// dithered fill (dither applies to fills only), or a Perceptual one (its colours don't mix
    /// linearly between the stops, as SVG's and PDF's gradients do)?
    pub fn draws_as_pixels(&self, fill: bool) -> bool {
        self.gradient.kind == GradientKind::Freeform || (fill && self.gradient.dither) || !self.gradient.method.is_classic()
    }
    /// A linear or radial gradient placed on box `bounds` as a `cols` × `rows` grid of
    /// unpremultiplied RGBA8 pixels over `area`, row by row, each sampled at its centre (colours
    /// as `rgb` gives them, mixed as the renderer mixes them: [`Ramp`]). Dithered and painting a
    /// fill (`fill`; dither applies to fills only), each colour channel of pixel (column, row) gets
    /// [`dither_noise`] before it is rounded to a level.
    pub fn pixels(
        &self,
        bounds: Rect,
        area: Rect,
        cols: u16,
        rows: u16,
        fill: bool,
        rgb: &dyn Fn(&Color) -> [f32; 3],
    ) -> impl Iterator<Item = [u8; 4]> + use<> {
        self.pixels_within(bounds, area, cols, rows, [0..cols, 0..rows], fill, rgb)
    }
    /// [`Self::pixels`] for the cells in columns `cells[0]` and rows `cells[1]` of the grid only,
    /// row by row: the same pixels, noise and all, the whole grid has there.
    #[allow(clippy::too_many_arguments)]
    pub fn pixels_within(
        &self,
        bounds: Rect,
        area: Rect,
        cols: u16,
        rows: u16,
        cells: [std::ops::Range<u16>; 2],
        fill: bool,
        rgb: &dyn Fn(&Color) -> [f32; 3],
    ) -> impl Iterator<Item = [u8; 4]> + use<> {
        let (kind, geom, dither) = (self.gradient.kind, self.resolve(bounds), fill && self.gradient.dither);
        let ramp = Ramp::new(&self.gradient, rgb);
        let (cw, ch) = (area.width() / f64::from(cols.max(1)), area.height() / f64::from(rows.max(1)));
        let (x0, y0) = (area.x0, area.y0);
        let [across, down] = cells.map(|r| u32::from(r.start)..u32::from(r.end));
        down.flat_map(move |j| across.clone().map(move |i| (i, j))).map(move |(i, j)| {
            let p = Point::new(x0 + (f64::from(i) + 0.5) * cw, y0 + (f64::from(j) + 0.5) * ch);
            let c = ramp.at(geom.param_at(kind, p) as f32);
            let level = |k: usize| {
                let noise = if dither { dither_noise(i, j, k as u32) } else { 0.0 };
                (c[k].clamp(0.0, 1.0) * 255.0 + noise).round().clamp(0.0, 255.0) as u8
            };
            [level(0), level(1), level(2), (c[3].clamp(0.0, 1.0) * 255.0).round() as u8]
        })
    }
}

/// A gradient's colours ready to be sampled pixel by pixel, in a colour space of the caller's
/// (display RGB, or an ink's coverage): its stops, midpoints and a Perceptual gradient's
/// in-between colours as stops (as [`Gradient::expanded_stops`]), each turned into that space
/// once, with colour and opacity mixed linearly between them, each on its own (as the renderer's
/// vector gradients mix them).
pub struct Ramp {
    stops: Vec<(f32, [f32; 4])>,
}

impl Ramp {
    pub fn new(g: &Gradient, rgb: &dyn Fn(&Color) -> [f32; 3]) -> Self {
        let stops = g
            .expanded()
            .map(|(t, c, o, _)| {
                let [r, gr, b] = rgb(&c);
                (t, [r, gr, b, o])
            })
            .collect();
        Self { stops }
    }
    /// Unpremultiplied colour and opacity at `t`, padded past the ends (transparent black
    /// without stops).
    pub fn at(&self, t: f32) -> [f32; 4] {
        let Some(&(t0, first)) = self.stops.first() else { return [0.0; 4] };
        // Not after the first stop (or not a number): its colour.
        if t.partial_cmp(&t0) != Some(std::cmp::Ordering::Greater) {
            return first;
        }
        for w in self.stops.windows(2) {
            let ((a, ca), (b, cb)) = (w[0], w[1]);
            if t <= b {
                let u = if b - a > 1e-9 { (t - a) / (b - a) } else { 1.0 };
                return std::array::from_fn(|k| ca[k] + (cb[k] - ca[k]) * u);
            }
        }
        self.stops.last().map_or([0.0; 4], |s| s.1)
    }
}

/// Seed of [`dither_noise`]: fixed, so a dithered gradient draws and exports the same pixels on
/// every run.
const DITHER_SEED: u32 = 0x5eed_d17e;

/// An integer hash that mixes every input bit into every output bit (the "lowbias32" mix).
fn mix32(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb_352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846c_a68b);
    v ^ (v >> 16)
}

/// Dither noise for colour channel `channel` of pixel (`x`, `y`), in colour levels: -1..1,
/// densest around 0 (the sum of two uniform values: noise that hides steps between levels without
/// a pattern of its own), the same for the same pixel on every run.
pub fn dither_noise(x: u32, y: u32, channel: u32) -> f32 {
    let h = mix32(x ^ mix32(y ^ mix32(channel ^ DITHER_SEED)));
    ((h & 0xffff) as f32 + (h >> 16) as f32) / 65535.0 - 1.0
}

#[cfg(test)]
mod tests;
