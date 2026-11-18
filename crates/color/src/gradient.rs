//! Gradients: definitions (swatch-able) and their placement on an object.

use std::borrow::Cow;

use kurbo::{Affine, Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::Color;
use crate::freeform::Freeform;

mod spread;
pub use spread::{MAX_SPREAD_LEVEL, MAX_SPREAD_RUNS, MAX_SPREAD_STOPS, Padded};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum GradientKind {
    #[default]
    Linear,
    Radial,
    /// Freeform gradients: colour points and lines (see [`crate::freeform`]).
    Freeform,
    /// Conic (angle) gradients: the stops sweep once around the center (`start`), clockwise on
    /// the page from the vector, with a seam where the last stop meets the first. A negative
    /// `aspect` mirrors the sweep (see [`GradientGeom::transform`]).
    Conic,
    /// Diamond gradients: the stops run from the center (`start`) out to a square whose
    /// half-diagonal is the vector (a diamond when the vector is level), stretched across the
    /// vector by `aspect` (see [`GradientGeom::diamond_frame`]).
    Diamond,
}

impl GradientKind {
    /// Every kind, in the order the Gradient panel lists them.
    pub const ALL: [GradientKind; 5] =
        [GradientKind::Linear, GradientKind::Radial, GradientKind::Conic, GradientKind::Diamond, GradientKind::Freeform];
    /// Parse a kind name (`linear`, `radial`, `conic`, `diamond`, `freeform`; any case). `angle`
    /// and `conical` also mean conic.
    pub fn parse(s: &str) -> Option<Self> {
        if s.eq_ignore_ascii_case("angle") || s.eq_ignore_ascii_case("conical") {
            return Some(GradientKind::Conic);
        }
        Self::ALL.into_iter().find(|k| k.label().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            GradientKind::Linear => "Linear",
            GradientKind::Radial => "Radial",
            GradientKind::Freeform => "Freeform",
            GradientKind::Conic => "Conic",
            GradientKind::Diamond => "Diamond",
        }
    }
    /// Is it placed by a center (`start`) and an extent around it (radial, conic and diamond),
    /// rather than by a line?
    pub fn is_centered(self) -> bool {
        matches!(self, GradientKind::Radial | GradientKind::Conic | GradientKind::Diamond)
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
    /// Mixed around the hue wheel, clockwise as the color wheels draw it: the hue angle falls
    /// from each stop to the next (red, magenta, blue, cyan, green, yellow, red), and saturation
    /// and brightness change evenly ([`hue_mix`]).
    HueClockwise,
    /// Mixed around the hue wheel the other way: the hue angle rises from each stop to the next
    /// (red, yellow, green, cyan, blue, magenta, red).
    HueCounterclockwise,
}

impl GradientMethod {
    /// Every method, in the order the Gradient panel lists them.
    pub const ALL: [GradientMethod; 4] = [Self::Classic, Self::Perceptual, Self::HueClockwise, Self::HueCounterclockwise];
    /// Parse a method name (`classic`, `perceptual`, `hue-clockwise`, `hue-counterclockwise`;
    /// any case, and the hyphen may be a space or an underscore, or left out). Any other text,
    /// one with a separator elsewhere included, is `None`.
    pub fn parse(s: &str) -> Option<Self> {
        let name = s.to_ascii_lowercase();
        Self::ALL.into_iter().find(|m| ["-", " ", "_", ""].iter().any(|sep| m.id().replace('-', sep) == name))
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Classic => "Classic",
            Self::Perceptual => "Perceptual",
            Self::HueClockwise => "Hue Clockwise",
            Self::HueCounterclockwise => "Hue Counterclockwise",
        }
    }
    /// The method's name in command params (`classic`, `perceptual`, `hue-clockwise`,
    /// `hue-counterclockwise`).
    pub fn id(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Perceptual => "perceptual",
            Self::HueClockwise => "hue-clockwise",
            Self::HueCounterclockwise => "hue-counterclockwise",
        }
    }
    pub fn is_classic(&self) -> bool {
        *self == Self::Classic
    }
    /// Does it mix around the hue wheel (Hue Clockwise or Hue Counterclockwise)? The colors
    /// between two stops then are not mixes of the stops' inks.
    pub fn is_hue(&self) -> bool {
        matches!(self, Self::HueClockwise | Self::HueCounterclockwise)
    }
    /// The method that mixes `b` to `a` through the colors this one mixes `a` to `b` through: a
    /// hue method turned the other way, Classic and Perceptual as they are.
    pub fn mirrored(self) -> Self {
        match self {
            Self::HueClockwise => Self::HueCounterclockwise,
            Self::HueCounterclockwise => Self::HueClockwise,
            m => m,
        }
    }
    /// `a` and `b` mixed at `t` (0..1): in display RGB (Classic), in OKLab (Perceptual: an
    /// RGB colour between the two), or around the hue wheel ([`hue_mix`]: an RGB colour).
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
            Self::HueClockwise => hue_mix(a, b, t, false),
            Self::HueCounterclockwise => hue_mix(a, b, t, true),
        }
    }
}

/// The brightness (0..1) at or below which a color is black for [`hue_mix`], and the spread
/// between its largest and smallest RGB numbers (saturation × brightness) at or below which it
/// has no hue of its own: under half a color level.
const NO_HUE: f32 = 0.4 / 255.0;

/// The hue difference (degrees) that moves an RGB number by half a color level in a color whose
/// largest and smallest RGB numbers are 1 apart; a color with a smaller spread takes
/// proportionally more degrees. [`hue_mix`] counts two hues closer than this as one hue.
const SAME_HUE: f32 = 60.0 * 0.5 / 255.0;

/// `a` and `b` mixed at `t` (0..1) around the hue wheel, as display RGB: hue (degrees),
/// saturation and brightness each change evenly from `a`'s to `b`'s. The hue angle only rises
/// (`rising`) or only falls, by the fixed-direction rules of
/// <https://www.w3.org/TR/css-color-4/#hue-interpolation>, and two equal hues stay where they are.
/// Two hues that differ by less than half a color level ([`SAME_HUE`]) count as equal and take the
/// short way, so a color and a tint or shade of it, whose computed hues differ by rounding, do
/// not turn the whole wheel. A color whose RGB numbers are less than half a color level apart
/// ([`NO_HUE`]) has no hue and takes the other color's; black has no saturation either and takes
/// the other color's too. Two colors without a hue mix as grays. A `t` that is not a number
/// gives `a`.
pub fn hue_mix(a: &Color, b: &Color, t: f32, rising: bool) -> Color {
    let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
    let ([mut h0, mut s0, v0], [mut h1, mut s1, v1]) = (a.to_hsb(), b.to_hsb());
    let (black0, black1) = (v0 <= NO_HUE, v1 <= NO_HUE);
    // The spread between each color's own largest and smallest RGB numbers.
    let (spread0, spread1) = (s0 * v0, s1 * v1);
    // Black takes the other color's saturation, so it darkens that color rather than graying it.
    if black0 && !black1 {
        s0 = s1;
    } else if black1 && !black0 {
        s1 = s0;
    }
    let (gray0, gray1) = (black0 || spread0 <= NO_HUE, black1 || spread1 <= NO_HUE);
    match (gray0, gray1) {
        (true, _) => h0 = h1,
        (false, true) => h1 = h0,
        (false, false) => {
            // The short way from the first hue to the second, -180..180.
            let short = (h1 - h0 + 180.0).rem_euclid(360.0) - 180.0;
            h1 = if short.abs() * spread0.min(spread1) <= SAME_HUE {
                h0 + short
            } else if rising {
                h0 + short.rem_euclid(360.0)
            } else {
                h0 + short.rem_euclid(360.0) - 360.0
            };
        }
    }
    let at = |p: f32, q: f32| p + (q - p) * t;
    let (h, s, v) = (at(h0, h1), at(s0, s1), at(v0, v1));
    if !(h.is_finite() && s.is_finite() && v.is_finite()) {
        return Color::rgb(0.0, 0.0, 0.0);
    }
    Color::from_hsb(h.rem_euclid(360.0), s.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// What a linear, radial, conic or diamond gradient paints past the ends of its stops (the
/// Gradient panel's Spread). With Repeat and Reflect, the stops run [`Gradient::repeats`] times
/// along the vector (around the circle, for a conic), and keep going past both of its ends.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub enum GradientSpread {
    /// The first and last stops' colors carry on past the ends, as gradients always did.
    #[default]
    Pad,
    /// The stops start again at each end.
    Repeat,
    /// The stops run back the other way at each end, so every other run is mirrored.
    Reflect,
}

impl GradientSpread {
    /// Every spread, in the order the Gradient panel lists them.
    pub const ALL: [GradientSpread; 3] = [GradientSpread::Pad, GradientSpread::Repeat, GradientSpread::Reflect];
    /// Parse a spread name (`pad`, `repeat`, `reflect`; any case).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.label().eq_ignore_ascii_case(s))
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Pad => "Pad",
            Self::Repeat => "Repeat",
            Self::Reflect => "Reflect",
        }
    }
    pub fn is_pad(&self) -> bool {
        *self == Self::Pad
    }
}

impl<'de> Deserialize<'de> for GradientSpread {
    /// A spread name; a file's unknown name or other value reads as Pad.
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            Other(serde::de::IgnoredAny),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::Name(s) => Self::parse(&s).unwrap_or_default(),
            Raw::Other(_) => Self::Pad,
        })
    }
}

/// Most times a gradient's stops run along its vector ([`Gradient::repeats`]): VectorCraft's own
/// limit.
pub const MAX_REPEATS: u32 = 100;

fn one_repeat() -> u32 {
    1
}

fn is_one_repeat(n: &u32) -> bool {
    *n == 1
}

/// A file's repeat count: a number, rounded and kept within 1 to [`MAX_REPEATS`]; anything else
/// reads as 1.
fn lenient_repeats<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Raw {
        Number(f64),
        Other(serde::de::IgnoredAny),
    }
    Ok(match Raw::deserialize(d)? {
        Raw::Number(n) if n.is_finite() => n.round().clamp(1.0, f64::from(MAX_REPEATS)) as u32,
        _ => 1,
    })
}

/// How far (display RGB, 0..1) the straight line between a Perceptual gradient's neighbouring
/// expanded stops may stray from the perceptual mix: under half a colour level.
const PERCEPTUAL_TOLERANCE: f32 = 0.4 / 255.0;
/// Most times each half of a Perceptual gradient's segment (either side of its midpoint) is
/// halved to keep within [`PERCEPTUAL_TOLERANCE`].
const PERCEPTUAL_DEPTH: u32 = 10;
/// How many times [`Gradient::ink_tints`] halves the range of tints it searches: to within
/// 1/65536 of a tint.
const TINT_STEPS: u32 = 16;
/// The least OKLab lightness by which an ink at 100% must differ from paper white for
/// [`Gradient::ink_tints`] to tell its tints apart by lightness.
const MIN_INK_LIGHTNESS_SPAN: f64 = 1.0 / 255.0;
/// Most stops on a gradient's curve that [`Gradient::ink_tints`] finds tints for.
const MAX_INK_TINTS: usize = 4096;
/// How many even steps [`Gradient::ink_mixes`] checks the lightness of two stops' display RGB mix
/// at, to tell whether it moves steadily from one stop's lightness to the other's.
const INK_MIX_STEPS: u32 = 64;

/// The mix positions (0..1, after `v0`, before `v1`) between which display RGB mixed linearly
/// stays within [`PERCEPTUAL_TOLERANCE`] of `method`'s mix of `a` and `b` (Perceptual, or around
/// the hue wheel): the span halved where a straight line strays, at most [`PERCEPTUAL_DEPTH`]
/// times, in order.
fn perceptual_steps(method: GradientMethod, a: &Color, b: &Color, (v0, v1): (f32, f32), depth: u32, out: &mut Vec<f32>) {
    let at = |v: f32| method.mix(a, b, v).to_rgb();
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
        perceptual_steps(method, a, b, (v0, m), depth + 1, out);
        out.push(m);
        perceptual_steps(method, a, b, (m, v1), depth + 1, out);
    }
}

/// The mix positions (`v`, 0..1) of the stops the curve of a gradient mixed by `method`
/// (Perceptual, or around the hue wheel) has between stops `a` and `b`, in order:
/// [`perceptual_steps`] on either side of the midpoint, and the midpoint (½).
fn perceptual_mixes(method: GradientMethod, a: &GradientStop, b: &GradientStop) -> Vec<f32> {
    let mut vs = Vec::new();
    perceptual_steps(method, &a.color, &b.color, (0.0, 0.5), 1, &mut vs);
    vs.push(0.5);
    perceptual_steps(method, &a.color, &b.color, (0.5, 1.0), 1, &mut vs);
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
    /// Process inks (C, M, Y, K, 0..1) printed together with the linked spot swatch's tint: a
    /// DeviceN colour of a file that mixes a spot ink with process plates. `color` is how the mix
    /// looks; separations print the spot ink at `tint` and these plates. `None`: the ink alone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plates: Option<[f32; 4]>,
}

impl GradientStop {
    /// An opaque, unlinked stop of `color` at `offset` with a centred midpoint.
    pub fn new(offset: f32, color: Color) -> Self {
        Self { offset, color, opacity: 1.0, midpoint: 0.5, swatch: None, tint: 1.0, plates: None }
    }
    /// Give the stop `color`, linked to `link` (a global swatch and tint) or unlinked, without
    /// process plates.
    pub fn set_color(&mut self, color: Color, link: Option<(String, f32)>) {
        self.color = color;
        self.plates = None;
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

/// For each pair of neighbouring stops (`k`, `k + 1`), the largest offset among stops `1..=k + 1`.
/// [`Gradient::sample_with`] picks the first pair whose end offset `t` doesn't pass; that is the
/// first pair whose reach `t` doesn't pass, and the reaches never decrease, so the pair is found by
/// binary search whether or not the stops are in order.
struct StopReach(Vec<f32>);

impl StopReach {
    fn new(stops: &[GradientStop]) -> Self {
        // `f32::max` skips a NaN offset, which `t <= offset` never chooses either.
        let reach = stops.iter().skip(1).scan(f32::NEG_INFINITY, |m, s| {
            *m = m.max(s.offset);
            Some(*m)
        });
        Self(reach.collect())
    }

    /// What [`Gradient::sample_with`] gives at `t` for `stops`, the stops this was built from.
    fn sample_with(&self, stops: &[GradientStop], t: f32, mix: &impl Fn(&Color, &Color, f32) -> Color) -> (Color, f32) {
        let Some(first) = stops.first() else { return (Color::BLACK, 1.0) };
        if t <= first.offset {
            return (first.color, first.opacity);
        }
        let last = || stops.last().map_or((Color::BLACK, 1.0), |l| (l.color, l.opacity));
        if t.is_nan() {
            return last();
        }
        let k = self.0.partition_point(|r| *r < t);
        match (stops.get(k), stops.get(k + 1)) {
            (Some(a), Some(b)) => between(a, b, t, mix),
            _ => last(),
        }
    }
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
    /// What it paints past the ends of its stops (Pad unless set; written only when not). A
    /// freeform gradient has no ends and ignores it.
    #[serde(default, skip_serializing_if = "GradientSpread::is_pad")]
    pub spread: GradientSpread,
    /// How many times the stops run along the vector with Repeat and Reflect, 1 to
    /// [`MAX_REPEATS`] (use [`Self::repeat_count`]: a value set in code may be out of range); Pad
    /// ignores it. Written only when it is not 1.
    #[serde(default = "one_repeat", skip_serializing_if = "is_one_repeat", deserialize_with = "lenient_repeats")]
    pub repeats: u32,
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
        Self { kind, stops, dither: false, method: GradientMethod::Classic, spread: GradientSpread::Pad, repeats: 1 }
    }
    /// [`Self::repeats`] kept within 1 to [`MAX_REPEATS`].
    pub fn repeat_count(&self) -> u32 {
        self.repeats.clamp(1, MAX_REPEATS)
    }
    /// Does it paint something other than its end colors past the ends of its stops, or run its
    /// stops more than once: a linear, radial, conic or diamond gradient that repeats or reflects?
    pub fn is_spread(&self) -> bool {
        !self.spread.is_pad() && self.kind != GradientKind::Freeform
    }
    /// The spread and count as the gradient paints them: Pad and 1 for one that pads (whatever
    /// count it keeps) and for a freeform one, else its spread and [`Self::repeat_count`]. Equal
    /// for two gradients whose spreads paint alike.
    pub fn painted_spread(&self) -> (GradientSpread, u32) {
        if self.is_spread() { (self.spread, self.repeat_count()) } else { (GradientSpread::Pad, 1) }
    }
    /// The position (0..1) along the stops that gradient parameter `t` (0 at the start of the
    /// vector, 1 at its end; [`GradientGeom::param_at`]) samples: `t` itself with Pad (the stops
    /// clamp it), else `t` × [`Self::repeat_count`] wrapped back into 0..1, every other run
    /// mirrored with Reflect. A `t` that is not finite is returned as it is.
    pub fn spread_param(&self, t: f64) -> f64 {
        self.spreader()(t)
    }
    /// How many wedges a conic gradient is written as ([`GradientGeom::conic_wedges`]) where
    /// `per_turn` would do for stops that run once around: that many for each time the stops run
    /// around, at most [`MAX_CONIC_WEDGES`]. Sample each wedge at [`Self::spread_param`] of its
    /// turn.
    pub fn conic_wedge_count(&self, per_turn: usize) -> usize {
        let runs = if self.is_spread() { self.repeat_count() as usize } else { 1 };
        per_turn.saturating_mul(runs).clamp(1, MAX_CONIC_WEDGES)
    }
    /// [`Self::spread_param`] as a function that holds no borrow of the gradient.
    pub fn spreader(&self) -> impl Fn(f64) -> f64 + Copy + use<> {
        let spread = if self.is_spread() { self.spread } else { GradientSpread::Pad };
        let n = f64::from(self.repeat_count());
        move |t: f64| {
            if !t.is_finite() {
                return t;
            }
            match spread {
                GradientSpread::Pad => t,
                GradientSpread::Repeat => (t * n).rem_euclid(1.0),
                GradientSpread::Reflect => {
                    let w = (t * n).rem_euclid(2.0);
                    if w > 1.0 { 2.0 - w } else { w }
                }
            }
        }
    }
    /// Colour and opacity at `t` (honours midpoints and the method).
    pub fn sample(&self, t: f32) -> (Color, f32) {
        self.sample_with(t, |a, b, v| self.method.mix(a, b, v))
    }
    /// [`Self::sample`] for many points in no order (a gradient drawn pixel by pixel): the stops
    /// are prepared once, so each sample costs a binary search rather than a scan of the stops.
    pub fn sampler(&self) -> impl Fn(f32) -> (Color, f32) + '_ {
        let reach = StopReach::new(&self.stops);
        move |t| reach.sample_with(&self.stops, t, &|a, b, v| self.method.mix(a, b, v))
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
    /// Which of its stops color some part of it that draws, by the stretches [`Self::sample`]
    /// mixes in, for stops in any order: the pair of stop `k` and stop `k + 1` covers the offsets
    /// above every offset of stops `0..=k` up to stop `k + 1`'s, and both its stops count when that
    /// stretch has positive width and either stop is above opacity 0. The first stop also counts
    /// when it is above opacity 0, since it colors the offsets up to its own, and so does the last
    /// stop, since it colors the offsets above every stop's. Any other stop, opaque or not, counts
    /// only through a pair: a stop out of order whose pairs have no width (another stop before it
    /// reaches past both ends) colors nothing. A freeform gradient draws from its points, so each of
    /// its stops counts while any stop is above opacity 0.
    pub fn drawn_stops(&self) -> Vec<bool> {
        self.stops_with_color(|s| s.opacity > 0.0)
    }
    /// [`Self::drawn_stops`] with every stop taken as opaque, for output that leaves stop opacity
    /// out: the stops that color a stretch of positive width, and the first and last stops.
    pub fn reached_stops(&self) -> Vec<bool> {
        self.stops_with_color(|_| true)
    }
    /// [`Self::drawn_stops`] with `shows` telling whether a stop's opacity shows its color.
    fn stops_with_color(&self, shows: impl Fn(&GradientStop) -> bool) -> Vec<bool> {
        let stops = &self.stops;
        if self.kind == GradientKind::Freeform {
            let any = stops.iter().any(&shows);
            return vec![any; stops.len()];
        }
        let mut drawn = vec![false; stops.len()];
        if let (Some(first), Some(d)) = (stops.first(), drawn.first_mut()) {
            *d = shows(first);
        }
        if let (Some(last), Some(d)) = (stops.last(), drawn.last_mut()) {
            *d |= shows(last);
        }
        // `f32::max` skips a NaN offset, which the sampler never picks either.
        let mut reach = f32::NEG_INFINITY;
        for k in 0..stops.len().saturating_sub(1) {
            reach = reach.max(stops[k].offset);
            let (a, b) = (&stops[k], &stops[k + 1]);
            if b.offset > reach && (shows(a) || shows(b)) {
                drawn[k] = true;
                drawn[k + 1] = true;
            }
        }
        drawn
    }
    /// [`Self::sample_with`] at each of `ts`, which must not decrease. Each search starts where
    /// the one before it stopped, so the work is the samples plus the stops, not their product.
    pub fn sample_increasing_with(&self, ts: impl IntoIterator<Item = f32>, mix: impl Fn(&Color, &Color, f32) -> Color) -> Vec<(Color, f32)> {
        let stops = &self.stops;
        // The shared cursor assumes ordered stops; others (a file's, unsorted) look each sample up
        // in a [`StopReach`], so they cost a binary search each rather than a scan of the stops.
        if stops.windows(2).any(|w| w[1].offset < w[0].offset) {
            let reach = StopReach::new(stops);
            return ts.into_iter().map(|t| reach.sample_with(stops, t, &mix)).collect();
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
        self.expanded_with_stops().map(|(t, c, o, mid, _)| (t, c, o, mid))
    }
    /// [`Self::expanded`], each of the gradient's own stops with its index in [`Self::stops`]
    /// (`None` on the stops in between).
    fn expanded_with_stops(&self) -> impl Iterator<Item = (f32, Color, f32, Option<f32>, Option<usize>)> + '_ {
        // Direct interpolation matches `sample` only for strictly increasing offsets.
        let increasing = self.stops.windows(2).all(|w| w[0].offset < w[1].offset);
        // Otherwise (repeated or unsorted offsets) each midpoint is looked up as `sample` would
        // find it, but by binary search: a scan of all the stops per midpoint cost their square.
        let reach = if increasing || !self.method.is_classic() { None } else { Some(StopReach::new(&self.stops)) };
        self.stops.iter().enumerate().flat_map(move |(i, s)| {
            let next = self.stops.get(i + 1);
            let off_centre = (s.midpoint - 0.5).abs() > 1e-3;
            // Classic: a stop at the midpoint, when it is off-centre.
            let classic = next.filter(|_| off_centre && self.method.is_classic()).map(|n| {
                let t = s.offset + (n.offset - s.offset) * s.midpoint;
                // Mixed from the two stops it lies between, as `sample` would: searching all the
                // stops again for each midpoint made many stops cost their square.
                let mix = |a: &Color, b: &Color, v| self.method.mix(a, b, v);
                let (c, o) = match &reach {
                    Some(reach) => reach.sample_with(&self.stops, t, &mix),
                    None => between(s, n, t, &mix),
                };
                (t, c, o, Some(s.midpoint), None)
            });
            // Perceptual and the hue methods: stops along the mix (`v`) where a straight line would
            // stray from it, the midpoint (v = ½) among them, placed through the midpoint as
            // [`Self::sample`] maps them.
            let perceptual = next.filter(|_| !self.method.is_classic()).map(|n| {
                perceptual_mixes(self.method, s, n).into_iter().map(move |v| {
                    let mid = v == 0.5;
                    let t = curve_offset(s, n, v);
                    let o = s.opacity + (n.opacity - s.opacity) * v;
                    (t, self.method.mix(&s.color, &n.color, v), o, (mid && off_centre).then_some(s.midpoint), None)
                })
            });
            std::iter::once((s.offset, s.color, s.opacity, None, Some(i))).chain(classic).chain(perceptual.into_iter().flatten())
        })
    }
    /// A gradient of tints of one ink `ink` (its color at 100%; paper white is 0% of it) as
    /// (offset, tint, opacity) at each of [`Self::expanded_stops`]: the tint (0..1) whose OKLab
    /// lightness is the lightness of the color there, found to [`TINT_STEPS`] halvings, except at
    /// the gradient's own stops, which take their tints from `stop_tints` (one per stop, in
    /// order) where it has one. Mixed linearly between them, the tints follow a Perceptual
    /// gradient's curve. `None` when the ink's lightness is within [`MIN_INK_LIGHTNESS_SPAN`] of
    /// paper white's (a white or nearly white ink), so lightness does not tell its tints apart,
    /// and when the curve has more than [`MAX_INK_TINTS`] stops ([`Self::fits_ink_tints`]).
    pub fn ink_tints(&self, ink: &Color, stop_tints: &[f32]) -> Option<Vec<(f32, f32, f32)>> {
        use crate::libraries::srgb_to_oklab;
        let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
        if lightness(&ink.tinted(0.0)) - lightness(ink) < MIN_INK_LIGHTNESS_SPAN {
            return None;
        }
        let curve: Vec<_> = self.expanded_with_stops().take(MAX_INK_TINTS + 1).collect();
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
        Some(curve.into_iter().map(|(t, c, o, _, i)| (t, i.and_then(|i| stop_tints.get(i).copied()).unwrap_or_else(|| tint(&c)), o)).collect())
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
    /// [`MIN_INK_LIGHTNESS_SPAN`], or when the display RGB mix's lightness turns back somewhere
    /// on the way from one color's lightness to the other's (checked at [`INK_MIX_STEPS`] even
    /// steps), as from red to blue. `None` for a Classic gradient, for one mixed around the hue
    /// wheel (its colors between two stops are not mixes of the stops' inks), and when the curve
    /// has more than [`MAX_INK_TINTS`] stops ([`Self::fits_ink_tints`]).
    pub fn ink_mixes(&self) -> Option<Vec<(f32, usize, f32, f32)>> {
        use crate::libraries::srgb_to_oklab;
        if self.method != GradientMethod::Perceptual || !self.fits_ink_tints() {
            return None;
        }
        let lightness = |c: &Color| srgb_to_oklab(c.to_rgb())[0];
        let mut out = Vec::new();
        for (i, s) in self.stops.iter().enumerate() {
            out.push((s.offset, i, 0.0, s.opacity));
            let Some(n) = self.stops.get(i + 1) else { continue };
            let (l0, l1) = (lightness(&s.color), lightness(&n.color));
            let mixed_lightness = |w: f32| lightness(&GradientMethod::Classic.mix(&s.color, &n.color, w));
            // The mix position itself when the lightness doesn't tell the mixes apart.
            let by_position = (l1 - l0).abs() < MIN_INK_LIGHTNESS_SPAN
                || !(0..=INK_MIX_STEPS)
                    .map(|k| mixed_lightness(k as f32 / INK_MIX_STEPS as f32))
                    .collect::<Vec<_>>()
                    .windows(2)
                    .all(|w| (w[1] - w[0]) * (l1 - l0) >= 0.0);
            for v in perceptual_mixes(self.method, s, n) {
                let weight = if by_position {
                    v
                } else {
                    let want = lightness(&self.method.mix(&s.color, &n.color, v));
                    let (mut lo, mut hi) = (0.0f32, 1.0f32);
                    for _ in 0..TINT_STEPS {
                        let mid = (lo + hi) / 2.0;
                        // The mix at `mid` has not yet gone from stop `i`'s lightness to the wanted one.
                        if (mixed_lightness(mid) - want) * (l1 - l0) < 0.0 {
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
    /// A gradient whose stops are all CMYK as a ramp of C, M, Y, K and opacity: the stops' own
    /// inks, an off-center midpoint halfway between its two stops' inks, and a Perceptual
    /// gradient's curve as [`Self::ink_mixes`] mixes the inks. `None` when a stop is in another
    /// model, there are no stops, or [`Self::ink_mixes`] gives none for the curve.
    fn ink_ramp(&self) -> Option<Ramp<5>> {
        let inks: Vec<[f32; 4]> = self
            .stops
            .iter()
            .map(|s| match s.color {
                Color::Cmyk { c, m, y, k } => Some([c, m, y, k]),
                _ => None,
            })
            .collect::<Option<_>>()?;
        if inks.is_empty() {
            return None;
        }
        let entry = |i: usize, w: f32, o: f32| {
            let (a, b) = (inks[i], inks.get(i + 1).copied().unwrap_or(inks[i]));
            let [c, m, y, k]: [f32; 4] = std::array::from_fn(|n| a[n] + (b[n] - a[n]) * w);
            [c, m, y, k, o]
        };
        if !self.method.is_classic() {
            return Some(Ramp { stops: self.ink_mixes()?.into_iter().map(|(t, i, w, o)| (t, entry(i, w, o))).collect() });
        }
        let mut stops = Vec::with_capacity(inks.len());
        for (i, s) in self.stops.iter().enumerate() {
            stops.push((s.offset, entry(i, 0.0, s.opacity)));
            if let Some(n) = self.stops.get(i + 1)
                && (s.midpoint - 0.5).abs() > 1e-3
            {
                stops.push((s.offset + (n.offset - s.offset) * s.midpoint, entry(i, 0.5, (s.opacity + n.opacity) / 2.0)));
            }
        }
        Some(Ramp { stops })
    }
    /// The same ramp the other way round: the stops in reverse order at mirrored offsets, each
    /// segment's midpoint mirrored with it, and a hue method turned the other way (red to blue
    /// through magenta, reversed, is blue to red through magenta).
    pub fn reverse(&mut self) {
        self.method = self.method.mirrored();
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
    /// The gradient's colors as a Classic gradient: its curve's stops ([`Self::expanded_stops`]) as
    /// stops of their own, mixed in display RGB, within [`PERCEPTUAL_TOLERANCE`] of it. The stops
    /// keep no swatch links, tints or process plates.
    pub fn as_classic_curve(&self) -> Gradient {
        let stops = self.expanded().map(|(t, color, opacity, _)| GradientStop { opacity, ..GradientStop::new(t, color) }).collect();
        Gradient { kind: self.kind, stops, dither: self.dither, method: GradientMethod::Classic, spread: self.spread, repeats: self.repeats }
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
        let expanded = self.expanded_stops();
        // Where stops share the offset `from` (a sharp step, as where the runs of a repeating
        // gradient meet), the piece starts with the color after the step; sampling gives the one
        // before it.
        let first = match expanded.iter().rev().find(|(t, ..)| *t == from) {
            Some(&(_, color, opacity)) if d > 1e-6 => GradientStop { opacity, ..GradientStop::new(0.0, color) },
            _ => stop(0.0, from),
        };
        let mut stops = vec![first];
        if d > 1e-6 {
            stops.extend(
                expanded
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

/// Most wedges [`GradientGeom::conic_wedges`] draws a conic gradient as.
pub const MAX_CONIC_WEDGES: usize = 4096;

fn one64() -> f64 {
    1.0
}

impl GradientGeom {
    /// Default placement for a bounding box: linear spans the box through the centre at `angle_deg`,
    /// radial and conic are centered with radius = half the larger dimension and their vector at
    /// `angle_deg`, and diamond is centered with its vector at `angle_deg`, its square's edge
    /// running through the box's farthest corner (at 0 or 90 degrees, through all four).
    pub fn fit(kind: GradientKind, b: Rect, angle_deg: f64) -> Self {
        let c = b.center();
        let a = angle_deg.to_radians();
        let d = Vec2::new(a.cos(), -a.sin());
        match kind {
            GradientKind::Radial | GradientKind::Conic => Self { start: c, end: c + d * (b.width().max(b.height()) / 2.0), aspect: 1.0, focal: None },
            GradientKind::Diamond => {
                // The square through the box's farthest corner: the largest reach along the vector
                // plus across it, at one corner.
                let (w, h) = (b.width() / 2.0, b.height() / 2.0);
                let level = |x: f64, y: f64| (x * d.x + y * d.y).abs() + (y * d.x - x * d.y).abs();
                let half = level(w, h).max(level(w, -h));
                Self { start: c, end: c + d * half, aspect: 1.0, focal: None }
            }
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
    /// new end (on the axis nearest the mapped vector) and aspect. Conic gradients keep their
    /// vector exactly (where the sweep starts, so the seam stays in place) and take the mapped
    /// extent across it as their aspect, its sign flipping under a mirror so the sweep turns the
    /// other way: exact for moves, rotations, uniform scales and mirrors, and the nearest frame
    /// that keeps the seam under a shear or a non-uniform scale. A conic whose vector has no
    /// length, or whose aspect is 0, still has the sign of its aspect flipped by a mirror.
    /// Diamond gradients keep their vector (a corner of the square) the same way, with the size
    /// of the mapped extent across it as their aspect: exact for moves, rotations, scales along or
    /// across the vector and mirrors, and the nearest square stretched across its vector under a
    /// shear or another scale.
    pub fn transform(&mut self, a: Affine, kind: GradientKind) {
        // The focal point is a point like any other.
        self.focal = self.focal.map(|f| a * f);
        let [m0, m1, m2, m3, _, _] = a.as_coeffs();
        let lin = |v: Vec2| Vec2::new(m0 * v.x + m2 * v.y, m1 * v.x + m3 * v.y);
        let perp = |v: Vec2| Vec2::new(-v.y, v.x);
        let start = a * self.start;
        let u = self.end - self.start;
        let mirror = m0 * m3 - m1 * m2 < 0.0;
        if u.hypot2() < 1e-18 {
            self.start = start;
            self.end = start;
            if kind == GradientKind::Conic && mirror {
                self.aspect = -self.aspect;
            }
            return;
        }
        if kind == GradientKind::Conic {
            let (lu, lw) = (lin(u), lin(perp(u) * self.aspect));
            self.start = start;
            self.end = start + lu;
            let aspect = lw.dot(perp(lu)) / lu.hypot2();
            self.aspect = match (lu.hypot2() > 1e-18 && aspect.is_finite(), aspect == 0.0) {
                (false, _) => 1.0,
                // A flat extent keeps its sign, so a mirror still turns the sweep the other way.
                (true, true) => 0.0f64.copysign(if self.aspect.is_sign_negative() != mirror { -1.0 } else { 1.0 }),
                (true, false) => aspect,
            };
            return;
        }
        if kind == GradientKind::Diamond {
            let (lu, lw) = (lin(u), lin(perp(u) * self.aspect));
            self.start = start;
            self.end = start + lu;
            let aspect = (lw.dot(perp(lu)) / lu.hypot2()).abs();
            self.aspect = if lu.hypot2() > 1e-18 && aspect.is_finite() { aspect } else { 1.0 };
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
    /// focal point to the centre). For conic gradients it is the turn from the vector in
    /// [`Self::conic_frame`], clockwise on the page (counterclockwise when mirrored), in 0..=1.
    /// For diamond gradients it is the size of the square through `p`, as a fraction of the one
    /// [`Self::diamond_frame`] gives (1 everywhere for a diamond without a frame).
    pub fn param_at(&self, kind: GradientKind, p: Point) -> f64 {
        if kind == GradientKind::Conic {
            let q = (self.conic_frame().inverse() * p).to_vec2();
            let t = q.y.atan2(q.x) / std::f64::consts::TAU;
            let t = if t < 0.0 { t + 1.0 } else { t };
            return if t.is_finite() { t.clamp(0.0, 1.0) } else { 0.0 };
        }
        if kind == GradientKind::Diamond {
            // Without a frame the square has no inside: every point is past its edge.
            let Some(m) = self.diamond_frame() else { return 1.0 };
            let q = (m.inverse() * p).to_vec2();
            let t = q.x.abs() + q.y.abs();
            return if t.is_nan() { 1.0 } else { t };
        }
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

    /// The map from the unit circle onto a radial's (or conic's) extent ellipse: (1, 0) to the
    /// end, (0, 1) to `aspect` × the radius across the vector (a conic's negative aspect mirrors
    /// it). None when the ellipse is flat (no inverse).
    pub fn unit_frame(&self) -> Option<Affine> {
        let u = self.end - self.start;
        (u.hypot2() * self.aspect.abs() > 1e-18).then(|| Affine::new([u.x, u.y, -u.y * self.aspect, u.x * self.aspect, self.start.x, self.start.y]))
    }

    /// The frame a conic gradient sweeps in: its [`Self::unit_frame`], or, when the extent
    /// ellipse is flat or not finite (a vector without length, an aspect of 0, or a file's
    /// out-of-range numbers), the unit circle about the start turning from the vector's direction
    /// (from the right when the vector has none), mirrored by a negative aspect. The canvas,
    /// Expand and every export draw a conic in this frame.
    pub fn conic_frame(&self) -> Affine {
        if let Some(m) = self.unit_frame().filter(|m| m.is_finite() && m.determinant().abs() > 1e-18) {
            return m;
        }
        if !self.start.is_finite() {
            return Affine::IDENTITY;
        }
        let u = self.end - self.start;
        let len = u.hypot();
        let d = if len.is_finite() && len > 1e-9 { u / len } else { Vec2::new(1.0, 0.0) };
        let s = if self.aspect.is_sign_negative() && !self.aspect.is_nan() { -1.0 } else { 1.0 };
        Affine::new([d.x, d.y, -d.y * s, d.x * s, self.start.x, self.start.y])
    }

    /// Is the sweep mirrored (counterclockwise on the page): a conic's negative aspect?
    pub fn is_mirrored(&self) -> bool {
        self.conic_frame().determinant() < 0.0
    }

    /// A conic gradient as `n` solid wedges around its center (at most [`MAX_CONIC_WEDGES`]), for
    /// formats without conic gradients: each wedge's outline (the center, then an arc that
    /// reaches past every corner of `b`) and the turn its color is sampled at (its middle). With
    /// `overlap` (opaque stops), each wedge runs on to the seam and the next is drawn over it, so
    /// no hairline of what lies below shows between neighbors and the seam stays sharp.
    pub fn conic_wedges(&self, b: Rect, n: usize, overlap: bool) -> Vec<(Vec<Point>, f64)> {
        let n = n.clamp(1, MAX_CONIC_WEDGES);
        let frame = self.conic_frame();
        let r = fan_radius(frame, b);
        (0..n)
            .map(|k| {
                let from = k as f64 / n as f64;
                let to = if overlap { 1.0 } else { (k + 1) as f64 / n as f64 };
                (fan(frame, r, from, to), (k as f64 + 0.5) / n as f64)
            })
            .collect()
    }

    /// The frame a diamond gradient is drawn in: its [`Self::unit_frame`], which maps the square
    /// with corners (±1, 0) and (0, ±1) onto the diamond (its first corner at the end of the
    /// vector). `None` when the square is flat or not finite (a vector without length, an aspect
    /// of 0, or a file's out-of-range numbers): such a diamond has no inside, and every point
    /// takes its last stop.
    pub fn diamond_frame(&self) -> Option<Affine> {
        self.unit_frame().filter(|m| m.is_finite() && m.determinant().abs() > 1e-18)
    }

    /// A diamond gradient as four linear gradients, one for each quadrant of its
    /// [`Self::diamond_frame`], for formats without diamond gradients: each quadrant's outline (the
    /// center, then an arc that reaches past every corner of `b`) and the linear gradient that
    /// equals the diamond there (its stops run across the square's edge in that quadrant), in the
    /// order they are drawn. Each of the first three runs on past its last edge under the next,
    /// and the first also back past its first edge under the last, so every edge drawn lies on
    /// the quadrant before it and no hairline of what lies below shows between them; where they
    /// overlap, translucent stops would add up, so the stops must be opaque. `None` for a diamond
    /// without a frame.
    pub fn diamond_quadrants(&self, b: Rect) -> Option<[(Vec<Point>, GradientGeom); 4]> {
        let frame = self.diamond_frame()?;
        let r = fan_radius(frame, b);
        Some(std::array::from_fn(|k| {
            let first = k as f64 / 4.0;
            let (from, to) = match k {
                0 => (-0.125, 0.375),
                3 => (0.75, 1.0),
                _ => (first, first + 0.375),
            };
            // The quadrant's corner of the square, halfway along whose edge t is 1/2.
            let (sx, sy) = [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)][k];
            let mut geom = GradientGeom { start: Point::ORIGIN, end: Point::new(sx / 2.0, sy / 2.0), aspect: 1.0, focal: None };
            geom.transform(frame, GradientKind::Linear);
            (fan(frame, r, from, to), geom)
        }))
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
    /// The placement whose level `t` is this one's level `k`·`t` (`k` > 0), for a linear, radial or
    /// diamond gradient: the vector `k` times as long from the same start, and for an off-center
    /// radial the extent ellipse scaled `k` times about the focal point, which stays. A conic keeps
    /// its placement (its level is a turn).
    pub fn levels_scaled(&self, kind: GradientKind, k: f64) -> GradientGeom {
        if kind == GradientKind::Conic || !(k.is_finite() && k > 0.0) {
            return *self;
        }
        let u = self.end - self.start;
        let mut out = *self;
        if kind == GradientKind::Radial
            && let (Some(f), Some(frame)) = (self.focal, self.unit_frame())
        {
            // Level t of an off-center radial is the extent ellipse scaled by t about the line
            // from the focal point to the center: the new center sits on that line, `k` times as
            // far from the focal point.
            let f = (frame.inverse() * f).to_vec2();
            out.start = frame * (f * (1.0 - k)).to_point();
        }
        out.end = out.start + u * k;
        out
    }

    /// The placement over which one run of the stops of a gradient that repeats `n` times along
    /// its vector lies ([`Self::levels_scaled`] by 1/`n`).
    pub fn period(&self, kind: GradientKind, n: u32) -> GradientGeom {
        self.levels_scaled(kind, 1.0 / f64::from(n.max(1)))
    }

    pub fn angle_deg(&self) -> f64 {
        let v = self.end - self.start;
        (-v.y).atan2(v.x).to_degrees()
    }
    pub fn length(&self) -> f64 {
        (self.end - self.start).hypot()
    }
}

/// Arc points of [`fan`] are at most this much of a turn apart.
const FAN_ARC: f64 = 1.0 / 32.0;

/// The radius of the unit circle of `frame` at which the chords of [`fan`]'s arcs stay outside
/// every corner of `b`.
fn fan_radius(frame: Affine, b: Rect) -> f64 {
    let to_unit = frame.inverse();
    let corners = [b.origin(), Point::new(b.x1, b.y0), Point::new(b.x1, b.y1), Point::new(b.x0, b.y1)];
    let reach = corners.iter().map(|&c| (to_unit * c).to_vec2().hypot()).fold(0.0f64, f64::max).max(1e-6);
    let reach = if reach.is_finite() { reach * 1.01 } else { 1.0 };
    reach / (std::f64::consts::PI * FAN_ARC).cos()
}

/// The sector of `frame`'s unit circle, scaled to radius `r`, from turn `from` to turn `to`
/// (counted from the frame's x axis toward its y axis) as an outline: the center, then points on
/// the arc at most [`FAN_ARC`] of a turn apart.
fn fan(frame: Affine, r: f64, from: f64, to: f64) -> Vec<Point> {
    let at = |turn: f64| {
        let a = turn * std::f64::consts::TAU;
        frame * Point::new(r * a.cos(), r * a.sin())
    };
    let steps = ((to - from) / FAN_ARC).ceil().max(1.0) as usize;
    let mut pts = Vec::with_capacity(steps + 2);
    pts.push(frame * Point::ORIGIN);
    pts.extend((0..=steps).map(|i| at(from + (to - from) * i as f64 / steps as f64)));
    pts
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
    /// dithered fill (dither applies to fills only), or a Perceptual one or one mixed around the
    /// hue wheel (its colours don't mix linearly between the stops, as SVG's and PDF's gradients
    /// do)?
    pub fn draws_as_pixels(&self, fill: bool) -> bool {
        self.gradient.kind == GradientKind::Freeform || (fill && self.gradient.dither) || !self.gradient.method.is_classic()
    }
    /// A linear, radial, conic or diamond gradient placed on box `bounds` as a `cols` × `rows` grid of
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
        self.grid(Ramp::new(&self.gradient, rgb), bounds, area, cols, rows, cells, fill, false)
    }
    /// [`Self::pixels`] of a linear, radial, conic or diamond gradient whose stops are all CMYK, as C, M, Y, K
    /// and opacity levels mixed from the stops' inks; `None` when a stop is in another model,
    /// there are no stops, or a Perceptual curve has too many stops for [`Gradient::ink_mixes`].
    /// Dither's noise adds no ink where the ramp has none.
    pub fn ink_pixels(&self, bounds: Rect, area: Rect, cols: u16, rows: u16, fill: bool) -> Option<impl Iterator<Item = [u8; 5]> + use<>> {
        let ramp = self.gradient.ink_ramp()?;
        Some(self.grid(ramp, bounds, area, cols, rows, [0..cols, 0..rows], fill, true))
    }
    /// `ramp` sampled at each cell's center in columns `cells[0]` and rows `cells[1]` of the grid,
    /// its last channel (opacity) rounded and each other channel with dither's noise, except
    /// (`inks`) where that channel is empty.
    #[allow(clippy::too_many_arguments)]
    fn grid<const N: usize>(
        &self,
        ramp: Ramp<N>,
        bounds: Rect,
        area: Rect,
        cols: u16,
        rows: u16,
        cells: [std::ops::Range<u16>; 2],
        fill: bool,
        inks: bool,
    ) -> impl Iterator<Item = [u8; N]> + use<N> {
        let (kind, geom, dither) = (self.gradient.kind, self.resolve(bounds), fill && self.gradient.dither);
        let spread = self.gradient.spreader();
        let (cw, ch) = (area.width() / f64::from(cols.max(1)), area.height() / f64::from(rows.max(1)));
        let (x0, y0) = (area.x0, area.y0);
        let [across, down] = cells.map(|r| u32::from(r.start)..u32::from(r.end));
        down.flat_map(move |j| across.clone().map(move |i| (i, j))).map(move |(i, j)| {
            let p = Point::new(x0 + (f64::from(i) + 0.5) * cw, y0 + (f64::from(j) + 0.5) * ch);
            let c = ramp.at(spread(geom.param_at(kind, p)) as f32);
            std::array::from_fn(|k| {
                if k + 1 == N {
                    return (c[k].clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                let noise = if dither && !(inks && c[k] <= 0.0) { dither_noise(i, j, k as u32) } else { 0.0 };
                (c[k].clamp(0.0, 1.0) * 255.0 + noise).round().clamp(0.0, 255.0) as u8
            })
        })
    }
}

/// A gradient's colours ready to be sampled pixel by pixel, in a colour space of the caller's
/// (display RGB, or an ink's coverage): its stops, midpoints and a Perceptual gradient's
/// in-between colours as stops (as [`Gradient::expanded_stops`]), each turned into that space
/// once, with colour and opacity mixed linearly between them, each on its own (as the renderer's
/// vector gradients mix them). `N` counts the channels, with opacity last.
pub struct Ramp<const N: usize = 4> {
    stops: Vec<(f32, [f32; N])>,
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
}

impl<const N: usize> Ramp<N> {
    /// Unpremultiplied colour and opacity at `t`, padded past the ends (transparent black
    /// without stops).
    pub fn at(&self, t: f32) -> [f32; N] {
        let Some(&(t0, first)) = self.stops.first() else { return [0.0; N] };
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
        self.stops.last().map_or([0.0; N], |s| s.1)
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
