use super::profile::{PressureProfile, WidthProfile};
use super::stroke_style::{ArrowAlign, Arrowhead, Dash, LineCap, LineJoin, StrokeAlign};
use super::*;

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
