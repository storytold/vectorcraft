//! Entities to drawing items: their geometry in world coordinates (drawing units, y up) and the
//! properties they take from their layer or block (resolved when the items become art).

use std::collections::BTreeMap;

use kurbo::{BezPath, PathEl};
use vectorcraft_doc::Justify;
use vectorcraft_geom::{Affine, Point, Vec2};

use craft_dxf::model::{self, EntityRecord, Groups, HatchEdge, HatchPath};

use super::curve::{self, Vertex};
use super::reader::{aci_paper, point};

/// How far below the baseline the lowest descender reaches, as a fraction of the cap height.
const DESCENT: f64 = 0.3;
/// Baseline-to-baseline distance of multiline text at line spacing 1, in cap heights.
const LINE_SPACING: f64 = 5.0 / 3.0;
/// The most copies one array insert (MINSERT) makes.
const MAX_CELLS: i64 = 10_000;
/// The most copies all the array inserts of a file make together.
const MAX_TOTAL_CELLS: i64 = 2_000_000;
/// Art farther than this from the origin (drawing units) is damaged data, left out.
const MAX_COORD: f64 = 1e12;
/// How far fit and aligned text may be squeezed or stretched to span their two points: at width
/// factor 1, the 1 to 10,000% horizontal scale the Character panel allows.
const SPAN_SCALE: (f64, f64) = (0.01, 100.0);

/// An entity's colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Col {
    ByLayer,
    ByBlock,
    Rgb([u8; 3]),
}

/// An entity's lineweight.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Lw {
    ByLayer,
    ByBlock,
    Default,
    /// Hundredths of a millimetre.
    Hundredths(i32),
}

/// An entity's opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Alpha {
    ByLayer,
    ByBlock,
    Opacity(f32),
}

/// What an entity takes from its layer or block unless it sets it itself.
#[derive(Clone, Debug)]
pub(crate) struct Props {
    pub layer: String,
    pub color: Col,
    pub lineweight: Lw,
    /// The linetype name in upper case (`BYLAYER`, `BYBLOCK`, `CONTINUOUS` or a table entry).
    pub linetype: String,
    pub ltscale: f64,
    pub alpha: Alpha,
}

impl Props {
    fn read(c: &model::Common) -> Self {
        let color = match c.color {
            model::Color::Rgb(rgb) => Col::Rgb(rgb),
            model::Color::ByBlock => Col::ByBlock,
            model::Color::ByLayer => Col::ByLayer,
            model::Color::Index(i) => Col::Rgb(aci_paper(i)),
        };
        let lineweight = match c.lineweight {
            model::Lineweight::ByBlock => Lw::ByBlock,
            model::Lineweight::Default => Lw::Default,
            model::Lineweight::Hundredths(v @ 0..=211) => Lw::Hundredths(i32::from(v)),
            _ => Lw::ByLayer,
        };
        let alpha = match c.transparency {
            model::Transparency::Alpha(a) => Alpha::Opacity(f32::from(a) / 255.0),
            model::Transparency::ByBlock => Alpha::ByBlock,
            model::Transparency::ByLayer => Alpha::ByLayer,
        };
        Self {
            layer: c.layer.clone(),
            color,
            lineweight,
            linetype: c.linetype.as_deref().unwrap_or("BYLAYER").to_uppercase(),
            ltscale: c.ltscale,
            alpha,
        }
    }
}

/// A line of text.
#[derive(Clone, Debug)]
pub(crate) struct TextItem {
    pub text: String,
    /// Cap height, in drawing units.
    pub height: f64,
    /// Text space (the first baseline's start at the origin, x along it, y up; drawing units) to
    /// world coordinates.
    pub xf: Affine,
    /// Width factor (1: as designed).
    pub width: f64,
    pub justify: Justify,
    /// The text style (upper case).
    pub style: String,
    /// A font family the text names itself (multiline text's font changes).
    pub family: Option<String>,
    /// Baseline-to-baseline distance (drawing units), for several lines.
    pub leading: Option<f64>,
    /// The distance aligned and fit text run along their baseline.
    pub span: Option<Span>,
}

/// How text fills the distance between its two points (drawing units).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Span {
    /// Stretched or squeezed across, its height kept.
    Fit(f64),
    /// Scaled as a whole, height included.
    Aligned(f64),
}

impl Span {
    /// The scale that makes text `natural` long run the span, within [`SPAN_SCALE`] (None when
    /// either length is degenerate).
    pub fn scale(self, natural: f64) -> Option<f64> {
        let (Self::Fit(d) | Self::Aligned(d)) = self;
        let k = d / natural;
        (k.is_finite() && k > 0.0).then(|| k.clamp(SPAN_SCALE.0, SPAN_SCALE.1))
    }
}

/// A block reference: one transform per copy (an array insert makes several), each from the
/// block's coordinates (less its base point) to world coordinates.
#[derive(Clone, Debug)]
pub(crate) struct InsertItem {
    /// The block (upper case).
    pub block: String,
    pub cells: Vec<Affine>,
    /// Its attributes' text.
    pub attribs: Vec<Item>,
}

#[derive(Clone, Debug)]
pub(crate) enum Geom {
    /// A path in world coordinates; `fill`: a filled area (hatch, solid), else a line of
    /// `width` drawing units (0: the lineweight).
    Path {
        bp: BezPath,
        fill: bool,
        width: f64,
    },
    Text(TextItem),
    Insert(InsertItem),
}

/// An entity as art: its geometry and its properties.
#[derive(Clone, Debug)]
pub(crate) struct Item {
    pub props: Props,
    pub geom: Geom,
}

/// Turns entity records into items, noting what it leaves out.
#[derive(Default)]
pub(crate) struct Converter {
    /// Entity types left out, with how many.
    pub skipped: BTreeMap<String, usize>,
    pub warnings: Vec<String>,
    /// Array insert copies made so far, against [`MAX_TOTAL_CELLS`].
    cells: i64,
}

const PATTERN_HATCH: &str = "pattern hatches come in as their boundaries";
const GRADIENT_HATCH: &str = "gradient hatches are filled with their first colour";
const MESHES: &str = "3D polygon meshes and polyface meshes are left out";
const BIG_ARRAY: &str = "array inserts are limited to 10,000 copies each";
const MANY_ARRAYS: &str = "array inserts are limited to 2,000,000 copies in all";
const FAR_AWAY: &str = "entities with coordinates out of range are left out";

impl Converter {
    fn warn(&mut self, w: &str) {
        if !self.warnings.iter().any(|x| x == w) {
            self.warnings.push(w.to_string());
        }
    }

    fn skip(&mut self, kind: &str) {
        *self.skipped.entry(kind.to_string()).or_default() += 1;
    }

    /// The items of `records` (an entity section or a block), in drawing order.
    pub fn items(&mut self, records: &[EntityRecord<'_>]) -> Vec<Item> {
        let mut out = Vec::new();
        for e in records {
            let m = model::Entity::read(e);
            // Invisible entities.
            if !m.common.visible {
                continue;
            }
            match self.geom(e, &m) {
                Some(geom) if !in_range(&geom) => self.warn(FAR_AWAY),
                Some(geom) => out.push(Item { props: Props::read(&m.common), geom }),
                None if is_known(e.kind) => {}
                None => self.skip(e.kind),
            }
        }
        out
    }

    fn geom(&mut self, e: &EntityRecord<'_>, m: &model::Entity) -> Option<Geom> {
        let g = e.groups();
        let path = |bp: BezPath| Some(Geom::Path { bp, fill: false, width: 0.0 });
        let ocs = ocs(m.common.extrusion);
        match &m.kind {
            model::EntityKind::Line { start, end } => {
                let mut bp = BezPath::new();
                bp.move_to(point(*start));
                bp.line_to(point(*end));
                path(bp)
            }
            model::EntityKind::Circle { center, radius } => path(transformed(curve::arc(point(*center), *radius, 0.0, std::f64::consts::TAU)?, ocs)),
            model::EntityKind::Arc { center, radius, start_angle, end_angle } => {
                let sweep = ccw_sweep(*start_angle, *end_angle);
                path(transformed(curve::arc(point(*center), *radius, start_angle.to_radians(), sweep.to_radians())?, ocs))
            }
            model::EntityKind::Ellipse { center, major, ratio, start, end } => {
                path(ellipse(m.common.extrusion, *center, *major, *ratio, *start, *end)?)
            }
            model::EntityKind::LwPolyline(p) => lwpolyline(p, ocs),
            model::EntityKind::Polyline(p) => self.polyline(p, ocs),
            model::EntityKind::Spline(s) => path(spline(s)?),
            model::EntityKind::Solid { corners } | model::EntityKind::Trace { corners } => {
                let [a, b, c, d] = corners.map(point);
                // The corners zigzag: the outline runs 1, 2, 4, 3.
                Some(Geom::Path { bp: transformed(polygon(&[a, b, d, c]), ocs), fill: true, width: 0.0 })
            }
            model::EntityKind::Face3d { corners, .. } => path(polygon(&corners.map(point))),
            model::EntityKind::Hatch(h) => self.hatch(h, ocs),
            model::EntityKind::Text(t) => text(t, &g, false, ocs).map(Geom::Text),
            model::EntityKind::MText(t) => mtext(t, e.tags).map(Geom::Text),
            model::EntityKind::Insert(ins) => self.insert(ins, &e.followers, ocs),
            // A dimension draws the anonymous block of its lines, arrows and text.
            model::EntityKind::Dimension(d) => {
                Some(Geom::Insert(InsertItem { block: d.block.as_ref()?.to_uppercase(), cells: vec![Affine::IDENTITY], attribs: vec![] }))
            }
            _ => None,
        }
    }

    fn polyline(&mut self, p: &model::Polyline, ocs: Affine) -> Option<Geom> {
        if p.is_mesh() || p.is_polyface() {
            self.warn(MESHES);
            return None;
        }
        // Spline frame control points and polyface faces aren't on the line.
        let verts: Vec<Vertex> =
            p.vertices.iter().filter(|v| v.flags & (16 | 128) == 0).map(|v| Vertex { p: point(v.location), bulge: v.bulge }).collect();
        let bp = curve::polyline(&verts, p.closed())?;
        // 3D polylines are in world coordinates; 2D ones in their plane's.
        let bp = if p.is_3d() { bp } else { transformed(bp, ocs) };
        Some(Geom::Path { bp, fill: false, width: p.start_width.max(0.0) })
    }

    fn insert(&mut self, ins: &model::Insert, followers: &[model::Record<'_>], ocs: Affine) -> Option<Geom> {
        let block = Some(ins.block.trim()).filter(|b| !b.is_empty())?.to_uppercase();
        let [sx, sy, _] = ins.scale;
        if sx.abs() < 1e-12 || sy.abs() < 1e-12 {
            return None;
        }
        let place = ocs * Affine::translate(point(ins.insert).to_vec2()) * Affine::rotate(ins.rotation.to_radians());
        let (cols, rows) = (ins.columns, ins.rows);
        let (cs, rs) = (ins.column_spacing, ins.row_spacing);
        let (cols, rows) = if cols.saturating_mul(rows) > MAX_CELLS {
            self.warn(BIG_ARRAY);
            (cols.min(MAX_CELLS), (MAX_CELLS / cols.min(MAX_CELLS)).max(1).min(rows))
        } else {
            (cols, rows)
        };
        // Past the file-wide budget, each insert keeps only its first copy.
        let left = (MAX_TOTAL_CELLS - self.cells).max(1);
        let (cols, rows) = if cols * rows > left {
            self.warn(MANY_ARRAYS);
            (cols.min(left), (left / cols.min(left)).max(1).min(rows))
        } else {
            (cols, rows)
        };
        self.cells = self.cells.saturating_add(cols * rows);
        let cells = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .map(|(r, c)| place * Affine::translate((c as f64 * cs, r as f64 * rs)) * Affine::scale_non_uniform(sx, sy))
            .collect();
        let attribs = self.attribs(followers);
        Some(Geom::Insert(InsertItem { block, cells, attribs }))
    }

    /// An insert's attributes as text.
    fn attribs(&mut self, followers: &[model::Record<'_>]) -> Vec<Item> {
        let mut out = Vec::new();
        for r in followers {
            let c = model::Common::read(r.tags);
            if !c.visible {
                continue;
            }
            let a = model::Attrib::read("ATTRIB", r.tags);
            match text(&a.text, &r.groups(), true, ocs(c.extrusion)) {
                Some(t) if !in_range(&Geom::Text(t.clone())) => self.warn(FAR_AWAY),
                Some(t) => out.push(Item { props: Props::read(&c), geom: Geom::Text(t) }),
                None => {}
            }
        }
        out
    }

    fn hatch(&mut self, h: &model::Hatch, ocs: Affine) -> Option<Geom> {
        let mut bp = BezPath::new();
        for l in &h.loops {
            let Some(path) = boundary(&l.path) else { break };
            bp.extend(path);
        }
        if bp.elements().is_empty() {
            return None;
        }
        let bp = transformed(bp, ocs);
        if h.gradient.is_some() {
            self.warn(GRADIENT_HATCH);
        } else if !h.solid {
            self.warn(PATTERN_HATCH);
            return Some(Geom::Path { bp, fill: false, width: 0.0 });
        }
        Some(Geom::Path { bp, fill: true, width: 0.0 })
    }
}

/// Is the geometry within [`MAX_COORD`] of the origin?
fn in_range(g: &Geom) -> bool {
    let near = |p: Point| p.x.abs() <= MAX_COORD && p.y.abs() <= MAX_COORD;
    let origin = |m: &Affine| near(m.translation().to_point());
    match g {
        Geom::Path { bp, width, .. } => {
            curve::bounds(bp).is_some_and(|b| near(Point::new(b.x0, b.y0)) && near(Point::new(b.x1, b.y1))) && *width <= MAX_COORD
        }
        Geom::Text(t) => origin(&t.xf) && t.height <= MAX_COORD,
        Geom::Insert(i) => i.cells.iter().all(|m| origin(m) && m.as_coeffs().iter().all(|v| v.is_finite() && v.abs() <= MAX_COORD)),
    }
}

/// Entity types that are read but draw nothing themselves (or were reported when read).
fn is_known(kind: &str) -> bool {
    matches!(
        kind,
        "LINE"
            | "CIRCLE"
            | "ARC"
            | "ELLIPSE"
            | "LWPOLYLINE"
            | "POLYLINE"
            | "SPLINE"
            | "SOLID"
            | "TRACE"
            | "3DFACE"
            | "HATCH"
            | "TEXT"
            | "ATTRIB"
            | "MTEXT"
            | "INSERT"
            | "DIMENSION"
            | "ATTDEF"
            | "SEQEND"
            | "VERTEX"
    )
}

/// The sweep from `start` to `end` degrees counter-clockwise, in (0, 360].
fn ccw_sweep(start: f64, end: f64) -> f64 {
    let s = (end - start).rem_euclid(360.0);
    if s < 1e-9 { 360.0 } else { s }
}

fn transformed(mut bp: BezPath, m: Affine) -> BezPath {
    if m != Affine::IDENTITY {
        bp.apply_affine(m);
    }
    bp
}

fn polygon(pts: &[Point]) -> BezPath {
    let mut bp = BezPath::new();
    for (i, p) in pts.iter().enumerate() {
        if i == 0 { bp.move_to(*p) } else { bp.line_to(*p) }
    }
    bp.close_path();
    bp
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// A planar entity's coordinate system seen from above: its x and y axes in world space by the
/// arbitrary axis algorithm (identity for drawings in the world's plane; extruded along −z it
/// mirrors x).
fn ocs(extrusion: model::Point) -> Affine {
    let [ax, ay, _] = model::ocs_axes(extrusion);
    if ax == [1.0, 0.0, 0.0] && ay == [0.0, 1.0, 0.0] {
        return Affine::IDENTITY;
    }
    Affine::new([ax[0], ax[1], ay[0], ay[1], 0.0, 0.0])
}

fn ellipse(extrusion: model::Point, centre: model::Point, major: model::Point, ratio: f64, start: f64, end: f64) -> Option<BezPath> {
    let centre = point(centre);
    let major = point(major).to_vec2();
    if ratio <= 0.0 || major.hypot() < 1e-12 {
        return None;
    }
    // The minor axis is the major one turned a quarter about the extrusion direction.
    let n = model::ocs_axes(extrusion)[2];
    let m = cross(n, [major.x, major.y, 0.0]);
    let minor = Vec2::new(m[0], m[1]) * ratio;
    let sweep = ccw_sweep(start.to_degrees(), end.to_degrees()).to_radians();
    curve::ellipse(Affine::new([major.x, major.y, minor.x, minor.y, centre.x, centre.y]), start, sweep)
}

fn lwpolyline(p: &model::LwPolyline, ocs: Affine) -> Option<Geom> {
    let verts: Vec<Vertex> = p.vertices.iter().map(|v| Vertex { p: Point::new(v.x, v.y), bulge: v.bulge }).collect();
    let bp = transformed(curve::polyline(&verts, p.closed)?, ocs);
    // One width for every vertex is the line's width.
    let widths: Vec<f64> = p.vertices.iter().flat_map(|v| [v.start_width, v.end_width]).collect();
    let uniform = widths.first().filter(|w| widths.iter().all(|x| (x - *w).abs() < 1e-12)).copied();
    let width = Some(p.const_width).filter(|w| *w > 0.0).or(uniform).unwrap_or(0.0).max(0.0);
    Some(Geom::Path { bp, fill: false, width })
}

fn spline(s: &model::Spline) -> Option<BezPath> {
    let ctrl: Vec<Point> = s.control.iter().map(|p| point(*p)).collect();
    let degree = usize::try_from(s.degree).ok()?;
    let weights = (s.flags & 4 != 0 && s.weights.len() == ctrl.len()).then_some(s.weights.as_slice());
    curve::nurbs(degree, &s.knots, &ctrl, weights)
        .map(|mut path| {
            if s.flags & 1 != 0 {
                path.close_path();
            }
            path
        })
        .or_else(|| curve::through(&s.fit.iter().map(|p| point(*p)).collect::<Vec<_>>(), s.flags & 1 != 0))
}

/// One hatch boundary loop, closed.
fn boundary(path: &HatchPath) -> Option<BezPath> {
    let xy = |p: &[f64; 2]| Point::new(p[0], p[1]);
    match path {
        HatchPath::Polyline { vertices, .. } => {
            let verts: Vec<Vertex> = vertices.iter().map(|[x, y, b]| Vertex { p: Point::new(*x, *y), bulge: *b }).collect();
            curve::polyline(&verts, true)
        }
        HatchPath::Edges(edges) => {
            let mut bp = BezPath::new();
            for e in edges {
                let seg = match e {
                    HatchEdge::Line { start, end } => {
                        let mut bp = BezPath::new();
                        bp.move_to(xy(start));
                        bp.line_to(xy(end));
                        Some(bp)
                    }
                    HatchEdge::Arc { center, radius, start_angle, end_angle, ccw } => {
                        let sweep = ccw_sweep(*start_angle, *end_angle);
                        // Clockwise arcs have their angles measured clockwise.
                        let (start, sweep) = if *ccw { (*start_angle, sweep) } else { (-start_angle, -sweep) };
                        curve::arc(xy(center), *radius, start.to_radians(), sweep.to_radians())
                    }
                    HatchEdge::Ellipse { center, major, ratio, start_angle, end_angle, ccw } => {
                        let (centre, major) = (xy(center), xy(major).to_vec2());
                        let minor = Vec2::new(-major.y, major.x) * *ratio * if *ccw { 1.0 } else { -1.0 };
                        let sweep = ccw_sweep(*start_angle, *end_angle).to_radians();
                        curve::ellipse(Affine::new([major.x, major.y, minor.x, minor.y, centre.x, centre.y]), start_angle.to_radians(), sweep)
                    }
                    HatchEdge::Spline { degree, rational, knots, control, weights, fit, .. } => {
                        let ctrl: Vec<Point> = control.iter().map(xy).collect();
                        let degree = usize::try_from(*degree).ok()?;
                        let weights = (*rational && weights.len() == ctrl.len()).then_some(weights.as_slice());
                        curve::nurbs(degree, knots, &ctrl, weights).or_else(|| curve::through(&fit.iter().map(xy).collect::<Vec<_>>(), false))
                    }
                };
                append(&mut bp, &seg?);
            }
            if bp.elements().is_empty() {
                return None;
            }
            bp.close_path();
            Some(bp)
        }
    }
}

/// `seg` continuing `bp`: joined by a line when it starts elsewhere.
fn append(bp: &mut BezPath, seg: &BezPath) {
    for el in seg.elements() {
        match *el {
            PathEl::MoveTo(p) => {
                let here = bp.elements().last().and_then(|e| e.end_point());
                match here {
                    None => bp.move_to(p),
                    Some(q) if q.distance(p) > 1e-9 => bp.line_to(p),
                    Some(_) => {}
                }
            }
            PathEl::ClosePath => {}
            other => bp.push(other),
        }
    }
}

/// The baseline offset of vertical alignment `v` (0 baseline, 1 bottom, 2 middle, 3 top).
fn baseline(v: i64, height: f64) -> f64 {
    match v {
        1 => height * DESCENT,
        2 => -height / 2.0,
        3 => -height,
        _ => 0.0,
    }
}

/// TEXT, or an ATTRIB's text. The string is read from group 1 as written, so that the caret and
/// percent codes are turned into text once, by `super::text`.
fn text(t: &model::Text, g: &Groups<'_>, attrib: bool, ocs: Affine) -> Option<TextItem> {
    if attrib && g.i_or(70, 0) & 1 != 0 {
        return None;
    }
    let s = super::text::plain(&g.s(1)?);
    if s.trim().is_empty() {
        return None;
    }
    let height = Some(t.height).filter(|h| *h > 0.0)?;
    // An ATTRIB's 73 is its field length: its vertical alignment is 74.
    let (h, v) = (t.halign, if attrib { g.i_or(74, 0) } else { t.valign });
    let (p1, p2) = (point(t.insert), t.align.map(point));
    let mut angle = t.rotation.to_radians();
    let mut span = None;
    // Aligned and fit text run from the first point to the second.
    let anchor = match (h, p2) {
        (3 | 5, Some(p)) => {
            angle = (p - p1).atan2();
            let d = p1.distance(p);
            span = (d > 1e-12).then_some(if h == 3 { Span::Aligned(d) } else { Span::Fit(d) });
            p1
        }
        (0, _) if v == 0 => p1,
        (_, Some(p)) => p,
        _ => p1,
    };
    let justify = match h {
        1 | 4 => Justify::Center,
        2 => Justify::Right,
        _ => Justify::Left,
    };
    let v = if h == 4 { 2 } else { v };
    let flags = t.generation;
    let mirror = Affine::scale_non_uniform(if flags & 2 != 0 { -1.0 } else { 1.0 }, if flags & 4 != 0 { -1.0 } else { 1.0 });
    let xf = ocs * Affine::translate(anchor.to_vec2()) * Affine::rotate(angle) * mirror * Affine::translate((0.0, baseline(v, height)));
    Some(TextItem {
        text: s,
        height,
        xf,
        width: Some(t.width_factor).filter(|w| *w > 0.0).unwrap_or(1.0),
        justify,
        style: t.style.to_uppercase(),
        family: None,
        leading: None,
        span,
    })
}

/// MTEXT. The contents are read from groups 3 and 1 as written, as for [`text`].
fn mtext(t: &model::MText, tags: &[craft_dxf::Tag]) -> Option<TextItem> {
    // The text comes in 250-character chunks (3) before the last (1), up to an embedded object.
    let (own, _) = model::text::split_embedded(tags);
    let g = Groups(own);
    let raw: String = own.iter().filter(|t| t.code == 3).map(craft_dxf::Tag::str).chain(g.s(1)).collect();
    let (s, family) = super::text::mtext(&raw);
    if s.trim().is_empty() {
        return None;
    }
    let height = Some(t.height).filter(|h| *h > 0.0)?;
    let leading = height * LINE_SPACING * Some(t.line_spacing).filter(|f| *f > 0.0).unwrap_or(1.0);
    let lines = s.lines().count().max(1) as f64;
    let attach = i64::from(t.attach) - 1;
    let justify = [Justify::Left, Justify::Center, Justify::Right].get((attach % 3) as usize).copied().unwrap_or_default();
    // The first baseline below the attachment point: under the top, around the middle, above
    // the bottom of the lines.
    let total = height + (lines - 1.0) * leading;
    let first = match attach / 3 {
        0 => -height,
        1 => total / 2.0 - height,
        _ => (lines - 1.0) * leading,
    };
    let xf = Affine::translate(point(t.insert).to_vec2()) * Affine::rotate(t.angle()) * Affine::translate((0.0, first));
    Some(TextItem { text: s, height, xf, width: 1.0, justify, style: t.style.to_uppercase(), family, leading: Some(leading), span: None })
}
