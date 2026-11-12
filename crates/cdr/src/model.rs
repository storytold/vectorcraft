//! What a drawing reads as. Lengths are in points (1/72 inch), from the centre of the page, with y
//! pointing up, as the file stores them.

/// A point in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pt {
    pub x: f64,
    pub y: f64,
}

/// An affine map `[a, b, c, d, e, f]`: x' = a x + c y + e, y' = b x + d y + f.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix(pub [f64; 6]);

impl Matrix {
    pub const IDENTITY: Matrix = Matrix([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    /// `self` after `first`: the map that applies `first`, then `self`.
    pub fn after(&self, first: &Matrix) -> Matrix {
        let [a, b, c, d, e, f] = self.0;
        let [p, q, r, s, t, u] = first.0;
        Matrix([a * p + c * q, b * p + d * q, a * r + c * s, b * r + d * s, a * t + c * u + e, b * t + d * u + f])
    }

    pub fn apply(&self, p: Pt) -> Pt {
        let [a, b, c, d, e, f] = self.0;
        Pt { x: a * p.x + c * p.y + e, y: b * p.x + d * p.y + f }
    }
}

/// One segment after the previous point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Segment {
    Line(Pt),
    /// Two control points, then the end point.
    Cubic(Pt, Pt, Pt),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SubPath {
    pub start: Pt,
    pub segments: Vec<Segment>,
    pub closed: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Path {
    pub subpaths: Vec<SubPath>,
}

/// The shape of an object, in its own space ([`Shape::matrix`] places it).
#[derive(Clone, Debug, PartialEq)]
pub enum Geometry {
    Path(Path),
    /// A rectangle from the origin to (`w`, `h`) (either may be negative). `radii` round its
    /// corners counterclockwise from the one at (`w`, 0): (`w`, 0), (`w`, `h`), (0, `h`), (0, 0).
    Rect {
        w: f64,
        h: f64,
        radii: [f64; 4],
    },
    /// A whole ellipse in the box from the origin to (`w`, `h`).
    Ellipse {
        w: f64,
        h: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Color {
    Rgb([f32; 3]),
    Cmyk([f32; 4]),
    /// Ink: 0 is white, 1 black.
    Gray(f32),
    /// Every ink at full strength.
    Registration,
    /// A color of a named library, printed on its own plate. `look` is its full-strength color
    /// when the file stores one (an approximation in sRGB); `tint` is 0 to 1.
    Spot {
        name: String,
        tint: f32,
        look: Option<[f32; 3]>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cap {
    Butt,
    Round,
    Square,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Join {
    Miter,
    Round,
    Bevel,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub width: f64,
    pub color: Color,
    pub cap: Cap,
    pub join: Join,
    /// Dash and gap lengths, in outline widths; empty for a solid line.
    pub dashes: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub geometry: Geometry,
    pub matrix: Matrix,
    pub fill: Option<Color>,
    pub outline: Option<Outline>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Object {
    Shape(Box<Shape>),
    Group(Vec<Object>),
}

/// A guideline through `at`, at `angle` degrees counterclockwise from horizontal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guideline {
    pub at: Pt,
    pub angle: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LayerKind {
    Drawing,
    /// The objects left beside the pages.
    Desktop,
    Guides,
    Grid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub name: String,
    pub kind: LayerKind,
    /// Front to back, as the file lists them.
    pub objects: Vec<Object>,
    pub guides: Vec<Guideline>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    /// Width and height in points, when the page gives its own.
    pub size: Option<(f64, f64)>,
    /// Front to back, as the file lists them.
    pub layers: Vec<Layer>,
}

/// What was left out or changed while reading. Each kind is listed once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Warning {
    /// Text, bitmaps, splines and other objects not read yet.
    UnsupportedObjects,
    /// Fountain, pattern, texture and PostScript fills.
    UnsupportedFills,
    /// Colors in models not read; shown black.
    UnknownColors,
    /// Library spot colors whose look the file doesn't store; shown black.
    SpotLooks,
    /// Outline arrowheads.
    Arrowheads,
    /// Envelopes, which are not applied: the shapes under them keep their outlines.
    Envelopes,
    /// Content nested deeper, or past the counts, that a read takes.
    TooDeep,
    /// Chunks or records that end early or don't parse.
    Damaged,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Drawing {
    /// In hundreds, as a RIFF file stores it: 300 to 2799. A file of version 1 or 2 stores no
    /// number: it gives 200 when its marker byte is the one of version 2, and 100 otherwise.
    pub version: u16,
    /// Page size from the document settings, in points.
    pub page_size: Option<(f64, f64)>,
    pub pages: Vec<Page>,
    /// Layers every page shows, from the master page.
    pub master: Vec<Layer>,
    /// Guidelines every page shows, kept outside any layer.
    pub guides: Vec<Guideline>,
    pub warnings: Vec<Warning>,
}
