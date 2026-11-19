//! Map Art: the art of a document symbol mapped onto the numbered surfaces of Extrude & Bevel
//! (Classic) and Revolve (Classic).
//!
//! The effect keeps its maps in its `mapArt` parameter, each `{surface, symbol, x, y, scaleX,
//! scaleY, rotation}`: the surface's number (from 1), the symbol's name, the offset of the art's
//! center from the center of the surface laid out flat (points, x to the right and y down as the
//! surface is seen from outside the solid), its size as a share of the symbol's own, and its turn
//! (degrees, counterclockwise). `mapShade` shades the mapped art with its surface, and
//! `mapInvisible` draws the mapped art without the solid's faces.
//!
//! The surfaces of Extrude & Bevel (Classic) are numbered through its solids in the order they are
//! built, each solid's as [`vectorcraft_three_d::extrude_surfaces`] lists them. Revolve (Classic)
//! has one surface, its swept side (in a group, the side of the member with the longest profile).
//! A map whose number is past the object's surfaces draws nothing.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect};
use vectorcraft_three_d::{Extrude, FaceMap, Facet, MapCell, PlanarFace, SolidSurface, SurfaceKind, View};

use super::{
    MAX_ELEMENTS, MAX_SURFACE_CUTS, MAX_SURFACE_POINTS, NotMapped, SURFACE_SEAM, SURFACE_TOLERANCE, Unrolled, counterclockwise, map_path, pieces_of,
    straight_pieces, takes_seam, triangle_map,
};

/// The effect parameter that holds the maps.
pub const MAP_ART: &str = "mapArt";

/// The effect parameter of Shade Artwork.
pub const MAP_SHADE: &str = "mapShade";

/// The effect parameter of Invisible Geometry.
pub const MAP_INVISIBLE: &str = "mapInvisible";

/// The most surfaces of one 3D effect that carry a map.
pub const MAX_MAPS: usize = 64;

/// The highest surface number a map may name.
pub const MAX_MAP_SURFACE: usize = 100_000;

/// The error for maps on a 3D effect that takes none.
pub const MAP_ART_NOT_TAKEN: &str = "Map Art maps symbols onto Extrude & Bevel (Classic) and Revolve (Classic)";

/// The symbol mapped onto one numbered surface of a 3D object.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfaceMap {
    /// The surface's number, from 1.
    pub surface: usize,
    pub symbol: String,
    pub x: f64,
    pub y: f64,
    /// Percent of the symbol's own width.
    pub scale_x: f64,
    /// Percent of the symbol's own height.
    pub scale_y: f64,
    /// Degrees, counterclockwise.
    pub rotation: f64,
}

impl SurfaceMap {
    pub fn to_value(&self) -> Value {
        json!({
            "surface": self.surface,
            "symbol": self.symbol,
            "x": self.x,
            "y": self.y,
            "scaleX": self.scale_x,
            "scaleY": self.scale_y,
            "rotation": self.rotation,
        })
    }

    /// The map `v` describes. `surface` and `symbol` are required: the offsets and the rotation
    /// default to 0 and the scales to 100.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let Value::Object(m) = v else { return Err(format!("each of `{MAP_ART}` must be an object")) };
        if let Some(k) = m.keys().find(|k| !matches!(k.as_str(), "surface" | "symbol" | "x" | "y" | "scaleX" | "scaleY" | "rotation")) {
            return Err(format!("`{MAP_ART}` entries take surface, symbol, x, y, scaleX, scaleY and rotation, not `{k}`"));
        }
        let surface = m
            .get("surface")
            .and_then(Value::as_u64)
            .and_then(|n| usize::try_from(n).ok())
            .filter(|n| (1..=MAX_MAP_SURFACE).contains(n))
            .ok_or_else(|| format!("each of `{MAP_ART}` needs `surface`, a whole number from 1 to {MAX_MAP_SURFACE}"))?;
        let symbol = match m.get("symbol") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            _ => return Err(format!("each of `{MAP_ART}` needs `symbol`, the name of a symbol")),
        };
        let number = |key: &str, default: f64, (lo, hi): (f64, f64)| -> Result<f64, String> {
            match m.get(key) {
                None => Ok(default),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite() && (lo..=hi).contains(n))
                    .ok_or_else(|| format!("`{MAP_ART}` {key} must be a number from {lo} to {hi}")),
            }
        };
        Ok(Self {
            surface,
            symbol,
            x: number("x", 0.0, (-super::MAX_OFFSET, super::MAX_OFFSET))?,
            y: number("y", 0.0, (-super::MAX_OFFSET, super::MAX_OFFSET))?,
            scale_x: number("scaleX", 100.0, super::SCALE_RANGE)?,
            scale_y: number("scaleY", 100.0, super::SCALE_RANGE)?,
            rotation: number("rotation", 0.0, (-360.0, 360.0))?,
        })
    }

    /// Where the symbol's art goes on the flat surface, for art whose bounds are `art` on a
    /// surface with center `center`. With `mirrored` (the back cap) x runs to the right as the
    /// surface is seen from behind the solid.
    pub fn placement(&self, art: Rect, center: Point, mirrored: bool) -> Affine {
        let place = Affine::translate((center.x + self.x, center.y + self.y))
            * Affine::rotate(-self.rotation.to_radians())
            * Affine::scale_non_uniform(self.scale_x / 100.0, self.scale_y / 100.0)
            * Affine::translate(-art.center().to_vec2());
        if mirrored {
            Affine::translate(center.to_vec2()) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate(-center.to_vec2()) * place
        } else {
            place
        }
    }

    /// The scales (percent) at which art with bounds `art`, turned by the map's rotation, fills a
    /// surface `width` by `height` points: Scale to Fit. `None` for art or a surface with no area.
    pub fn fitting_scales(&self, art: Rect, (width, height): (f64, f64)) -> Option<(f64, f64)> {
        let (w, h) = (art.width(), art.height());
        if w <= 0.0 || h <= 0.0 || width <= 0.0 || height <= 0.0 {
            return None;
        }
        // The box of the art turned by the rotation, at its own size.
        let (sin, cos) = self.rotation.to_radians().sin_cos();
        let (bw, bh) = (w * cos.abs() + h * sin.abs(), w * sin.abs() + h * cos.abs());
        let clamp = |v: f64| v.clamp(super::SCALE_RANGE.0, super::SCALE_RANGE.1);
        if sin.abs() < 1e-9 || cos.abs() < 1e-9 {
            // Square to the surface, each side takes its own scale.
            let (sx, sy) = if cos.abs() >= 0.5 { (width / w, height / h) } else { (height / w, width / h) };
            return Some((clamp(sx * 100.0), clamp(sy * 100.0)));
        }
        let fit = (width / bw).min(height / bh);
        Some((clamp(fit * 100.0), clamp(fit * 100.0)))
    }
}

/// The 3D effects that take Map Art: Extrude & Bevel (Classic) and Revolve (Classic).
pub fn takes_map_art(id: &str) -> bool {
    id == super::super::CLASSIC_EXTRUDE || id == crate::CLASSIC_REVOLVE
}

/// The maps in parameters `p`, leaving out entries that are not valid and, of two maps of one
/// surface, the later one (a file may hold anything; commands check every entry with
/// [`validate`]).
pub fn surface_maps(p: &Value) -> Vec<SurfaceMap> {
    let Some(Value::Array(list)) = p.get(MAP_ART) else { return vec![] };
    let mut seen = HashSet::new();
    list.iter().filter_map(|v| SurfaceMap::from_value(v).ok()).filter(|m| seen.insert(m.surface)).take(MAX_MAPS).collect()
}

/// Are the maps in parameters `p` of 3D effect `id` valid? Only Extrude & Bevel (Classic) and
/// Revolve (Classic) take maps, at most [`MAX_MAPS`] of them, one for a surface.
pub fn validate(id: &str, p: &Value) -> Result<(), String> {
    let list = match p.get(MAP_ART) {
        None => return Ok(()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(format!("`{MAP_ART}` must be an array")),
    };
    if list.is_empty() {
        return Ok(());
    }
    if !takes_map_art(id) {
        return Err(MAP_ART_NOT_TAKEN.into());
    }
    if list.len() > MAX_MAPS {
        return Err(format!("`{MAP_ART}` takes at most {MAX_MAPS} maps"));
    }
    let mut seen = HashSet::new();
    for v in list {
        let m = SurfaceMap::from_value(v)?;
        if !seen.insert(m.surface) {
            return Err(format!("`{MAP_ART}` maps surface {} more than once", m.surface));
        }
    }
    Ok(())
}

/// `p` with the map of surface `surface` replaced by `map`, or removed with `None`. The maps stay
/// in the order of their surfaces.
pub fn with_surface_map(p: &Value, surface: usize, map: Option<&SurfaceMap>) -> Value {
    let mut maps: Vec<SurfaceMap> = surface_maps(p).into_iter().filter(|m| m.surface != surface).collect();
    maps.extend(map.cloned());
    maps.sort_by_key(|m| m.surface);
    let mut out = p.as_object().cloned().unwrap_or_else(Map::new);
    out.insert(MAP_ART.into(), Value::Array(maps.iter().map(SurfaceMap::to_value).collect()));
    Value::Object(out)
}

/// One solid's place among its object's numbered surfaces: the index of its first surface, its
/// surfaces, and for a solid with planar ends the bounds, options and view it was built with.
pub(crate) struct SolidLayout {
    pub(crate) first: usize,
    pub(crate) surfaces: Vec<SolidSurface>,
    pub(crate) ends: Option<(Rect, Extrude, View)>,
}

/// What the mapped art of one object has drawn so far, against the limits its maps share.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct MapBudget {
    elements: usize,
    /// The points of the maps' symbols cut into straight pieces, against [`MAX_ELEMENTS`].
    input: usize,
    /// The points the maps' art draws on the faces, against [`MAX_SURFACE_POINTS`].
    points: usize,
    cuts: usize,
}

/// The maps of one 3D effect, with the document whose symbols they place.
pub(crate) struct MapArt<'a> {
    doc: &'a Document,
    maps: Vec<SurfaceMap>,
    /// Shade Artwork: the mapped art takes the shading of its surface.
    pub(crate) shade: bool,
    /// Invisible Geometry: only the mapped art is drawn.
    pub(crate) invisible: bool,
}

impl<'a> MapArt<'a> {
    /// The maps of 3D effect `id` with parameters `p`. `None` without a document, for an effect
    /// that takes no maps and for one that has none.
    pub(crate) fn of(doc: Option<&'a Document>, id: &str, p: &Value) -> Option<Self> {
        let maps = surface_maps(p);
        let flag = |key: &str| p.get(key).and_then(Value::as_bool).unwrap_or(false);
        (takes_map_art(id) && !maps.is_empty()).then_some(Self { doc: doc?, maps, shade: flag(MAP_SHADE), invisible: flag(MAP_INVISIBLE) })
    }

    fn map_of(&self, index: usize) -> Option<&SurfaceMap> {
        self.maps.iter().find(|m| m.surface == index + 1)
    }

    /// The mapped art that falls on each of `facets`, one solid's facets in drawing order, for an
    /// object with id `id`: for each facet, the nodes to draw right after it. Only facets turned
    /// toward the viewer carry art. `shade` gives a color as its facet shades it, for Shade
    /// Artwork. A map whose symbol the document does not have, or whose art is past the limits of
    /// a graphic's art, draws nothing. `None` when the art is past the limits the maps of one
    /// object share, or the copies of its paints take `copies` past the object's allowance: the
    /// solid then draws without mapped art.
    pub(crate) fn parts(
        &self,
        id: NodeId,
        facets: &[Facet],
        layout: &SolidLayout,
        shade: &dyn Fn(&Facet, Color) -> Color,
        budget: &mut MapBudget,
        copies: &mut usize,
    ) -> Option<Vec<Vec<Arc<Node>>>> {
        let mut out: Vec<Vec<Arc<Node>>> = vec![vec![]; facets.len()];
        let mut flat: HashMap<usize, Vec<Unrolled>> = HashMap::new();
        for (f, parts) in facets.iter().zip(out.iter_mut()) {
            if f.away {
                continue;
            }
            let recolor = |c: Color| if self.shade { shade(f, c) } else { c };
            if let Some(end) = f.end {
                let Some(local) = layout.surfaces.iter().position(|s| s.kind == SurfaceKind::End(end)) else { continue };
                let (Some(m), Some(center)) = (self.map_of(layout.first + local), layout.surfaces.get(local).and_then(|s| s.center)) else {
                    continue;
                };
                let Some((bounds, options, view)) = layout.ends else { continue };
                let Some(map) = vectorcraft_three_d::extrude_face_map(bounds, options, view, end) else { continue };
                parts.extend(self.end_art(id, m, (end, &map), center, f, &recolor, budget, copies)?);
            } else if let Some(cell) = f.cell.as_deref() {
                let (Some(m), Some(surface)) = (self.map_of(layout.first + cell.surface), layout.surfaces.get(cell.surface)) else { continue };
                if let std::collections::hash_map::Entry::Vacant(e) = flat.entry(cell.surface) {
                    e.insert(self.unrolled(m, surface, budget)?);
                }
                let Some(pieces) = flat.get(&cell.surface) else { continue };
                parts.extend(cell_art(id, pieces, cell, f, &recolor, budget, copies)?);
            }
        }
        Some(out)
    }

    /// The art of map `m` on planar end `end` of a solid, drawn on its facet `f` through `map`,
    /// for an end whose bounds have center `center`: one clipping group, as a graphic on a cap is
    /// drawn. `None` past the limits.
    #[allow(clippy::too_many_arguments)]
    fn end_art(
        &self,
        id: NodeId,
        m: &SurfaceMap,
        (end, map): (PlanarFace, &FaceMap),
        center: Point,
        f: &Facet,
        recolor: &dyn Fn(Color) -> Color,
        budget: &mut MapBudget,
        copies: &mut usize,
    ) -> Option<Vec<Arc<Node>>> {
        let Some(symbol) = self.doc.symbols.iter().find(|s| s.name == m.symbol) else { return Some(vec![]) };
        let Ok(pieces) = pieces_of(self.doc, symbol) else { return Some(vec![]) };
        let Some(bounds) = pieces.iter().filter_map(|p| p.path.bounds()).reduce(|a, b| a.union(b)) else { return Some(vec![]) };
        let place = m.placement(bounds, center, end == PlanarFace::Back);
        let affine = map.local_affine(center).filter(|_| map.is_affine());
        // Gradients follow the end's map as it is near the art's center, or near the middle of the
        // part in front of the eye when the center is not.
        let local = map.local_affine(place * bounds.center()).or_else(|| {
            let part = map.mapped_part(place.transform_rect_bbox(bounds))?;
            let sum = part.iter().fold(Point::ZERO, |s, p| s + p.to_vec2());
            map.local_affine(Point::new(sum.x / part.len() as f64, sum.y / part.len() as f64))
        });
        let Some(local) = local else { return Some(vec![]) };
        let gradient_map = local * place;
        let mut children = vec![];
        for piece in pieces {
            let placed = piece.path.transformed(place);
            let least: usize = placed.subpaths.iter().map(|s| s.anchors.len()).sum();
            let before = budget.elements;
            if before.saturating_add(least) > MAX_ELEMENTS {
                return None;
            }
            let room = MAX_ELEMENTS - before;
            let mut rule = piece.rule;
            let mapped = match map_path(&placed, map, affine, room) {
                Err(NotMapped::Behind) => {
                    // Part of it is at or behind the eye: the part in front is drawn.
                    let Some(part) = placed.bounds().and_then(|b| map.mapped_part(b)) else { continue };
                    let Some(&first) = part.first() else { continue };
                    let mut front = BezPath::new();
                    front.move_to(first);
                    part.iter().skip(1).for_each(|p| front.line_to(*p));
                    front.close_path();
                    let cut = vectorcraft_pathops::boolean(
                        &placed,
                        piece.rule,
                        &PathData::from_bezpath(&front),
                        FillRule::NonZero,
                        vectorcraft_pathops::BoolOp::Intersect,
                    );
                    rule = FillRule::NonZero;
                    map_path(&cut, map, affine, room)
                }
                done => done,
            };
            let mapped = match mapped {
                Ok(mapped) => mapped,
                Err(NotMapped::Full) => return None,
                Err(NotMapped::Behind) => continue,
            };
            budget.elements = before.saturating_add(mapped.elements().len());
            if !crate::revolve::paint_copies_add(copies, 1, &piece.paint) {
                return None;
            }
            children.push(Arc::new(piece_node(id, piece.paint, (piece.opacity, piece.blend), (mapped, rule), gradient_map, recolor, None)));
        }
        if children.is_empty() {
            return Some(vec![]);
        }
        // An end's holes are contours of the same path wound against its outline, so turning the
        // whole path keeps them holes under the nonzero rule.
        let mut outline = f.path().to_bezpath();
        if vectorcraft_geom::kurbo::Shape::area(&outline) < 0.0 {
            outline = outline.reverse_subpaths();
        }
        let clip = Node::path(id, PathData::from_bezpath(&outline), Appearance::default());
        Some(vec![Arc::new(Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(Node::group(id, children))], clip: true }))])
    }

    /// The areas of map `m`'s symbol placed on `surface` laid out flat, cut into straight pieces.
    /// Empty for a symbol the document does not have or whose art is past the limits of a
    /// graphic's art. `None` when the pieces of the object's maps take more than
    /// [`MAX_ELEMENTS`] points together.
    fn unrolled(&self, m: &SurfaceMap, surface: &SolidSurface, budget: &mut MapBudget) -> Option<Vec<Unrolled>> {
        let Some(symbol) = self.doc.symbols.iter().find(|s| s.name == m.symbol) else { return Some(vec![]) };
        let Ok(art) = pieces_of(self.doc, symbol) else { return Some(vec![]) };
        let Some(bounds) = art.iter().filter_map(|p| p.path.bounds()).reduce(|a, b| a.union(b)) else { return Some(vec![]) };
        let place = m.placement(bounds, flat_center(surface), false);
        let mut pieces = vec![];
        for piece in art {
            let placed = place * piece.path.to_bezpath();
            let mut polygons = straight_pieces(&placed, &mut budget.input)?;
            polygons.retain(|p| p.len() >= 3 && p.iter().all(|q| q.is_finite()));
            let boxes: Vec<Rect> = polygons.iter().filter_map(|polygon| box_of(polygon)).collect();
            let Some(bounds) = boxes.iter().copied().reduce(|a, b| a.union(b)) else { continue };
            pieces.push(Unrolled {
                polygons,
                boxes,
                bounds,
                rule: piece.rule,
                paint: piece.paint,
                opacity: piece.opacity,
                blend: piece.blend,
                place,
            });
        }
        Some(pieces)
    }
}

/// The center of `surface` laid out flat, which a map's offset is measured from: the middle of a
/// wall or a bevel, whose flat coordinates start at its corner, and the origin of Revolve's side,
/// whose flat coordinates start at its middle.
pub fn flat_center(surface: &SolidSurface) -> Point {
    match surface.kind {
        SurfaceKind::Side => Point::ZERO,
        SurfaceKind::End(_) => surface.center.unwrap_or(Point::ZERO),
        SurfaceKind::Wall | SurfaceKind::Bevel => Point::new(surface.width / 2.0, surface.height / 2.0),
    }
}

fn box_of(points: &[Point]) -> Option<Rect> {
    points.iter().map(|p| Rect::from_points(*p, *p)).reduce(|a, b| a.union(b))
}

/// A map from a region of a flat surface onto the page.
enum Mapper {
    Affine(Affine),
    /// The projective map that takes the rectangle with corner `origin` and size `size` onto a
    /// quadrilateral: `rows` are its coefficients on the unit square.
    Quad {
        origin: Point,
        size: (f64, f64),
        rows: [f64; 8],
    },
}

impl Mapper {
    fn at(&self, p: Point) -> Point {
        match self {
            Mapper::Affine(a) => *a * p,
            Mapper::Quad { origin, size, rows: [a, b, c, d, e, f, g, h] } => {
                let (u, v) = ((p.x - origin.x) / size.0, (p.y - origin.y) / size.1);
                let w = g * u + h * v + 1.0;
                Point::new((a * u + b * v + c) / w, (d * u + e * v + f) / w)
            }
        }
    }

    /// The point of the flat surface this map takes to `p` on the page. `None` where there is none.
    fn back(&self, p: Point) -> Option<Point> {
        let q = match self {
            Mapper::Affine(a) => a.inverse() * p,
            Mapper::Quad { origin, size, rows: [a, b, c, d, e, f, g, h] } => {
                // x (g u + h v + 1) = a u + b v + c, and the same for y, solved for u and v.
                let (m00, m01, r0) = (a - g * p.x, b - h * p.x, p.x - c);
                let (m10, m11, r1) = (d - g * p.y, e - h * p.y, p.y - f);
                let det = m00 * m11 - m01 * m10;
                let (u, v) = ((r0 * m11 - m01 * r1) / det, (m00 * r1 - r0 * m10) / det);
                Point::new(origin.x + u * size.0, origin.y + v * size.1)
            }
        };
        q.is_finite().then_some(q)
    }

    /// The affine map nearest this one around `p`, for gradients and patterns.
    fn local(&self, p: Point, step: f64) -> Affine {
        match self {
            Mapper::Affine(a) => *a,
            Mapper::Quad { .. } => {
                let (o, x, y) = (self.at(p), self.at(Point::new(p.x + step, p.y)), self.at(Point::new(p.x, p.y + step)));
                let (dx, dy) = ((x - o) / step, (y - o) / step);
                Affine::new([dx.x, dx.y, dy.x, dy.y, o.x - dx.x * p.x - dy.x * p.y, o.y - dx.y * p.x - dy.y * p.y])
            }
        }
    }
}

/// The projective map of `cell`, a cell whose flat corners are those of an upright rectangle, in
/// order along its edge nearest the origin and back: exact for a flat wall under a perspective. `None`
/// for any other cell and for a quadrilateral the map turns inside out.
fn quad_mapper(cell: &MapCell) -> Option<(Vec<Point>, Mapper)> {
    let [mut u0, mut u1, mut u2, mut u3] = <[Point; 4]>::try_from(cell.uv.as_slice()).ok()?;
    let [mut p0, mut p1, mut p2, mut p3] = <[Point; 4]>::try_from(cell.page.as_slice()).ok()?;
    // A cell whose first edge runs from right to left is read from its other end.
    if u1.x < u0.x {
        (u0, u1, u2, u3) = (u1, u0, u3, u2);
        (p0, p1, p2, p3) = (p1, p0, p3, p2);
    }
    let size = (u1.x - u0.x, u3.y - u0.y);
    let upright = u1.y == u0.y && u2.x == u1.x && u2.y == u3.y && u3.x == u0.x && size.0 > 1e-9 && size.1 > 1e-9;
    if !upright {
        return None;
    }
    let (dx1, dx2, sx) = (p1.x - p2.x, p3.x - p2.x, p0.x - p1.x + p2.x - p3.x);
    let (dy1, dy2, sy) = (p1.y - p2.y, p3.y - p2.y, p0.y - p1.y + p2.y - p3.y);
    let den = dx1 * dy2 - dx2 * dy1;
    let scale = dx1.abs() + dx2.abs() + dy1.abs() + dy2.abs();
    if den.is_nan() || den.abs() <= 1e-12 * scale * scale {
        return None;
    }
    let (g, h) = ((sx * dy2 - dx2 * sy) / den, (dx1 * sy - sx * dy1) / den);
    let rows = [p1.x - p0.x + g * p1.x, p3.x - p0.x + h * p3.x, p0.x, p1.y - p0.y + g * p1.y, p3.y - p0.y + h * p3.y, p0.y, g, h];
    let inside = g + 1.0 > 1e-6 && h + 1.0 > 1e-6 && g + h + 1.0 > 1e-6;
    (inside && rows.iter().all(|v| v.is_finite())).then(|| (vec![u0, u1, u2, u3], Mapper::Quad { origin: u0, size, rows }))
}

/// The regions of `cell` on its flat surface, each a convex polygon wound as [`clip_convex`] reads
/// it, with its map onto the page: the whole of an upright rectangle ([`quad_mapper`]), else the
/// triangles the cell is cut into.
fn regions(cell: &MapCell) -> Vec<(Vec<Point>, Mapper)> {
    if let Some(quad) = quad_mapper(cell) {
        return vec![quad];
    }
    let triangles: &[[usize; 3]] = if cell.uv.len() == 3 { &[[0, 1, 2]] } else { &[[0, 1, 2], [0, 2, 3]] };
    triangles
        .iter()
        .filter_map(|corners| {
            let pick = |of: &[Point]| corners.iter().map(|k| of.get(*k).copied()).collect::<Option<Vec<Point>>>();
            let (uv, page) = (pick(&cell.uv)?, pick(&cell.page)?);
            Some((counterclockwise(&uv)?.to_vec(), Mapper::Affine(triangle_map(&uv, &page)?)))
        })
        .collect()
}

/// The part of `polygon` inside the convex `window` (one step of Sutherland and Hodgman's clip for
/// each of its edges), wound so that its inside is on the positive side of each edge.
fn clip_convex(polygon: &[Point], window: &[Point]) -> Vec<Point> {
    let mut out = polygon.to_vec();
    for (k, &a) in window.iter().enumerate() {
        let Some(&b) = window.get((k + 1) % window.len()) else { continue };
        let side = |p: Point| (b - a).cross(p - a);
        let input = std::mem::take(&mut out);
        for (i, &p) in input.iter().enumerate() {
            let Some(&q) = input.get((i + 1) % input.len()) else { continue };
            let (sp, sq) = (side(p), side(q));
            if sp >= 0.0 {
                out.push(p);
            }
            if (sp >= 0.0) != (sq >= 0.0) {
                out.push(p.lerp(q, sp / (sp - sq)));
            }
        }
        if out.len() < 3 {
            return vec![];
        }
    }
    out
}

/// How far `p` lies outside the convex `polygon`, wound with its inside on the positive side of each
/// edge, across the edges for which `counts` holds: 0 or less when it lies inside them.
fn beyond(polygon: &[Point], p: Point, counts: &dyn Fn(Point, Point) -> bool) -> f64 {
    let mut most = f64::NEG_INFINITY;
    for (k, &a) in polygon.iter().enumerate() {
        let Some(&b) = polygon.get((k + 1) % polygon.len()) else { continue };
        let length = (b - a).hypot();
        if length > 0.0 && counts(a, b) {
            most = most.max(-(b - a).cross(p - a) / length);
        }
    }
    most
}

/// `regions` of `cell` ([`regions`]) with the edges they share with the cell's own outline moved out
/// on the flat surface, far enough that their maps reach the outline of facet `f` where its curved
/// edges bow out of the cell: each outline point is taken back onto the flat surface through the
/// region's own map. The edge between two triangles stays, so the regions do not overlap. A region
/// moves out at most four times the size of the cell, and one whose projective map would turn
/// inside out where it reaches stays as it is. Without a curved outline, or where the outline stays
/// inside the cell, the regions are returned unchanged.
fn reaching_outline(regions: Vec<(Vec<Point>, Mapper)>, cell: &MapCell, f: &Facet) -> Vec<(Vec<Point>, Mapper)> {
    let Some(outline) = f.outline.as_ref() else { return regions };
    let Some(uv) = box_of(&cell.uv) else { return regions };
    // The cell on the page, wound with its inside on the positive side of each edge.
    let mut around = cell.page.clone();
    let turn: f64 = around.iter().zip(around.iter().cycle().skip(1)).map(|(a, b)| a.to_vec2().cross(b.to_vec2())).sum();
    if turn < 0.0 {
        around.reverse();
    }
    // The points of the outline that lie outside the cell on the page, or near enough that the
    // curve between them may: the curve strays from its flattened points by up to the tolerance,
    // so each point counts as the corners of a square of that half width around it.
    let mut out = vec![];
    let outline = outline.to_bezpath();
    let t = SURFACE_TOLERANCE;
    vectorcraft_geom::kurbo::flatten(outline.elements().iter().copied(), t, |el| {
        if let vectorcraft_geom::kurbo::PathEl::MoveTo(p) | vectorcraft_geom::kurbo::PathEl::LineTo(p) = el
            && beyond(&around, p, &|_, _| true) > -2.0 * t
        {
            out.extend([(t, t), (t, -t), (-t, t), (-t, -t)].map(|(x, y)| p + vectorcraft_geom::Vec2::new(x, y)));
        }
    });
    if out.is_empty() {
        return regions;
    }
    let most = 4.0 * uv.width().hypot(uv.height());
    let on_outline = |a: Point, b: Point| {
        let at = |q: Point| cell.uv.iter().position(|u| *u == q);
        let count = cell.uv.len();
        matches!((at(a), at(b)), (Some(i), Some(j)) if (i + 1) % count == j || (j + 1) % count == i)
    };
    regions
        .into_iter()
        .map(|(window, mapper)| {
            let reach = out.iter().filter_map(|p| mapper.back(*p)).map(|q| beyond(&window, q, &on_outline)).fold(0.0f64, f64::max);
            if reach <= 0.0 {
                return (window, mapper);
            }
            // A little past the farthest point, so the cut to the outline meets it.
            let reach = (reach * 1.01 + 1e-9).min(most);
            let grown = grown_window(&window, reach, &on_outline);
            let holds = match &mapper {
                Mapper::Affine(_) => true,
                Mapper::Quad { origin, size, rows } => grown.iter().all(|p| {
                    let (u, v) = ((p.x - origin.x) / size.0, (p.y - origin.y) / size.1);
                    rows[6] * u + rows[7] * v + 1.0 > 1e-6
                }),
            };
            (if holds && grown.len() == window.len() { grown } else { window }, mapper)
        })
        .collect()
}

/// The convex `window`, wound as [`clip_convex`] reads it, with each edge for which `moves` holds
/// moved out by `by`, and its corners where the moved edges meet.
fn grown_window(window: &[Point], by: f64, moves: &dyn Fn(Point, Point) -> bool) -> Vec<Point> {
    let count = window.len();
    // Each edge as a point on its line and its direction, moved out where it moves.
    let lines: Vec<(Point, vectorcraft_geom::Vec2)> = (0..count)
        .filter_map(|k| {
            let (a, b) = (*window.get(k)?, *window.get((k + 1) % count)?);
            let d = b - a;
            let length = d.hypot();
            let out = if moves(a, b) && length > 0.0 { vectorcraft_geom::Vec2::new(d.y, -d.x) * (by / length) } else { vectorcraft_geom::Vec2::ZERO };
            Some((a + out, d))
        })
        .collect();
    (0..count)
        .filter_map(|k| {
            let ((p1, d1), (p2, d2)) = (*lines.get((k + count - 1) % count)?, *lines.get(k)?);
            let den = d1.cross(d2);
            if den.abs() <= 1e-12 * d1.hypot() * d2.hypot() {
                return Some(p2);
            }
            let t = (p2 - p1).cross(d2) / den;
            let p = p1 + d1 * t;
            p.is_finite().then_some(p)
        })
        .collect()
}

/// The parts of `pieces`, a map's art on its flat surface, that fall on `cell`, the cell of facet
/// `f`, drawn on the page. `None` past the limits.
fn cell_art(
    id: NodeId,
    pieces: &[Unrolled],
    cell: &MapCell,
    f: &Facet,
    recolor: &dyn Fn(Color) -> Color,
    budget: &mut MapBudget,
    copies: &mut usize,
) -> Option<Vec<Arc<Node>>> {
    if cell.uv.len() != cell.page.len() || !(3..=4).contains(&cell.uv.len()) {
        return Some(vec![]);
    }
    let Some(reach) = box_of(&cell.uv) else { return Some(vec![]) };
    // A facet cut after it was built, or one whose edges follow the profile's curves, draws only the
    // part of the art inside its own outline, which the regions reach where it bows out of the cell.
    let whole = f.outline.is_none() && f.contours.first() == Some(&cell.page);
    let regions = if whole { regions(cell) } else { reaching_outline(regions(cell), cell, f) };
    let within = regions.iter().filter_map(|(w, _)| box_of(w)).fold(reach, |a, b| a.union(b));
    let apart = |b: &Rect| b.x1 < within.x0 || b.x0 > within.x1 || b.y1 < within.y0 || b.y0 > within.y1;
    let mut out = vec![];
    for piece in pieces.iter().filter(|p| !apart(&p.bounds)) {
        // Each contour's bounds test counts as one point cut, whether or not the contour reaches
        // the cell.
        budget.cuts = budget.cuts.saturating_add(piece.boxes.len());
        if budget.cuts > MAX_SURFACE_CUTS {
            return None;
        }
        let near: Vec<&Vec<Point>> = piece.polygons.iter().zip(&piece.boxes).filter(|(_, b)| !apart(b)).map(|(p, _)| p).collect();
        let mut path = BezPath::new();
        for (window, mapper) in &regions {
            for polygon in &near {
                budget.cuts = budget.cuts.saturating_add(polygon.len());
                if budget.cuts > MAX_SURFACE_CUTS {
                    return None;
                }
                let cut = clip_convex(polygon, window);
                let Some((first, rest)) = cut.split_first() else { continue };
                path.move_to(mapper.at(*first));
                rest.iter().for_each(|q| path.line_to(mapper.at(*q)));
                path.close_path();
                budget.points = budget.points.saturating_add(cut.len());
            }
        }
        if budget.points > MAX_SURFACE_POINTS {
            return None;
        }
        if path.elements().is_empty() {
            continue;
        }
        let (path, rule) = if whole {
            (path, piece.rule)
        } else {
            let cut = vectorcraft_pathops::boolean(
                &PathData::from_bezpath(&path),
                piece.rule,
                &f.path(),
                FillRule::NonZero,
                vectorcraft_pathops::BoolOp::Intersect,
            );
            (cut.to_bezpath(), FillRule::NonZero)
        };
        if path.elements().is_empty() || !path.elements().iter().all(el_is_finite) {
            continue;
        }
        let seam = takes_seam(piece);
        if !crate::revolve::paint_copies_add(copies, 1 + usize::from(seam), &piece.paint) {
            return None;
        }
        // Gradients and patterns follow the cell's map as it is at the cell's middle.
        let middle = reach.center();
        let local = regions.first().map_or(Affine::IDENTITY, |(_, m)| m.local(middle, (reach.width().max(reach.height()) * 1e-3).max(1e-6)));
        let node = piece_node(
            id,
            piece.paint.clone(),
            (piece.opacity, piece.blend),
            (path, rule),
            local * piece.place,
            recolor,
            seam.then_some(SURFACE_SEAM),
        );
        out.push(Arc::new(node));
    }
    Some(out)
}

fn el_is_finite(el: &vectorcraft_geom::kurbo::PathEl) -> bool {
    use vectorcraft_geom::kurbo::PathEl;
    match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => p.is_finite(),
        PathEl::QuadTo(a, p) => a.is_finite() && p.is_finite(),
        PathEl::CurveTo(a, b, p) => a.is_finite() && b.is_finite() && p.is_finite(),
        PathEl::ClosePath => true,
    }
}

/// One painted area of mapped art on the page: `path` filled under `rule` with `paint`, its
/// gradients and pattern tiles placed through `gradient_map`, its colors through `recolor`, and
/// with `seam` a stroke of that width in the same paint over the seams between neighboring parts.
fn piece_node(
    id: NodeId,
    mut paint: Paint,
    (opacity, blend): (f32, vectorcraft_doc::color::BlendMode),
    (path, rule): (BezPath, FillRule),
    gradient_map: Affine,
    recolor: &dyn Fn(Color) -> Color,
    seam: Option<f64>,
) -> Node {
    if let Paint::Pattern { xf, .. } = &mut paint {
        *xf = gradient_map * *xf;
    }
    let mut fill = FillLayer::new(paint);
    fill.opacity = opacity;
    fill.blend = blend;
    let mut appearance = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() };
    appearance.transform_gradients(gradient_map);
    for item in appearance.items.iter_mut() {
        if let AppearanceItem::Fill(f) = item {
            // Pattern tiles keep their colors: the document's pattern draws them.
            vectorcraft_doc::recolor::map_paint(&mut f.paint, recolor);
        }
    }
    if let (Some(width), Some(AppearanceItem::Fill(f))) = (seam, appearance.items.first()) {
        let mut stroke = vectorcraft_doc::StrokeLayer::new(f.paint.clone(), width);
        stroke.join = vectorcraft_doc::LineJoin::Round;
        appearance.items.push(AppearanceItem::Stroke(stroke));
    }
    let mut node = Node::path(id, PathData::from_bezpath(&path), appearance);
    if let NodeKind::Path { rule: r, .. } = &mut node.kind {
        *r = rule;
    }
    node
}

#[cfg(test)]
#[path = "tests_map_art.rs"]
mod tests;
