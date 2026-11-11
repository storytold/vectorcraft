//! Persistent dimension recipes. Children are ordinary paths and type for all renderers/exporters.
use crate::{Document, NodeId, Unit};
use serde::{Deserialize, Serialize};
use vectorcraft_geom::{Affine, Anchor, PathData, Point, Vec2};

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

/// The direction a linear dimension measures in: along the line through its two points, or only
/// their horizontal or vertical distance.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Aligned,
    Horizontal,
    Vertical,
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
        /// The first point when it is not an anchor, held against a path's anchors.
        #[serde(default, rename = "freeA", skip_serializing_if = "Option::is_none")]
        free_a: Option<FreePoint>,
        /// The second point when it is not an anchor.
        #[serde(default, rename = "freeB", skip_serializing_if = "Option::is_none")]
        free_b: Option<FreePoint>,
    },
}

impl DimensionSource {
    pub fn nodes(&self) -> Vec<NodeId> {
        match self {
            Self::Segment { edge } | Self::Arc { edge } => vec![edge.node],
            Self::Angle { first, second } => vec![first.node, second.node],
            Self::Circle { node } => vec![*node],
            Self::Points { a, b, vertex, free_a, free_b } => [a, b, vertex]
                .into_iter()
                .filter_map(|p| p.as_ref().map(|r| r.node))
                .chain([free_a, free_b].into_iter().filter_map(|p| p.as_ref().map(FreePoint::node)))
                .collect(),
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
    /// The map that took the measured art from `before` to `after`, when every path it measures
    /// went through one [`art_transform`] that reflects it (a negative determinant). None when a
    /// measured path is unchanged, or when two of them went through different maps.
    pub fn reflection(&self, before: &Document, after: &Document) -> Option<Affine> {
        let mut first: Option<Affine> = None;
        for id in self.nodes() {
            let (old, new) = (before.node(id)?, after.node(id)?);
            if std::ptr::eq(old, new) || old.path_data() == new.path_data() {
                return None;
            }
            let xf = fitted(before, after, id, |xf| xf.determinant() < 0.0)?;
            match first {
                None => first = Some(xf),
                Some(f) => {
                    // The linear parts and the translations, each against its own size.
                    let (a, b) = (f.as_coeffs(), xf.as_coeffs());
                    let differ = |range: std::ops::Range<usize>, tolerance: f64| {
                        let size = a[range.clone()].iter().map(|c| c.abs()).fold(1.0, f64::max);
                        a[range.clone()].iter().zip(&b[range]).any(|(p, q)| (p - q).abs() > tolerance * size)
                    };
                    if differ(0..4, 1e-9) || differ(4..6, 1e-9) {
                        return None;
                    }
                }
            }
        }
        first
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

/// The point of an anchor that an [`AnchorWeight`] names: the anchor, or one of its handles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AnchorPart {
    #[default]
    Point,
    In,
    Out,
}

impl AnchorPart {
    fn of(self, a: &Anchor) -> Point {
        match self {
            Self::Point => a.p,
            Self::In => a.h_in,
            Self::Out => a.h_out,
        }
    }
}

/// One anchor's share of a [`FreePoint`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AnchorWeight {
    pub subpath: usize,
    pub anchor: usize,
    /// The anchor itself, or its in or out handle.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub part: AnchorPart,
    pub weight: f64,
}

/// A measured point that is not an anchor (a point on a segment, or a point a Smart Guide gave),
/// linked to one path so that it moves with it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum FreePoint {
    /// A point on a segment at curve parameter `t`, 0 at the segment's first anchor and 1 at its
    /// last. It stays on the segment when the segment's anchors or handles are edited.
    OnSegment { edge: SegmentRef, t: f64 },
    /// A weighted sum of two or three anchors or handles of one path, the weights adding up to 1,
    /// plus `across` times the vector from the first anchor to the second turned a quarter turn,
    /// from (x, y) to (-y, x), so that a vector pointing right in the y-down document space
    /// turns to point down (two anchors only). When the
    /// path is moved, scaled, rotated, reflected or sheared, a point held by three moves with
    /// it. A point held by two and `across`, beside a straight path, moves, turns and scales
    /// uniformly with it, and a reflection or a non-uniform scale moves it as a half turn or a
    /// uniform scale would.
    Anchors {
        node: NodeId,
        anchors: Vec<AnchorWeight>,
        #[serde(default, skip_serializing_if = "crate::skip::is_default")]
        across: f64,
    },
}

/// A point of a path: (subpath, anchor, part, place).
type PathPoint = (usize, usize, AnchorPart, Point);

/// Every anchor of `path` in order, each followed by its in and out handles when `handles` is set.
fn path_points(path: &PathData, handles: bool) -> impl Iterator<Item = PathPoint> + Clone + '_ {
    let parts: &'static [AnchorPart] = if handles { &[AnchorPart::Point, AnchorPart::In, AnchorPart::Out] } else { &[AnchorPart::Point] };
    path.subpaths
        .iter()
        .enumerate()
        .flat_map(move |(si, sp)| sp.anchors.iter().enumerate().flat_map(move |(ai, a)| parts.iter().map(move |&part| (si, ai, part, part.of(a)))))
}

/// Three points that span `points`: the first, the one farthest from it, and the one farthest
/// from the line through those two. None when every point is in one place.
fn spanning(points: impl Iterator<Item = PathPoint> + Clone) -> Option<[PathPoint; 3]> {
    let first = points.clone().next()?;
    let o = first.3;
    let second = points.clone().max_by(|x, y| x.3.distance(o).total_cmp(&y.3.distance(o)))?;
    let u = second.3 - o;
    let span = u.hypot();
    if !span.is_finite() || span <= 1e-6 {
        return None;
    }
    let third = points.max_by(|x, y| u.cross(x.3 - o).abs().total_cmp(&u.cross(y.3 - o).abs()))?;
    Some([first, second, third])
}

/// The map that takes the anchors and handles of path `node` in `before` to where they are in
/// `after`, when one affine map does that for every one of them: the path was moved, scaled,
/// rotated, reflected or sheared as a whole. None when its anchors changed in number, moved in
/// another way, or lie on one line with all their handles, as on a straight line: a reflection
/// across a line through such a path puts its points where a turn can put them too.
pub fn art_transform(before: &Document, after: &Document, node: NodeId) -> Option<Affine> {
    fitted(before, after, node, |_| true)
}

/// [`art_transform`] when the map passes `wanted`. The map is found from three points, and
/// `wanted` is asked before every other point is checked against it.
fn fitted(before: &Document, after: &Document, node: NodeId, wanted: impl Fn(&Affine) -> bool) -> Option<Affine> {
    let (old, new) = (before.node(node)?.path_data()?, after.node(node)?.path_data()?);
    if old.subpaths.len() != new.subpaths.len() || old.subpaths.iter().zip(&new.subpaths).any(|(o, n)| o.anchors.len() != n.anchors.len()) {
        return None;
    }
    let [o, f, g] = spanning(path_points(old, true))?;
    let place = |q: PathPoint| new.subpaths.get(q.0).and_then(|sp| sp.anchors.get(q.1)).map(|a| q.2.of(a));
    let (o2, f2, g2) = (place(o)?, place(f)?, place(g)?);
    let (u, v) = (f.3 - o.3, g.3 - o.3);
    let span = u.hypot();
    if u.cross(v).abs() <= 1e-9 * span * span {
        return None;
    }
    let basis = |u: Vec2, v: Vec2| Affine::new([u.x, u.y, v.x, v.y, 0.0, 0.0]);
    let linear = basis(f2 - o2, g2 - o2) * basis(u, v).inverse();
    let xf = Affine::translate(o2.to_vec2()) * linear * Affine::translate(-o.3.to_vec2());
    if !xf.as_coeffs().iter().all(|c| c.is_finite()) || !wanted(&xf) {
        return None;
    }
    let tolerance = 1e-6 * span.max(1.0);
    path_points(old, true).zip(path_points(new, true)).all(|(o, n)| (xf * o.3).distance(n.3) <= tolerance).then_some(xf)
}

impl FreePoint {
    /// Largest weight: a point far outside the anchors that hold it is not held.
    const MAX_WEIGHT: f64 = 1e4;

    /// The path it is linked to.
    pub fn node(&self) -> NodeId {
        match self {
            Self::OnSegment { edge, .. } => edge.node,
            Self::Anchors { node, .. } => *node,
        }
    }

    /// `p` held against path `node`: three anchors that span the path; when all its anchors lie
    /// on one line or in one place, as on an arch, three of its anchors and handles; when its
    /// handles lie on that line too, or so nearly that the weights would be too large, two anchors
    /// and `p`'s distance across that line. None when none of these holds `p`.
    pub fn on(doc: &Document, node: NodeId, p: Point) -> Option<Self> {
        let path = doc.node(node)?.path_data()?;
        let weight = |(subpath, anchor, part, _): PathPoint, weight| AnchorWeight { subpath, anchor, part, weight };
        for handles in [false, true] {
            let Some([o, f, g]) = spanning(path_points(path, handles)) else { continue };
            let (u, v) = (f.3 - o.3, g.3 - o.3);
            let span = u.hypot();
            let det = u.cross(v);
            if det.abs() > 1e-9 * span * span {
                let s = (p - o.3).cross(v) / det;
                let t = u.cross(p - o.3) / det;
                let anchors = vec![weight(o, 1.0 - s - t), weight(f, s), weight(g, t)];
                // A nearly flat set of points gives weights too large to keep: the next fit is tried.
                if let Some(held) = (Self::Anchors { node, anchors, across: 0.0 }).holding(doc, p, span) {
                    return Some(held);
                }
            }
        }
        // Every anchor and handle on one line, or nearly: along it from the first anchor, and across it.
        let [o, f, _] = spanning(path_points(path, false))?;
        let u = f.3 - o.3;
        let span = u.hypot();
        let (t, across) = ((p - o.3).dot(u) / (span * span), u.cross(p - o.3) / (span * span));
        Self::Anchors { node, anchors: vec![weight(o, 1.0 - t), weight(f, t)], across }.holding(doc, p, span)
    }

    /// `p` on the segment `edge`, at the curve parameter of the segment's point nearest to it.
    /// None when the segment is gone or `p` is not on it.
    pub fn on_segment(doc: &Document, edge: &SegmentRef, p: Point) -> Option<Self> {
        use vectorcraft_geom::kurbo::ParamCurveNearest;
        let sp = doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
        if edge.segment >= sp.segment_count() {
            return None;
        }
        let (a, b) = edge.endpoints(doc)?;
        let t = sp.segment(edge.segment).nearest(p, 1e-12).t;
        Self::OnSegment { edge: edge.clone(), t }.holding(doc, p, a.distance(b))
    }

    /// Itself when it is [`Self::valid`] and puts the point at `p`, within a tolerance that grows
    /// with `span`, the size of the art that holds it.
    fn holding(self, doc: &Document, p: Point, span: f64) -> Option<Self> {
        let at = self.position(doc)?;
        (at.distance(p) <= 1e-6 * span.max(1.0)).then_some(self)
    }

    /// Whether its numbers can be used: a finite `t` from 0 to 1 on a segment; or two or three
    /// weights that add up to 1, with `across` set only for two, each finite and at most
    /// [`Self::MAX_WEIGHT`] across.
    pub fn valid(&self) -> bool {
        match self {
            Self::OnSegment { t, .. } => (0.0..=1.0).contains(t),
            Self::Anchors { anchors, across, .. } => {
                (2..=3).contains(&anchors.len())
                    && anchors.iter().all(|w| w.weight.is_finite() && w.weight.abs() <= Self::MAX_WEIGHT)
                    && (anchors.iter().map(|w| w.weight).sum::<f64>() - 1.0).abs() <= 1e-9
                    && across.is_finite()
                    && across.abs() <= Self::MAX_WEIGHT
                    && (*across == 0.0 || anchors.len() == 2)
            }
        }
    }

    /// Where the point is now. None when its segment or an anchor is gone or the point is not
    /// [`Self::valid`].
    pub fn position(&self, doc: &Document) -> Option<Point> {
        use vectorcraft_geom::kurbo::ParamCurve;
        if !self.valid() {
            return None;
        }
        let at = match self {
            Self::OnSegment { edge, t } => {
                let sp = doc.node(edge.node)?.path_data()?.subpaths.get(edge.subpath)?;
                if edge.segment >= sp.segment_count() {
                    return None;
                }
                sp.segment(edge.segment).eval(*t)
            }
            Self::Anchors { node, anchors, across } => {
                let path = doc.node(*node)?.path_data()?;
                let place = |w: &AnchorWeight| path.subpaths.get(w.subpath).and_then(|sp| sp.anchors.get(w.anchor)).map(|a| w.part.of(a).to_vec2());
                let mut at = Vec2::ZERO;
                for w in anchors {
                    at += place(w)? * w.weight;
                }
                if *across != 0.0 {
                    let u = place(anchors.get(1)?)? - place(anchors.first()?)?;
                    at += Vec2::new(-u.y, u.x) * *across;
                }
                at.to_point()
            }
        };
        (at.x.is_finite() && at.y.is_finite()).then_some(at)
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
    /// What a linear dimension measures: the distance between its points (aligned), or only its
    /// horizontal or vertical part. Other modes ignore it.
    #[serde(default, skip_serializing_if = "crate::skip::is_default")]
    pub orientation: Orientation,
    /// How far a linked dimension was moved on its own since the last refresh: the refresh turns
    /// it into a new placement unless the measured art moved too.
    #[serde(skip)]
    pub moved: Option<Vec2>,
}

impl Dimension {
    /// A point on the drawn dimension line (linear), arc (angular) or radius line.
    pub fn handle(&self) -> Point {
        match self.mode {
            DimensionMode::Linear => {
                let (a, b) = self.line_ends();
                a.midpoint(b)
            }
            DimensionMode::Angular => self.vertex + self.bisector() * self.offset.abs().max(12.0),
            DimensionMode::Radius | DimensionMode::Diameter => self.b,
        }
    }
    /// The unit direction a linear dimension measures along: a → b when aligned, else the x axis
    /// (horizontal) or the y axis (vertical), whichever way the points lie.
    pub fn direction(&self) -> Vec2 {
        match self.orientation {
            Orientation::Horizontal if self.mode == DimensionMode::Linear => Vec2::new(1.0, 0.0),
            Orientation::Vertical if self.mode == DimensionMode::Linear => Vec2::new(0.0, 1.0),
            _ => {
                let u = self.b - self.a;
                let len = u.hypot();
                if len > 1e-9 { u / len } else { Vec2::new(1.0, 0.0) }
            }
        }
    }
    /// The left normal of [`Self::direction`].
    pub fn normal(&self) -> Vec2 {
        let u = self.direction();
        Vec2::new(-u.y, u.x)
    }
    /// The length a linear dimension shows: the distance between its points along
    /// [`Self::direction`].
    pub fn measured(&self) -> f64 {
        (self.b - self.a).dot(self.direction()).abs()
    }
    /// The two ends of a linear dimension's line: the measured points moved onto one line along
    /// [`Self::direction`], `offset` from a along the normal.
    pub fn line_ends(&self) -> (Point, Point) {
        let (u, n) = (self.direction(), self.normal());
        let a = self.a + n * self.offset;
        (a, a + u * (self.b - self.a).dot(u))
    }
    /// A horizontal or vertical dimension whose points moved from `old`'s keeps its line as far
    /// outside them as it was, on the same side: the gap is measured from the outermost point
    /// on the line's side, so the line stays clear of art that turns. A line that lay between
    /// the two points keeps its distance from the first point.
    pub fn keep_gap(&mut self, old: &Dimension) {
        if self.mode != DimensionMode::Linear
            || self.orientation == Orientation::Aligned
            || old.orientation != self.orientation
            || (self.a, self.b) == (old.a, old.b)
        {
            return;
        }
        let n = self.normal();
        let across = |p: Point| p.to_vec2().dot(n);
        let line = across(old.a) + old.offset;
        let (low, high) = (across(old.a).min(across(old.b)), across(old.a).max(across(old.b)));
        let (new_low, new_high) = (across(self.a).min(across(self.b)), across(self.a).max(across(self.b)));
        let at = if line >= high {
            new_high + (line - high)
        } else if line <= low {
            new_low + (line - low)
        } else {
            return;
        };
        let offset = at - across(self.a);
        if offset.is_finite() && offset.abs() <= 1e6 {
            self.offset = offset;
        }
    }
    /// A linear dimension whose art was reflected, its points now moved from `old`'s: its line goes
    /// to the side of the points that `image` (where the reflection put `old`'s line) is on, as far
    /// past the outermost point on that side as `old`'s line was. A line that lay between the two
    /// points is left as it is.
    pub fn mirror_line(&mut self, old: &Dimension, image: Point) {
        if self.mode != DimensionMode::Linear || old.mode != DimensionMode::Linear {
            return;
        }
        let gap = {
            let n = old.normal();
            let across = |p: Point| p.to_vec2().dot(n);
            let line = across(old.a) + old.offset;
            let (low, high) = (across(old.a).min(across(old.b)), across(old.a).max(across(old.b)));
            if line >= high {
                line - high
            } else if line <= low {
                low - line
            } else {
                return;
            }
        };
        let n = self.normal();
        let across = |p: Point| p.to_vec2().dot(n);
        let (low, high) = (across(self.a).min(across(self.b)), across(self.a).max(across(self.b)));
        let at = if 2.0 * across(image) >= low + high { high + gap } else { low - gap };
        let offset = at - across(self.a);
        if offset.is_finite() && offset.abs() <= 1e6 {
            self.offset = offset;
        }
    }
    /// Whether a linear dimension's line lies strictly between its two points along the normal.
    fn line_between_points(&self) -> bool {
        let n = self.normal();
        let across = |p: Point| p.to_vec2().dot(n);
        let line = across(self.a) + self.offset;
        across(self.a).min(across(self.b)) < line && line < across(self.a).max(across(self.b))
    }
    /// Makes a linear dimension measure in `orientation`, with its line on the side of the points
    /// it was on. The line goes through the middle of the old line when that point lies outside
    /// the two points along the new normal; otherwise it lies as far past the outermost point on
    /// that side as the old line lay from its points.
    pub fn set_orientation(&mut self, orientation: Orientation) {
        if self.mode != DimensionMode::Linear || self.orientation == orientation {
            self.orientation = orientation;
            return;
        }
        let (handle, gap) = (self.handle(), self.offset.abs());
        self.orientation = orientation;
        let n = self.normal();
        let across = |p: Point| p.to_vec2().dot(n);
        let (low, high) = (across(self.a).min(across(self.b)), across(self.a).max(across(self.b)));
        let h = across(handle);
        let at = if h >= high || h <= low {
            h
        } else if 2.0 * h >= low + high {
            high + gap
        } else {
            low - gap
        };
        let offset = at - across(self.a);
        if offset.is_finite() && offset.abs() <= 1e6 {
            self.offset = offset;
        }
        self.length = self.measured();
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
        // A horizontal or vertical dimension keeps measuring along its axis, between the points
        // where they now are, and its line goes through where the transform puts it. When that
        // puts the line between the points (a quarter turn), the line keeps its gap from them.
        let before = (self.mode == DimensionMode::Linear && self.orientation != Orientation::Aligned).then(|| (self.clone(), xf * self.handle()));
        self.a = xf * self.a;
        self.b = xf * self.b;
        self.vertex = xf * self.vertex;
        if let Some((old, p)) = before {
            let offset = (p - self.a).dot(self.normal());
            if offset.is_finite() && offset.abs() <= 1e6 {
                self.offset = offset;
            }
            if self.line_between_points() && !old.line_between_points() {
                self.keep_gap(&old);
            }
        }
        if self.mode == DimensionMode::Linear {
            self.length = self.measured();
        }
    }
    pub fn resolve(&mut self, doc: &Document) -> Result<(), String> {
        match &self.source {
            Some(DimensionSource::Points { a, b, vertex, free_a, free_b }) => {
                for (reference, free, point) in [(a, free_a, &mut self.a), (b, free_b, &mut self.b), (vertex, &None, &mut self.vertex)] {
                    match (reference, free) {
                        // An end is an anchor or a free point, never both.
                        (Some(_), Some(_)) => return Err("Invalid measured point".into()),
                        (Some(r), None) => *point = r.position(doc).ok_or("Measured anchor no longer exists")?,
                        (None, Some(f)) if !f.valid() => return Err("Invalid measured point".into()),
                        (None, Some(f)) => *point = f.position(doc).ok_or("Measured anchor no longer exists")?,
                        (None, None) => {}
                    }
                }
                self.length = if self.mode == DimensionMode::Linear { self.measured() } else { self.a.distance(self.b) };
            }
            Some(DimensionSource::Segment { edge }) => {
                let (a, b) = edge.endpoints(doc).ok_or("Measured segment no longer exists")?;
                self.a = a;
                self.b = b;
                self.length = self.measured();
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
            DimensionMode::Linear => self.measured(),
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
