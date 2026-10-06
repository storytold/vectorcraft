//! Entities to drawing items: their geometry in world coordinates (drawing units, y up) and the
//! properties they take from their layer or block (resolved when the items become art).

use std::collections::BTreeMap;

use kurbo::{BezPath, PathEl};
use vectorcraft_doc::Justify;
use vectorcraft_geom::{Affine, Point, Vec2};

use super::curve::{self, Vertex};
use super::reader::{Groups, Pair, Record, groups, number};
use crate::aci::aci_rgb;

/// How far below the baseline the lowest descender reaches, as a fraction of the cap height.
const DESCENT: f64 = 0.3;
/// Baseline-to-baseline distance of multiline text at line spacing 1, in cap heights.
const LINE_SPACING: f64 = 5.0 / 3.0;
/// The most copies one array insert (MINSERT) makes.
const MAX_CELLS: i64 = 10_000;
/// Art farther than this from the origin (drawing units) is damaged data, left out.
const MAX_COORD: f64 = 1e12;

/// The colour of index `i` on paper: index 7 (white on a CAD screen) is black.
pub(crate) fn aci_paper(i: u64) -> [u8; 3] {
    match u8::try_from(i) {
        Ok(7) | Err(_) => [0; 3],
        Ok(i) => aci_rgb(i),
    }
}

/// The true colour (group 420, `0xRRGGBB`) of an entity or layer.
pub(crate) fn true_color(g: &[Pair<'_>]) -> Option<[u8; 3]> {
    let v = g.int(420)?;
    let [_, r, gr, b] = u32::try_from(v & 0xFF_FFFF).ok()?.to_be_bytes();
    Some([r, gr, b])
}

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
    fn read(g: &[Pair<'_>]) -> Self {
        let color = match (true_color(g), g.int(62)) {
            (Some(rgb), _) => Col::Rgb(rgb),
            (None, Some(0)) => Col::ByBlock,
            (None, None | Some(256..)) => Col::ByLayer,
            (None, Some(i)) => Col::Rgb(aci_paper(i.unsigned_abs())),
        };
        let lineweight = match g.int(370) {
            Some(-2) => Lw::ByBlock,
            Some(-3) => Lw::Default,
            Some(v @ 0..=211) => Lw::Hundredths(v as i32),
            _ => Lw::ByLayer,
        };
        let alpha = match g.int(440) {
            Some(v) if v & 0x0200_0000 != 0 => Alpha::Opacity((v & 0xFF) as f32 / 255.0),
            Some(v) if v & 0x0100_0000 != 0 => Alpha::ByBlock,
            _ => Alpha::ByLayer,
        };
        Self {
            layer: g.name(8).unwrap_or("0").to_string(),
            color,
            lineweight,
            linetype: g.name(6).unwrap_or("BYLAYER").to_uppercase(),
            ltscale: g.num(48).filter(|v| *v > 0.0).unwrap_or(1.0),
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
    /// Fit or Aligned text: what it must reach from its start to its second point.
    pub span: Option<Span>,
}

/// How Fit and Aligned text reach their second point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Span {
    /// Stretched or squeezed to this length (drawing units), keeping its height.
    Fit(f64),
    /// Scaled as a whole to this length.
    Aligned(f64),
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
}

const PATTERN_HATCH: &str = "pattern hatches come in as their boundaries";
const GRADIENT_HATCH: &str = "gradient hatches are filled with their first colour";
const MESHES: &str = "3D polygon meshes and polyface meshes are left out";
const BIG_ARRAY: &str = "array inserts are limited to 10,000 copies each";
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
    pub fn items(&mut self, records: &[Record<'_>]) -> Vec<Item> {
        let mut out = Vec::new();
        for group in groups(records) {
            let Some(head) = group.first() else { continue };
            // Invisible entities.
            if head.g.int(60) == Some(1) {
                continue;
            }
            match self.geom(head, group) {
                Some(geom) if !in_range(&geom) => self.warn(FAR_AWAY),
                Some(geom) => out.push(Item { props: Props::read(head.g), geom }),
                None if is_known(head.kind) => {}
                None => self.skip(head.kind),
            }
        }
        out
    }

    fn geom(&mut self, head: &Record<'_>, group: &[Record<'_>]) -> Option<Geom> {
        let g = head.g;
        let path = |bp: BezPath| Some(Geom::Path { bp, fill: false, width: 0.0 });
        match head.kind {
            "LINE" => {
                let mut bp = BezPath::new();
                bp.move_to(g.point(10));
                bp.line_to(g.point(11));
                path(bp)
            }
            "CIRCLE" => path(transformed(curve::arc(g.point(10), g.num(40)?, 0.0, std::f64::consts::TAU)?, ocs(g))),
            "ARC" => {
                let (start, end) = (g.num_or(50, 0.0), g.num_or(51, 360.0));
                let sweep = ccw_sweep(start, end);
                path(transformed(curve::arc(g.point(10), g.num(40)?, start.to_radians(), sweep.to_radians())?, ocs(g)))
            }
            "ELLIPSE" => path(ellipse(g)?),
            "LWPOLYLINE" => lwpolyline(g),
            "POLYLINE" => self.polyline(g, group.get(1..).unwrap_or(&[])),
            "SPLINE" => path(spline(g)?),
            "SOLID" | "TRACE" => {
                let [a, b, c, d] = [g.point(10), g.point(11), g.point(12), g.point_opt(13).unwrap_or(g.point(12))];
                // The corners zigzag: the outline runs 1, 2, 4, 3.
                Some(Geom::Path { bp: transformed(polygon(&[a, b, d, c]), ocs(g)), fill: true, width: 0.0 })
            }
            "3DFACE" => {
                let [a, b, c, d] = [g.point(10), g.point(11), g.point(12), g.point_opt(13).unwrap_or(g.point(12))];
                path(polygon(&[a, b, c, d]))
            }
            "HATCH" => self.hatch(g),
            "TEXT" | "ATTRIB" => text(g, head.kind == "ATTRIB").map(Geom::Text),
            "MTEXT" => mtext(g).map(Geom::Text),
            "INSERT" => self.insert(g, group.get(1..).unwrap_or(&[])),
            // A dimension draws the anonymous block of its lines, arrows and text.
            "DIMENSION" => Some(Geom::Insert(InsertItem { block: g.name(2)?.to_uppercase(), cells: vec![Affine::IDENTITY], attribs: vec![] })),
            _ => None,
        }
    }

    fn polyline(&mut self, g: &[Pair<'_>], rest: &[Record<'_>]) -> Option<Geom> {
        let flags = g.int_or(70, 0);
        if flags & (16 | 64) != 0 {
            self.warn(MESHES);
            return None;
        }
        // Spline frame control points and polyface faces aren't on the line.
        let verts: Vec<Vertex> = rest
            .iter()
            .filter(|r| r.kind == "VERTEX" && r.g.int_or(70, 0) & (16 | 128) == 0)
            .map(|r| Vertex { p: r.g.point(10), bulge: r.g.num_or(42, 0.0) })
            .collect();
        let bp = curve::polyline(&verts, flags & 1 != 0)?;
        // 3D polylines are in world coordinates; 2D ones in their plane's.
        let bp = if flags & 8 != 0 { bp } else { transformed(bp, ocs(g)) };
        Some(Geom::Path { bp, fill: false, width: g.num_or(40, 0.0).max(0.0) })
    }

    fn insert(&mut self, g: &[Pair<'_>], rest: &[Record<'_>]) -> Option<Geom> {
        let block = g.name(2)?.to_uppercase();
        let (sx, sy) = (g.num_or(41, 1.0), g.num_or(42, 1.0));
        if sx.abs() < 1e-12 || sy.abs() < 1e-12 {
            return None;
        }
        let place = ocs(g) * Affine::translate(g.point(10).to_vec2()) * Affine::rotate(g.num_or(50, 0.0).to_radians());
        let (cols, rows) = (g.int_or(70, 1).max(1), g.int_or(71, 1).max(1));
        let (cs, rs) = (g.num_or(44, 0.0), g.num_or(45, 0.0));
        let (cols, rows) = if cols.saturating_mul(rows) > MAX_CELLS {
            self.warn(BIG_ARRAY);
            (cols.min(MAX_CELLS), (MAX_CELLS / cols.min(MAX_CELLS)).max(1).min(rows))
        } else {
            (cols, rows)
        };
        let cells = (0..rows)
            .flat_map(|r| (0..cols).map(move |c| (r, c)))
            .map(|(r, c)| place * Affine::translate((c as f64 * cs, r as f64 * rs)) * Affine::scale_non_uniform(sx, sy))
            .collect();
        let attribs = self.items(rest);
        Some(Geom::Insert(InsertItem { block, cells, attribs }))
    }

    fn hatch(&mut self, g: &[Pair<'_>]) -> Option<Geom> {
        let start = g.iter().position(|p| p.code == 91)?;
        let count = g.get(start).and_then(|p| number(p.value)).filter(|n| *n >= 0.0)? as usize;
        let mut c = Cursor { g, i: start + 1 };
        let mut bp = BezPath::new();
        for _ in 0..count {
            let Some(path) = boundary(&mut c) else { break };
            bp.extend(path);
        }
        if bp.elements().is_empty() {
            return None;
        }
        let bp = transformed(bp, ocs(g));
        let solid = g.int_or(70, 0) == 1;
        if g.int_or(450, 0) == 1 {
            self.warn(GRADIENT_HATCH);
        } else if !solid {
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

/// The extrusion direction (210, 220, 230) of a planar entity, normalised.
fn extrusion(g: &[Pair<'_>]) -> [f64; 3] {
    let n = [g.num_or(210, 0.0), g.num_or(220, 0.0), g.num_or(230, 1.0)];
    let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
    if len > 1e-12 && len.is_finite() { n.map(|v| v / len) } else { [0.0, 0.0, 1.0] }
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn unit(v: [f64; 3]) -> [f64; 3] {
    let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if len > 1e-12 { v.map(|x| x / len) } else { v }
}

/// A planar entity's coordinate system seen from above: its x and y axes in world space by the
/// arbitrary axis algorithm (identity for drawings in the world's plane; extruded along −z it
/// mirrors x).
fn ocs(g: &[Pair<'_>]) -> Affine {
    let n = extrusion(g);
    if n[2] > 1.0 - 1e-12 {
        return Affine::IDENTITY;
    }
    let ax = unit(if n[0].abs() < 1.0 / 64.0 && n[1].abs() < 1.0 / 64.0 { cross([0.0, 1.0, 0.0], n) } else { cross([0.0, 0.0, 1.0], n) });
    let ay = unit(cross(n, ax));
    Affine::new([ax[0], ax[1], ay[0], ay[1], 0.0, 0.0])
}

fn ellipse(g: &[Pair<'_>]) -> Option<BezPath> {
    let centre = g.point(10);
    let major = g.point(11).to_vec2();
    let ratio = g.num_or(40, 1.0);
    if ratio <= 0.0 || major.hypot() < 1e-12 {
        return None;
    }
    // The minor axis is the major one turned a quarter about the extrusion direction.
    let n = extrusion(g);
    let m = cross(n, [major.x, major.y, 0.0]);
    let minor = Vec2::new(m[0], m[1]) * ratio;
    let (start, end) = (g.num_or(41, 0.0), g.num_or(42, std::f64::consts::TAU));
    let sweep = ccw_sweep(start.to_degrees(), end.to_degrees()).to_radians();
    curve::ellipse(Affine::new([major.x, major.y, minor.x, minor.y, centre.x, centre.y]), start, sweep)
}

fn lwpolyline(g: &[Pair<'_>]) -> Option<Geom> {
    let mut verts: Vec<Vertex> = Vec::new();
    let (mut widths, mut uniform) = (None, true);
    for p in g {
        match p.code {
            10 => verts.push(Vertex { p: Point::new(number(p.value)?, 0.0), bulge: 0.0 }),
            20 => verts.last_mut()?.p.y = number(p.value)?,
            42 => verts.last_mut()?.bulge = number(p.value).unwrap_or(0.0),
            40 | 41 => {
                let w = number(p.value).unwrap_or(0.0);
                uniform &= widths.is_none_or(|x: f64| (x - w).abs() < 1e-12);
                widths = Some(w);
            }
            _ => {}
        }
    }
    let bp = transformed(curve::polyline(&verts, g.int_or(70, 0) & 1 != 0)?, ocs(g));
    let width = g.num(43).filter(|w| *w > 0.0).or(widths.filter(|_| uniform)).unwrap_or(0.0).max(0.0);
    Some(Geom::Path { bp, fill: false, width })
}

/// The points at `code` (x) and `code + 10` (y), in order.
fn points(g: &[Pair<'_>], code: i32) -> Option<Vec<Point>> {
    let mut out: Vec<Point> = Vec::new();
    for p in g {
        if p.code == code {
            out.push(Point::new(number(p.value)?, 0.0));
        } else if p.code == code + 10 {
            out.last_mut()?.y = number(p.value)?;
        }
    }
    Some(out)
}

fn spline(g: &[Pair<'_>]) -> Option<BezPath> {
    let flags = g.int_or(70, 0);
    let ctrl = points(g, 10)?;
    let degree = usize::try_from(g.int_or(71, 3)).ok()?;
    let knots: Vec<f64> = g.iter().filter(|p| p.code == 40).map(|p| number(p.value)).collect::<Option<_>>()?;
    let weights: Vec<f64> = g.iter().filter(|p| p.code == 41).filter_map(|p| number(p.value)).collect();
    let weights = (flags & 4 != 0 && weights.len() == ctrl.len()).then_some(weights.as_slice());
    curve::nurbs(degree, &knots, &ctrl, weights).or_else(|| curve::through(&points(g, 11)?, flags & 1 != 0))
}

/// Reads a hatch's boundary groups in order.
struct Cursor<'g, 'a> {
    g: &'g [Pair<'a>],
    i: usize,
}

impl Cursor<'_, '_> {
    /// The next group's value when it has `code` (consumed).
    fn opt(&mut self, code: i32) -> Option<&str> {
        let p = self.g.get(self.i).filter(|p| p.code == code)?;
        self.i += 1;
        Some(p.value)
    }
    fn num(&mut self, code: i32) -> Option<f64> {
        self.opt(code).and_then(number)
    }
    fn count(&mut self, code: i32) -> Option<usize> {
        self.num(code).filter(|n| *n >= 0.0 && *n <= 1e7).map(|n| n as usize)
    }
    fn point(&mut self, code: i32) -> Option<Point> {
        Some(Point::new(self.num(code)?, self.num(code + 10)?))
    }
}

/// One hatch boundary loop, closed.
fn boundary(c: &mut Cursor<'_, '_>) -> Option<BezPath> {
    let flags = c.num(92)? as i64;
    let mut bp = BezPath::new();
    if flags & 2 != 0 {
        let bulges = c.num(72).unwrap_or(0.0) != 0.0;
        let _closed = c.opt(73);
        let n = c.count(93)?;
        let mut verts = Vec::with_capacity(n.min(4096));
        for _ in 0..n {
            let p = c.point(10)?;
            let bulge = if bulges { c.num(42).unwrap_or(0.0) } else { 0.0 };
            verts.push(Vertex { p, bulge });
        }
        bp = curve::polyline(&verts, true)?;
    } else {
        let n = c.count(93)?;
        for _ in 0..n {
            let edge = edge(c)?;
            append(&mut bp, &edge);
        }
        if bp.elements().is_empty() {
            return None;
        }
        bp.close_path();
    }
    // The boundary objects it was made from.
    if let Some(n) = c.count(97) {
        for _ in 0..n {
            c.opt(330);
        }
    }
    Some(bp)
}

/// One edge of a hatch boundary.
fn edge(c: &mut Cursor<'_, '_>) -> Option<BezPath> {
    match c.num(72)? as i64 {
        1 => {
            let mut bp = BezPath::new();
            bp.move_to(c.point(10)?);
            bp.line_to(c.point(11)?);
            Some(bp)
        }
        2 => {
            let (centre, r, start, end) = (c.point(10)?, c.num(40)?, c.num(50)?, c.num(51)?);
            let ccw = c.num(73).unwrap_or(1.0) != 0.0;
            let sweep = ccw_sweep(start, end);
            // Clockwise arcs have their angles measured clockwise.
            let (start, sweep) = if ccw { (start, sweep) } else { (-start, -sweep) };
            curve::arc(centre, r, start.to_radians(), sweep.to_radians())
        }
        3 => {
            let (centre, major, ratio, start, end) = (c.point(10)?, c.point(11)?.to_vec2(), c.num(40)?, c.num(50)?, c.num(51)?);
            let ccw = c.num(73).unwrap_or(1.0) != 0.0;
            let minor = Vec2::new(-major.y, major.x) * ratio * if ccw { 1.0 } else { -1.0 };
            let sweep = ccw_sweep(start, end).to_radians();
            curve::ellipse(Affine::new([major.x, major.y, minor.x, minor.y, centre.x, centre.y]), start.to_radians(), sweep)
        }
        4 => {
            let degree = c.count(94)?;
            let rational = c.num(73).unwrap_or(0.0) != 0.0;
            let _periodic = c.opt(74);
            let (nk, nc) = (c.count(95)?, c.count(96)?);
            let knots: Vec<f64> = (0..nk).map(|_| c.num(40)).collect::<Option<_>>()?;
            let mut ctrl = Vec::with_capacity(nc.min(4096));
            let mut weights = Vec::with_capacity(nc.min(4096));
            for _ in 0..nc {
                ctrl.push(c.point(10)?);
                weights.push(c.num(42).unwrap_or(1.0));
            }
            // Fit data (DXF 2010 and later): a count, then that many points. Older files have none,
            // and the 97 that may follow is the boundary's count of source objects (then their
            // handles), so it is only the fit count when it is 0 or points follow.
            let fit_count = match (c.g.get(c.i), c.g.get(c.i + 1)) {
                (Some(count), next) if count.code == 97 => number(count.value) == Some(0.0) || next.is_some_and(|p| p.code == 11),
                _ => false,
            };
            let fit: Vec<Point> = match fit_count.then(|| c.count(97)).flatten() {
                Some(n) => (0..n).map(|_| c.point(11)).collect::<Option<_>>()?,
                None => vec![],
            };
            // Start and end tangents.
            for code in [12, 13] {
                c.point(code);
            }
            curve::nurbs(degree, &knots, &ctrl, rational.then_some(weights.as_slice())).or_else(|| curve::through(&fit, false))
        }
        _ => None,
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

fn text(g: &[Pair<'_>], attrib: bool) -> Option<TextItem> {
    if attrib && g.int_or(70, 0) & 1 != 0 {
        return None;
    }
    let s = super::text::plain(g.raw(1)?);
    if s.trim().is_empty() {
        return None;
    }
    let height = g.num(40).filter(|h| *h > 0.0)?;
    let (h, v) = (g.int_or(72, 0), g.int_or(if attrib { 74 } else { 73 }, 0));
    let (p1, p2) = (g.point(10), g.point_opt(11));
    let mut angle = g.num_or(50, 0.0).to_radians();
    let mut span = None;
    // Aligned and fit text run from the first point to the second.
    let anchor = match (h, p2) {
        (3 | 5, Some(p)) => {
            angle = (p - p1).atan2();
            let length = (p - p1).hypot();
            span = (length <= MAX_COORD).then_some(if h == 5 { Span::Fit(length) } else { Span::Aligned(length) });
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
    let flags = g.int_or(71, 0);
    let mirror = Affine::scale_non_uniform(if flags & 2 != 0 { -1.0 } else { 1.0 }, if flags & 4 != 0 { -1.0 } else { 1.0 });
    let xf = ocs(g) * Affine::translate(anchor.to_vec2()) * Affine::rotate(angle) * mirror * Affine::translate((0.0, baseline(v, height)));
    Some(TextItem {
        text: s,
        height,
        xf,
        width: g.num(41).filter(|w| *w > 0.0).unwrap_or(1.0),
        justify,
        style: g.name(7).unwrap_or("STANDARD").to_uppercase(),
        family: None,
        leading: None,
        span,
    })
}

fn mtext(g: &[Pair<'_>]) -> Option<TextItem> {
    // The text comes in 250-character chunks (3) before the last (1).
    let raw: String = g.iter().filter(|p| p.code == 3).map(|p| p.value).chain(g.raw(1)).collect();
    let (s, family) = super::text::mtext(&raw);
    if s.trim().is_empty() {
        return None;
    }
    let height = g.num(40).filter(|h| *h > 0.0)?;
    let leading = height * LINE_SPACING * g.num(44).filter(|f| *f > 0.0).unwrap_or(1.0);
    let lines = s.lines().count().max(1) as f64;
    let attach = g.int_or(71, 1).clamp(1, 9) - 1;
    let justify = [Justify::Left, Justify::Center, Justify::Right].get((attach % 3) as usize).copied().unwrap_or_default();
    // The first baseline below the attachment point: under the top, around the middle, above
    // the bottom of the lines.
    let total = height + (lines - 1.0) * leading;
    let first = match attach / 3 {
        0 => -height,
        1 => total / 2.0 - height,
        _ => (lines - 1.0) * leading,
    };
    let direction = g.point_opt(11).map(|p| p.to_vec2()).filter(|d| d.hypot() > 1e-12);
    let angle = direction.map_or_else(|| g.num_or(50, 0.0), |d| d.atan2());
    let xf = Affine::translate(g.point(10).to_vec2()) * Affine::rotate(angle) * Affine::translate((0.0, first));
    Some(TextItem {
        text: s,
        height,
        xf,
        width: 1.0,
        justify,
        style: g.name(7).unwrap_or("STANDARD").to_uppercase(),
        family,
        leading: Some(leading),
        span: None,
    })
}
