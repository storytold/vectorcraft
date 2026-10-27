use super::*;

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
