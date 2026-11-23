use std::sync::Arc;

use vectorcraft_geom::{PathData, Rect};

use super::{Deserialize, Serialize};

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

/// How far the arrowheads drawn in the 4 × 4 box reach from their tip, in units of their weight
/// (the open arrow's arms are cut square a little behind the box).
const HEAD_REACH: f64 = 4.6;

/// An arrowhead the user drew (Stroke panel menu › Edit Arrowheads…), carried with its art so a
/// document that uses it draws it wherever it opens.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "CustomArrowheadData")]
pub struct CustomArrowhead {
    /// The name the arrowhead menus show (the symbol's name in the arrowheads file).
    pub name: String,
    /// The filled outline (non-zero rule) in head units: x from the tip (0) back along the line, y
    /// across it, positive to the right of the direction the head points. A head is 4 units wide,
    /// as the built-in heads are, and a unit is the head's weight.
    pub path: PathData,
    /// Where the stroke ends under the head, in head units back from the tip, when the head says
    /// so ([`Self::with_stop`]); `None`: halfway along its length ([`Self::stop`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<f64>,
}

#[derive(Deserialize)]
struct CustomArrowheadData {
    name: String,
    #[serde(default)]
    path: PathData,
    #[serde(default)]
    stop: Option<f64>,
}

impl From<CustomArrowheadData> for CustomArrowhead {
    fn from(d: CustomArrowheadData) -> Self {
        let head = CustomArrowhead::new(d.name, d.path);
        match d.stop {
            Some(stop) => head.with_stop(stop),
            None => head,
        }
    }
}

impl CustomArrowhead {
    /// The most anchors a custom head's outline may have, and the most subpaths.
    pub const MAX_ANCHORS: usize = 4096;
    /// How far from the tip a custom head's outline may reach, in head units.
    pub const MAX_UNITS: f64 = 64.0;

    /// Head `name` with outline `path` (in head units). An outline with more than
    /// [`Self::MAX_ANCHORS`] anchors or subpaths, a point that is not finite, or a point farther
    /// than [`Self::MAX_UNITS`] from the tip is dropped: the head then draws nothing.
    pub fn new(name: String, path: PathData) -> Self {
        let path = if Self::fits(&path) { path } else { PathData::default() };
        CustomArrowhead { name, path, stop: None }
    }
    /// The head with the stroke ending `stop` head units back from its tip (an arrowhead an
    /// Illustrator file defines draws its line to a point of its own). A stop that is not finite,
    /// is negative or is farther than [`Self::MAX_UNITS`] is left out: the stroke then ends
    /// halfway along the head.
    /// A head whose outline was dropped ([`Self::new`]) draws nothing and keeps no stop.
    pub fn with_stop(mut self, stop: f64) -> Self {
        let fits = !self.path.subpaths.is_empty() && stop.is_finite() && (0.0..=Self::MAX_UNITS).contains(&stop);
        self.stop = fits.then_some(stop);
        self
    }
    /// Whether outline `path` has at most [`Self::MAX_ANCHORS`] anchors and as many subpaths, with
    /// finite points within [`Self::MAX_UNITS`] only.
    pub fn fits(path: &PathData) -> bool {
        let anchors = path.subpaths.iter().map(|s| s.anchors.len()).sum::<usize>();
        let near = |p: vectorcraft_geom::Point| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= Self::MAX_UNITS && p.y.abs() <= Self::MAX_UNITS;
        anchors <= Self::MAX_ANCHORS
            && path.subpaths.len() <= Self::MAX_ANCHORS
            && path.subpaths.iter().flat_map(|s| &s.anchors).all(|a| near(a.p) && near(a.h_in) && near(a.h_out))
    }
    fn bounds(&self) -> Option<Rect> {
        self.path.control_bounds()
    }
    /// How far any part of the head can be from its tip, in head units.
    pub fn reach(&self) -> f64 {
        self.bounds().map_or(0.0, |b| b.x0.hypot(b.y0).max(b.x1.hypot(b.y0)).max(b.x0.hypot(b.y1)).max(b.x1.hypot(b.y1)))
    }
    /// How far the head spreads either side of its line, in head units.
    pub fn half_width(&self) -> f64 {
        self.bounds().map_or(0.0, |b| b.y0.abs().max(b.y1.abs()))
    }
    /// Where the stroke ends under the head, in head units back from the tip: its own stop
    /// ([`Self::with_stop`]), else halfway along the head's length.
    pub fn stop(&self) -> f64 {
        self.stop.unwrap_or_else(|| self.bounds().map_or(0.0, |b| b.x1.max(0.0) / 2.0))
    }
}

/// An arrowhead shape. Saved by variant name: new shapes are appended, never renamed or reordered.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
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
    /// Two arms of the head's weight (the line's weight at 100 % scale, for lines of 0.25 pt or
    /// more) meeting at right angles in a sharp tip, cut square at their ends. Twice as wide as
    /// the other heads.
    LineArrow,
    /// A slender filled head, a little over half as wide as it is long, whose sides bend in
    /// halfway back, with a V-shaped notch in its back: the arrow of mathematical diagrams.
    NotchedArrow,
    /// A head the user drew, with its art.
    Custom(Arc<CustomArrowhead>),
}

impl Arrowhead {
    /// Every arrowhead, in menu order (arrows, then outlined shapes, then bars and marks).
    pub const ALL: [Arrowhead; 42] = [
        Arrowhead::Arrow,
        Arrowhead::NotchedArrow,
        Arrowhead::ArrowOpen,
        Arrowhead::LineArrow,
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

    /// How far any part of the head can be from its tip, in units of its weight.
    pub fn reach(&self) -> f64 {
        match self {
            // Its farthest corners are 4 + ½√2 back and 4 − ½√2 out.
            Arrowhead::LineArrow => 5.75,
            Arrowhead::Custom(c) => c.reach(),
            _ => HEAD_REACH,
        }
    }
    /// How far the head spreads either side of its line, in units of its weight.
    pub fn half_width(&self) -> f64 {
        match self {
            Arrowhead::LineArrow => 4.0,
            Arrowhead::Custom(c) => c.half_width(),
            _ => 2.0,
        }
    }
    /// How far past its end point a head placed with [`ArrowAlign::Extend`] can paint, in units of
    /// its weight: the built-in heads' length, or a custom head's reach past its tip, which sits
    /// [`CustomArrowhead::stop`] past the end point.
    pub fn length(&self) -> f64 {
        match self {
            Arrowhead::Custom(c) => c.stop() + c.reach(),
            _ => 4.0,
        }
    }

    /// The name the arrowhead menus show: [`Self::label`], or a custom head's own name.
    pub fn name(&self) -> &str {
        match self {
            Arrowhead::Custom(c) => &c.name,
            _ => self.label(),
        }
    }

    /// The name the arrowhead menus show for a built-in head ("Custom" for a custom one, whose
    /// own name is [`Self::name`]).
    pub fn label(&self) -> &'static str {
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
            Arrowhead::LineArrow => "Line Arrow",
            Arrowhead::NotchedArrow => "Notched Arrow",
            Arrowhead::Custom(_) => "Custom",
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
