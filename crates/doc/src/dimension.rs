//! Persistent dimension recipes. Children are ordinary paths and type for all renderers/exporters.
use crate::{Document, NodeId, Unit};
use serde::{Deserialize, Serialize};
use vectorcraft_geom::{Affine, Point, Vec2};

/// What a dimension measures.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DimensionMode {
    #[default]
    Linear,
    Angular,
    Radius,
    Diameter,
}

impl DimensionMode {
    pub fn id(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Angular => "angular",
            Self::Radius => "radius",
            Self::Diameter => "diameter",
        }
    }
}

/// The unit a length is labelled in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LengthUnit {
    #[default]
    Pt,
    Pc,
    Px,
    In,
    Mm,
    Cm,
    M,
    Ft,
    Yd,
}

impl LengthUnit {
    pub const ALL: [LengthUnit; 9] = [
        LengthUnit::In,
        LengthUnit::Mm,
        LengthUnit::Pt,
        LengthUnit::Pc,
        LengthUnit::Px,
        LengthUnit::Cm,
        LengthUnit::M,
        LengthUnit::Ft,
        LengthUnit::Yd,
    ];
    pub fn unit(self) -> Unit {
        match self {
            Self::Pt => Unit::Points,
            Self::Pc => Unit::Picas,
            Self::Px => Unit::Pixels,
            Self::In => Unit::Inches,
            Self::Mm => Unit::Millimeters,
            Self::Cm => Unit::Centimeters,
            Self::M => Unit::Meters,
            Self::Ft => Unit::Feet,
            Self::Yd => Unit::Yards,
        }
    }
    /// The suffix written after a length.
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Pt => "pt",
            Self::Pc => "pc",
            Self::Px => "px",
            Self::In => "in",
            Self::Mm => "mm",
            Self::Cm => "cm",
            Self::M => "m",
            Self::Ft => "ft",
            Self::Yd => "yd",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LineType {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArrowStyle {
    #[default]
    Filled,
    Open,
    Tick,
    None,
}

/// Where the label sits across the dimension line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextPosition {
    /// On the side of the dimension line away from the measured object.
    #[default]
    Away,
    /// Between the dimension line and the measured object.
    Between,
}

/// Where the label sits along the dimension line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TextAlignment {
    Left,
    #[default]
    Center,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DimensionOptions {
    pub units: LengthUnit,
    pub precision: usize,
    /// Drawing : real size. 1:10 multiplies the drawn length by ten.
    pub scale: String,
    pub hide_units: bool,
    pub line_weight: f64,
    pub line_type: LineType,
    pub line_color: [u8; 3],
    pub arrow_style: ArrowStyle,
    pub arrow_scale: f64,
    pub extension_weight: f64,
    pub extension_type: LineType,
    pub extension_color: [u8; 3],
    pub extension_offset: f64,
    pub extension_length: f64,
    pub hide_extensions: bool,
    pub font_family: String,
    pub font_style: String,
    pub font_size: f64,
    pub text_color: [u8; 3],
    pub text_position: TextPosition,
    pub text_alignment: TextAlignment,
    pub text_offset: f64,
}

impl Default for DimensionOptions {
    fn default() -> Self {
        // The document's own type defaults: a font every build has, black, in points.
        let type_style = crate::CharStyle::default();
        Self {
            units: LengthUnit::Pt,
            precision: 2,
            scale: "1:1".into(),
            hide_units: false,
            line_weight: 0.5,
            line_type: LineType::Solid,
            line_color: [0, 0, 0],
            arrow_style: ArrowStyle::Filled,
            arrow_scale: 100.0,
            extension_weight: 0.5,
            extension_type: LineType::Solid,
            extension_color: [0, 0, 0],
            extension_offset: 4.0,
            extension_length: 6.0,
            hide_extensions: false,
            font_family: type_style.font_family,
            font_style: type_style.font_style,
            font_size: type_style.size,
            text_color: [0, 0, 0],
            text_position: TextPosition::Away,
            text_alignment: TextAlignment::Center,
            text_offset: 4.0,
        }
    }
}

impl DimensionOptions {
    /// Options given to a command or a tool: unknown keys are refused there, while files load
    /// tolerantly so a newer version's options don't stop a document opening.
    pub fn from_params(v: serde_json::Value) -> Result<Self, String> {
        if let (Some(given), Ok(serde_json::Value::Object(known))) = (v.as_object(), serde_json::to_value(Self::default()))
            && let Some(k) = given.keys().find(|k| !known.contains_key(*k))
        {
            return Err(format!("Unknown dimension option {k}"));
        }
        let o: Self = serde_json::from_value(v).map_err(|e| e.to_string())?;
        o.validate()?;
        Ok(o)
    }
    pub fn factor(&self) -> Result<f64, String> {
        let (a, b) = self.scale.split_once(':').ok_or("Scale must be drawing:real, for example 1:10")?;
        let parse = |s: &str| s.trim().parse::<f64>().ok().filter(|n| n.is_finite() && *n > 0.0 && *n <= 1e9);
        let a = parse(a).ok_or("Invalid drawing scale")?;
        let b = parse(b).ok_or("Invalid real scale")?;
        let f = b / a;
        if !(1e-6..=1e6).contains(&f) {
            return Err("Scale ratio must be between 1:1000000 and 1000000:1".into());
        }
        Ok(f)
    }
    pub fn validate(&self) -> Result<(), String> {
        self.factor()?;
        if self.precision > 6 {
            return Err("Choose a precision from 0 to 6".into());
        }
        for (v, min, max) in [
            (self.line_weight, 0.1, 100.0),
            (self.extension_weight, 0.1, 100.0),
            (self.arrow_scale, 1.0, 1000.0),
            (self.font_size, 1.0, 1000.0),
            (self.extension_offset, 0.0, 10000.0),
            (self.extension_length, 0.0, 10000.0),
            (self.text_offset, 0.0, 10000.0),
        ] {
            if !v.is_finite() || v < min || v > max {
                return Err("Dimension size is outside its supported range".into());
            }
        }
        if self.font_family.is_empty() || self.font_family.len() > 256 || self.font_style.len() > 128 {
            return Err("Invalid dimension font name".into());
        }
        Ok(())
    }
    /// `value` in points for a length, in degrees for an angle.
    pub fn label(&self, value: f64, angle: bool) -> Result<String, String> {
        self.validate()?;
        let value = if angle { value } else { value * self.factor()? / self.units.unit().points() };
        if !value.is_finite() {
            return Err("Non-finite dimension".into());
        }
        let suffix = if self.hide_units {
            ""
        } else if angle {
            "°"
        } else {
            self.units.suffix()
        };
        let sep = if suffix.is_empty() || angle { "" } else { " " };
        Ok(format!("{:.*}{sep}{suffix}", self.precision, value))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SegmentRef {
    pub node: NodeId,
    pub subpath: usize,
    pub segment: usize,
}

impl SegmentRef {
    /// The segment's two end points.
    pub fn endpoints(&self, doc: &Document) -> Option<(Point, Point)> {
        let sp = doc.node(self.node)?.path_data()?.subpaths.get(self.subpath)?;
        if self.segment >= sp.segment_count() {
            return None;
        }
        let seg = sp.segment(self.segment);
        Some((seg.p0, seg.p3))
    }
    /// Whether the segment is a straight line.
    pub fn is_straight(&self, doc: &Document) -> bool {
        doc.node(self.node)
            .and_then(|n| n.path_data())
            .and_then(|p| p.subpaths.get(self.subpath))
            .is_some_and(|sp| self.segment < sp.segment_count() && sp.segment_is_line(self.segment))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum DimensionSource {
    Segment {
        edge: SegmentRef,
    },
    Angle {
        first: SegmentRef,
        second: SegmentRef,
    },
    Circle {
        node: NodeId,
    },
    /// One segment that is a perfect arc.
    Arc {
        edge: SegmentRef,
    },
    Points {
        a: Option<PointRef>,
        b: Option<PointRef>,
        vertex: Option<PointRef>,
    },
}

impl DimensionSource {
    pub fn nodes(&self) -> Vec<NodeId> {
        match self {
            Self::Segment { edge } | Self::Arc { edge } => vec![edge.node],
            Self::Angle { first, second } => vec![first.node, second.node],
            Self::Circle { node } => vec![*node],
            Self::Points { a, b, vertex } => [a, b, vertex].into_iter().filter_map(|p| p.as_ref().map(|r| r.node)).collect(),
        }
    }
    /// Anchor indexes cannot safely follow a topology replacement. Ordinary moves and handle
    /// edits keep their topology and remain linked.
    pub fn topology_changed(&self, before: &Document, after: &Document) -> bool {
        if matches!(self, Self::Circle { .. }) {
            return false;
        }
        self.nodes().iter().any(|id| {
            let counts = |doc: &Document| {
                doc.node(*id).and_then(|n| n.path_data()).map(|p| p.subpaths.iter().map(|sp| (sp.anchors.len(), sp.closed)).collect::<Vec<_>>())
            };
            let old = counts(before);
            old.is_some() && old != counts(after)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PointRef {
    pub node: NodeId,
    pub subpath: usize,
    pub anchor: usize,
}

impl PointRef {
    pub fn position(&self, doc: &Document) -> Option<Point> {
        Some(doc.node(self.node)?.path_data()?.subpaths.get(self.subpath)?.anchors.get(self.anchor)?.p)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Dimension {
    pub mode: DimensionMode,
    pub a: Point,
    pub b: Point,
    /// Angular vertex. For radius/diameter a is the centre and b lies on the circle.
    pub vertex: Point,
    pub offset: f64,
    pub length: f64,
    pub options: DimensionOptions,
    pub source: Option<DimensionSource>,
    /// How far a linked dimension was moved on its own since the last refresh: the refresh turns
    /// it into a new placement unless the measured art moved too.
    #[serde(skip)]
    pub moved: Option<Vec2>,
}

impl Dimension {
    /// A point on the drawn dimension line (linear), arc (angular) or radius line.
    pub fn handle(&self) -> Point {
        match self.mode {
            DimensionMode::Linear => self.a.midpoint(self.b) + self.normal() * self.offset,
            DimensionMode::Angular => self.vertex + self.bisector() * self.offset.abs().max(12.0),
            DimensionMode::Radius | DimensionMode::Diameter => self.b,
        }
    }
    /// The left normal of a → b.
    pub fn normal(&self) -> Vec2 {
        let u = self.b - self.a;
        let len = u.hypot();
        if len > 1e-9 { Vec2::new(-u.y, u.x) / len } else { Vec2::new(0.0, 1.0) }
    }
    fn bisector(&self) -> Vec2 {
        let start = (self.a - self.vertex).atan2();
        Vec2::from_angle(start + self.angle() * 0.5)
    }
    /// Places the dimension through `p`: the line's distance from the art (linear), the arc's
    /// radius (angular) or the label's direction (radial).
    pub fn place(&mut self, p: Point) {
        match self.mode {
            DimensionMode::Linear => self.offset = (p - self.a).dot(self.normal()),
            DimensionMode::Angular => self.offset = p.distance(self.vertex).max(12.0),
            DimensionMode::Radius | DimensionMode::Diameter => {
                if p.distance(self.a) > 1e-6 {
                    let r = self.a.distance(self.b);
                    let u = p - self.a;
                    self.b = self.a + u / u.hypot() * r;
                }
            }
        }
    }
    pub fn transform(&mut self, xf: Affine) {
        if self.source.is_some() {
            // The art it measures decides where its ends are: record the move, and the refresh
            // after this edit turns it into a new placement unless the art moved with it.
            let at = self.handle();
            self.moved = Some((xf * at) - at + self.moved.unwrap_or_default());
            return;
        }
        self.a = xf * self.a;
        self.b = xf * self.b;
        self.vertex = xf * self.vertex;
        if self.mode == DimensionMode::Linear {
            self.length = self.a.distance(self.b);
        }
    }
    pub fn resolve(&mut self, doc: &Document) -> Result<(), String> {
        match &self.source {
            Some(DimensionSource::Points { a, b, vertex }) => {
                for (reference, point) in [(a, &mut self.a), (b, &mut self.b), (vertex, &mut self.vertex)] {
                    if let Some(r) = reference {
                        *point = r.position(doc).ok_or("Measured anchor no longer exists")?;
                    }
                }
                self.length = self.a.distance(self.b);
            }
            Some(DimensionSource::Segment { edge }) => {
                let (a, b) = edge.endpoints(doc).ok_or("Measured segment no longer exists")?;
                self.a = a;
                self.b = b;
                self.length = a.distance(b);
            }
            Some(DimensionSource::Angle { first, second }) => {
                for edge in [first, second] {
                    if !edge.is_straight(doc) {
                        return Err("Angles require two straight segments".into());
                    }
                }
                let (a, b) = first.endpoints(doc).ok_or("First measured segment no longer exists")?;
                let (c, d) = second.endpoints(doc).ok_or("Second measured segment no longer exists")?;
                let u = b - a;
                let v = d - c;
                let den = u.cross(v);
                if den.abs() < 1e-9 {
                    return Err("Choose two non-parallel straight segments".into());
                }
                let p = a + u * ((c - a).cross(v) / den);
                self.vertex = p;
                self.a = if a.distance(p) > b.distance(p) { a } else { b };
                self.b = if c.distance(p) > d.distance(p) { c } else { d };
            }
            Some(DimensionSource::Circle { node }) => {
                let (center, radius) = circle(doc, *node).ok_or("Choose a circle or a perfect arc")?;
                self.set_radial(center, radius);
            }
            Some(DimensionSource::Arc { edge }) => {
                let (center, radius) = arc(doc, edge).ok_or("Choose a circle or a perfect arc")?;
                self.set_radial(center, radius);
            }
            None => {}
        }
        self.validate()
    }
    fn set_radial(&mut self, center: Point, radius: f64) {
        let direction = self.b - self.a;
        let dir = if direction.hypot() > 1e-9 { direction / direction.hypot() } else { Vec2::new(1.0, 0.0) };
        self.a = center;
        self.b = center + dir * radius;
        self.length = radius;
    }
    pub fn validate(&self) -> Result<(), String> {
        self.options.validate()?;
        for p in [self.a, self.b, self.vertex] {
            if !p.x.is_finite() || !p.y.is_finite() || p.x.abs() > 1e8 || p.y.abs() > 1e8 {
                return Err("Dimension point is outside the canvas".into());
            }
        }
        if !self.offset.is_finite() || self.offset.abs() > 1e6 || !self.length.is_finite() || self.length < 0.0 || self.length > 1e9 {
            return Err("Invalid dimension distance".into());
        }
        if self.a.distance(self.b) < 1e-6 {
            return Err("Choose two different points".into());
        }
        if self.mode == DimensionMode::Angular && (self.a.distance(self.vertex) < 1e-6 || self.b.distance(self.vertex) < 1e-6) {
            return Err("Choose two nonzero angle arms".into());
        }
        Ok(())
    }
    pub fn angle(&self) -> f64 {
        let u = self.a - self.vertex;
        let v = self.b - self.vertex;
        u.cross(v).atan2(u.dot(v))
    }
    pub fn label(&self) -> Result<String, String> {
        let value = match self.mode {
            DimensionMode::Angular => self.angle().abs().to_degrees(),
            DimensionMode::Radius => self.a.distance(self.b),
            DimensionMode::Diameter => self.a.distance(self.b) * 2.0,
            DimensionMode::Linear => self.a.distance(self.b),
        };
        self.options.label(value, self.mode == DimensionMode::Angular)
    }
}

/// Accept circular paths (including imported circles), reject ellipses and arbitrary closed shapes.
pub fn circle(doc: &Document, id: NodeId) -> Option<(Point, f64)> {
    use vectorcraft_geom::kurbo::ParamCurve;
    let path = doc.node(id)?.path_data()?;
    let sp = path.subpaths.first()?;
    if path.subpaths.len() != 1 || !sp.closed || !(2..=64).contains(&sp.segment_count()) {
        return None;
    }
    let bounds = path.bounds()?;
    let center = bounds.center();
    let radius = bounds.width() * 0.5;
    if radius < 1e-6 || (bounds.height() * 0.5 - radius).abs() > radius * 0.002 {
        return None;
    }
    for i in 0..sp.segment_count() {
        for t in [0.0, 0.25, 0.5, 0.75] {
            if (sp.segment(i).eval(t).distance(center) - radius).abs() > radius * 0.002 {
                return None;
            }
        }
    }
    Some((center, radius))
}

/// A curved segment that follows one circle (a perfect arc): its centre and radius.
pub fn arc(doc: &Document, edge: &SegmentRef) -> Option<(Point, f64)> {
    use vectorcraft_geom::kurbo::ParamCurve;
    let sp = doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
    if edge.segment >= sp.segment_count() || sp.segment_is_line(edge.segment) {
        return None;
    }
    let seg = sp.segment(edge.segment);
    let (p, q, r) = (seg.p0, seg.eval(0.5), seg.p3);
    // The circle through the ends and the middle of the segment.
    let d = 2.0 * (p.x * (q.y - r.y) + q.x * (r.y - p.y) + r.x * (p.y - q.y));
    if d.abs() < 1e-9 {
        return None;
    }
    let sq = |a: Point| a.x * a.x + a.y * a.y;
    let center = Point::new(
        (sq(p) * (q.y - r.y) + sq(q) * (r.y - p.y) + sq(r) * (p.y - q.y)) / d,
        (sq(p) * (r.x - q.x) + sq(q) * (p.x - r.x) + sq(r) * (q.x - p.x)) / d,
    );
    let radius = center.distance(p);
    if !radius.is_finite() || !(1e-6..=1e8).contains(&radius) {
        return None;
    }
    for t in [0.125, 0.25, 0.375, 0.625, 0.75, 0.875] {
        if (seg.eval(t).distance(center) - radius).abs() > radius * 0.002 {
            return None;
        }
    }
    Some((center, radius))
}

/// The two straight segments that meet at a corner anchor, if both are straight.
pub fn corner(doc: &Document, node: NodeId, subpath: usize, anchor: usize) -> Option<(SegmentRef, SegmentRef)> {
    let sp = doc.node(node)?.path_data()?.subpaths.get(subpath)?;
    let count = sp.segment_count();
    let n = sp.anchors.len();
    if count < 2 || anchor >= n {
        return None;
    }
    // Segment i runs from anchor i to anchor i + 1 (wrapping on closed paths).
    let before = if anchor > 0 {
        anchor - 1
    } else if sp.closed {
        count - 1
    } else {
        return None;
    };
    let after = anchor;
    if after >= count {
        return None;
    }
    let first = SegmentRef { node, subpath, segment: before };
    let second = SegmentRef { node, subpath, segment: after };
    (first.is_straight(doc) && second.is_straight(doc)).then_some((first, second))
}
