//! Add Bounding Rectangle and Add Bounding Circle: a live rectangle on the selection's geometric
//! bounding box, or the smallest live circle around its outlines. The circle is the geometry crate's
//! ([`enclosing_circle`](vectorcraft_geom::shapes::enclosing_circle)).

use serde_json::{Value, json};
use vectorcraft_doc::{Document, LiveShape, Node, NodeId, NodeKind, SymbolBoxes};
use vectorcraft_geom::kurbo::ParamCurveExtrema;
use vectorcraft_geom::{Affine, CubicBez, FillRule, ParamCurve, PathSeg, Point, Rect, shapes};

use super::super::super::edit::{no_symbol_part, selected_roots_to_transform};
use super::super::super::{CommandSpec, bad, cmd, has_selection, number_or, switch_or};
use crate::sanity::rect_in_range;
use crate::{Result, Session};

#[cfg(test)]
mod tests;

/// Add Bounding Rectangle's command.
pub(crate) const RECTANGLE: &str = "addon.pathtools.boundingRectangle";
/// Add Bounding Circle's command.
pub(crate) const CIRCLE: &str = "addon.pathtools.boundingCircle";

/// How far a curve may stray from the points the circle is fitted to, in points. The circle's
/// radius grows by as much, so it holds the curves themselves.
const FLATNESS: f64 = 0.01;

pub(super) fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "addon.pathtools.boundingRectangle",
            "Add Bounding Rectangle",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{each?: bool (default false: one rectangle around the whole selection; true: one around each selected object), offset?: pt (0 or more; default 0: added on every side)} draw a live rectangle on the geometric bounding box of the selected objects (strokes and effects left out; a path of a compound path selected on its own counts as an object), with the current paint, on top of the current layer or of the isolated group, as other new art; the rectangles are selected; one undo step. An object whose box has no width or no height (a straight horizontal or vertical line) gets no rectangle unless `offset` is more than 0, nor does an object with no bounds (an empty group); with `each` the result's `skipped` counts them. It is an error when no rectangle is left (\"the shape would have no size\" when a rectangle with no size was left out, else \"nothing to enclose\") and when an edge of a rectangle would lie more than 4,000,000 pt from the origin along either axis (\"the shape would reach past the edge of the canvas\"). The menu item Add Bounding Rectangle… opens a dialog with Offset and One per Object (`each`) and a Preview → {id: the first rectangle, ids, skipped?}",
            whole_objects,
            bounding_rectangle
        ),
        cmd!(
            "addon.pathtools.boundingCircle",
            "Add Bounding Circle",
            ["Object", "Plug-ins", "Path Precision Tools"],
            None,
            "{each?: bool (default false: one circle around the whole selection; true: one around each selected object), offset?: pt (0 or more; default 0: added to the radius)} draw the smallest live circle that holds the outlines of the selected objects (paths and compound paths by their curves, clip groups and sublayers that clip their contents by their clipping paths, other sublayers by their shown objects, images by their corners, type and other objects by the corners of their bounding box; strokes and effects left out; a path of a compound path selected on its own counts as an object), with the radius 0.01 pt larger so it holds the curves themselves, with the current paint, on top of the current layer or of the isolated group, as other new art; the circles are selected; one undo step. An object at a single point gets no circle unless `offset` is more than 0, nor does an object with no outline (an empty group); with `each` the result's `skipped` counts them. It is an error when no circle is left (\"the shape would have no size\" when a circle with no size was left out, else \"nothing to enclose\") and when an edge of a circle's bounding box would lie more than 4,000,000 pt from the origin along either axis (\"the shape would reach past the edge of the canvas\"). The menu item Add Bounding Circle… opens a dialog with Offset and One per Object (`each`) and a Preview → {id: the first circle, ids, skipped?}",
            whole_objects,
            bounding_circle
        ),
    ]
}

/// Enabled with a selection that is not an object of a symbol instance's art, which
/// [`selected_roots_to_transform`] refuses.
pub(super) fn whole_objects(s: &Session) -> std::result::Result<(), String> {
    has_selection(s)?;
    no_symbol_part(s)
}

/// The shape a command draws.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Rectangle,
    Circle,
}

fn bounding_rectangle(s: &mut Session, p: &Value) -> Result<Value> {
    add(s, p, Kind::Rectangle)
}

fn bounding_circle(s: &mut Session, p: &Value) -> Result<Value> {
    add(s, p, Kind::Circle)
}

fn add(s: &mut Session, p: &Value, kind: Kind) -> Result<Value> {
    let (c, label) = match kind {
        Kind::Rectangle => (RECTANGLE, "Add Bounding Rectangle"),
        Kind::Circle => (CIRCLE, "Add Bounding Circle"),
    };
    let each = switch_or(p, "each", false, c)?;
    let offset = number_or(p, "offset", 0.0, c)?;
    if !(offset.is_finite() && offset >= 0.0) {
        return Err(bad(c, "`offset` must be 0 or more"));
    }
    // The objects `object.transform` moves: a path of a compound path selected on its own is one.
    let roots = selected_roots_to_transform(s)?;
    let d = &s.doc()?.doc;
    let symbols = d.symbol_boxes();
    let groups: Vec<Vec<NodeId>> = if each { roots.iter().map(|id| vec![*id]).collect() } else { vec![roots] };
    let mut made: Vec<LiveShape> = vec![];
    // Objects that get no shape: no outline, or a shape with no size.
    let mut skipped = 0;
    let mut no_size = false;
    // The shapes share the halvings, so the work stays bounded however many objects are selected.
    let splits = split_share(groups.len());
    for ids in &groups {
        match shape_around(d, &symbols, ids, kind, offset, splits) {
            Around::Shape(live) => made.push(live),
            Around::NoOutline => skipped += 1,
            Around::NoSize => {
                skipped += 1;
                no_size = true;
            }
            Around::PastCanvas => return Err(bad(c, "the shape would reach past the edge of the canvas")),
        }
    }
    if made.is_empty() {
        return Err(bad(c, if no_size { "the shape would have no size" } else { "nothing to enclose" }));
    }
    let parent = s.doc()?.target_parent()?;
    let look = s.new_art_look(s.paint.fill.clone(), s.paint.stroke.clone(), s.paint.stroke_width);
    let ids = s.edit(label, |d, sel| {
        let mut ids = vec![];
        for live in made {
            let kind = NodeKind::Path { path: live.to_path(), rule: FillRule::NonZero, live: Some(Box::new(live)), clipping: false, guide: false };
            let mut n = Node::new(d.alloc_id(), kind);
            look.apply(d, &mut n);
            ids.push(n.id);
            d.insert(parent, usize::MAX, n)?;
        }
        sel.set(ids.iter().copied());
        Ok(ids)
    })?;
    let first = ids.first().map_or(0, |id| id.0);
    let mut out = json!({ "id": first, "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() });
    if skipped > 0 {
        out["skipped"] = json!(skipped);
    }
    Ok(out)
}

/// What [`shape_around`] finds.
enum Around {
    Shape(LiveShape),
    /// The objects have no bounds or no outline.
    NoOutline,
    /// The rectangle would have no width or no height, or the circle no radius.
    NoSize,
    /// An edge of the shape would lie more than [`MAX_COORD`](crate::MAX_COORD) from the origin
    /// along either axis.
    PastCanvas,
}

/// The live rectangle or circle around `ids`, grown by `offset`. A circle halves its curves as
/// `splits` allows.
fn shape_around(d: &Document, symbols: &SymbolBoxes, ids: &[NodeId], kind: Kind, offset: f64, splits: Splits) -> Around {
    match kind {
        Kind::Rectangle => {
            let Some(r) = d.bounds_of_with(ids, false, symbols) else { return Around::NoOutline };
            let r = r.inflate(offset, offset);
            if !rect_in_range(r) {
                return Around::PastCanvas;
            }
            if r.width() <= 0.0 || r.height() <= 0.0 {
                return Around::NoSize;
            }
            Around::Shape(LiveShape::Rectangle {
                w: r.width(),
                h: r.height(),
                radii: [0.0; 4],
                kinds: Default::default(),
                radii_relative: false,
                xf: Affine::translate(r.origin().to_vec2()),
            })
        }
        Kind::Circle => {
            let mut gathered = Gathered::new();
            gathered.splits = splits;
            for n in ids.iter().filter_map(|id| d.node(*id)) {
                outline_points(n, symbols, &mut gathered, 0);
                gathered.compact();
            }
            let Some((center, radius)) = shapes::enclosing_circle(&gathered.points) else { return Around::NoOutline };
            if radius + offset <= 0.0 {
                return Around::NoSize;
            }
            let r = radius + FLATNESS + offset;
            let b = Rect::new(center.x - r, center.y - r, center.x + r, center.y + r);
            if !rect_in_range(b) {
                return Around::PastCanvas;
            }
            Around::Shape(LiveShape::Ellipse { w: b.width(), h: b.height(), pie: (0.0, 360.0), xf: Affine::translate(b.origin().to_vec2()) })
        }
    }
}

/// Adds to `out` points on the outline of `n`, as its geometric bounds measure it: the points of
/// its paths ([`curve_points`]); the clipping path of a clip group or of a sublayer that clips its
/// contents; the shown contents of a sublayer (a layer that an opened file put inside a group); the
/// corners of an image; and the corners of the geometric bounding box of anything else. A group or
/// layer `depth` levels below the selected object that is nested deeper than a document holds
/// ([`vectorcraft_doc::MAX_DEPTH`]) counts by the corners of its geometric bounding box.
fn outline_points(n: &Node, symbols: &SymbolBoxes, out: &mut Gathered, depth: usize) {
    let nests = matches!(n.kind, NodeKind::Group { .. } | NodeKind::Layer { .. } | NodeKind::Compound { .. });
    if nests && depth >= vectorcraft_doc::MAX_DEPTH {
        out.points.extend(n.geometric_bounds_with(symbols).map(corners).into_iter().flatten());
        return;
    }
    match &n.kind {
        NodeKind::Path { path, .. } => {
            // The anchors count too, so a path of one anchor, which has no segments, is held.
            for p in path.subpaths.iter().flat_map(|sp| sp.anchors.iter().map(|a| a.p)) {
                out.points.push(p);
                out.compact();
            }
            for seg in path.to_bezpath().segments() {
                match seg {
                    PathSeg::Line(l) => out.points.extend([l.p0, l.p1]),
                    _ => curve_points(seg.to_cubic(), 0, &mut out.points, &mut out.splits),
                }
                out.compact();
            }
        }
        NodeKind::Group { children, clip: true } | NodeKind::Layer { children, clip: true, .. } => {
            if let Some(c) = children.first() {
                outline_points(c, symbols, out, depth + 1);
            }
        }
        NodeKind::Layer { .. } | NodeKind::Group { .. } | NodeKind::Compound { .. } => {
            let layer = matches!(n.kind, NodeKind::Layer { .. });
            for c in n.editing_children().iter().filter(|c| c.visible || !layer) {
                outline_points(c, symbols, out, depth + 1);
                out.compact();
            }
        }
        NodeKind::Image(im) => {
            let r = Rect::new(0.0, 0.0, im.width as f64, im.height as f64);
            out.points.extend(corners(r).map(|p| im.xf * p));
        }
        _ => out.points.extend(n.geometric_bounds_with(symbols).map(corners).into_iter().flatten()),
    }
}

/// Adds to `out` points whose convex hull, grown by [`FLATNESS`], holds the curve `c`: the ends of
/// pieces whose control points lie within [`FLATNESS`] of their chord, and for pieces still curved
/// after [`MAX_SPLITS`] halvings, or when `splits` allows no more ([`Splits::take`]), the corners of
/// [`held`]. A flat piece that ends where it starts adds its control points too, so a small loop is
/// not taken for a single point. A curve with a point that is not finite adds nothing.
fn curve_points(c: CubicBez, depth: u32, out: &mut Vec<Point>, splits: &mut Splits) {
    if ![c.p0, c.p1, c.p2, c.p3].iter().all(|p| p.is_finite()) {
        return;
    }
    let flat = [c.p1, c.p2].iter().all(|p| distance_to_segment(*p, c.p0, c.p3) <= FLATNESS);
    if flat {
        out.extend([c.p0, c.p3]);
        if c.p0 == c.p3 {
            out.extend([c.p1, c.p2]);
        }
    } else if depth >= MAX_SPLITS || !splits.take(depth) {
        out.extend(held(c));
    } else {
        let (a, b) = c.subdivide();
        curve_points(a, depth + 1, out, splits);
        curve_points(b, depth + 1, out, splits);
    }
}

/// The corners of the convex hull of `c`'s control points cut down to `c`'s bounding box. Each of
/// the two holds the curve, so the part they share holds it too. When the cut leaves nothing, which
/// rounding can cause, it gives the box's corners.
fn held(c: CubicBez) -> Vec<Point> {
    let b = ParamCurveExtrema::bounding_box(&c);
    let mut poly = shapes::convex_hull(vec![c.p0, c.p1, c.p2, c.p3]);
    for (x, bound, below) in [(true, b.x0, false), (true, b.x1, true), (false, b.y0, false), (false, b.y1, true)] {
        poly = clip(&poly, x, bound, below);
    }
    if poly.is_empty() { corners(b).to_vec() } else { poly }
}

/// The convex polygon `poly` cut at `bound` on the x axis (`x`) or the y axis: the part at or below
/// `bound` when `below`, else at or above it.
fn clip(poly: &[Point], x: bool, bound: f64, below: bool) -> Vec<Point> {
    let v = |p: Point| if x { p.x } else { p.y };
    let inside = |p: Point| if below { v(p) <= bound } else { v(p) >= bound };
    let mut out = vec![];
    for (&a, &b) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        if inside(a) {
            out.push(a);
        }
        if inside(a) != inside(b) {
            let mut p = a.lerp(b, (bound - v(a)) / (v(b) - v(a)));
            if x {
                p.x = bound;
            } else {
                p.y = bound;
            }
            out.push(p);
        }
    }
    out
}

/// How many halvings [`curve_points`] makes for one command, over every curve it measures, before
/// it halves each further curve [`MIN_SPLITS`] times only; with `each`, every shape gets an equal
/// share ([`split_share`]).
const TOTAL_SPLITS: usize = 1 << 20;

/// How many more halvings [`curve_points`] makes for one command once [`TOTAL_SPLITS`] is used up,
/// each within the first [`MIN_SPLITS`] halvings of a curve; with `each`, shared as
/// [`TOTAL_SPLITS`] is. After that, curves count by [`held`] whole.
const EARLY_SPLITS: usize = 7 << 20;

/// How many times [`curve_points`] halves a curve once [`TOTAL_SPLITS`] is used up: 8 pieces, which
/// keeps the circle fitted to a circle of four curves less than 0.3% larger than that circle.
const MIN_SPLITS: u32 = 3;

/// The halvings [`curve_points`] may still make.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Splits {
    /// Halvings at any depth.
    any: usize,
    /// Halvings within the first [`MIN_SPLITS`] of a curve, once `any` is used up.
    early: usize,
}

impl Splits {
    /// Takes one halving of a piece `depth` halvings into its curve; false when none is left.
    fn take(&mut self, depth: u32) -> bool {
        if self.any > 0 {
            self.any -= 1;
        } else if depth < MIN_SPLITS && self.early > 0 {
            self.early -= 1;
        } else {
            return false;
        }
        true
    }
}

/// The halvings each of `shapes` shapes may make: an equal share of [`TOTAL_SPLITS`] and of
/// [`EARLY_SPLITS`].
fn split_share(shapes: usize) -> Splits {
    let n = shapes.max(1);
    Splits { any: TOTAL_SPLITS / n, early: EARLY_SPLITS / n }
}

/// How many points [`outline_points`] gathers before it keeps only the corners of their convex hull.
const COMPACT_AT: usize = 1 << 16;

/// Points gathered for a circle, kept down to the corners of their convex hull whenever they grow
/// past `limit`: only the hull decides the circle. The points held stay within about twice the
/// hull's corners, and grow only with the anchors and objects measured and the halvings
/// ([`TOTAL_SPLITS`] and [`EARLY_SPLITS`]).
struct Gathered {
    points: Vec<Point>,
    limit: usize,
    /// The halvings [`curve_points`] may still make (one shape's whole share at first).
    splits: Splits,
}

impl Gathered {
    fn new() -> Self {
        Self { points: vec![], limit: COMPACT_AT, splits: split_share(1) }
    }

    /// Keeps only the hull's corners once there are more than `limit` points; the next time waits
    /// until there are twice as many as the hull has, so each point is sorted a few times at most.
    fn compact(&mut self) {
        if self.points.len() > self.limit {
            self.points = shapes::convex_hull(std::mem::take(&mut self.points));
            self.limit = (self.points.len() * 2).max(COMPACT_AT);
        }
    }
}

/// How many times [`curve_points`] halves a curve at most: up to 1,024 pieces.
const MAX_SPLITS: u32 = 10;

/// The distance from `p` to the segment from `a` to `b`.
fn distance_to_segment(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).hypot()
}

fn corners(r: Rect) -> [Point; 4] {
    [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]
}
