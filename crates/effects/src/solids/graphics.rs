//! Graphics mapped onto a 3D object's planar faces (Window › 3D and Materials › Materials ›
//! Graphics): the art of a document symbol drawn on Extrude & Bevel's front or back cap, or on a
//! plane, turned, foreshortened and shaded with that face and clipped to it.
//!
//! The effect keeps its graphics in its `graphics` parameter, each `{symbol, face, x, y, scale,
//! rotation}`: the symbol's name, the face, the offset of the graphic's center from the center of
//! the object's bounds in the face's own plane (points, x to the right and y down as the face is
//! seen from the front of that face), its size as a share of the symbol's own, and its turn
//! (degrees, counterclockwise). On the back cap, x runs to the right as the back is seen from
//! behind, so the graphic reads the right way round there.
//!
//! On Revolve the graphics lie on the swept side surface, unrolled flat
//! ([`vectorcraft_three_d::SurfaceCell`]): x and y are the offset of the graphic's center from the
//! middle of that surface, x around the axis and y along the profile, and each face of the side
//! draws the part of the graphics that falls on it ([`surface_parts`]).

use std::sync::Arc;

use serde_json::{Map, Value, json};
use vectorcraft_doc::color::{BlendMode, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Document, FillLayer, Node, NodeId, NodeKind, Symbol};
use vectorcraft_geom::kurbo::{CubicBez, ParamCurve, PathEl, Shape};
use vectorcraft_geom::{Affine, BezPath, FillRule, PathData, Point, Rect, SubPath, Vec2};
use vectorcraft_three_d::{FaceMap, Facet, PlanarFace};

/// The effect parameter that holds the graphics.
pub const GRAPHICS: &str = "graphics";

/// The most graphics one 3D effect maps.
pub const MAX_GRAPHICS: usize = 64;

/// The most fills and stroke outlines the art of one symbol draws as a graphic.
const MAX_PIECES: usize = 4096;

/// The most path elements the graphics of one 3D effect draw together, after a perspective cuts
/// their curves.
const MAX_ELEMENTS: usize = 400_000;

/// The pieces each curve of a graphic is cut into under a perspective map, which bends it.
const PERSPECTIVE_PIECES: usize = 8;

/// How deep symbol instances in a graphic's art may nest.
const MAX_NESTING: usize = 8;

/// How deep the objects of a graphic's art may be, counting groups and the art of the symbols it
/// places: as deep as an object may be drawn.
const MAX_LEVELS: usize = 4 * vectorcraft_doc::MAX_DEPTH;

/// The error for graphics on a 3D effect that takes none.
pub const NOT_TAKEN: &str = "Graphics map onto the Plane, Extrude and Revolve object types";

/// The buckets around the unrolled side surface that the pieces of Revolve's graphics are sorted
/// into, so each face looks only at the pieces near it.
const SURFACE_BUCKETS: usize = 256;

/// How closely the curves of a graphic on Revolve's side follow their shape when they are cut into
/// straight pieces, in points of the unrolled surface.
const SURFACE_TOLERANCE: f64 = 0.05;

/// How far from the middle of Revolve's unrolled side a curve of a graphic may reach, in points:
/// as far as a Revolve profile's coordinates may.
const SURFACE_REACH: f64 = 1e8;

/// The most points the parts of Revolve's graphics draw on its faces together, a graphic's area cut
/// at every face it crosses. Past it no graphic is drawn on that surface.
const MAX_SURFACE_POINTS: usize = 2_000_000;

/// The most points of Revolve's graphics cut at its faces together, counted before each cut whether
/// or not it keeps any of them. Past it no graphic is drawn on that surface.
const MAX_SURFACE_CUTS: usize = 32 * MAX_SURFACE_POINTS;

/// The width of the stroke of the same paint around each opaque part of a graphic on Revolve's
/// side, which covers the seams between the parts on neighboring faces.
const SURFACE_SEAM: f64 = 0.3;

/// The size range of a graphic, as a percentage of its symbol's own size.
pub const SCALE_RANGE: (f64, f64) = (1.0, 10_000.0);

/// How far a graphic's center may be from the center of its object, in points.
pub const MAX_OFFSET: f64 = 1e6;

/// One graphic mapped onto a face of a 3D object.
#[derive(Clone, Debug, PartialEq)]
pub struct MappedGraphic {
    pub symbol: String,
    pub face: PlanarFace,
    pub x: f64,
    pub y: f64,
    /// Percent of the symbol's own size.
    pub scale: f64,
    /// Degrees, counterclockwise.
    pub rotation: f64,
}

impl MappedGraphic {
    pub fn to_value(&self) -> Value {
        json!({"symbol": self.symbol, "face": self.face.id(), "x": self.x, "y": self.y, "scale": self.scale, "rotation": self.rotation})
    }

    /// The graphic `v` describes. Only `symbol` is required: `face` defaults to front, the offsets
    /// and the rotation to 0 and the scale to 100.
    pub fn from_value(v: &Value) -> Result<Self, String> {
        let Value::Object(m) = v else { return Err(format!("each of `{GRAPHICS}` must be an object")) };
        if let Some(k) = m.keys().find(|k| !matches!(k.as_str(), "symbol" | "face" | "x" | "y" | "scale" | "rotation")) {
            return Err(format!("`{GRAPHICS}` entries take symbol, face, x, y, scale and rotation, not `{k}`"));
        }
        let symbol = match m.get("symbol") {
            Some(Value::String(s)) if !s.trim().is_empty() => s.clone(),
            _ => return Err(format!("each of `{GRAPHICS}` needs `symbol`, the name of a symbol")),
        };
        let face = match m.get("face") {
            None => PlanarFace::Front,
            Some(v) => v.as_str().and_then(PlanarFace::parse).ok_or_else(|| format!("`{GRAPHICS}` face must be front or back"))?,
        };
        let number = |key: &str, default: f64, (lo, hi): (f64, f64)| -> Result<f64, String> {
            match m.get(key) {
                None => Ok(default),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite() && (lo..=hi).contains(n))
                    .ok_or_else(|| format!("`{GRAPHICS}` {key} must be a number from {lo} to {hi}")),
            }
        };
        Ok(Self {
            symbol,
            face,
            x: number("x", 0.0, (-MAX_OFFSET, MAX_OFFSET))?,
            y: number("y", 0.0, (-MAX_OFFSET, MAX_OFFSET))?,
            scale: number("scale", 100.0, SCALE_RANGE)?,
            rotation: number("rotation", 0.0, (-360.0, 360.0))?,
        })
    }

    /// Where the graphic's art goes in the face's plane, for symbol art whose bounds are `art` on an
    /// object whose bounds have center `center`.
    fn placement(&self, art: Rect, center: Point) -> Affine {
        let place = Affine::translate((center.x + self.x, center.y + self.y))
            * Affine::rotate(-self.rotation.to_radians())
            * Affine::scale(self.scale / 100.0)
            * Affine::translate(-art.center().to_vec2());
        match self.face {
            PlanarFace::Front => place,
            // Seen from behind, the back's right is the front's left.
            PlanarFace::Back => {
                Affine::translate(center.to_vec2()) * Affine::scale_non_uniform(-1.0, 1.0) * Affine::translate(-center.to_vec2()) * place
            }
        }
    }
}

/// The 3D effects that take graphics: Extrude & Bevel (its caps), Rotate (the plane) and Revolve
/// (its swept side).
pub fn takes_graphics(id: &str) -> bool {
    id == super::EXTRUDE || id == super::ROTATE || id == crate::REVOLVE
}

/// The graphics in parameters `p`, leaving out entries that are not valid (a file may hold
/// anything; commands check every entry with [`validate`]).
pub fn mapped_graphics(p: &Value) -> Vec<MappedGraphic> {
    match p.get(GRAPHICS) {
        Some(Value::Array(list)) => list.iter().take(MAX_GRAPHICS).filter_map(|v| MappedGraphic::from_value(v).ok()).collect(),
        _ => vec![],
    }
}

/// Are the graphics in parameters `p` of 3D effect `id` valid? Only Extrude & Bevel, Rotate and
/// Revolve take graphics; a plane has only a front, and Revolve's graphics go on its side, named
/// the front.
pub fn validate(id: &str, p: &Value) -> Result<(), String> {
    let list = match p.get(GRAPHICS) {
        None => return Ok(()),
        Some(Value::Array(list)) => list,
        Some(_) => return Err(format!("`{GRAPHICS}` must be an array")),
    };
    if list.is_empty() {
        return Ok(());
    }
    if !takes_graphics(id) {
        return Err(NOT_TAKEN.into());
    }
    if list.len() > MAX_GRAPHICS {
        return Err(format!("`{GRAPHICS}` takes at most {MAX_GRAPHICS} graphics"));
    }
    for v in list {
        let g = MappedGraphic::from_value(v)?;
        if id == super::ROTATE && g.face == PlanarFace::Back {
            return Err("A plane has one face: map its graphics onto the front".into());
        }
        if id == crate::REVOLVE && g.face == PlanarFace::Back {
            return Err("Revolve maps graphics onto its side: map them onto the front".into());
        }
    }
    Ok(())
}

/// The first symbol the graphics in `p` name that `doc` does not have, leaving out the symbols the
/// graphics in `old` (the effect's parameters before an edit) already name: a graphic whose symbol
/// was deleted may stay.
pub fn missing_symbol(doc: &Document, p: &Value, old: Option<&Value>) -> Option<String> {
    let kept = old.map(mapped_graphics).unwrap_or_default();
    mapped_graphics(p)
        .into_iter()
        .map(|g| g.symbol)
        .find(|name| !doc.symbols.iter().any(|s| s.name == *name) && !kept.iter().any(|g| g.symbol == *name))
}

/// Does the art of the symbols the graphics in `p` name fit the limits? A graphic whose symbol
/// `doc` does not have draws nothing and is not counted. With a perspective, each curve counts as
/// the [`PERSPECTIVE_PIECES`] pieces it is cut into; without one, it counts once.
pub fn validate_symbols(doc: &Document, p: &Value) -> Result<(), String> {
    let curve = if super::view_options(p).perspective > 0.0 { PERSPECTIVE_PIECES } else { 1 };
    let mut elements = 0usize;
    for g in mapped_graphics(p) {
        let Some(symbol) = doc.symbols.iter().find(|s| s.name == g.symbol) else { continue };
        let pieces = pieces_of(doc, symbol)?;
        elements = elements.saturating_add(pieces.iter().map(|p| drawn_elements(&p.path, curve)).sum());
        if elements > MAX_ELEMENTS {
            return Err("The graphics are too complex: simplify the symbols' art".into());
        }
    }
    Ok(())
}

/// The path elements `path` draws when each curve draws `curve` elements, before curves near the
/// eye are split further: one for each straight line, move and close.
fn drawn_elements(path: &PathData, curve: usize) -> usize {
    let segments = |s: &SubPath| (0..s.segment_count()).map(|i| if s.segment_is_line(i) { 1 } else { curve }).sum::<usize>();
    path.subpaths.iter().map(|s| 1 + usize::from(s.closed) + segments(s)).sum()
}

/// One area of a graphic's art, drawn with one paint: a fill, or a stroke's outline.
struct Piece {
    path: PathData,
    rule: FillRule,
    paint: Paint,
    opacity: f32,
    blend: BlendMode,
}

/// The areas the art of `symbol` paints, back to front, at the symbol's natural size: the fills of
/// its paths, compound paths and type (as outlines), and its strokes' outlines, through the symbol
/// instances it holds (with their changes to dynamic symbols, and through 9-slice guides). Hidden objects, guides, clipping paths and images are left out, and clipping
/// groups draw their art unclipped.
fn pieces_of(doc: &Document, symbol: &Symbol) -> Result<Vec<Piece>, String> {
    fn walk(doc: &Document, n: &Node, outer: f32, xf: Affine, (depth, level): (usize, usize), out: &mut Vec<Piece>) -> Result<(), String> {
        if !n.visible || n.is_guide() {
            return Ok(());
        }
        if level > MAX_LEVELS {
            return Err(format!("A graphic's art may be nested at most {MAX_LEVELS} levels deep: simplify the symbol's art"));
        }
        let opacity = outer * n.opacity;
        let shape = match &n.kind {
            NodeKind::Path { path, rule, clipping: false, .. } => Some((path.clone(), *rule)),
            NodeKind::Compound { children, rule } => {
                Some((PathData::new(children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect()), *rule))
            }
            NodeKind::Text(_) => {
                if let Some(mut outlined) = crate::outline_text(n) {
                    // The outlines take the type's place, with its opacity.
                    outlined.visible = true;
                    outlined.opacity = n.opacity;
                    walk(doc, &outlined, outer, xf, (depth, level + 1), out)?;
                }
                None
            }
            NodeKind::SymbolInstance { symbol, xf: at } => {
                if let Some(s) = doc.symbols.iter().find(|s| s.name == *symbol) {
                    if depth >= MAX_NESTING {
                        return Err(format!("A graphic's art may nest symbols at most {MAX_NESTING} levels deep: simplify the symbol's art"));
                    }
                    // The instance shows its own changes to a dynamic symbol's art, and a symbol
                    // with 9-slice guides is placed through them, as the canvas draws it. The
                    // pieces are not clipped, so paths that cross a guide are cut at it.
                    let art = vectorcraft_doc::symbol_overrides::dynamic_art(&s.art, n);
                    if doc.symbol_slice_map(symbol, *at).is_some() {
                        let Some(placed) = doc.place_symbol_art_unclipped(symbol, &art, *at) else {
                            return Err("A graphic's art holds too many objects: simplify the symbol's art".into());
                        };
                        walk(doc, &placed, opacity, xf, (depth + 1, level + 1), out)?;
                    } else {
                        walk(doc, &art, opacity, xf * *at, (depth + 1, level + 1), out)?;
                    }
                }
                None
            }
            _ => {
                for c in n.children().into_iter().flatten() {
                    walk(doc, c, opacity, xf, (depth, level + 1), out)?;
                }
                None
            }
        };
        let Some((path, rule)) = shape else { return Ok(()) };
        // Symbol art keeps its strokes' widths as drawn while its geometry is scaled to the
        // symbol's square, so a stroke is outlined after the art takes its size.
        let path = path.transformed(xf);
        for item in n.appearance.items.iter().filter(|i| i.visible()) {
            let (area, rule, paint, item_opacity, blend) = match item {
                AppearanceItem::Fill(f) => (path.clone(), rule, f.paint.clone(), f.opacity, f.blend),
                AppearanceItem::Stroke(s) if s.width > 0.0 && s.width.is_finite() => {
                    (crate::stroke::outline_region(&path, rule, s), FillRule::EvenOdd, s.paint.clone(), s.opacity, s.blend)
                }
                AppearanceItem::Stroke(_) => continue,
            };
            if matches!(paint, Paint::None) || area.bounds().is_none() {
                continue;
            }
            if out.len() >= MAX_PIECES {
                return Err(format!("A graphic draws at most {MAX_PIECES} fills and strokes: simplify the symbol's art"));
            }
            // A placed gradient or pattern takes the same size and place as its path.
            let mut paint = paint;
            match &mut paint {
                Paint::Gradient(g) => g.transform(xf),
                Paint::Pattern { xf: placed, .. } => *placed = xf * *placed,
                Paint::None | Paint::Solid { .. } => {}
            }
            out.push(Piece { path: area, rule, paint, opacity: opacity * item_opacity, blend });
        }
        Ok(())
    }
    let mut out = vec![];
    walk(doc, &symbol.art, 1.0, doc.symbol_natural_xf(&symbol.name), (0, 1), &mut out)?;
    Ok(out)
}

/// `path` through `map`: exactly for an affine map, otherwise with each curve cut into
/// [`PERSPECTIVE_PIECES`] pieces whose control points go through it (straight lines stay straight
/// under a perspective). Fails when part of it has no page point, or when it takes more than `room`
/// elements.
fn map_path(path: &PathData, map: &FaceMap, affine: Option<Affine>, room: usize) -> Result<BezPath, NotMapped> {
    let bp = path.to_bezpath();
    if let Some(a) = affine {
        return if bp.elements().len() <= room { Ok(a * bp) } else { Err(NotMapped::Full) };
    }
    let to = |p: Point| map.to_page(p).ok_or(NotMapped::Behind);
    let mut out = BezPath::new();
    let mut at = Point::ZERO;
    for el in bp.elements() {
        if out.elements().len() > room {
            return Err(NotMapped::Full);
        }
        match *el {
            PathEl::MoveTo(p) => {
                out.move_to(to(p)?);
                at = p;
            }
            PathEl::LineTo(p) => {
                out.line_to(to(p)?);
                at = p;
            }
            PathEl::QuadTo(c, p) => {
                cubic_to(&mut out, vectorcraft_geom::kurbo::QuadBez::new(at, c, p).raise(), map).ok_or(NotMapped::Behind)?;
                at = p;
            }
            PathEl::CurveTo(c1, c2, p) => {
                cubic_to(&mut out, CubicBez::new(at, c1, c2, p), map).ok_or(NotMapped::Behind)?;
                at = p;
            }
            PathEl::ClosePath => out.close_path(),
        }
    }
    if out.elements().len() <= room { Ok(out) } else { Err(NotMapped::Full) }
}

/// Why [`map_path`] drew nothing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NotMapped {
    /// Part of the path has no page point: it is at or behind the eye.
    Behind,
    /// The path takes more elements than it has room for.
    Full,
}

/// Curve `c` through `map`, cut into [`PERSPECTIVE_PIECES`] pieces. A piece that lies in front of
/// the eye while a control point does not is split further ([`piece_to`]). `None` when part of a
/// piece has no page point.
fn cubic_to(out: &mut BezPath, c: CubicBez, map: &FaceMap) -> Option<()> {
    for k in 0..PERSPECTIVE_PIECES {
        let (t0, t1) = (k as f64 / PERSPECTIVE_PIECES as f64, (k + 1) as f64 / PERSPECTIVE_PIECES as f64);
        let s = c.subsegment(t0..t1);
        if !map.maps_curve([s.p0, s.p1, s.p2, s.p3]) {
            return None;
        }
        piece_to(out, s, map, MAX_SPLITS)?;
    }
    Some(())
}

/// How many times [`piece_to`] halves a piece whose control points are behind the eye.
const MAX_SPLITS: usize = 4;

/// Piece `s`, which lies in front of the eye, through `map`: halved until the control points of
/// each half have page points, at most `splits` times, and drawn straight past that.
fn piece_to(out: &mut BezPath, s: CubicBez, map: &FaceMap, splits: usize) -> Option<()> {
    let end = map.to_page(s.p3)?;
    match (map.to_page(s.p1), map.to_page(s.p2)) {
        (Some(c1), Some(c2)) => out.curve_to(c1, c2, end),
        _ if splits == 0 => out.line_to(end),
        _ => {
            let (a, b) = s.subdivide();
            piece_to(out, a, map, splits - 1)?;
            piece_to(out, b, map, splits - 1)?;
        }
    }
    Some(())
}

/// The graphics `graphics` lists for `face`, drawn on caps `caps` (the faces of that end of the
/// object's solids that the viewer sees) through `map`, for an object whose bounds have
/// center `center`: one clipping group of every such graphic, shaded with the first cap's light.
/// Pattern tiles keep their colors: the document's pattern draws them. A graphic whose symbol
/// `doc` does not have draws nothing. `elements` counts the path elements
/// the effect's graphics have drawn so far, after a perspective cuts their curves, and is shared by
/// every call for one effect. `None` when there is nothing to draw or that count passes
/// [`MAX_ELEMENTS`].
pub(super) fn face_art(
    doc: Option<&Document>,
    id: NodeId,
    graphics: &[MappedGraphic],
    (face, map): (PlanarFace, &FaceMap),
    center: Point,
    caps: &[&Facet],
    elements: &mut usize,
) -> Option<Node> {
    let doc = doc?;
    let light = caps.first()?.illumination;
    let affine = map.local_affine(center).filter(|_| map.is_affine());
    let mut children = vec![];
    for g in graphics.iter().filter(|g| g.face == face) {
        let Some(symbol) = doc.symbols.iter().find(|s| s.name == g.symbol) else { continue };
        // A symbol past the limits is left out; the other graphics still draw.
        let Ok(pieces) = pieces_of(doc, symbol) else { continue };
        let Some(bounds) = pieces.iter().filter_map(|p| p.path.bounds()).reduce(|a, b| a.union(b)) else { continue };
        let place = g.placement(bounds, center);
        // Gradients follow the face's map as it is near the graphic's center.
        let middle = place * bounds.center();
        // When the center is at or behind the eye, they follow it near the middle of the part that
        // is in front; a graphic with no part in front is left out.
        let local = map.local_affine(middle).or_else(|| {
            let part = map.mapped_part(place.transform_rect_bbox(bounds))?;
            let sum = part.iter().fold(Point::ZERO, |s, p| s + p.to_vec2());
            map.local_affine(Point::new(sum.x / part.len() as f64, sum.y / part.len() as f64))
        });
        let Some(local) = local else { continue };
        let gradient_map = local * place;
        for piece in pieces {
            let placed = piece.path.transformed(place);
            // Each anchor draws at least one element. A perspective cuts curves into more, and
            // straight lines stay one each: the path may take what is left of the limit, and is
            // counted as it is drawn. Past the limit, the face draws nothing.
            let least: usize = placed.subpaths.iter().map(|s| s.anchors.len()).sum();
            let before = *elements;
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
            *elements = before.saturating_add(mapped.elements().len());
            let mut paint = piece.paint;
            vectorcraft_doc::recolor::map_paint(&mut paint, &|c| crate::lighting::shade(c, light));
            // A pattern's tiles follow the graphic onto the face, as its gradients do below.
            if let Paint::Pattern { xf, .. } = &mut paint {
                *xf = gradient_map * *xf;
            }
            let mut fill = FillLayer::new(paint);
            fill.opacity = piece.opacity;
            fill.blend = piece.blend;
            let mut appearance = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() };
            appearance.transform_gradients(gradient_map);
            let mut node = Node::path(id, PathData::from_bezpath(&mapped), appearance);
            if let NodeKind::Path { rule: r, .. } = &mut node.kind {
                *r = rule;
            }
            children.push(Arc::new(node));
        }
    }
    if children.is_empty() {
        return None;
    }
    let mut clip = BezPath::new();
    for cap in caps {
        // A cap's holes are contours of the same path wound against its outline, so turning the
        // whole path keeps them holes under the nonzero rule.
        let mut outline = cap.path().to_bezpath();
        if outline.area() < 0.0 {
            outline = outline.reverse_subpaths();
        }
        clip.extend(outline.iter());
    }
    let clip = Node::path(id, PathData::from_bezpath(&clip), Appearance::default());
    Some(Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), Arc::new(Node::group(id, children))], clip: true }))
}

/// One area of a graphic placed on Revolve's unrolled side surface, cut into straight pieces.
struct Unrolled {
    polygons: Vec<Vec<Point>>,
    /// The bounds of each of `polygons`.
    boxes: Vec<Rect>,
    bounds: Rect,
    rule: FillRule,
    paint: Paint,
    opacity: f32,
    blend: BlendMode,
    /// Where the graphic's art goes on the surface: its gradients and patterns are placed with it.
    place: Affine,
}

/// The areas of `graphics` (those of the front: Revolve's side) placed on the unrolled side
/// surface, cut into straight pieces. A graphic whose symbol `doc` does not have, or whose art is
/// past the limits of [`pieces_of`], is left out. `None` when the pieces take more than
/// [`MAX_ELEMENTS`] points together.
fn unrolled(doc: &Document, graphics: &[MappedGraphic]) -> Option<Vec<Unrolled>> {
    let mut pieces = vec![];
    let mut points = 0usize;
    for g in graphics.iter().filter(|g| g.face == PlanarFace::Front) {
        let Some(symbol) = doc.symbols.iter().find(|s| s.name == g.symbol) else { continue };
        let Ok(art) = pieces_of(doc, symbol) else { continue };
        let Some(bounds) = art.iter().filter_map(|p| p.path.bounds()).reduce(|a, b| a.union(b)) else { continue };
        let place = g.placement(bounds, Point::ZERO);
        for piece in art {
            let placed = place * piece.path.to_bezpath();
            let mut polygons = straight_pieces(&placed, &mut points)?;
            polygons.retain(|p| p.len() >= 3 && p.iter().all(|q| q.is_finite()));
            let boxes: Vec<Rect> =
                polygons.iter().filter_map(|polygon| polygon.iter().map(|p| Rect::from_points(*p, *p)).reduce(|a, b| a.union(b))).collect();
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
    }
    Some(pieces)
}

/// The contours of `path` cut into straight pieces within [`SURFACE_TOLERANCE`], adding the points
/// made to `points`. Each curve is cut on its own and the count is checked before the next one, so
/// the work stops at the first curve past [`MAX_ELEMENTS`] points: `None` then, and for a path with
/// a coordinate of a line or a curve farther than [`SURFACE_REACH`] from the origin, which bounds
/// the pieces of one curve and the arithmetic of the cuts at the faces.
fn straight_pieces(path: &BezPath, points: &mut usize) -> Option<Vec<Vec<Point>>> {
    let near = |p: &Point| p.x.abs() <= SURFACE_REACH && p.y.abs() <= SURFACE_REACH;
    let mut polygons: Vec<Vec<Point>> = vec![];
    let mut current: Vec<Point> = vec![];
    let (mut start, mut last) = (Point::ZERO, Point::ZERO);
    for el in path.elements() {
        if *points > MAX_ELEMENTS {
            return None;
        }
        match *el {
            PathEl::MoveTo(p) | PathEl::LineTo(p) if !near(&p) => return None,
            PathEl::MoveTo(p) => {
                polygons.push(std::mem::take(&mut current));
                current.push(p);
                *points += 1;
                (start, last) = (p, p);
            }
            PathEl::LineTo(p) => {
                current.push(p);
                *points += 1;
                last = p;
            }
            PathEl::ClosePath => {
                polygons.push(std::mem::take(&mut current));
                last = start;
            }
            PathEl::QuadTo(a, p) | PathEl::CurveTo(_, a, p) => {
                let first = if let PathEl::CurveTo(b, ..) = *el { b } else { a };
                if ![last, first, a, p].iter().all(near) {
                    return None;
                }
                vectorcraft_geom::kurbo::flatten([PathEl::MoveTo(last), *el], SURFACE_TOLERANCE, |cut| {
                    if let PathEl::LineTo(q) = cut {
                        current.push(q);
                        *points += 1;
                    }
                });
                last = p;
            }
        }
    }
    polygons.push(current);
    (*points <= MAX_ELEMENTS).then_some(polygons)
}

/// Do the graphics in Revolve parameters `p` fit the limits with the symbols of `doc`? Their art,
/// cut into straight pieces on the side surface, may take up to [`MAX_ELEMENTS`] points together.
pub(crate) fn validate_surface_symbols(doc: &Document, p: &Value) -> Result<(), String> {
    let list = mapped_graphics(p);
    if list.is_empty() || unrolled(doc, &list).is_some() {
        return Ok(());
    }
    Err("The graphics are too complex: simplify the symbols' art".into())
}

/// The parts of `graphics` that fall on each of `faces`, a revolved surface's faces in drawing
/// order, for an object with id `id`: for each face, the nodes to draw right after it. Only faces
/// of the swept side that show their outside to the viewer carry a part, so a graphic narrows
/// toward the silhouette, is hidden past it and is covered by the nearer faces drawn after it.
/// `shade` gives the paint of a solid color on a face, as the surface under it is shaded; the
/// colors of gradients and patterns are multiplied by the face's light. A graphic whose symbol
/// `doc` does not have, or that is past the limits of its art, draws nothing. `None` past
/// [`MAX_ELEMENTS`] points together before they are cut at the faces, [`MAX_SURFACE_POINTS`]
/// after, [`MAX_SURFACE_CUTS`] points cut, or when the copies of their paints in the parts take
/// `copies`, the bytes of the copies of the object's graphics so far, past Revolve's budget for
/// them: commands refuse such graphics, and a loaded file draws the surface without them.
pub(crate) fn surface_parts(
    doc: Option<&Document>,
    id: NodeId,
    graphics: &[MappedGraphic],
    faces: &[vectorcraft_three_d::Face],
    shade: &dyn Fn(&vectorcraft_three_d::Face, vectorcraft_doc::color::Color) -> Paint,
    copies: &mut usize,
) -> Option<Vec<Vec<Arc<Node>>>> {
    let mut out: Vec<Vec<Arc<Node>>> = vec![vec![]; faces.len()];
    let Some(doc) = doc else { return Some(out) };
    let pieces = unrolled(doc, graphics)?;
    let Some(all) = pieces.iter().map(|p| p.bounds).reduce(|a, b| a.union(b)) else { return Some(out) };
    // Each piece listed in the buckets of the surface's x its bounds reach across.
    let width = (all.width() / SURFACE_BUCKETS as f64).max(1e-9);
    let bucket = |x: f64| (((x - all.x0) / width).floor().max(0.0) as usize).min(SURFACE_BUCKETS - 1);
    let mut buckets: Vec<Vec<usize>> = vec![vec![]; SURFACE_BUCKETS];
    for (k, p) in pieces.iter().enumerate() {
        for b in buckets.get_mut(bucket(p.bounds.x0)..=bucket(p.bounds.x1)).into_iter().flatten() {
            b.push(k);
        }
    }
    let mut seen = vec![usize::MAX; pieces.len()];
    let mut drawn = 0usize;
    let mut cut_points = 0usize;
    let mut triangle_id = 0usize;
    for (face, parts) in faces.iter().zip(out.iter_mut()) {
        let Some(cell) = face.surface.as_ref().filter(|c| c.outward && !face.cap) else { continue };
        if cell.uv.len() != cell.page.len() || !(3..=4).contains(&cell.uv.len()) {
            continue;
        }
        let triangles: &[[usize; 3]] = if cell.uv.len() == 3 { &[[0, 1, 2]] } else { &[[0, 1, 2], [0, 2, 3]] };
        for corners in triangles {
            triangle_id += 1;
            let pick = |of: &[Point]| corners.iter().map(|k| of.get(*k).copied()).collect::<Option<Vec<Point>>>();
            let (Some(uv), Some(page)) = (pick(&cell.uv), pick(&cell.page)) else { continue };
            let (Some(map), Some(window)) = (triangle_map(&uv, &page), counterclockwise(&uv)) else { continue };
            // A face along an end cap's ring has that edge curved on the page: its parts are cut
            // from a larger triangle and then to the face's own outline, so they reach the curve
            // where the graphic runs past the end of the profile. A four-cornered face is cut into
            // two triangles along its diagonal from corner 0 to corner 2: each takes the part of
            // the outline on its own side of it.
            let window = if face.outline.is_some() { grown(window, 3.0) } else { window };
            // The pieces are looked up over the whole of the window they are cut to.
            let Some(reach) = window.iter().map(|p| Rect::from_points(*p, *p)).reduce(|a, b| a.union(b)) else { continue };
            if reach.x1 < all.x0 || reach.x0 > all.x1 || reach.y1 < all.y0 || reach.y0 > all.y1 {
                continue;
            }
            // The pieces near the triangle, each once, in the graphics' drawing order.
            let mut near = vec![];
            for b in buckets.get(bucket(reach.x0)..=bucket(reach.x1)).into_iter().flatten() {
                for &k in b {
                    let Some(slot) = seen.get_mut(k) else { continue };
                    if *slot != triangle_id {
                        *slot = triangle_id;
                        near.push(k);
                    }
                }
            }
            near.retain(|k| {
                pieces
                    .get(*k)
                    .is_some_and(|p| p.bounds.x1 >= reach.x0 && p.bounds.x0 <= reach.x1 && p.bounds.y1 >= reach.y0 && p.bounds.y0 <= reach.y1)
            });
            if near.is_empty() {
                continue;
            }
            near.sort_unstable();
            let own = corners.iter().find(|k| **k != 0 && **k != 2).and_then(|k| cell.page.get(*k)).filter(|_| triangles.len() == 2);
            let rim = face.outline.as_ref().map(|outline| match (own, cell.page.first(), cell.page.get(2)) {
                (Some(&toward), Some(&a), Some(&b)) => {
                    let side = side_of(a, b, toward);
                    vectorcraft_pathops::boolean(outline, FillRule::NonZero, &side, FillRule::NonZero, vectorcraft_pathops::BoolOp::Intersect)
                }
                _ => outline.clone(),
            });
            let rim = rim.as_ref();
            for k in near {
                let Some(piece) = pieces.get(k) else { continue };
                let mut path = BezPath::new();
                // Each contour is cut only where its own bounds reach the triangle.
                for (polygon, b) in piece.polygons.iter().zip(&piece.boxes) {
                    if b.x1 < reach.x0 || b.x0 > reach.x1 || b.y1 < reach.y0 || b.y0 > reach.y1 {
                        continue;
                    }
                    cut_points = cut_points.saturating_add(polygon.len());
                    if cut_points > MAX_SURFACE_CUTS {
                        return None;
                    }
                    let cut = clip_to(polygon, &window);
                    let Some((first, rest)) = cut.split_first() else { continue };
                    path.move_to(map * *first);
                    rest.iter().for_each(|q| path.line_to(map * *q));
                    path.close_path();
                    drawn = drawn.saturating_add(cut.len());
                }
                if drawn > MAX_SURFACE_POINTS {
                    return None;
                }
                if path.elements().is_empty() {
                    continue;
                }
                let (path, rule) = match rim {
                    Some(outline) => {
                        let cut = vectorcraft_pathops::boolean(
                            &PathData::from_bezpath(&path),
                            piece.rule,
                            outline,
                            FillRule::NonZero,
                            vectorcraft_pathops::BoolOp::Intersect,
                        );
                        (cut.to_bezpath(), FillRule::NonZero)
                    }
                    None => (path, piece.rule),
                };
                if path.elements().is_empty() {
                    continue;
                }
                // Each part copies its paint into its fill, and into the stroke over its seams when
                // it takes one.
                if !crate::revolve::paint_copies_add(copies, 1 + usize::from(takes_seam(piece)), &piece.paint) {
                    return None;
                }
                parts.push(Arc::new(surface_piece(id, piece, (path, rule), map, face, shade)));
            }
        }
    }
    Some(out)
}

/// The node of the part `path` (on the page, filled under `rule`) of `piece`, which `map` takes
/// from the unrolled surface onto `face`.
fn surface_piece(
    id: NodeId,
    piece: &Unrolled,
    (path, rule): (BezPath, FillRule),
    map: Affine,
    face: &vectorcraft_three_d::Face,
    shade: &dyn Fn(&vectorcraft_three_d::Face, vectorcraft_doc::color::Color) -> Paint,
) -> Node {
    let gradient_map = map * piece.place;
    // Under several lights a face's light varies across it in each channel: a solid color draws
    // as the face's own lit art in that color, clipped to the part. An opaque part also draws that
    // art clipped to a band of the seam stroke's width around its edges, as the other parts take
    // a stroke over their seams.
    if let (Some(c), Some(light)) = (piece.paint.color().filter(|_| matches!(piece.paint, Paint::Solid { .. })), face.rgb_gradient) {
        let lit = Arc::new(crate::lighting::rgb_gradient_art(id, &face.path(), &Paint::solid(c), light));
        let clipped = |area: &BezPath, rule: FillRule| {
            let mut clip = Node::path(id, PathData::from_bezpath(area), Appearance::default());
            if let NodeKind::Path { rule: r, .. } = &mut clip.kind {
                *r = rule;
            }
            Node::new(id, NodeKind::Group { children: vec![Arc::new(clip), lit.clone()], clip: true })
        };
        let mut group = clipped(&path, rule);
        if takes_seam(piece) {
            let style = vectorcraft_geom::kurbo::Stroke::new(SURFACE_SEAM).with_join(vectorcraft_geom::kurbo::Join::Round);
            let band = vectorcraft_geom::kurbo::stroke(path.iter(), &style, &vectorcraft_geom::kurbo::StrokeOpts::default(), SURFACE_TOLERANCE);
            group = Node::group(id, vec![Arc::new(group), Arc::new(clipped(&band, FillRule::NonZero))]);
        }
        group.opacity = piece.opacity;
        group.blend = piece.blend;
        return group;
    }
    let mut paint = piece.paint.clone();
    if let Paint::Pattern { xf, .. } = &mut paint {
        *xf = gradient_map * *xf;
    }
    let solid = paint.color().filter(|_| matches!(paint, Paint::Solid { .. }));
    let mut fill = FillLayer::new(paint);
    fill.opacity = piece.opacity;
    fill.blend = piece.blend;
    let mut appearance = Appearance { items: vec![AppearanceItem::Fill(fill)], ..Default::default() };
    appearance.transform_gradients(gradient_map);
    for item in appearance.items.iter_mut() {
        if let AppearanceItem::Fill(f) = item {
            match solid {
                Some(c) => f.paint = shade(face, c),
                None => {
                    vectorcraft_doc::recolor::map_paint(&mut f.paint, &|c| crate::lighting::shade(c, face.illumination));
                }
            }
        }
    }
    // Parts on neighboring faces meet edge to edge: an opaque part takes a narrow stroke of its
    // paint over the seam, as the faces do.
    if takes_seam(piece)
        && let Some(AppearanceItem::Fill(f)) = appearance.items.first()
    {
        let mut seam = vectorcraft_doc::StrokeLayer::new(f.paint.clone(), SURFACE_SEAM);
        seam.join = vectorcraft_doc::LineJoin::Round;
        appearance.items.push(AppearanceItem::Stroke(seam));
    }
    let mut node = Node::path(id, PathData::from_bezpath(&path), appearance);
    if let NodeKind::Path { rule: r, .. } = &mut node.kind {
        *r = rule;
    }
    node
}

/// Does each part of `piece` take a stroke of its paint over its seams? Only an opaque part does:
/// the overlap of a transparent one, of one whose gradient has a stop or a freeform point that is
/// not fully opaque, or of a pattern, whose tile may hold transparent art, would show.
fn takes_seam(piece: &Unrolled) -> bool {
    let clear = match &piece.paint {
        Paint::Gradient(g) => {
            g.gradient.stops.iter().any(|s| s.opacity < 1.0) || g.freeform.as_ref().is_some_and(|f| f.points.iter().any(|p| p.opacity < 1.0))
        }
        Paint::Pattern { .. } => true,
        _ => false,
    };
    piece.opacity >= 1.0 && piece.blend == BlendMode::Normal && !clear
}

/// A region far larger than the face on the side of the line through `a` and `b` that `toward` is
/// on: a rectangle with one edge along the line, reaching a thousand times the line's length past
/// its ends and away from it.
fn side_of(a: Point, b: Point, toward: Point) -> PathData {
    let along = b - a;
    let mut across = Vec2::new(-along.y, along.x);
    if across.dot(toward - a) < 0.0 {
        across = -across;
    }
    let (far, wide) = (across * 1000.0, along * 1000.0);
    let mut out = BezPath::new();
    out.move_to(a - wide);
    out.line_to(b + wide);
    out.line_to(b + wide + far);
    out.line_to(a - wide + far);
    out.close_path();
    PathData::from_bezpath(&out)
}

/// Triangle `t` scaled by `factor` about its centroid, wound as it was.
fn grown(t: [Point; 3], factor: f64) -> [Point; 3] {
    let c = Point::new((t[0].x + t[1].x + t[2].x) / 3.0, (t[0].y + t[1].y + t[2].y) / 3.0);
    t.map(|p| c + (p - c) * factor)
}

/// The affine map that takes the corners `from` of a triangle onto `to`. `None` for a triangle
/// with no area or a map that is not finite.
fn triangle_map(from: &[Point], to: &[Point]) -> Option<Affine> {
    let ([u0, u1, u2], [p0, p1, p2]) = (<[Point; 3]>::try_from(from).ok()?, <[Point; 3]>::try_from(to).ok()?);
    let (a, b) = (u1 - u0, u2 - u0);
    let det = a.x * b.y - b.x * a.y;
    if det.abs() <= 1e-12 * (a.hypot2() + b.hypot2()).max(1e-300) {
        return None;
    }
    let (c, d) = (p1 - p0, p2 - p0);
    let m = [(c.x * b.y - d.x * a.y) / det, (c.y * b.y - d.y * a.y) / det, (d.x * a.x - c.x * b.x) / det, (d.y * a.x - c.y * b.x) / det];
    let xf = Affine::new([m[0], m[1], m[2], m[3], p0.x - m[0] * u0.x - m[2] * u0.y, p0.y - m[1] * u0.x - m[3] * u0.y]);
    xf.as_coeffs().iter().all(|v| v.is_finite()).then_some(xf)
}

/// Triangle `t` wound counterclockwise (y up), as [`clip_to`] reads its window. `None` without
/// three corners.
fn counterclockwise(t: &[Point]) -> Option<[Point; 3]> {
    let [a, b, c] = <[Point; 3]>::try_from(t).ok()?;
    Some(if (b - a).cross(c - a) < 0.0 { [a, c, b] } else { [a, b, c] })
}

/// The part of `polygon` inside the convex `window` (one step of Sutherland and Hodgman's clip for
/// each of its edges). A polygon that is not convex keeps its area, with edges along the window's
/// sides where it leaves and comes back.
fn clip_to(polygon: &[Point], window: &[Point; 3]) -> Vec<Point> {
    let mut out = polygon.to_vec();
    for k in 0..3 {
        let (a, b) = (window[k], window[(k + 1) % 3]);
        let side = |p: Point| (b - a).cross(p - a);
        let input = std::mem::take(&mut out);
        for (i, &p) in input.iter().enumerate() {
            let q = input[(i + 1) % input.len()];
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

/// The bounds of the art symbol `name` of `doc` draws as a graphic, in the symbol's coordinates.
pub(super) fn art_bounds(doc: &Document, name: &str) -> Option<Rect> {
    let symbol = doc.symbols.iter().find(|s| s.name == name)?;
    pieces_of(doc, symbol).ok()?.iter().filter_map(|p| p.path.bounds()).reduce(|a, b| a.union(b))
}

/// Where a 3D object's graphics are drawn: the bounds its solids are built with (the graphics'
/// offsets are from their center) and the maps of its front and back faces, when it has them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GraphicFrame {
    pub bounds: Rect,
    front: Option<FaceMap>,
    back: Option<FaceMap>,
}

impl GraphicFrame {
    pub(super) fn new(bounds: Rect, front: Option<FaceMap>, back: Option<FaceMap>) -> Self {
        Self { bounds, front, back }
    }

    pub fn map(&self, face: PlanarFace) -> Option<&FaceMap> {
        match face {
            PlanarFace::Front => self.front.as_ref(),
            PlanarFace::Back => self.back.as_ref(),
        }
    }

    /// The point of `face`'s plane that offset `(x, y)` names, as [`MappedGraphic`] places a
    /// graphic's center.
    fn art_point(&self, face: PlanarFace, (x, y): (f64, f64)) -> Point {
        let c = self.bounds.center();
        match face {
            PlanarFace::Front => Point::new(c.x + x, c.y + y),
            PlanarFace::Back => Point::new(c.x - x, c.y + y),
        }
    }

    /// Where graphic `g`'s center lands on the page, and whether its face is turned toward the
    /// viewer. `None` when the object has no such face or the point has no page point.
    pub fn handle(&self, g: &MappedGraphic) -> Option<(Point, bool)> {
        let map = self.map(g.face)?;
        Some((map.to_page(self.art_point(g.face, (g.x, g.y)))?, map.visible))
    }

    /// The offset of a graphic on `face` whose center lands on page point `page`.
    pub fn offset_at(&self, face: PlanarFace, page: Point) -> Option<(f64, f64)> {
        let art = self.map(face)?.to_art(page)?;
        let c = self.bounds.center();
        let (x, y) = match face {
            PlanarFace::Front => (art.x - c.x, art.y - c.y),
            PlanarFace::Back => (c.x - art.x, art.y - c.y),
        };
        (x.abs() <= MAX_OFFSET && y.abs() <= MAX_OFFSET).then_some((x, y))
    }

    /// The scale (percent) at which art with bounds `art` fits inside the object's bounds, at most
    /// 100: where a newly mapped graphic starts.
    pub fn fitting_scale(&self, art: Rect) -> f64 {
        let fit = (self.bounds.width() / art.width().max(1e-9)).min(self.bounds.height() / art.height().max(1e-9));
        (fit * 100.0).clamp(SCALE_RANGE.0, 100.0)
    }
}

/// The symbols the graphics of `n`'s 3D effects name (its own effects and its fills' and strokes').
pub fn graphic_symbols(n: &Node) -> Vec<String> {
    appearance_graphic_symbols(&n.appearance)
}

/// The symbols the graphics of the 3D effects in `appearance` name, as [`graphic_symbols`]: for a
/// graphic style's appearance.
pub fn appearance_graphic_symbols(appearance: &Appearance) -> Vec<String> {
    std::iter::once(&appearance.effects)
        .chain(appearance.items.iter().map(AppearanceItem::effects))
        .flatten()
        .filter(|e| takes_graphics(&e.id))
        .flat_map(|e| mapped_graphics(&e.params))
        .map(|g| g.symbol)
        .collect()
}

/// Makes the graphics of `n`'s 3D effects that name symbol `from` name `to` instead. Returns
/// whether any changed.
pub fn rename_graphic_symbol(n: &mut Node, from: &str, to: &str) -> bool {
    // Checked first, so an appearance that names another symbol is not copied.
    graphic_symbols(n).iter().any(|s| s == from) && rename_appearance_graphic_symbol(&mut n.appearance, from, to)
}

/// [`rename_graphic_symbol`] for the 3D effects in `appearance`: for a graphic style's appearance.
pub fn rename_appearance_graphic_symbol(appearance: &mut Appearance, from: &str, to: &str) -> bool {
    if !appearance_graphic_symbols(appearance).iter().any(|s| s == from) {
        return false;
    }
    let mut lists: Vec<&mut Vec<vectorcraft_doc::Effect>> = vec![];
    let (effects, items) = (&mut appearance.effects, &mut appearance.items);
    lists.push(effects);
    lists.extend(items.iter_mut().map(AppearanceItem::effects_mut));
    for e in lists.into_iter().flatten().filter(|e| takes_graphics(&e.id)) {
        if let Some(Value::Array(list)) = e.params.get_mut(GRAPHICS) {
            for g in list.iter_mut() {
                if g.get("symbol").and_then(Value::as_str) == Some(from) {
                    g["symbol"] = json!(to);
                }
            }
        }
    }
    true
}

/// Does `n` or any object in it, opacity masks included, have a graphic that names symbol `from`?
pub fn names_graphic_symbol(n: &Node, from: &str) -> bool {
    let mut found = false;
    vectorcraft_doc::swatches::walk_with_masks(n, &mut |c| found |= graphic_symbols(c).iter().any(|s| s == from));
    found
}

/// [`rename_graphic_symbol`] for `n` and every object in it, at any depth, opacity masks included.
/// Only the parts that name `from` are copied. Returns whether any changed.
pub fn rename_graphic_symbol_in(n: &mut Node, from: &str, to: &str) -> bool {
    let names = names_graphic_symbol;
    let mut changed = rename_graphic_symbol(n, from, to);
    if let Some(mask) = n.extras_if_any().and_then(|x| x.mask.as_deref_mut()).filter(|m| names(&m.art, from)) {
        changed |= rename_graphic_symbol_in(Arc::make_mut(&mut mask.art), from, to);
    }
    if let Some(children) = n.children_mut() {
        for c in children.iter_mut().filter(|c| names(c, from)) {
            changed |= rename_graphic_symbol_in(Arc::make_mut(c), from, to);
        }
    }
    changed
}

/// `p` with graphic `g` added at the end of its list.
pub fn with_graphic(p: &Value, g: &MappedGraphic) -> Value {
    let mut out = p.as_object().cloned().unwrap_or_else(Map::new);
    let mut list = match out.get(GRAPHICS) {
        Some(Value::Array(list)) => list.clone(),
        _ => vec![],
    };
    list.push(g.to_value());
    out.insert(GRAPHICS.into(), Value::Array(list));
    Value::Object(out)
}

#[cfg(test)]
#[path = "tests_graphics.rs"]
mod tests;
