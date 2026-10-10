//! The appearance model: a stack of fills and strokes, each with its own opacity, blend mode and
//! effects, plus object-level effects.

use serde::{Deserialize, Serialize};
use vectorcraft_color::{BlendMode, Color, Paint};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StrokeAlign {
    #[default]
    Center,
    Inside,
    Outside,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Dash {
    /// Dash, gap, dash, gap… (up to 6 values in the Stroke panel).
    pub pattern: Vec<f64>,
    #[serde(default)]
    pub offset: f64,
    /// "Aligns dashes to corners and path ends, adjusting lengths to fit".
    #[serde(default)]
    pub align_corners: bool,
}

impl Dash {
    /// Whether the pattern draws dashes. As in PDF (`d` operator) and SVG (`stroke-dasharray`), a
    /// pattern with a negative or non-finite value, or whose values are all zero, is invalid and the
    /// stroke is drawn solid.
    pub fn is_dashed(&self) -> bool {
        self.pattern.iter().all(|v| v.is_finite() && *v >= 0.0) && self.pattern.iter().any(|v| *v > 0.0)
    }
    /// Scale the dash and gap lengths and the offset by `s`.
    pub fn scale(&mut self, s: f64) {
        self.pattern.iter_mut().for_each(|v| *v *= s);
        self.offset *= s;
    }
}

/// An arrowhead shape. Saved by variant name: new shapes are appended, never renamed or reordered.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arrowhead {
    Triangle,
    TriangleOpen,
    Circle,
    CircleOpen,
    Square,
    SquareOpen,
    Bar,
    Diamond,
    Arrow,
    ArrowOpen,
    Barbed,
    /// One barb, on the left of the direction the head points.
    HalfArrowLeft,
    /// One barb, on the right of the direction the head points.
    HalfArrowRight,
    Concave,
    DoubleBar,
    Feather,
    DotOnBar,
    Chevron,
    DoubleArrow,
    Target,
    Star,
    Cross,
    Plus,
    Hexagon,
    HexagonOpen,
    Tag,
    TagOpen,
    HalfCircle,
    Drop,
    Slash,
    DoubleSlash,
    DiamondOpen,
    TriangleReverse,
    Swallowtail,
    Bracket,
    Fork,
    Leaf,
    Kite,
    TriangleBar,
    Oval,
}

impl Arrowhead {
    /// Every arrowhead, in menu order (arrows, then outlined shapes, then bars and marks).
    pub const ALL: [Arrowhead; 40] = [
        Arrowhead::Arrow,
        Arrowhead::ArrowOpen,
        Arrowhead::Barbed,
        Arrowhead::Concave,
        Arrowhead::DoubleArrow,
        Arrowhead::HalfArrowLeft,
        Arrowhead::HalfArrowRight,
        Arrowhead::Chevron,
        Arrowhead::Feather,
        Arrowhead::Swallowtail,
        Arrowhead::Triangle,
        Arrowhead::TriangleOpen,
        Arrowhead::TriangleReverse,
        Arrowhead::TriangleBar,
        Arrowhead::Kite,
        Arrowhead::Leaf,
        Arrowhead::Drop,
        Arrowhead::Circle,
        Arrowhead::CircleOpen,
        Arrowhead::HalfCircle,
        Arrowhead::Oval,
        Arrowhead::Target,
        Arrowhead::Square,
        Arrowhead::SquareOpen,
        Arrowhead::Tag,
        Arrowhead::TagOpen,
        Arrowhead::Diamond,
        Arrowhead::DiamondOpen,
        Arrowhead::Hexagon,
        Arrowhead::HexagonOpen,
        Arrowhead::Star,
        Arrowhead::Cross,
        Arrowhead::Plus,
        Arrowhead::Bar,
        Arrowhead::DoubleBar,
        Arrowhead::DotOnBar,
        Arrowhead::Slash,
        Arrowhead::DoubleSlash,
        Arrowhead::Bracket,
        Arrowhead::Fork,
    ];

    /// The name the arrowhead menus show.
    pub fn label(self) -> &'static str {
        match self {
            Arrowhead::Triangle => "Triangle",
            Arrowhead::TriangleOpen => "Triangle (open)",
            Arrowhead::Circle => "Circle",
            Arrowhead::CircleOpen => "Circle (open)",
            Arrowhead::Square => "Square",
            Arrowhead::SquareOpen => "Square (open)",
            Arrowhead::Bar => "Bar",
            Arrowhead::Diamond => "Diamond",
            Arrowhead::Arrow => "Arrow",
            Arrowhead::ArrowOpen => "Arrow (open)",
            Arrowhead::Barbed => "Barbed",
            Arrowhead::HalfArrowLeft => "Half Arrow (left)",
            Arrowhead::HalfArrowRight => "Half Arrow (right)",
            Arrowhead::Concave => "Concave",
            Arrowhead::DoubleBar => "Double Bar",
            Arrowhead::Feather => "Feather",
            Arrowhead::DotOnBar => "Dot on Bar",
            Arrowhead::Chevron => "Chevron",
            Arrowhead::DoubleArrow => "Double Arrow",
            Arrowhead::Target => "Target",
            Arrowhead::Star => "Star",
            Arrowhead::Cross => "Cross",
            Arrowhead::Plus => "Plus",
            Arrowhead::Hexagon => "Hexagon",
            Arrowhead::HexagonOpen => "Hexagon (open)",
            Arrowhead::Tag => "Tag",
            Arrowhead::TagOpen => "Tag (open)",
            Arrowhead::HalfCircle => "Half Circle",
            Arrowhead::Drop => "Drop",
            Arrowhead::Slash => "Slash",
            Arrowhead::DoubleSlash => "Double Slash",
            Arrowhead::DiamondOpen => "Diamond (open)",
            Arrowhead::TriangleReverse => "Reverse Triangle",
            Arrowhead::Swallowtail => "Swallowtail",
            Arrowhead::Bracket => "Bracket",
            Arrowhead::Fork => "Fork",
            Arrowhead::Leaf => "Leaf",
            Arrowhead::Kite => "Kite",
            Arrowhead::TriangleBar => "Triangle to Bar",
            Arrowhead::Oval => "Oval",
        }
    }
}

/// Where an arrowhead sits relative to the end of its path. In both modes the stroke stops under
/// the head, so the line never shows through a hollow head or past its tip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ArrowAlign {
    /// The tip extends past the end point (the path keeps its length).
    #[default]
    Extend,
    /// The tip sits on the end point (the stroke is shortened by the head).
    Tip,
}

/// Variable-width profile: (position 0..1 along the path, left width factor, right width factor),
/// in order along the path. Two points at the same position make a discontinuous point: the
/// first is the width just before it, the second the width just after it (a step).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct WidthProfile {
    pub points: Vec<(f64, f64, f64)>,
}

/// A built-in width profile (the Stroke panel's Profile list).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProfilePreset {
    /// Stable id used by `stroke.set {profile}`.
    pub id: &'static str,
    /// Menu label.
    pub label: &'static str,
    /// (t, left, right) width points.
    pub points: &'static [(f64, f64, f64)],
}

impl WidthProfile {
    /// (left, right) width factors at `t`, linear between points.
    pub fn at(&self, t: f64) -> (f64, f64) {
        Self::at_points(&self.points, t)
    }
    /// [`Self::at`] of a profile's points (a preset's or a saved profile's).
    pub fn at_points(p: &[(f64, f64, f64)], t: f64) -> (f64, f64) {
        if p.is_empty() {
            return (1.0, 1.0);
        }
        if t <= p[0].0 {
            return (p[0].1, p[0].2);
        }
        for w in p.windows(2) {
            if t <= w[1].0 {
                let u = (t - w[0].0) / (w[1].0 - w[0].0).max(1e-9);
                return (w[0].1 + (w[1].1 - w[0].1) * u, w[0].2 + (w[1].2 - w[0].2) * u);
            }
        }
        p.last().map_or((1.0, 1.0), |l| (l.1, l.2))
    }
    /// The built-in profiles, in menu order. "uniform" is the plain stroke (no profile).
    pub const PRESETS: [ProfilePreset; 7] = [
        ProfilePreset { id: "uniform", label: "Uniform", points: &[(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] },
        ProfilePreset { id: "lens", label: "Lens", points: &[(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        ProfilePreset { id: "taperStart", label: "Taper Start", points: &[(0.0, 0.0, 0.0), (1.0, 1.0, 1.0)] },
        ProfilePreset { id: "taperEnd", label: "Taper End", points: &[(0.0, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        // Full width at both ends, pinched to a quarter in the middle.
        ProfilePreset { id: "pinch", label: "Pinch", points: &[(0.0, 1.0, 1.0), (0.5, 0.25, 0.25), (1.0, 1.0, 1.0)] },
        // A round head a fifth of the way along, then a long taper to the end.
        ProfilePreset { id: "teardrop", label: "Teardrop", points: &[(0.0, 0.0, 0.0), (0.2, 1.0, 1.0), (1.0, 0.0, 0.0)] },
        // Two swells between narrow necks.
        ProfilePreset { id: "wave", label: "Wave", points: &[(0.0, 0.3, 0.3), (0.25, 1.0, 1.0), (0.5, 0.3, 0.3), (0.75, 1.0, 1.0), (1.0, 0.3, 0.3)] },
    ];
    /// The built-in profile with this id.
    pub fn preset(id: &str) -> Option<Self> {
        Self::PRESETS.iter().find(|p| p.id == id).map(|p| Self { points: p.points.to_vec() })
    }
    /// The id of the built-in profile these points match, if any.
    pub fn preset_id(&self) -> Option<&'static str> {
        Self::PRESETS.iter().find(|p| p.points == self.points.as_slice()).map(|p| p.id)
    }
    /// The id of a stroke's profile: "uniform" without one, "custom" when it matches no preset.
    pub fn id_of(p: Option<&Self>) -> &'static str {
        p.map_or(Some("uniform"), Self::preset_id).unwrap_or("custom")
    }
    /// The lens profile (thin ends, full width in the middle).
    pub fn lens() -> Self {
        Self::preset("lens").unwrap_or_default()
    }
    pub fn taper_end() -> Self {
        Self::preset("taperEnd").unwrap_or_default()
    }
    pub fn taper_start() -> Self {
        Self::preset("taperStart").unwrap_or_default()
    }
    /// The (left, right) factors just before and just after `t`: they differ only at a
    /// discontinuous point (two points at `t`, within `1e-7`).
    pub fn around(&self, t: f64) -> ((f64, f64), (f64, f64)) {
        let p = &self.points;
        if let Some(i) = p.iter().position(|q| (q.0 - t).abs() <= 1e-7) {
            let j = i + p[i..].iter().take_while(|q| (q.0 - p[i].0).abs() <= 1e-9).count() - 1;
            if j > i {
                return ((p[i].1, p[i].2), (p[j].1, p[j].2));
            }
        }
        let v = self.at(t);
        (v, v)
    }
}

/// Pen pressure recorded along a stroke (the Paintbrush with a graphics tablet): (position 0..1
/// along the path, pressure 0..1) samples in order, positions measured as a [`WidthProfile`]'s
/// are. Calligraphic brushes with Pressure variation read it; everything else ignores it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PressureProfile {
    pub points: Vec<(f64, f64)>,
}

impl PressureProfile {
    /// The pressure of a stroke drawn without one (a mouse, or a path the brush was applied to):
    /// half way, so a Pressure brush draws its own value there.
    pub const MID: f64 = 0.5;
    /// The most samples a stroke keeps (after simplifying them).
    pub const MAX_POINTS: usize = 512;
    /// How far (in pressure) simplifying may move the recorded curve.
    const TOLERANCE: f64 = 0.01;

    /// The pressure at `t` (0..1), linear between samples; [`Self::MID`] without any.
    pub fn at(&self, t: f64) -> f64 {
        let p = &self.points;
        let (Some(first), Some(last)) = (p.first(), p.last()) else { return Self::MID };
        let v = if t <= first.0 {
            first.1
        } else {
            p.windows(2)
                .find_map(|w| match w {
                    [a, b] if t <= b.0 => Some(a.1 + (b.1 - a.1) * ((t - a.0) / (b.0 - a.0).max(1e-9)).clamp(0.0, 1.0)),
                    _ => None,
                })
                .unwrap_or(last.1)
        };
        if v.is_finite() { v.clamp(0.0, 1.0) } else { Self::MID }
    }

    /// A profile from (position, pressure) samples in order along the stroke: values are clamped
    /// into 0..1 (non-finite ones dropped), the curve is simplified to within 1 % and kept to
    /// [`Self::MAX_POINTS`]. `None` without a sample.
    pub fn from_samples(samples: impl IntoIterator<Item = (f64, f64)>) -> Option<Self> {
        let pts: Vec<(f64, f64)> =
            samples.into_iter().filter(|(t, p)| t.is_finite() && p.is_finite()).map(|(t, p)| (t.clamp(0.0, 1.0), p.clamp(0.0, 1.0))).collect();
        if pts.is_empty() {
            return None;
        }
        // Simplifying is quadratic at worst: a huge stroke is thinned evenly first.
        let mut pts = thin(pts, 16 * Self::MAX_POINTS);
        // Positions only ever go forward.
        let mut prev = 0.0;
        for p in &mut pts {
            p.0 = p.0.max(prev);
            prev = p.0;
        }
        let keep = simplify_1d(&pts, Self::TOLERANCE);
        let pts = thin(pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect(), Self::MAX_POINTS);
        let round = |v: f64| (v * 1e4).round() / 1e4;
        Some(Self { points: pts.into_iter().map(|(t, p)| (round(t), round(p))).collect() })
    }

    /// The pressure of a path `old_len` long (`old`; None: drawn without pressure) continued at its
    /// end by a stroke `added_len` long (`added`). None when neither has pressure.
    pub fn extended(old: Option<&Self>, old_len: f64, added: Option<&Self>, added_len: f64) -> Option<Self> {
        if old.is_none() && added.is_none() {
            return None;
        }
        let (old_len, added_len) = (old_len.max(0.0), added_len.max(0.0));
        let total = old_len + added_len;
        if !total.is_finite() || total <= 1e-9 {
            return old.or(added).cloned();
        }
        let flat = Self { points: vec![(0.0, Self::MID), (1.0, Self::MID)] };
        let (old, added) = (old.unwrap_or(&flat), added.unwrap_or(&flat));
        let o = old_len / total;
        let old_pts = old.points.iter().map(|&(t, p)| (t * o, p));
        Self::from_samples(old_pts.chain(added.points.iter().map(|&(u, p)| (o + u * (1.0 - o), p))))
    }

    /// The pressure of the path reversed.
    pub fn reversed(&self) -> Self {
        Self { points: self.points.iter().rev().map(|&(t, p)| (1.0 - t, p)).collect() }
    }

    /// A light–heavy–light stroke, for previews of pressure-sensitive brushes.
    pub fn sample() -> Self {
        Self { points: vec![(0.0, 0.0), (0.5, 1.0), (1.0, 0.0)] }
    }
}

/// At most `max` of `pts`, evenly spread (the ends kept).
fn thin(pts: Vec<(f64, f64)>, max: usize) -> Vec<(f64, f64)> {
    let n = pts.len();
    if n <= max || max < 2 {
        return pts;
    }
    (0..max).filter_map(|i| pts.get(i * (n - 1) / (max - 1)).copied()).collect()
}

/// Douglas–Peucker over a function sampled at `pts`: which samples to keep so the polyline through
/// them stays within `tol` (vertically) of every sample. The ends are always kept.
fn simplify_1d(pts: &[(f64, f64)], tol: f64) -> Vec<bool> {
    let mut keep = vec![false; pts.len()];
    let Some(last) = pts.len().checked_sub(1) else { return keep };
    for i in [0, last] {
        if let Some(k) = keep.get_mut(i) {
            *k = true;
        }
    }
    let mut stack = vec![(0, last)];
    while let Some((i, j)) = stack.pop() {
        let (Some(&a), Some(&b)) = (pts.get(i), pts.get(j)) else { continue };
        let line = |t: f64| if b.0 - a.0 > 1e-12 { a.1 + (b.1 - a.1) * (t - a.0) / (b.0 - a.0) } else { (a.1 + b.1) / 2.0 };
        let far = (i + 1..j).filter_map(|k| pts.get(k).map(|q| (k, (q.1 - line(q.0)).abs()))).max_by(|x, y| x.1.total_cmp(&y.1));
        if let Some((k, d)) = far
            && d > tol
        {
            if let Some(f) = keep.get_mut(k) {
                *f = true;
            }
            stack.push((i, k));
            stack.push((k, j));
        }
    }
    keep
}

/// A width profile saved to the Profile list under a name (Add to Profiles), kept with the
/// preferences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SavedProfile {
    pub name: String,
    pub profile: WidthProfile,
}

/// A live effect in an appearance stack. Parameters are interpreted by `vectorcraft-effects`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Effect {
    /// Stable id, e.g. `stylize.dropShadow`, `distort.roughen`, `path.offsetPath`, `warp.arc`.
    pub id: String,
    #[serde(default)]
    pub params: serde_json::Value,
    #[serde(default = "yes")]
    pub visible: bool,
}

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FillLayer {
    pub paint: Paint,
    #[serde(default = "one", skip_serializing_if = "crate::skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub blend: BlendMode,
    #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Overprint Fill: on press the fill's inks print over the inks below instead of knocking
    /// them out (Overprint Preview and Separations Preview show it).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub overprint: bool,
}

impl FillLayer {
    pub fn new(paint: Paint) -> Self {
        Self { paint, opacity: 1.0, blend: BlendMode::Normal, visible: true, effects: vec![], overprint: false }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StrokeLayer {
    pub paint: Paint,
    /// Weight in points.
    pub width: f64,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub cap: LineCap,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub join: LineJoin,
    #[serde(default = "ten", skip_serializing_if = "is_ten")]
    pub miter_limit: f64,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub align: StrokeAlign,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash: Option<Dash>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_arrow: Option<Arrowhead>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_arrow: Option<Arrowhead>,
    /// Arrowhead scale in percent (start, end).
    #[serde(default = "hundreds", skip_serializing_if = "is_hundreds")]
    pub arrow_scale: (f64, f64),
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<WidthProfile>,
    /// Brush applied to the stroke (by brush name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brush: Option<String>,
    #[serde(default = "one", skip_serializing_if = "crate::skip::is_one")]
    pub opacity: f32,
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub blend: BlendMode,
    #[serde(default = "yes", skip_serializing_if = "crate::skip::is_true")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Arrowhead placement at both ends.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub arrow_align: ArrowAlign,
    /// Overprint Stroke (see [`FillLayer::overprint`]).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub overprint: bool,
    /// How a gradient paint maps onto the stroke (within, along or across it).
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub gradient_mode: StrokeGradientMode,
    /// The pen pressure the stroke was drawn with (Paintbrush with a tablet): pressure-sensitive
    /// brushes vary with it. It belongs to this path's shape, so appearances copied to other
    /// art leave it behind ([`Appearance::without_pressure`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pressure: Option<PressureProfile>,
}

/// How a gradient on a stroke is laid out (the Gradient panel's Stroke buttons). Saved by variant
/// name: new modes are appended, never renamed.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StrokeGradientMode {
    /// Placed on the page like a fill's gradient and seen through the stroke.
    #[default]
    Within,
    /// From the start of each subpath to its end, following the path.
    Along,
    /// From the stroke's left edge to its right edge (left of the path's direction), all along it.
    Across,
}

impl StrokeGradientMode {
    pub const ALL: [StrokeGradientMode; 3] = [StrokeGradientMode::Within, StrokeGradientMode::Along, StrokeGradientMode::Across];
    /// The name commands use (`within`, `along`, `across`).
    pub fn name(self) -> &'static str {
        match self {
            StrokeGradientMode::Within => "within",
            StrokeGradientMode::Along => "along",
            StrokeGradientMode::Across => "across",
        }
    }
    /// Parse a mode name (any case).
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name().eq_ignore_ascii_case(s))
    }
}

/// How far any arrowhead reaches from its tip, in units of its weight (the open arrow's arms are
/// cut square a little behind its 4 × 4 box).
pub(crate) const HEAD_REACH: f64 = 4.6;

fn ten() -> f64 {
    10.0
}
fn hundreds() -> (f64, f64) {
    (100.0, 100.0)
}
fn is_ten(v: &f64) -> bool {
    *v == 10.0
}
fn is_hundreds(v: &(f64, f64)) -> bool {
    *v == (100.0, 100.0)
}

impl StrokeLayer {
    pub fn new(paint: Paint, width: f64) -> Self {
        Self {
            paint,
            width,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            align: StrokeAlign::Center,
            dash: None,
            start_arrow: None,
            end_arrow: None,
            arrow_scale: (100.0, 100.0),
            profile: None,
            brush: None,
            opacity: 1.0,
            blend: BlendMode::Normal,
            visible: true,
            effects: vec![],
            arrow_align: ArrowAlign::Extend,
            overprint: false,
            gradient_mode: StrokeGradientMode::Within,
            pressure: None,
        }
    }
    /// Weight of the start (`end == false`) or end arrowhead: stroke weight × its scale, at least
    /// a quarter point. A head of weight `hw` fits a box about `4·hw` long and wide.
    pub fn arrow_weight(&self, end: bool) -> f64 {
        let pct = if end { self.arrow_scale.1 } else { self.arrow_scale.0 };
        (self.width * pct / 100.0).max(0.25)
    }
    /// How far the start (`end == false`) or end arrowhead can reach from its end point (0
    /// without one): the head's diagonal, or with [`ArrowAlign::Extend`] its length, plus the cap.
    pub fn head_reach(&self, end: bool) -> f64 {
        let head = if end { self.end_arrow } else { self.start_arrow };
        head.map_or(0.0, |_| HEAD_REACH * self.arrow_weight(end) + self.width / 2.0)
    }
    /// How far the arrowheads can reach from the path's end points (0 without heads).
    pub fn arrow_reach(&self) -> f64 {
        self.head_reach(false).max(self.head_reach(true))
    }
    /// The widest the width profile makes the stroke, as a factor of its weight (1 without one).
    pub fn profile_max(&self) -> f64 {
        self.profile.as_ref().and_then(|p| p.points.iter().map(|&(_, l, r)| l.max(r)).reduce(f64::max)).unwrap_or(1.0).max(0.0)
    }
    /// The stroke's body across a path that is `closed` or open: how far it reaches from the path
    /// at full width (half the weight centred; the whole weight on one side when aligned inside or
    /// outside a closed path, which open paths ignore), and for aligned strokes which side it
    /// paints (`Some(true)`: inside).
    pub fn body(&self, closed: bool) -> (f64, Option<bool>) {
        match self.align {
            StrokeAlign::Outside if closed => (self.width, Some(false)),
            StrokeAlign::Inside if closed => (self.width, Some(true)),
            _ => (self.width / 2.0, None),
        }
    }
    /// How far the body paints outside a path that is `closed` or open, at the profile's widest.
    pub fn side_reach(&self, closed: bool) -> f64 {
        match self.body(closed) {
            (_, Some(true)) => 0.0,
            (half, _) => half * self.profile_max(),
        }
    }
    /// The farthest this stroke can paint from any path: its body, miter spikes up to the miter
    /// limit, projecting caps' corners and the arrowheads ([`Appearance::outset`]).
    pub fn reach(&self) -> f64 {
        let side = self.side_reach(true).max(self.side_reach(false));
        let miter = if self.join == LineJoin::Miter { self.miter_limit.max(1.0) } else { 1.0 };
        let cap = if self.cap == LineCap::Square { std::f64::consts::SQRT_2 } else { 1.0 };
        (side * miter.max(cap)).max(self.arrow_reach())
    }
    /// The box an unplaced gradient on this stroke fits to (see [`stroke_paint_bounds`]).
    pub fn paint_bounds(&self, geometric: vectorcraft_geom::Rect) -> vectorcraft_geom::Rect {
        stroke_paint_bounds(geometric, self.width)
    }
    /// The gradient this stroke lays along or across its path: its linear or radial gradient
    /// paint when [`Self::gradient_mode`] isn't Within (freeform gradients always paint within).
    pub fn path_gradient(&self) -> Option<&vectorcraft_color::GradientPaint> {
        match &self.paint {
            Paint::Gradient(g)
                if self.gradient_mode != StrokeGradientMode::Within && g.gradient.kind != vectorcraft_color::GradientKind::Freeform =>
            {
                Some(g)
            }
            _ => None,
        }
    }
}

/// The box an unplaced gradient on a stroke of `width` fits to: the geometric bounds grown by half
/// the weight (object strokes and type strokes alike, on screen and in exports).
pub fn stroke_paint_bounds(geometric: vectorcraft_geom::Rect, width: f64) -> vectorcraft_geom::Rect {
    geometric.inflate(width / 2.0, width / 2.0)
}

/// One entry of the appearance stack.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum AppearanceItem {
    Fill(FillLayer),
    Stroke(StrokeLayer),
}

impl AppearanceItem {
    pub fn is_fill(&self) -> bool {
        matches!(self, AppearanceItem::Fill(_))
    }
    /// `"fill"` or `"stroke"` (the serialized `kind`).
    pub fn kind_name(&self) -> &'static str {
        if self.is_fill() { "fill" } else { "stroke" }
    }
    pub fn paint(&self) -> &Paint {
        match self {
            AppearanceItem::Fill(f) => &f.paint,
            AppearanceItem::Stroke(s) => &s.paint,
        }
    }
    pub fn paint_mut(&mut self) -> &mut Paint {
        match self {
            AppearanceItem::Fill(f) => &mut f.paint,
            AppearanceItem::Stroke(s) => &mut s.paint,
        }
    }
    pub fn visible(&self) -> bool {
        match self {
            AppearanceItem::Fill(f) => f.visible,
            AppearanceItem::Stroke(s) => s.visible,
        }
    }
    pub fn opacity(&self) -> f32 {
        match self {
            AppearanceItem::Fill(f) => f.opacity,
            AppearanceItem::Stroke(s) => s.opacity,
        }
    }
    pub fn blend(&self) -> BlendMode {
        match self {
            AppearanceItem::Fill(f) => f.blend,
            AppearanceItem::Stroke(s) => s.blend,
        }
    }
    /// The item's own live effects (applied to this fill or stroke only).
    pub fn effects(&self) -> &Vec<Effect> {
        match self {
            AppearanceItem::Fill(f) => &f.effects,
            AppearanceItem::Stroke(s) => &s.effects,
        }
    }
    pub fn effects_mut(&mut self) -> &mut Vec<Effect> {
        match self {
            AppearanceItem::Fill(f) => &mut f.effects,
            AppearanceItem::Stroke(s) => &mut s.effects,
        }
    }
    /// Whether this fill or stroke overprints.
    pub fn overprint(&self) -> bool {
        match self {
            AppearanceItem::Fill(f) => f.overprint,
            AppearanceItem::Stroke(s) => s.overprint,
        }
    }
    pub fn overprint_mut(&mut self) -> &mut bool {
        match self {
            AppearanceItem::Fill(f) => &mut f.overprint,
            AppearanceItem::Stroke(s) => &mut s.overprint,
        }
    }
}

/// Appearance attributes. `items` is in paint order: `items[0]` is painted first (the bottom of the
/// Appearance panel list).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Appearance {
    #[serde(default)]
    pub items: Vec<AppearanceItem>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<Effect>,
    /// Groups, layers and type: how many items paint below the object's contents (its members, or
    /// type's characters), the slot of the Appearance panel's Contents (Characters) row. `None`:
    /// every item paints above the contents (see [`Self::contents_at`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents_index: Option<usize>,
}

impl Appearance {
    /// Illustrator's "basic appearance": one fill below one stroke.
    pub fn basic(fill: Paint, stroke: Paint, width: f64) -> Self {
        Self { items: vec![AppearanceItem::Fill(FillLayer::new(fill)), AppearanceItem::Stroke(StrokeLayer::new(stroke, width))], ..Self::default() }
    }
    /// White fill, 1 pt black stroke (the default for new art).
    pub fn default_art() -> Self {
        Self::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 1.0)
    }
    /// The topmost fill (what the Fill proxy shows).
    pub fn fill(&self) -> Option<&FillLayer> {
        self.items.iter().rev().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None })
    }
    pub fn fill_mut(&mut self) -> Option<&mut FillLayer> {
        self.items.iter_mut().rev().find_map(|i| if let AppearanceItem::Fill(f) = i { Some(f) } else { None })
    }
    /// The topmost stroke (what the Stroke proxy shows).
    pub fn stroke(&self) -> Option<&StrokeLayer> {
        self.items.iter().rev().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None })
    }
    pub fn stroke_mut(&mut self) -> Option<&mut StrokeLayer> {
        self.items.iter_mut().rev().find_map(|i| if let AppearanceItem::Stroke(s) = i { Some(s) } else { None })
    }
    /// This appearance for other art: the pen pressure its strokes were drawn with stays with them.
    pub fn without_pressure(mut self) -> Self {
        for it in &mut self.items {
            if let AppearanceItem::Stroke(s) = it {
                s.pressure = None;
            }
        }
        self
    }
    pub fn fill_paint(&self) -> Paint {
        self.fill().map(|f| f.paint.clone()).unwrap_or(Paint::None)
    }
    pub fn stroke_paint(&self) -> Paint {
        self.stroke().map(|s| s.paint.clone()).unwrap_or(Paint::None)
    }
    /// Set the top fill's paint, creating a fill if there is none.
    pub fn set_fill(&mut self, p: Paint) {
        match self.fill_mut() {
            Some(f) => f.paint = p,
            None => self.insert_item(0, AppearanceItem::Fill(FillLayer::new(p))),
        }
    }
    /// Set the top stroke's paint, creating a 1 pt stroke if there is none.
    pub fn set_stroke(&mut self, p: Paint) {
        match self.stroke_mut() {
            Some(s) => s.paint = p,
            None => self.items.push(AppearanceItem::Stroke(StrokeLayer::new(p, 1.0))),
        }
    }
    pub fn stroke_width(&self) -> f64 {
        self.stroke().filter(|s| !s.paint.is_none()).map(|s| s.width).unwrap_or(0.0)
    }
    /// Is this a basic appearance: at most one fill and one stroke, none hidden or with its own
    /// opacity, blend mode or effects, and no object effects?
    pub fn is_basic(&self) -> bool {
        let count = |fill: bool| self.items.iter().filter(|i| i.is_fill() == fill).count();
        self.effects.is_empty()
            && self.contents_index.is_none()
            && count(true) <= 1
            && count(false) <= 1
            && self.items.iter().all(|i| i.visible() && i.effects().is_empty() && i.opacity() == 1.0 && i.blend() == BlendMode::Normal)
    }
    /// Fill item `index`, or the topmost fill for `None`. `None` when that item is not a fill.
    pub fn fill_at(&self, index: Option<usize>) -> Option<&FillLayer> {
        match index {
            None => self.fill(),
            Some(i) => match self.items.get(i)? {
                AppearanceItem::Fill(f) => Some(f),
                AppearanceItem::Stroke(_) => None,
            },
        }
    }
    pub fn fill_at_mut(&mut self, index: Option<usize>) -> Option<&mut FillLayer> {
        match index {
            None => self.fill_mut(),
            Some(i) => match self.items.get_mut(i)? {
                AppearanceItem::Fill(f) => Some(f),
                AppearanceItem::Stroke(_) => None,
            },
        }
    }
    /// Stroke item `index`, or the topmost stroke for `None`. `None` when that item is not a stroke.
    pub fn stroke_at(&self, index: Option<usize>) -> Option<&StrokeLayer> {
        match index {
            None => self.stroke(),
            Some(i) => match self.items.get(i)? {
                AppearanceItem::Stroke(s) => Some(s),
                AppearanceItem::Fill(_) => None,
            },
        }
    }
    pub fn stroke_at_mut(&mut self, index: Option<usize>) -> Option<&mut StrokeLayer> {
        match index {
            None => self.stroke_mut(),
            Some(i) => match self.items.get_mut(i)? {
                AppearanceItem::Stroke(s) => Some(s),
                AppearanceItem::Fill(_) => None,
            },
        }
    }
    /// `index` when it names a fill (`fill`) or stroke item, else `None` (the topmost one): the
    /// item a fill or stroke edit changes while item `index` is the Appearance panel's target.
    pub fn item_of_kind(&self, index: Option<usize>, fill: bool) -> Option<usize> {
        index.filter(|i| self.items.get(*i).is_some_and(|it| it.is_fill() == fill))
    }
    /// The fill the Fill proxy shows while item `index` is targeted: that item when it is a fill,
    /// else the topmost fill.
    pub fn fill_for(&self, index: Option<usize>) -> Option<&FillLayer> {
        self.fill_at(self.item_of_kind(index, true))
    }
    /// The stroke the Stroke proxy and panel show while item `index` is targeted.
    pub fn stroke_for(&self, index: Option<usize>) -> Option<&StrokeLayer> {
        self.stroke_at(self.item_of_kind(index, false))
    }
    /// Set the paint of fill item `index` (`None`: the top fill, created if missing). False when
    /// `index` is not a fill.
    pub fn set_fill_at(&mut self, index: Option<usize>, p: Paint) -> bool {
        if index.is_none() {
            self.set_fill(p);
            return true;
        }
        self.fill_at_mut(index).map(|f| f.paint = p).is_some()
    }
    /// Set the paint of stroke item `index` (`None`: the top stroke, created if missing). False
    /// when `index` is not a stroke.
    pub fn set_stroke_at(&mut self, index: Option<usize>, p: Paint) -> bool {
        if index.is_none() {
            self.set_stroke(p);
            return true;
        }
        self.stroke_at_mut(index).map(|s| s.paint = p).is_some()
    }
    /// The paint of fill (`fill`) or stroke item `index` (`None`: the topmost one).
    pub fn paint_at(&self, index: Option<usize>, fill: bool) -> Option<&Paint> {
        if fill { self.fill_at(index).map(|f| &f.paint) } else { self.stroke_at(index).map(|s| &s.paint) }
    }
    /// [`Self::set_fill_at`] or [`Self::set_stroke_at`].
    pub fn set_paint_at(&mut self, index: Option<usize>, fill: bool, p: Paint) -> bool {
        if fill { self.set_fill_at(index, p) } else { self.set_stroke_at(index, p) }
    }
    /// The effects of item `index`, or the object-level effects for `None`.
    pub fn effects_at(&self, index: Option<usize>) -> Option<&Vec<Effect>> {
        match index {
            None => Some(&self.effects),
            Some(i) => self.items.get(i).map(AppearanceItem::effects),
        }
    }
    pub fn effects_mut(&mut self, index: Option<usize>) -> Option<&mut Vec<Effect>> {
        match index {
            None => Some(&mut self.effects),
            Some(i) => self.items.get_mut(i).map(AppearanceItem::effects_mut),
        }
    }
    /// The visible strokes that paint something.
    pub fn painted_strokes(&self) -> impl Iterator<Item = &StrokeLayer> {
        self.items.iter().filter_map(|i| match i {
            AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() => Some(s),
            _ => None,
        })
    }
    /// Largest distance the painted area can extend beyond any geometry (see
    /// [`StrokeLayer::reach`]); paths measure their own with [`Self::stroked_bounds`].
    pub fn outset(&self) -> f64 {
        self.painted_strokes().map(StrokeLayer::reach).fold(0.0, f64::max)
    }
    /// Scale stroke weights, dash lengths and dash offsets (Scale Strokes & Effects).
    pub fn scale_strokes(&mut self, s: f64) {
        for i in &mut self.items {
            if let AppearanceItem::Stroke(st) = i {
                st.width *= s;
                if let Some(d) = &mut st.dash {
                    d.scale(s);
                }
            }
        }
    }
    /// Scale the distance parameters of every effect (the object's and each fill's and stroke's)
    /// by `s` with `scale` (the effects catalogue knows which parameters are distances).
    pub fn scale_effects(&mut self, s: f64, scale: fn(&mut Effect, f64)) {
        let items = self.items.iter_mut().flat_map(|i| i.effects_mut().iter_mut());
        for e in self.effects.iter_mut().chain(items) {
            scale(e, s);
        }
    }
    /// Gradient paints of the fills and strokes.
    fn gradients_mut(&mut self) -> impl Iterator<Item = &mut vectorcraft_color::GradientPaint> {
        self.items.iter_mut().filter_map(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => Some(&mut **g),
            _ => None,
        })
    }
    /// Gradient paints of the fills and strokes.
    fn gradients(&self) -> impl Iterator<Item = &vectorcraft_color::GradientPaint> {
        self.items.iter().filter_map(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => Some(&**g),
            _ => None,
        })
    }
    /// Is `self` equal to `other`, placed gradients within rounding (1e-6 pt) of each other, as
    /// moving them between boxes leaves them?
    pub fn approx_eq(&self, other: &Appearance) -> bool {
        if self == other {
            return true;
        }
        let mut a = self.clone();
        for (g, o) in a.gradients_mut().zip(other.gradients()) {
            if let (Some(ga), Some(go)) = (&mut g.geom, &o.geom)
                && ga.start.distance(go.start) <= 1e-6
                && ga.end.distance(go.end) <= 1e-6
                && (ga.aspect - go.aspect).abs() <= 1e-6
                && ga.focal_point().distance(go.focal_point()) <= 1e-6
            {
                *ga = *go;
            }
            if let (Some(fa), Some(fo)) = (&mut g.freeform, &o.freeform)
                && fa.points.len() == fo.points.len()
                && fa.points.iter().zip(&fo.points).all(|(p, q)| p.at.distance(q.at) <= 1e-6)
            {
                fa.points.iter_mut().zip(&fo.points).for_each(|(p, q)| p.at = q.at);
            }
        }
        a == *other
    }
    /// Does a fill or stroke carry a gradient whose placement `placed` is (some: placed in document
    /// space; none: fitted to the bounds on each render)?
    fn has_gradient(&self, placed: bool) -> bool {
        self.items.iter().any(|i| match i {
            AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. })
            | AppearanceItem::Stroke(StrokeLayer { paint: Paint::Gradient(g), .. }) => g.is_placed() == placed,
            _ => false,
        })
    }
    /// Does a fill or stroke carry a gradient that is fitted to the bounds on each render?
    pub fn has_unplaced_gradient(&self) -> bool {
        self.has_gradient(false)
    }
    /// Does a fill or stroke carry a placed gradient (start and end in document space)?
    pub fn has_placed_gradient(&self) -> bool {
        self.has_gradient(true)
    }
    /// Fix unplaced gradients to their fit on `bounds` (the object's geometric bounds; strokes fit
    /// the stroke-inflated box), so they can follow transforms that refitting wouldn't reproduce.
    pub fn pin_gradients(&mut self, bounds: vectorcraft_geom::Rect) {
        for i in &mut self.items {
            match i {
                AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. }) => g.pin(bounds),
                AppearanceItem::Stroke(s) => {
                    let b = s.paint_bounds(bounds);
                    if let Paint::Gradient(g) = &mut s.paint {
                        g.pin(b);
                    }
                }
                _ => {}
            }
        }
    }
    /// Move placed gradients from an object whose geometric bounds are `from` to one whose bounds
    /// are `to`, keeping them at the same place relative to the box (strokes: relative to their
    /// stroke-inflated boxes, as they fit).
    pub fn rebase_gradients(&mut self, from: vectorcraft_geom::Rect, to: vectorcraft_geom::Rect) {
        for i in &mut self.items {
            match i {
                AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. }) => g.rebase(from, to),
                AppearanceItem::Stroke(s) => {
                    let (f, t) = (s.paint_bounds(from), s.paint_bounds(to));
                    if let Paint::Gradient(g) = &mut s.paint {
                        g.rebase(f, t);
                    }
                }
                _ => {}
            }
        }
    }
    /// Map placed gradients through `a`.
    pub fn transform_gradients(&mut self, a: vectorcraft_geom::Affine) {
        for g in self.gradients_mut() {
            g.transform(a);
        }
    }
    /// Envelope Options → Distort Linear Gradients: bend the linear gradients of an object whose
    /// geometric bounds are `bounds` with a warp, given its local affine approximation at a point
    /// (taken at the middle of each gradient's vector). Unplaced ones are fixed to their fit first;
    /// other gradients stay as they are.
    pub fn warp_linear_gradients(&mut self, bounds: vectorcraft_geom::Rect, near: &dyn Fn(vectorcraft_geom::Point) -> vectorcraft_geom::Affine) {
        for i in &mut self.items {
            let (g, b) = match i {
                AppearanceItem::Fill(FillLayer { paint: Paint::Gradient(g), .. }) => (g, bounds),
                AppearanceItem::Stroke(s) => {
                    let b = s.paint_bounds(bounds);
                    let Paint::Gradient(g) = &mut s.paint else { continue };
                    (g, b)
                }
                _ => continue,
            };
            if g.gradient.kind != vectorcraft_color::GradientKind::Linear {
                continue;
            }
            g.pin(b);
            if let Some(geom) = g.geom {
                g.transform(near(geom.start.midpoint(geom.end)));
            }
        }
    }
    /// Map placed gradients through a warp, given its local affine approximation at a point (taken
    /// at each gradient's centre: the start of a radial, the middle of a linear vector). Freeform
    /// points each follow the warp at their own position.
    pub fn warp_gradients(&mut self, near: &dyn Fn(vectorcraft_geom::Point) -> vectorcraft_geom::Affine) {
        for g in self.gradients_mut() {
            let freeform = g.freeform.take();
            if let Some(geom) = g.geom {
                let c = if g.gradient.kind == vectorcraft_color::GradientKind::Radial { geom.start } else { geom.start.midpoint(geom.end) };
                g.transform(near(c));
            }
            g.freeform = freeform.map(|mut f| {
                f.map_points(|p| near(p) * p);
                f
            });
        }
    }
}

impl Appearance {
    /// Is any fill, stroke or effect (the object's or an item's) hidden?
    pub fn has_hidden(&self) -> bool {
        let hidden = |fx: &[Effect]| fx.iter().any(|e| !e.visible);
        hidden(&self.effects) || self.items.iter().any(|i| !i.visible() || hidden(i.effects()))
    }

    /// Make every hidden fill, stroke and effect visible (Show All Hidden Attributes). Returns
    /// whether anything was hidden.
    pub fn show_all(&mut self) -> bool {
        let had = self.has_hidden();
        let show = |fx: &mut Vec<Effect>| fx.iter_mut().for_each(|e| e.visible = true);
        show(&mut self.effects);
        for i in &mut self.items {
            match i {
                AppearanceItem::Fill(f) => f.visible = true,
                AppearanceItem::Stroke(s) => s.visible = true,
            }
            show(i.effects_mut());
        }
        had
    }
}

/// The contents slot: where a group's or layer's members (type: its characters) paint among the
/// object's own fills and strokes.
impl Appearance {
    /// How many items paint below the contents (the Contents/Characters row sits above them).
    pub fn contents_at(&self) -> usize {
        self.contents_index.unwrap_or(0).min(self.items.len())
    }
    /// Put the contents above the bottom `k` items (clamped; 0, all items above, is stored as `None`).
    pub fn set_contents_at(&mut self, k: usize) {
        let k = k.min(self.items.len());
        self.contents_index = (k > 0).then_some(k);
    }
    /// The items painted below the contents and those painted above them.
    pub fn split_contents(&self) -> (&[AppearanceItem], &[AppearanceItem]) {
        self.items.split_at(self.contents_at())
    }
    /// Insert `item` at paint-order index `at` (clamped). The contents stay between the same items;
    /// an item landing right above them joins the item below it (below the contents if that one is).
    pub fn insert_item(&mut self, at: usize, item: AppearanceItem) {
        let k = self.contents_at();
        let at = at.min(self.items.len());
        self.items.insert(at, item);
        if at < k || (at == k && k > 0) {
            self.set_contents_at(k + 1);
        }
    }
    /// Remove item `i` (if it exists), keeping the contents between the same items.
    pub fn remove_item(&mut self, i: usize) -> Option<AppearanceItem> {
        let k = self.contents_at();
        (i < self.items.len()).then(|| {
            let it = self.items.remove(i);
            if i < k {
                self.set_contents_at(k - 1);
            }
            it
        })
    }
    /// Move item `from` to paint-order index `to` (clamped) and return where it landed, or `None`
    /// when there is no item `from`. The contents stay between the same other items, and the moved
    /// item keeps its side of them when it lands right next to them.
    pub fn move_item(&mut self, from: usize, to: usize) -> Option<usize> {
        let k = self.contents_at();
        let below = from < k;
        let it = self.remove_item(from)?;
        let others_below = self.contents_at();
        let to = to.min(self.items.len());
        self.items.insert(to, it);
        let now_below = if below { to <= others_below } else { to < others_below };
        self.set_contents_at(others_below + usize::from(now_below));
        Some(to)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hidden_attributes_show_again() {
        let mut a = Appearance::default_art();
        assert!(!a.has_hidden() && !a.show_all());
        a.stroke_mut().unwrap().visible = false;
        a.items[0].effects_mut().push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: false });
        assert!(a.has_hidden());
        assert!(a.show_all());
        assert!(!a.has_hidden() && a.items.iter().all(AppearanceItem::visible) && a.items[0].effects()[0].visible);
    }

    #[test]
    fn basic_appearance() {
        let a = Appearance::default_art();
        assert!(a.is_basic());
        assert_eq!(a.fill_paint(), Paint::solid(Color::WHITE));
        assert_eq!(a.stroke_width(), 1.0);
    }

    #[test]
    fn rebase_moves_fills_by_the_box_and_strokes_by_their_inflated_box() {
        use vectorcraft_color::{Gradient, GradientGeom, GradientPaint};
        use vectorcraft_geom::{Point, Rect};
        let placed = |x: f64| {
            let mut g = GradientPaint::new(Gradient::default());
            g.geom = Some(GradientGeom { start: Point::new(x, 0.0), end: Point::new(x + 10.0, 0.0), aspect: 1.0, focal: None });
            Paint::Gradient(Box::new(g))
        };
        let mut a = Appearance::basic(placed(0.0), placed(-5.0), 10.0);
        a.rebase_gradients(Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(100.0, 0.0, 300.0, 100.0));
        let start = |p: Paint| match p {
            Paint::Gradient(g) => g.geom.unwrap().start,
            _ => panic!("a gradient"),
        };
        // The fill's start stays on the left edge; the stroke's on its inflated box's (−5 → 95).
        assert_eq!((start(a.fill_paint()), start(a.stroke_paint())), (Point::new(100.0, 0.0), Point::new(95.0, 0.0)));
    }

    #[test]
    fn items_by_index() {
        let mut a = Appearance::default_art();
        a.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
        // [Fill white, Stroke black, Fill none]
        assert_eq!(a.fill_at(None).unwrap().paint, Paint::None);
        assert_eq!(a.fill_at(Some(0)).unwrap().paint, Paint::solid(Color::WHITE));
        assert!(a.fill_at(Some(1)).is_none() && a.stroke_at(Some(0)).is_none() && a.fill_at(Some(9)).is_none());
        assert_eq!(a.item_of_kind(Some(1), false), Some(1));
        assert_eq!(a.item_of_kind(Some(1), true), None);
        assert_eq!(a.fill_for(Some(1)).unwrap().paint, Paint::None);
        assert!(a.set_fill_at(Some(0), Paint::solid(Color::BLACK)));
        assert!(!a.set_stroke_at(Some(0), Paint::None));
        assert_eq!(a.fill_at(Some(0)).unwrap().paint, Paint::solid(Color::BLACK));
        a.effects_mut(Some(1)).unwrap().push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: true });
        assert_eq!(a.effects_at(Some(1)).unwrap().len(), 1);
        assert!(a.effects_at(None).unwrap().is_empty() && a.effects_mut(Some(3)).is_none());
        assert_eq!((a.items[1].kind_name(), a.items[2].kind_name()), ("stroke", "fill"));
    }

    #[test]
    fn basic_means_one_plain_fill_and_stroke() {
        let stroke = || AppearanceItem::Stroke(StrokeLayer::new(Paint::solid(Color::BLACK), 1.0));
        let mut a = Appearance { items: vec![stroke(), stroke()], ..Appearance::default() };
        assert!(!a.is_basic(), "two strokes");
        a.items.pop();
        assert!(a.is_basic() && Appearance::default().is_basic());
        let mut b = Appearance::default_art();
        b.stroke_mut().unwrap().visible = false;
        assert!(!b.is_basic(), "a hidden stroke");
        let mut c = Appearance::default_art();
        c.fill_mut().unwrap().opacity = 0.5;
        assert!(!c.is_basic(), "fill opacity");
        let mut d = Appearance::default_art();
        d.items.push(AppearanceItem::Fill(FillLayer::new(Paint::None)));
        assert!(!d.is_basic(), "two fills");
        let mut e = Appearance::default_art();
        e.effects.push(Effect { id: "distort.roughen".into(), params: serde_json::Value::Null, visible: true });
        assert!(!e.is_basic(), "an effect");
    }

    #[test]
    fn set_creates_missing() {
        let mut a = Appearance::default();
        a.set_fill(Paint::solid(Color::BLACK));
        a.set_stroke(Paint::solid(Color::WHITE));
        assert_eq!(a.items.len(), 2);
        assert!(matches!(a.items[0], AppearanceItem::Fill(_)));
    }

    /// #852: pen pressure is kept simplified along the stroke and read back by position.
    #[test]
    fn pressure_profile_samples_simplify_and_interpolate() {
        // A steady ramp keeps only its ends; a bump keeps its peak.
        let ramp = PressureProfile::from_samples((0..=100).map(|i| (i as f64 / 100.0, i as f64 / 100.0))).unwrap();
        assert_eq!(ramp.points, vec![(0.0, 0.0), (1.0, 1.0)]);
        assert!((ramp.at(0.25) - 0.25).abs() < 1e-9);
        let bump = PressureProfile::from_samples([(0.0, 0.2), (0.3, 0.2), (0.5, 0.9), (0.7, 0.2), (1.0, 0.2)]).unwrap();
        assert!(bump.points.contains(&(0.5, 0.9)) && (bump.at(0.4) - 0.55).abs() < 1e-9, "{bump:?}");
        // Junk is clamped or dropped; positions never go back; nothing at all is no profile.
        let junk = PressureProfile::from_samples([(f64::NAN, 0.5), (0.5, 7.0), (0.2, -1.0), (2.0, f64::INFINITY)]).unwrap();
        assert_eq!(junk.points, vec![(0.5, 1.0), (0.5, 0.0)]);
        assert!(PressureProfile::from_samples([(f64::NAN, 0.5)]).is_none());
        assert_eq!(PressureProfile::default().at(0.3), PressureProfile::MID);
        // A long noisy stroke stays bounded.
        let noisy = PressureProfile::from_samples((0..1_000_000).map(|i| (i as f64 / 1e6, (i % 2) as f64))).unwrap();
        assert!(noisy.points.len() <= PressureProfile::MAX_POINTS);
        // Out-of-order points (a hand-edited file) still read within 0..1.
        let odd = PressureProfile { points: vec![(1.0, 5.0), (0.0, -3.0)] };
        assert!((0.0..=1.0).contains(&odd.at(0.5)));
    }

    #[test]
    fn pressure_profile_extends_and_reverses() {
        let old = PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] };
        let added = PressureProfile { points: vec![(0.0, 1.0), (1.0, 0.0)] };
        // A 30 pt path continued by 10 pt: its ramp fills the first three quarters.
        let joined = PressureProfile::extended(Some(&old), 30.0, Some(&added), 10.0).unwrap();
        assert!((joined.at(0.75) - 1.0).abs() < 1e-9 && (joined.at(0.375) - 0.5).abs() < 1e-9 && joined.at(1.0) == 0.0, "{joined:?}");
        // A side without pressure is half way.
        let half = PressureProfile::extended(None, 10.0, Some(&added), 10.0).unwrap();
        assert_eq!(half.at(0.25), PressureProfile::MID);
        assert!(PressureProfile::extended(None, 10.0, None, 10.0).is_none());
        assert_eq!(PressureProfile::extended(Some(&old), 0.0, None, f64::NAN), Some(old.clone()));
        assert_eq!(old.reversed().points, vec![(0.0, 1.0), (1.0, 0.0)]);
    }

    /// Strokes saved before pen pressure load as they were, and are written without it.
    #[test]
    fn strokes_without_pressure_round_trip_unchanged() {
        let json = serde_json::to_value(StrokeLayer::new(Paint::None, 2.0)).unwrap();
        assert!(json.get("pressure").is_none());
        let st: StrokeLayer = serde_json::from_value(json).unwrap();
        assert_eq!(st.pressure, None);
        let pressed = StrokeLayer { pressure: Some(PressureProfile::sample()), ..st };
        let back: StrokeLayer = serde_json::from_value(serde_json::to_value(&pressed).unwrap()).unwrap();
        assert_eq!(back, pressed);
        let mut ap = Appearance::basic(Paint::None, Paint::None, 1.0);
        ap.stroke_mut().unwrap().pressure = Some(PressureProfile::sample());
        assert_eq!(ap.without_pressure().stroke().unwrap().pressure, None);
    }

    #[test]
    fn profile_interp() {
        let p = WidthProfile::lens();
        assert_eq!(p.at(0.5), (1.0, 1.0));
        assert_eq!(p.at(0.25), (0.5, 0.5));
        assert_eq!(p.at(2.0), (0.0, 0.0));
    }

    #[test]
    fn profile_presets_round_trip_their_ids() {
        for p in WidthProfile::PRESETS {
            let prof = WidthProfile::preset(p.id).unwrap();
            assert_eq!(prof.preset_id(), Some(p.id));
            assert_eq!(WidthProfile::id_of(Some(&prof)), p.id);
        }
        assert_eq!(WidthProfile::id_of(None), "uniform");
        assert_eq!(WidthProfile::id_of(Some(&WidthProfile { points: vec![(0.0, 0.3, 0.3)] })), "custom");
        assert!(WidthProfile::preset("nope").is_none());
        assert_eq!(WidthProfile::lens().points, vec![(0.0, 0.0, 0.0), (0.5, 1.0, 1.0), (1.0, 0.0, 0.0)]);
    }

    #[test]
    fn arrowhead_names_are_stable_and_every_one_is_listed_once() {
        // Saved files store these names: they may only ever be appended to.
        const SAVED: [&str; 40] = [
            "Triangle",
            "TriangleOpen",
            "Circle",
            "CircleOpen",
            "Square",
            "SquareOpen",
            "Bar",
            "Diamond",
            "Arrow",
            "ArrowOpen",
            "Barbed",
            "HalfArrowLeft",
            "HalfArrowRight",
            "Concave",
            "DoubleBar",
            "Feather",
            "DotOnBar",
            "Chevron",
            "DoubleArrow",
            "Target",
            "Star",
            "Cross",
            "Plus",
            "Hexagon",
            "HexagonOpen",
            "Tag",
            "TagOpen",
            "HalfCircle",
            "Drop",
            "Slash",
            "DoubleSlash",
            "DiamondOpen",
            "TriangleReverse",
            "Swallowtail",
            "Bracket",
            "Fork",
            "Leaf",
            "Kite",
            "TriangleBar",
            "Oval",
        ];
        for name in SAVED {
            let a: Arrowhead = serde_json::from_value(serde_json::json!(name)).unwrap();
            assert_eq!(serde_json::to_value(a).unwrap(), name);
            assert_eq!(Arrowhead::ALL.iter().filter(|x| **x == a).count(), 1, "{name} in the menu once");
        }
        let labels: std::collections::HashSet<&str> = Arrowhead::ALL.iter().map(|a| a.label()).collect();
        assert_eq!(labels.len(), Arrowhead::ALL.len(), "labels are distinct");
    }

    #[test]
    fn arrow_align_defaults_to_extend_and_round_trips() {
        let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), 2.0);
        assert_eq!(st.arrow_align, ArrowAlign::Extend);
        // The default is not written, so older readers see the same JSON as before.
        assert!(!serde_json::to_string(&st).unwrap().contains("arrow_align"));
        let old: StrokeLayer = serde_json::from_str(r#"{"paint":{"type":"none"},"width":2.0}"#).unwrap();
        assert_eq!(old.arrow_align, ArrowAlign::Extend);
        st.arrow_align = ArrowAlign::Tip;
        let back: StrokeLayer = serde_json::from_str(&serde_json::to_string(&st).unwrap()).unwrap();
        assert_eq!(back, st);
    }

    #[test]
    fn outset_covers_arrowheads() {
        let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 4.0);
        a.stroke_mut().unwrap().join = LineJoin::Round;
        assert_eq!(a.outset(), 2.0);
        let st = a.stroke_mut().unwrap();
        st.end_arrow = Some(Arrowhead::Triangle);
        st.arrow_scale = (100.0, 200.0);
        assert_eq!(st.arrow_weight(false), 4.0);
        assert_eq!(st.arrow_weight(true), 8.0);
        assert_eq!((st.head_reach(false), st.head_reach(true)), (0.0, HEAD_REACH * 8.0 + 2.0));
        // A 32 pt head whose tip sits up to its length (plus the cap) past the end point.
        assert_eq!(a.outset(), HEAD_REACH * 8.0 + 2.0);
        a.stroke_mut().unwrap().width = 0.01;
        assert_eq!(a.stroke().unwrap().arrow_weight(true), 0.25, "heads keep a minimum size");
    }

    #[test]
    fn outset_depends_on_align() {
        let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 10.0);
        a.stroke_mut().unwrap().join = LineJoin::Round;
        assert_eq!(a.outset(), 5.0);
        a.stroke_mut().unwrap().align = StrokeAlign::Outside;
        assert_eq!(a.outset(), 10.0);
        // Open paths stroke inside-aligned strokes centred.
        a.stroke_mut().unwrap().align = StrokeAlign::Inside;
        assert_eq!(a.outset(), 5.0);
        assert_eq!((a.stroke().unwrap().side_reach(true), a.stroke().unwrap().body(true)), (0.0, (10.0, Some(true))));
    }

    #[test]
    fn outset_covers_miter_spikes_projecting_caps_and_profile_maxima() {
        let mut a = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0);
        // Miter joins can spike out to the limit × half the weight.
        assert_eq!(a.outset(), 10.0);
        let st = a.stroke_mut().unwrap();
        st.join = LineJoin::Bevel;
        st.cap = LineCap::Square;
        assert!((a.outset() - std::f64::consts::SQRT_2).abs() < 1e-12, "a projecting cap's corner");
        let st = a.stroke_mut().unwrap();
        st.cap = LineCap::Round;
        st.profile = Some(WidthProfile { points: vec![(0.0, 0.5, 2.5), (1.0, 1.0, 1.0)] });
        assert_eq!(st.profile_max(), 2.5);
        assert_eq!(a.outset(), 2.5);
        a.stroke_mut().unwrap().paint = Paint::None;
        assert_eq!(a.outset(), 0.0, "an unpainted stroke");
    }

    #[test]
    fn contents_slot_follows_item_edits_and_round_trips() {
        let fill = || AppearanceItem::Fill(FillLayer::new(Paint::None));
        let mut a = Appearance { items: vec![fill(), fill(), fill()], ..Appearance::default() };
        assert_eq!((a.contents_at(), a.split_contents().1.len()), (0, 3), "all items above by default");
        a.set_contents_at(1);
        assert!(!a.is_basic());
        // [0 | 1 2]: removing below shifts the slot, removing above doesn't.
        a.remove_item(2);
        assert_eq!(a.contents_at(), 1);
        a.remove_item(0);
        assert_eq!((a.contents_at(), a.contents_index), (0, None));
        // Inserting next to the slot joins the item below it.
        a.items.push(fill());
        a.set_contents_at(1); // [0 | 1]
        a.insert_item(1, fill());
        assert_eq!(a.contents_at(), 2);
        a.insert_item(3, fill()); // [0 1 | 2 3]
        assert_eq!(a.contents_at(), 2);
        // Moves keep their side when landing next to the slot, cross it otherwise.
        assert_eq!(a.move_item(3, 2), Some(2));
        assert_eq!(a.contents_at(), 2, "stays above");
        a.move_item(3, 0);
        assert_eq!(a.contents_at(), 3, "crossed below");
        a.move_item(0, 3);
        assert_eq!(a.contents_at(), 2, "crossed above");
        assert_eq!(a.move_item(9, 0), None);
        // Saved only when set; older files load with every item above.
        let json = serde_json::to_string(&a).unwrap();
        assert!(json.contains("\"contents_index\":2"));
        assert_eq!(serde_json::from_str::<Appearance>(&json).unwrap(), a);
        let old: Appearance = serde_json::from_str(r#"{"items":[]}"#).unwrap();
        assert_eq!(old.contents_index, None);
        assert!(!serde_json::to_string(&Appearance::default_art()).unwrap().contains("contents_index"));
    }
}
