//! Shape Builder (Shift+M), Live Paint Bucket (K) and Live Paint Selection (Shift+L).
//!
//! The tools only emit commands (`shapeBuilder.merge`, `livePaint.fill`, `livePaint.strokeEdge`,
//! `select.*`); the planar arrangement comes from `vectorcraft_pathops::shape_builder` (Shape
//! Builder) and `vectorcraft_pathops::live_paint` (Live Paint).
//!
//! **Live Paint groups** are ordinary groups (no special node kind): the group's name starts with
//! [`LIVE_PAINT_PREFIX`]; its first child is a hidden group named [`SOURCES_NAME`] holding the
//! original paths (the planar map's inputs, kept for Release / Merge / re-computation), followed
//! by the face paths (named [`FACE_NAME`], fill only) and the edge paths (named [`EDGE_NAME`],
//! stroke only, open). Everything else in the document sees a plain group of paths, so rendering,
//! export, transforms and undo need no special cases.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use serde_json::{Value, json};
use vectorcraft_color::{Paint, Swatch};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Shape as _, SubPath};
use vectorcraft_pathops as po;

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, json_ids};

/// Names starting with this mark a Live Paint group, together with its sources group ([`SOURCES_NAME`]).
pub const LIVE_PAINT_PREFIX: &str = "Live Paint";
/// Default name of a new Live Paint group.
pub const LIVE_PAINT_NAME: &str = "Live Paint Group";
/// The hidden child group holding the source paths.
pub const SOURCES_NAME: &str = "Live Paint Sources";
pub const FACE_NAME: &str = "Face";
pub const EDGE_NAME: &str = "Edge";

/// Shape Builder hover hatch colour (neutral grey) and Live Paint highlight red.
pub const HATCH: [u8; 3] = [0x80, 0x80, 0x80];
pub const HIGHLIGHT_RED: [u8; 3] = [0xff, 0x2a, 0x2a];

/// Is `n` a Live Paint group: an unclipped group whose name starts with [`LIVE_PAINT_PREFIX`] and
/// that holds a group named [`SOURCES_NAME`]? A plain group with such a name is not one.
pub fn is_live_paint(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. })
        && n.name.as_deref().is_some_and(|s| s.starts_with(LIVE_PAINT_PREFIX))
        && sources(n).is_some_and(|s| matches!(s.kind, NodeKind::Group { .. }))
}

/// Whether Live Paint can take `n` as it is: a Live Paint group, a path or compound path, or a
/// group of those. Type, images, symbols, blends, meshes and the rest have to be converted to
/// paths first (<https://helpx.adobe.com/illustrator/desktop/paint-and-fill/learn-painting-basics/create-live-paint-groups.html>);
/// Make and Merge refuse them rather than hide them among the group's sources.
pub fn live_paint_takes(n: &Node) -> bool {
    is_live_paint(n)
        || match &n.kind {
            NodeKind::Path { .. } | NodeKind::Compound { .. } => true,
            NodeKind::Group { children, .. } => children.iter().all(|c| live_paint_takes(c)),
            _ => false,
        }
}

fn named(n: &Node, name: &str) -> bool {
    n.name.as_deref() == Some(name)
}

/// Face children of a Live Paint group (paint order).
pub fn faces(group: &Node) -> Vec<&Arc<Node>> {
    group.children().map(|c| c.iter().filter(|n| named(n, FACE_NAME)).collect()).unwrap_or_default()
}

/// Edge children of a Live Paint group.
pub fn edges(group: &Node) -> Vec<&Arc<Node>> {
    group.children().map(|c| c.iter().filter(|n| named(n, EDGE_NAME)).collect()).unwrap_or_default()
}

/// The hidden sources group of a Live Paint group.
pub fn sources(group: &Node) -> Option<&Arc<Node>> {
    group.children()?.iter().find(|n| named(n, SOURCES_NAME))
}

/// Does the path (filled) contain `p`?
pub fn path_contains(path: &PathData, rule: FillRule, p: Point) -> bool {
    if path.bounds().is_none_or(|b| !b.inflate(1e-6, 1e-6).contains(p)) {
        return false;
    }
    let w = path.to_bezpath().winding(p);
    match rule {
        FillRule::EvenOdd => w % 2 != 0,
        _ => w != 0,
    }
}

/// The face of `group` under `p` (front-most).
pub fn face_at(group: &Node, p: Point) -> Option<&Arc<Node>> {
    front_face(faces(group).into_iter(), p)
}

/// The face of `group` under `p` (front-most) among its faces that are not hidden.
pub fn visible_face_at(group: &Node, p: Point) -> Option<&Arc<Node>> {
    front_face(faces(group).into_iter().filter(|f| f.visible), p)
}

/// The face of `faces` (paint order) under `p` (front-most).
fn front_face<'a>(faces: impl DoubleEndedIterator<Item = &'a Arc<Node>>, p: Point) -> Option<&'a Arc<Node>> {
    faces.rev().find(|f| f.path_data().is_some_and(|pd| path_contains(pd, FillRule::NonZero, p)))
}

/// The edge of `group` nearest to `p` within `tol`.
pub fn edge_near(group: &Node, p: Point, tol: f64) -> Option<&Arc<Node>> {
    nearest_edge(edges(group), p, tol)
}

/// The edge of `group` nearest to `p` within `tol` among its edges that are not hidden.
pub fn visible_edge_near(group: &Node, p: Point, tol: f64) -> Option<&Arc<Node>> {
    nearest_edge(edges(group).into_iter().filter(|e| e.visible), p, tol)
}

/// The edge of `edges` nearest to `p` within `tol`.
fn nearest_edge<'a>(edges: impl IntoIterator<Item = &'a Arc<Node>>, p: Point, tol: f64) -> Option<&'a Arc<Node>> {
    let mut best: Option<(f64, &Arc<Node>)> = None;
    for e in edges {
        if let Some((.., d)) = e.path_data().and_then(|pd| pd.nearest(p))
            && d <= tol
            && best.is_none_or(|b| d < b.0)
        {
            best = Some((d, e));
        }
    }
    best.map(|b| b.1)
}

/// The visible Live Paint group with a face under `p` that is not hidden (front-most in paint order).
pub fn live_paint_at(doc: &Document, p: Point) -> Option<NodeId> {
    let mut hit = None;
    doc.walk(|n| {
        if is_live_paint(n) && doc.is_visible(n.id) && visible_face_at(n, p).is_some() {
            hit = Some(n.id);
        }
    });
    hit
}

/// Paths, compound paths (as one shape) of a subtree in paint order; guides, clipping paths and
/// hidden objects are skipped. Live Paint groups contribute their faces and edges.
pub fn leaf_shapes<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    collect_leaves(n, false, out);
}

/// [`leaf_shapes`]; with `filled_faces`, a Live Paint group contributes only its filled faces and
/// its stroked edges.
fn collect_leaves<'a>(n: &'a Node, filled_faces: bool, out: &mut Vec<&'a Node>) {
    if !n.visible {
        return;
    }
    if n.extras().shaper.is_some() {
        for c in n.children().into_iter().flatten().skip(1) {
            collect_leaves(c, filled_faces, out);
        }
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } => out.push(n),
        NodeKind::Group { .. } if filled_faces && is_live_paint(n) => {
            out.extend(faces(n).into_iter().filter(|f| f.visible && !f.appearance.fill_paint().is_none()).map(|f| &**f));
            out.extend(edges(n).into_iter().filter(|e| e.visible && !e.appearance.stroke_paint().is_none()).map(|e| &**e));
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                collect_leaves(c, filled_faces, out);
            }
        }
        _ => {}
    }
}

/// Filled outline of a path or compound path node.
pub fn node_outline(n: &Node) -> Option<(PathData, FillRule)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule)),
        NodeKind::Compound { children, rule } => {
            let subs = children.iter().filter_map(|c| c.path_data()).flat_map(|p| p.subpaths.iter().cloned()).collect();
            Some((PathData::new(subs), *rule))
        }
        _ => None,
    }
}

/// How far (pt) a piece of the planar map may lie from `outline`, the outline it follows, to be
/// matched to it or cut out of it: a ten-thousandth of the outline's larger side, at least 0.05.
/// The map's pieces follow the outlines to within a few hundredths of a point.
pub fn outline_snap(outline: &PathData) -> f64 {
    outline.bounds().map_or(0.05, |b| (b.width().max(b.height()) / 2000.0).max(0.25) * 0.2)
}

/// The outlines of `leaves` as they are drawn ([`node_outline`]), in their order, for
/// [`BuilderMap::stroke_at`]. A leaf without an outline, and one inside a Live Paint group or a
/// Shaper Group (which make their own paths), gets an empty outline, so no stroke of it splits.
pub fn drawn_outlines(doc: &Document, leaves: &[&Node]) -> Vec<PathData> {
    let in_live_group = |id: NodeId| {
        let mut up = doc.parent_of(id);
        while let Some(p) = up {
            if doc.node(p).is_some_and(|n| is_live_paint(n) || n.extras().shaper.is_some()) {
                return true;
            }
            up = doc.parent_of(p);
        }
        false
    };
    leaves.iter().map(|l| if in_live_group(l.id) { None } else { node_outline(l) }.map_or_else(PathData::default, |o| o.0)).collect()
}

/// Pathfinder shapes for the given roots (back → front by the order given), keyed by leaf index,
/// with the leaves they came from.
pub fn shapes_for<'a>(doc: &'a Document, roots: &[NodeId]) -> (Vec<po::Shape>, Vec<&'a Node>) {
    shapes_of(doc, roots, false)
}

/// [`shapes_for`]; with `filled_faces`, a Live Paint group contributes only its filled faces and
/// its stroked edges.
fn shapes_of<'a>(doc: &'a Document, roots: &[NodeId], filled_faces: bool) -> (Vec<po::Shape>, Vec<&'a Node>) {
    let mut lv = vec![];
    for id in roots {
        if let Some(n) = doc.node(*id) {
            collect_leaves(n, filled_faces, &mut lv);
        }
    }
    let mut shapes = vec![];
    let mut used = vec![];
    for l in lv {
        if let Some((p, r)) = node_outline(l)
            && !p.is_empty()
            && p.subpaths.iter().all(|s| s.anchors.iter().all(|a| a.p.x.is_finite() && a.p.y.is_finite()))
        {
            shapes.push(po::Shape::new(p, r, used.len() as u64));
            used.push(l);
        }
    }
    (shapes, used)
}

/// Shape Builder inputs: [`shapes_for`], with the open subpaths of filled paths closed where that
/// encloses an area (the fill bounds a region, as if an invisible edge closed it) when
/// `open_filled_as_closed`. Unfilled open paths and straight lines stay open and cut the regions
/// they cross, and so do filled open paths without `open_filled_as_closed`. A Live Paint group
/// contributes its filled faces as shapes and its stroked edges as open lines; its unfilled faces
/// and its edges without a stroke are left out.
fn shape_builder_shapes<'a>(doc: &'a Document, roots: &[NodeId], open_filled_as_closed: bool) -> (Vec<po::Shape>, Vec<&'a Node>) {
    let (mut shapes, leaves) = shapes_of(doc, roots, true);
    for (s, l) in shapes.iter_mut().zip(&leaves) {
        if named(l, EDGE_NAME) && doc.parent_of(l.id).and_then(|p| doc.node(p)).is_some_and(is_live_paint) {
            // A closed edge opens at its first anchor and keeps its closing segment.
            let start = BTreeSet::from([0]);
            s.path.subpaths = s.path.subpaths.iter().flat_map(|sp| sp.cut_at(&start)).collect();
            continue;
        }
        if !open_filled_as_closed || l.appearance.fill_paint().is_none() {
            continue;
        }
        for sp in s.path.subpaths.iter_mut().filter(|sp| !sp.closed) {
            sp.closed = po::encloses_area(sp);
        }
    }
    (shapes, leaves)
}

/// A planar-map face with its outline and bounds, ready for hit tests.
pub type Face = (po::Region, BezPath, Rect);

fn faces_of(regions: Vec<po::Region>) -> Vec<Face> {
    regions
        .into_iter()
        .filter_map(|r| {
            let bp = r.path.to_bezpath();
            let b = r.path.bounds()?;
            Some((r, bp, b))
        })
        .collect()
}

/// The Shape Builder's planar map of a selection.
#[derive(Default)]
pub struct BuilderMap {
    /// The inputs, back → front (see [`po::shape_builder`]).
    pub shapes: Vec<po::Shape>,
    pub regions: Vec<Face>,
    /// Pieces of the open paths that cut regions (see [`BuilderMap::new`]), keyed by input index,
    /// with their bounds.
    pub lines: Vec<(po::Shape, Rect)>,
    /// Pieces of the other inputs' outlines (cut wherever paths meet), keyed by input index, with
    /// their bounds: the edges erasing deletes. Only in a map made `with_edges`.
    pub edges: Vec<(po::Shape, Rect)>,
}

/// What a Shape Builder gesture touches at a point.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    Region(usize),
    Line(usize),
    Edge(usize),
}

/// The piece of `pieces` nearest `p` within `tol`, with its distance.
fn piece_near(pieces: &[(po::Shape, Rect)], p: Point, tol: f64) -> Option<(f64, usize)> {
    let mut best: Option<(f64, usize)> = None;
    for (i, (l, b)) in pieces.iter().enumerate() {
        if b.inflate(tol, tol).contains(p)
            && let Some((.., d)) = l.path.nearest(p)
            && d <= tol
            && best.is_none_or(|b| d < b.0)
        {
            best = Some((d, i));
        }
    }
    best
}

impl BuilderMap {
    /// The map of `roots` (sorted back → front), with the leaves the inputs came from; the outline
    /// edges only `with_edges`. With `open_filled_as_closed` (Consider Open Filled Paths as
    /// Closed), a filled open path bounds a region as if an invisible edge closed it; without it,
    /// the path cuts like a line.
    pub fn new<'a>(doc: &'a Document, roots: &[NodeId], with_edges: bool, open_filled_as_closed: bool) -> (Self, Vec<&'a Node>) {
        Self::with_gap(doc, roots, with_edges, open_filled_as_closed, 0.0)
    }

    /// The same arrangement with Gap Detection, using the shared Live Paint gap finder.
    pub fn with_gap<'a>(doc: &'a Document, roots: &[NodeId], with_edges: bool, open_filled_as_closed: bool, gap: f64) -> (Self, Vec<&'a Node>) {
        let (shapes, leaves) = shape_builder_shapes(doc, roots, open_filled_as_closed);
        (Self::of_shapes(shapes, with_edges, gap), leaves)
    }

    /// The map of `shapes` (the inputs, back → front), with the outline edges only `with_edges`,
    /// closing gaps up to `gap`.
    fn of_shapes(shapes: Vec<po::Shape>, with_edges: bool, gap: f64) -> Self {
        if shapes.is_empty() {
            return Self::default();
        }
        let arr = po::shape_builder_with_gap(&shapes, with_edges, gap);
        let bounded = |pieces: Vec<po::Shape>| {
            pieces
                .into_iter()
                .filter_map(|l| {
                    let b = l.path.bounds()?;
                    Some((l, b))
                })
                .collect()
        };
        Self { shapes, regions: faces_of(arr.regions), lines: bounded(arr.lines), edges: bounded(arr.edges) }
    }

    /// The region containing `p`.
    pub fn region_at(&self, p: Point) -> Option<usize> {
        self.regions.iter().position(|(_, bp, b)| b.contains(p) && bp.winding(p) != 0)
    }

    /// The regions a rectangular marquee from `from` (where the drag started) to `to` touches: each
    /// region whose outline crosses the rectangle or has an anchor inside it, and the region the
    /// whole rectangle lies in. Of those, the region under `from` comes first, then the one under
    /// `to`, then the others in map order. A marquee of no size touches the region under `from`.
    pub fn regions_in_marquee(&self, from: Point, to: Point) -> Vec<usize> {
        let r = Rect::from_points(from, to);
        if r.width() <= 0.0 && r.height() <= 0.0 {
            return self.region_at(from).into_iter().collect();
        }
        let c = r.center();
        let touches = |(reg, bp, b): &Face| (b.contains(c) && bp.winding(c) != 0) || vectorcraft_geom::hit::intersects_rect(&reg.path, r);
        let mut out: Vec<usize> = self.regions.iter().enumerate().filter(|(_, f)| touches(f)).map(|(i, _)| i).collect();
        for p in [to, from] {
            if let Some(k) = self.region_at(p).and_then(|i| out.iter().position(|&j| j == i)) {
                let i = out.remove(k);
                out.insert(0, i);
            }
        }
        out
    }

    /// What a gesture at `p` touches: with `piece_tol` (erasing), the line piece or edge that
    /// close, else the region under it.
    pub fn hit(&self, p: Point, piece_tol: Option<f64>) -> Option<Hit> {
        let piece = piece_tol.and_then(|t| match (piece_near(&self.lines, p, t), piece_near(&self.edges, p, t)) {
            (Some(l), Some(e)) if e.0 < l.0 => Some(Hit::Edge(e.1)),
            (Some(l), _) => Some(Hit::Line(l.1)),
            (None, e) => e.map(|e| Hit::Edge(e.1)),
        });
        piece.or_else(|| self.region_at(p).map(Hit::Region))
    }

    /// The stroke a click at `p` splits with In Merge Mode, Clicking Stroke Splits the Path: the
    /// line piece or edge nearest `p` within `tol` (as [`BuilderMap::hit`] finds it), from the
    /// anchor or crossing point before `p` to the one after it on the path it lies on. `outlines`
    /// are the inputs' outlines as they are drawn (a filled open path without the edge that closes
    /// it in the map). Returns the input's index and the run, going the way the path goes; None
    /// when no piece is that close, when the run is not on the outline as drawn, when it would be a
    /// whole closed subpath, when it is too short to cut out (see [`po::cut_out`]), or when it is
    /// all of the path (a lone line) or leaves no more of it than [`outline_snap`], which leaves
    /// nothing to split off.
    pub fn stroke_at(&self, outlines: &[PathData], p: Point, tol: f64) -> Option<(usize, SubPath)> {
        use kurbo::ParamCurve as _;
        let piece = match self.hit(p, Some(tol))? {
            Hit::Line(i) => self.lines.get(i),
            Hit::Edge(i) => self.edges.get(i),
            Hit::Region(_) => None,
        }?;
        let i = piece.0.key as usize;
        let drawn = outlines.get(i)?;
        let stops: Vec<Point> = drawn.anchors().map(|(.., a)| a.p).collect();
        let snap = outline_snap(drawn);
        let mut run = po::run_around(piece.0.path.subpaths.first()?, &stops, p, snap)?;
        let n = run.segment_count();
        if n == 0 {
            return None;
        }
        let mid = run.segment(n / 2).eval(0.5);
        drawn.nearest(mid).filter(|n| n.4 <= snap)?;
        // A run that cannot be cut out, or that is all of the path (such as a lone line) or all
        // but a speck of it, leaves nothing to split off.
        if !po::cut_out(drawn, &run, false, snap).is_some_and(|rest| rest.length() > snap) {
            return None;
        }
        // Reverse the run when the path's parameter goes down along it.
        let seg = run.segment(n / 2);
        if let (Some(a), Some(b)) = (drawn.nearest(seg.eval(0.25)), drawn.nearest(seg.eval(0.75)))
            && a.0 == b.0
            && (b.1 as f64 + b.2) < (a.1 as f64 + a.2)
        {
            run.reverse();
        }
        Some((i, run))
    }

    /// What a gesture along the polyline `pts` touches, in order: [`Self::hit`] at each vertex and
    /// at the points [`Self::probes`] picks between them, so it touches every region it crosses,
    /// however narrow.
    pub fn hits_along(&self, pts: &[Point], piece_tol: Option<f64>) -> Touched {
        let mut touched = Touched::default();
        let mut last: Option<Point> = None;
        for &p in pts {
            if let Some(a) = last {
                for q in self.probes(a, p, piece_tol) {
                    touched.add(self.hit(q, piece_tol));
                }
            }
            touched.add(self.hit(p, piece_tol));
            last = Some(p);
        }
        touched
    }

    /// The points strictly between `a` and `b`, in order, that [`Self::hits_along`] tests: the
    /// middle of each stretch between two places where the segment crosses a region's outline, and
    /// with `piece_tol` (erasing) those crossings, the places where it crosses a line piece or an
    /// edge, and points `piece_tol / 2` apart (at most 4,000 per segment), so the line pieces and
    /// edges the segment crosses or passes close to are touched too.
    fn probes(&self, a: Point, b: Point, piece_tol: Option<f64>) -> Vec<Point> {
        let len = a.distance(b);
        if !(len.is_finite() && len > 0.0) {
            return vec![];
        }
        let line = kurbo::Line::new(a, b);
        let lb = Rect::from_points(a, b);
        let mut cross: Vec<f64> = vec![];
        for (_, bp, rb) in &self.regions {
            if rb.x0 > lb.x1 || rb.x1 < lb.x0 || rb.y0 > lb.y1 || rb.y1 < lb.y0 {
                continue;
            }
            for seg in bp.segments() {
                cross.extend(seg.intersect_line(line).iter().map(|x| x.line_t));
            }
        }
        cross.retain(|t| t.is_finite() && *t > 0.0 && *t < 1.0);
        cross.sort_by(f64::total_cmp);
        // Crossings closer than this along the segment are one place.
        let gap = 1e-9 / len;
        let mut ts: Vec<f64> = vec![];
        let mut from = 0.0;
        for &t in cross.iter().chain([1.0].iter()) {
            if t - from > gap {
                ts.push((from + t) / 2.0);
                from = t;
            }
        }
        if let Some(tol) = piece_tol {
            ts.extend(cross.iter().copied());
            for (piece, pb) in self.lines.iter().chain(&self.edges) {
                if pb.x0 > lb.x1 || pb.x1 < lb.x0 || pb.y0 > lb.y1 || pb.y1 < lb.y0 {
                    continue;
                }
                for seg in piece.path.to_bezpath().segments() {
                    ts.extend(seg.intersect_line(line).iter().map(|x| x.line_t).filter(|t| t.is_finite() && *t > 0.0 && *t < 1.0));
                }
            }
            let n = ((len / (tol / 2.0).max(1e-3)).ceil() as usize).min(4000);
            ts.extend((1..n).map(|k| k as f64 / n as f64));
            ts.sort_by(f64::total_cmp);
        }
        ts.into_iter().map(|t| a.lerp(b, t)).collect()
    }

    /// The regions just either side of the middle of `piece` (None: outside every region).
    pub fn sides(&self, piece: &po::Shape) -> [Option<usize>; 2] {
        let Some(sp) = piece.path.subpaths.first() else { return [None; 2] };
        let n = sp.segment_count();
        if n == 0 {
            return [None; 2];
        }
        let c = sp.segment(n / 2);
        use kurbo::{ParamCurve as _, ParamCurveDeriv as _};
        let (m, d) = (c.eval(0.5), c.deriv().eval(0.5).to_vec2());
        let len = d.hypot();
        if !(len.is_finite() && len > 0.0) {
            return [None; 2];
        }
        // A little to each side: inside the faces next to it, which share it exactly.
        let off = d.turn_90() * (0.05f64.min(0.05 * c.p0.distance(c.p3)).max(1e-6) / len);
        [m + off, m - off].map(|q| self.region_at(q))
    }

    /// Does line piece `i` run between two different regions of `regions` (an edge that merging or
    /// deleting them removes)? A piece with the same region on both sides, such as the end of a
    /// line inside a shape, bounds nothing and stays.
    pub fn line_between(&self, i: usize, regions: &[usize]) -> bool {
        self.lines.get(i).is_some_and(|(l, _)| matches!(self.sides(l), [Some(a), Some(b)] if a != b && regions.contains(&a) && regions.contains(&b)))
    }

    /// The edges `touched` deletes: those that bound none of its regions (deleting a region takes
    /// its edges with it, so an erase drag that crosses an edge into a region deletes the region).
    pub fn erased_edges(&self, touched: &Touched) -> Vec<usize> {
        let bounds_one = |(e, _): &(po::Shape, Rect)| self.sides(e).iter().any(|r| r.is_some_and(|r| touched.regions.contains(&r)));
        touched.edges.iter().copied().filter(|&k| self.edges.get(k).is_some_and(|e| !bounds_one(e))).collect()
    }
}

/// The regions, line pieces and edges a Shape Builder gesture has touched, in the order touched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Touched {
    pub regions: Vec<usize>,
    pub lines: Vec<usize>,
    pub edges: Vec<usize>,
}

impl Touched {
    pub fn add(&mut self, hit: Option<Hit>) {
        let (list, i) = match hit {
            Some(Hit::Region(i)) => (&mut self.regions, i),
            Some(Hit::Line(i)) => (&mut self.lines, i),
            Some(Hit::Edge(i)) => (&mut self.edges, i),
            None => return,
        };
        if !list.contains(&i) {
            list.push(i);
        }
    }

    /// Adds what `other` touched, in its order.
    pub fn extend(&mut self, other: &Touched) {
        let regions = other.regions.iter().map(|&i| Hit::Region(i));
        for hit in regions.chain(other.lines.iter().map(|&i| Hit::Line(i))).chain(other.edges.iter().map(|&i| Hit::Edge(i))) {
            self.add(Some(hit));
        }
    }

    pub fn is_empty(&self) -> bool {
        self.regions.is_empty() && self.lines.is_empty() && self.edges.is_empty()
    }
}

/// Selected roots sorted back → front in document order.
pub fn sorted_roots(doc: &Document, ids: &[NodeId]) -> Vec<NodeId> {
    let mut v: Vec<(Vec<usize>, NodeId)> = ids.iter().filter_map(|id| doc.index_path(*id).map(|p| (p, *id))).collect();
    v.sort();
    v.dedup_by(|a, b| a.1 == b.1);
    v.into_iter().map(|(_, id)| id).collect()
}

/// Sample points along a polyline every `step` units (always including the vertices).
pub fn sample_polyline(pts: &[Point], step: f64) -> Vec<Point> {
    let mut out = vec![];
    let step = step.max(1e-3);
    for (i, &p) in pts.iter().enumerate() {
        if i > 0 {
            let a = pts[i - 1];
            let n = ((a.distance(p) / step).ceil() as usize).min(4000);
            for k in 1..n {
                out.push(a.lerp(p, k as f64 / n as f64));
            }
        }
        out.push(p);
    }
    out
}

/// Diagonal hatch lines filling `path` (the Shape Builder's grey mesh highlight), `gap` apart.
pub fn hatch(path: &BezPath, gap: f64) -> BezPath {
    let mut out = BezPath::new();
    let b = path.bounding_box();
    if b.width() <= 0.0 || b.height() <= 0.0 {
        return out;
    }
    // Rotate by -45° so the hatch lines become horizontal scanlines.
    let s = std::f64::consts::FRAC_1_SQRT_2;
    let rot = |p: Point| Point::new((p.x + p.y) * s, (p.y - p.x) * s);
    let unrot = |p: Point| Point::new((p.x - p.y) * s, (p.x + p.y) * s);
    let mut edges: Vec<(Point, Point)> = vec![];
    let mut first = Point::ZERO;
    let mut last = Point::ZERO;
    kurbo::flatten(path.iter(), gap.max(1e-3) * 0.1, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if last != first {
                edges.push((rot(last), rot(first)));
            }
            first = p;
            last = p;
        }
        kurbo::PathEl::LineTo(p) => {
            edges.push((rot(last), rot(p)));
            last = p;
        }
        kurbo::PathEl::ClosePath => {
            if last != first {
                edges.push((rot(last), rot(first)));
            }
            last = first;
        }
        _ => {}
    });
    if last != first {
        edges.push((rot(last), rot(first)));
    }
    let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
    for (a, c) in &edges {
        y0 = y0.min(a.y.min(c.y));
        y1 = y1.max(a.y.max(c.y));
    }
    let gap = gap.max((y1 - y0) / 400.0).max(1e-6);
    let mut y = (y0 / gap).floor() * gap + gap * 0.5;
    let mut xs: Vec<f64> = vec![];
    while y < y1 {
        xs.clear();
        for (a, c) in &edges {
            if (a.y <= y) != (c.y <= y) {
                xs.push(a.x + (y - a.y) / (c.y - a.y) * (c.x - a.x));
            }
        }
        xs.sort_by(f64::total_cmp);
        for pair in xs.as_chunks::<2>().0 {
            out.move_to(unrot(Point::new(pair[0], y)));
            out.line_to(unrot(Point::new(pair[1], y)));
        }
        y += gap;
    }
    out
}

/// What a [`RegionCache`] map was built for: the document's address and revision, the selected ids
/// and whether filled open paths count as closed.
type CacheKey = (usize, (u64, u64), Vec<NodeId>, bool);

/// Hovering builds the map of inputs with more segments than this on another thread
/// ([`RegionCache::ready`]); smaller maps are built on the calling thread.
const BACKGROUND_SEGMENTS: usize = 128;

/// A map being built on another thread: the key it is for, the drawn outlines of its inputs, and
/// the channel it arrives on.
struct Pending {
    key: CacheKey,
    outlines: Vec<PathData>,
    done: Receiver<BuilderMap>,
}

/// The planar map of the current selection, cached by document identity, revision, selection and
/// whether filled open paths count as closed.
#[derive(Default)]
struct RegionCache {
    key: Option<CacheKey>,
    map: BuilderMap,
    /// Live Paint faces (every path an edge, filled or not) rather than Shape Builder regions.
    live: bool,
    /// Gap Detection distance; changing tool options invalidates the cache.
    gap: f64,
    /// The outlines of the Shape Builder's inputs as they are drawn (see [`BuilderMap::stroke_at`]).
    outlines: Vec<PathData>,
    /// The map being built on another thread, if any.
    pending: Option<Pending>,
    /// The key of the last map whose build on another thread failed.
    failed: Option<CacheKey>,
    /// How many times the map has been built.
    #[cfg(test)]
    builds: usize,
    /// How many of those builds ran on another thread.
    #[cfg(test)]
    background: usize,
}

/// The map of `shapes`: Live Paint faces with `live`, else the Shape Builder map with its edges,
/// closing gaps up to `gap`.
fn build_map(shapes: Vec<po::Shape>, live: bool, gap: f64) -> BuilderMap {
    if live {
        let regions = if shapes.is_empty() { vec![] } else { faces_of(po::live_paint(&shapes).0) };
        BuilderMap { regions, ..Default::default() }
    } else {
        BuilderMap::of_shapes(shapes, true, gap)
    }
}

/// Builds the map of `shapes` ([`build_map`]) on a new thread; None when no thread starts.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_build(shapes: Vec<po::Shape>, live: bool, gap: f64) -> Option<Receiver<BuilderMap>> {
    let (tx, done) = std::sync::mpsc::channel();
    let spawned = std::thread::Builder::new().name("vectorcraft-region-map".into()).spawn(move || {
        // The tool may have dropped the receiver meanwhile (another tool, another selection).
        let _ = tx.send(build_map(shapes, live, gap));
    });
    spawned.is_ok().then_some(done)
}

/// The web build has no threads.
#[cfg(target_arch = "wasm32")]
fn spawn_build(_shapes: Vec<po::Shape>, _live: bool, _gap: f64) -> Option<Receiver<BuilderMap>> {
    None
}

impl RegionCache {
    fn live_paint() -> Self {
        Self { live: true, ..Default::default() }
    }

    fn key_for(cx: &ToolContext, open_filled_as_closed: bool) -> CacheKey {
        (cx.doc as *const Document as usize, cx.revision, cx.selection.objects.clone(), open_filled_as_closed)
    }

    /// The inputs of the selection's map, back → front, and their drawn outlines (Shape Builder
    /// only).
    fn inputs(&self, cx: &ToolContext, open_filled_as_closed: bool) -> (Vec<po::Shape>, Vec<PathData>) {
        let roots = sorted_roots(cx.doc, &cx.selection.objects);
        if self.live {
            (shapes_for(cx.doc, &roots).0, vec![])
        } else {
            let (shapes, leaves) = shape_builder_shapes(cx.doc, &roots, open_filled_as_closed);
            let outlines = drawn_outlines(cx.doc, &leaves);
            (shapes, outlines)
        }
    }

    fn install(&mut self, key: CacheKey, map: BuilderMap, outlines: Vec<PathData>) {
        self.map = map;
        self.outlines = outlines;
        self.key = Some(key);
        #[cfg(test)]
        {
            self.builds += 1;
        }
    }

    /// Forgets the map and any map being built (the tool options changed).
    fn clear(&mut self) {
        self.key = None;
        self.pending = None;
        self.failed = None;
    }

    /// Is a map being built on another thread?
    fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    /// The map, built again only when the key changes, with filled open paths closed when
    /// `open_filled_as_closed` (Shape Builder only). A Shape Builder map has the outline edges too,
    /// so hovering and erasing use the same map. Waits for a map being built on another thread for
    /// the same key.
    fn get(&mut self, cx: &ToolContext, open_filled_as_closed: bool) -> &BuilderMap {
        let key = Self::key_for(cx, open_filled_as_closed);
        if self.key.as_ref() != Some(&key) {
            let built = self.pending.take().filter(|p| p.key == key).and_then(|p| Some((p.done.recv().ok()?, p.outlines)));
            let (map, outlines) = match built {
                Some(b) => {
                    #[cfg(test)]
                    {
                        self.background += 1;
                    }
                    b
                }
                None => {
                    let (shapes, outlines) = self.inputs(cx, open_filled_as_closed);
                    (build_map(shapes, self.live, self.gap), outlines)
                }
            };
            self.install(key, map, outlines);
        }
        &self.map
    }

    /// Takes the map being built on another thread once it has arrived, or with `wait` once it
    /// arrives, as the map for the key it was built for.
    fn settle(&mut self, wait: bool) {
        let Some(p) = self.pending.take() else { return };
        let got = if wait { p.done.recv().map_err(|_| TryRecvError::Disconnected) } else { p.done.try_recv() };
        match got {
            Ok(map) => {
                #[cfg(test)]
                {
                    self.background += 1;
                }
                self.install(p.key, map, p.outlines);
            }
            // The build failed (a bug in the planar map): hovering does not build it again, and a
            // gesture builds it on the calling thread, which reports the failure.
            Err(TryRecvError::Disconnected) => self.failed = Some(p.key),
            Err(TryRecvError::Empty) => self.pending = Some(p),
        }
    }

    /// The map as [`Self::get`] gives it, without waiting: when the key changes and the inputs have
    /// more than [`BACKGROUND_SEGMENTS`] segments, the map is built on another thread, and this is
    /// None until it arrives ([`Self::waiting`] holds meanwhile). A map being built for an earlier
    /// key is let finish before the next one starts.
    fn ready(&mut self, cx: &ToolContext, open_filled_as_closed: bool) -> Option<&BuilderMap> {
        self.settle(false);
        let key = Self::key_for(cx, open_filled_as_closed);
        if self.key.as_ref() == Some(&key) {
            return Some(&self.map);
        }
        if self.pending.is_some() || self.failed.as_ref() == Some(&key) {
            return None;
        }
        let (shapes, outlines) = self.inputs(cx, open_filled_as_closed);
        let segments: usize = shapes.iter().map(|s| s.path.subpaths.iter().map(|sp| sp.segment_count()).sum::<usize>()).sum();
        if segments > BACKGROUND_SEGMENTS && cfg!(not(target_arch = "wasm32")) {
            if let Some(done) = spawn_build(shapes, self.live, self.gap) {
                self.pending = Some(Pending { key, outlines, done });
                return None;
            }
            // No thread started: build here.
            return Some(self.get(cx, open_filled_as_closed));
        }
        let map = build_map(shapes, self.live, self.gap);
        self.install(key, map, outlines);
        Some(&self.map)
    }

    fn at(&mut self, cx: &ToolContext, p: Point) -> Option<usize> {
        self.get(cx, true).region_at(p)
    }
}

// ---------- Shape Builder ----------

pub use po::SHAPE_BUILDER_MAX_GAP;
/// Small, Medium and Large gap lengths in points.
pub const GAP_LENGTHS: [f64; 3] = [3.0, 6.0, 12.0];

/// Shape Builder Tool Options (double-click the tool; `tool.setOption`), kept across tool switches
/// and saved with the preferences.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeBuilderOptions {
    /// Consider Open Filled Paths as Closed (`openFilledAsClosed`, on): a filled open path bounds a
    /// region as if an invisible edge closed it. When it is off, the path cuts the regions it crosses
    /// as a line does.
    pub open_filled_as_closed: bool,
    /// Pick Color From Color Swatches (`pickColorFrom: "swatches"`; off: `"artwork"`): a merge
    /// fills the new shape with the current fill, and the Left and Right arrow keys step the current
    /// fill through the document's color swatches. With Artwork, the shape takes the look of the
    /// object where the drag starts, else of the one where it ends, else of the front-most selected
    /// object.
    pub swatches: bool,
    /// Cursor Swatch Preview (`cursorSwatchPreview`, off): with Color Swatches, the current fill
    /// and the swatches before and after it show as chips above the pointer while no drag is on.
    pub cursor_swatch_preview: bool,
    /// Selection Straight Line (`selection: "straightLine"`; off: `"freeform"`): a drag touches
    /// what the straight line from where it started to the pointer crosses, not what the pointer
    /// passed over.
    pub straight_line: bool,
    /// Highlight Fill (`highlightFill`, on): in merge mode, the regions under the pointer or
    /// touched by the drag are shaded with a gray mesh. When it is off, they are not highlighted.
    pub highlight_fill: bool,
    /// Close gaps up to `gap_length` while finding regions.
    pub gap_detection: bool,
    /// Gap Length in points (at most [`SHAPE_BUILDER_MAX_GAP`]).
    pub gap_length: f64,
    /// In Merge Mode, Clicking Stroke Splits the Path (`clickStrokeSplitsPath`, off): in merge mode,
    /// a click on a stroke splits its path into the run under the pointer
    /// ([`BuilderMap::stroke_at`]) and the rest, and the pointer shows scissors over such a stroke.
    pub click_stroke_splits: bool,
    /// Highlight Stroke When Editable (`highlightStroke`, on): with `click_stroke_splits`, the run
    /// of stroke a click would split is drawn in `stroke_color` while the pointer is over it.
    pub highlight_stroke: bool,
    /// The color of that highlight (`highlightColor`: `#rrggbb` or a layer color's name; the layer
    /// color Red by default).
    pub stroke_color: [u8; 3],
}

/// The color Highlight Stroke When Editable draws in by default: the layer color Red.
pub const STROKE_HIGHLIGHT: [u8; 3] = [0xff, 0x4f, 0x4f];

impl Default for ShapeBuilderOptions {
    fn default() -> Self {
        Self {
            open_filled_as_closed: true,
            swatches: false,
            cursor_swatch_preview: false,
            straight_line: false,
            highlight_fill: true,
            gap_detection: false,
            gap_length: GAP_LENGTHS[0],
            click_stroke_splits: false,
            highlight_stroke: true,
            stroke_color: STROKE_HIGHLIGHT,
        }
    }
}

/// The document's color swatches in the Swatches panel's order (the swatches outside groups, then
/// each group's): the solid colors, without None.
pub fn color_swatches(doc: &Document) -> Vec<&Swatch> {
    let groups = doc.swatch_groups.iter().flat_map(|g| &g.swatches);
    doc.swatches.iter().chain(groups).filter(|s| matches!(s.paint, Paint::Solid { .. })).collect()
}

/// Is `fill` swatch `s`: linked to it by name, or the same paint?
fn is_swatch(s: &Swatch, fill: &Paint) -> bool {
    s.paint == *fill || matches!(fill, Paint::Solid { swatch: Some(name), tint, .. } if *tint >= 1.0 && *name == s.name)
}

/// The index in `list` of the swatch `fill` is: `last` (the swatch picked last) while `fill` is still
/// that swatch, so swatches of the same color are each reached in turn; otherwise the first swatch
/// it is.
fn swatch_index(list: &[&Swatch], fill: &Paint, last: Option<usize>) -> Option<usize> {
    last.filter(|&i| list.get(i).is_some_and(|s| is_swatch(s, fill))).or_else(|| list.iter().position(|s| is_swatch(s, fill)))
}

/// The index of the swatch of `list` after (`forward`) or before the one `fill` is (see
/// [`swatch_index`]), wrapping around; the first or the last one when `fill` is none of them.
fn step_swatch(list: &[&Swatch], fill: &Paint, last: Option<usize>, forward: bool) -> Option<usize> {
    let n = list.len();
    match (swatch_index(list, fill, last), forward) {
        (Some(i), true) => Some((i + 1) % n),
        (Some(i), false) => Some((i + n - 1) % n),
        (None, true) => (n > 0).then_some(0),
        (None, false) => n.checked_sub(1),
    }
}

/// Shape Builder tool: drag across regions to merge them. Holding Alt (Option) switches to erase
/// mode while it is held: the pointer shows a minus, what a click or drag would delete highlights in
/// red, and the gesture deletes the regions it touches, the pieces of open paths and the edges
/// (outline pieces between the points where paths meet) it passes over. With Shift held at the
/// press, the drag draws a rectangular marquee and touches the regions it reaches (see
/// [`BuilderMap::regions_in_marquee`]): they merge, or with Alt they are deleted. Its options are
/// [`ShapeBuilderOptions`].
#[derive(Default)]
pub struct ShapeBuilderTool {
    cache: RegionCache,
    hover: Option<Hit>,
    points: Vec<Point>,
    touched: Touched,
    dragging: bool,
    /// A Shift-drag's marquee: where the drag started and where the pointer is.
    marquee: Option<(Point, Point)>,
    /// Erase mode: Alt held while hovering, or when the drag started.
    alt: bool,
    /// Where the pointer last moved (the cursor swatch preview sits above it).
    at: Option<Point>,
    /// The index in [`color_swatches`] of the swatch the arrow keys picked last.
    picked: Option<usize>,
    opts: ShapeBuilderOptions,
    /// With In Merge Mode, Clicking Stroke Splits the Path: the run of stroke under the pointer,
    /// which a click splits off.
    stroke: Option<SubPath>,
    /// The pointer went more than [`CLICK_TOL_PX`] from the press during the drag, so the gesture
    /// is not a click.
    far: bool,
}

/// How close (screen px) the pointer must come to a line piece or an edge to delete it.
const LINE_TOL_PX: f64 = 4.0;

/// How far (screen px) the pointer may move between press and release for the gesture to be a
/// click, which with In Merge Mode, Clicking Stroke Splits the Path splits the stroke at the press.
const CLICK_TOL_PX: f64 = 3.0;

impl ShapeBuilderTool {
    fn gap(&self) -> f64 {
        if self.opts.gap_detection { self.opts.gap_length } else { 0.0 }
    }

    /// With In Merge Mode, Clicking Stroke Splits the Path and not in erase mode (`alt`): the run of
    /// stroke a click at `p` splits off ([`BuilderMap::stroke_at`]).
    fn stroke_at(&mut self, cx: &ToolContext, p: Point, alt: bool) -> Option<SubPath> {
        if alt || !self.opts.click_stroke_splits {
            return None;
        }
        self.cache.get(cx, self.opts.open_filled_as_closed);
        self.cache.map.stroke_at(&self.cache.outlines, p, cx.tol(LINE_TOL_PX)).map(|(_, run)| run)
    }

    /// What the pointer at `p` is over: a stroke a click would split, else what a gesture there
    /// touches. While the map of a large selection is being built on another thread, nothing.
    fn hover_at(&mut self, cx: &ToolContext, p: Point) {
        if self.cache.ready(cx, self.opts.open_filled_as_closed).is_none() {
            self.stroke = None;
            self.hover = None;
            return;
        }
        // Over a stroke a click would split, the tool highlights the stroke and not the region.
        self.stroke = self.stroke_at(cx, p, self.alt);
        self.hover = match self.stroke {
            Some(_) => None,
            None => self.cache.get(cx, self.opts.open_filled_as_closed).hit(p, Self::line_tol(cx, self.alt)),
        };
    }

    fn line_tol(cx: &ToolContext, alt: bool) -> Option<f64> {
        alt.then(|| cx.tol(LINE_TOL_PX))
    }

    /// Adds what the drag from `from` to `to` touches, picked as `shapeBuilder.merge` picks it.
    fn touch(&mut self, cx: &ToolContext, from: Option<Point>, to: Point) {
        let pts: Vec<Point> = from.into_iter().chain([to]).collect();
        let tol = Self::line_tol(cx, self.alt);
        let touched = self.cache.get(cx, self.opts.open_filled_as_closed).hits_along(&pts, tol);
        self.touched.extend(&touched);
    }

    /// Selection Straight Line: the drag touches what the line from its first point to `p` crosses.
    fn straight_to(&mut self, cx: &ToolContext, p: Point) {
        let Some(&start) = self.points.first() else { return };
        self.touched = Touched::default();
        if start == p {
            self.points = vec![start];
            self.touch(cx, None, start);
        } else {
            self.points = vec![start, p];
            self.touch(cx, Some(start), p);
        }
    }

    /// A Shift-drag: the drag touches the regions the marquee from where it started to `p` reaches;
    /// while the pointer is where the drag started, what a click there touches.
    fn marquee_to(&mut self, cx: &ToolContext, p: Point) {
        let Some((start, _)) = self.marquee else { return };
        self.marquee = Some((start, p));
        self.touched = Touched::default();
        if start == p {
            self.touch(cx, None, start);
        } else {
            self.touched.regions = self.cache.get(cx, self.opts.open_filled_as_closed).regions_in_marquee(start, p);
        }
    }

    /// The cursor swatch preview: the swatch before the current fill, the current fill (ringed)
    /// and the swatch after it, side by side above the pointer.
    fn swatch_chips(&self, cx: &ToolContext) -> Vec<Overlay> {
        let Some(at) = self.at else { return vec![] };
        let list = color_swatches(cx.doc);
        let fill = &cx.paint.fill;
        let rgba = |p: &Paint| match p {
            Paint::Solid { color, .. } => Some(color.to_rgba8(1.0)),
            _ => None,
        };
        let chip = |forward| step_swatch(&list, fill, self.picked, forward).and_then(|i| rgba(&list.get(i)?.paint));
        let (before, after) = (chip(false), chip(true));
        let y = at.y - cx.tol(22.0);
        [(before, -16.0, false), (rgba(fill), 0.0, true), (after, 16.0, false)]
            .into_iter()
            .filter_map(|(color, dx, selected)| Some(Overlay::Swatch { p: Point::new(at.x + cx.tol(dx), y), color: color?, selected }))
            .collect()
    }
}

impl Tool for ShapeBuilderTool {
    fn id(&self) -> &'static str {
        "shapeBuilder"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.alt = ev.mods.alt;
                self.at = Some(p);
                self.hover_at(cx, p);
                vec![]
            }
            PointerKind::Down => {
                if cx.selection.is_empty() {
                    return vec![];
                }
                self.dragging = true;
                self.alt = ev.mods.alt;
                self.marquee = ev.mods.shift.then_some((p, p));
                self.points = vec![p];
                self.far = false;
                self.touched = Touched::default();
                self.touch(cx, None, p);
                vec![]
            }
            PointerKind::Drag if self.dragging => {
                self.at = Some(p);
                if self.points.first().is_some_and(|s| s.distance(p) > cx.tol(CLICK_TOL_PX)) {
                    self.far = true;
                }
                if self.marquee.is_some() {
                    self.marquee_to(cx, p);
                    return vec![];
                }
                if self.opts.straight_line {
                    self.straight_to(cx, p);
                    return vec![];
                }
                let last = self.points.last().copied();
                if last.is_none_or(|q| q.distance(p) >= cx.tol(1.0)) {
                    self.touch(cx, last, p);
                    self.points.push(p);
                }
                vec![]
            }
            PointerKind::Up if self.dragging => {
                self.dragging = false;
                if self.marquee.is_some() {
                    self.marquee_to(cx, p);
                } else if self.opts.straight_line {
                    self.straight_to(cx, p);
                } else if self.points.last().is_none_or(|q| *q != p) {
                    let last = self.points.last().copied();
                    self.touch(cx, last, p);
                    self.points.push(p);
                }
                let points = std::mem::take(&mut self.points);
                // The mode the drag started in, which its highlight showed.
                let erase = self.alt;
                // A marquee of no size is a click.
                let marquee = self.marquee.take().filter(|(a, b)| a != b);
                // With In Merge Mode, Clicking Stroke Splits the Path, a click on a stroke splits it
                // where the press was; a marquee splits nothing.
                let press = points.first().copied().filter(|s| marquee.is_none() && !self.far && s.distance(p) <= cx.tol(CLICK_TOL_PX));
                let split = press.filter(|q| self.stroke_at(cx, *q, erase).is_some());
                if std::mem::take(&mut self.touched).is_empty() && split.is_none() {
                    return vec![];
                }
                self.stroke = None;
                let roots = sorted_roots(cx.doc, &cx.selection.objects);
                let points = split.map_or(points, |q| vec![q]);
                let pts: Vec<Value> = points.iter().map(|q| json!([q.x, q.y])).collect();
                let mut params = json!({ "ids": json_ids(&roots), "points": pts, "erase": erase, "gap": self.gap() });
                if let Some((a, b)) = marquee
                    && let Some(o) = params.as_object_mut()
                {
                    o.remove("points");
                    o.insert("rect".into(), json!([a.x, a.y, b.x - a.x, b.y - a.y]));
                }
                if let Some(t) = Self::line_tol(cx, erase) {
                    params["tolerance"] = json!(t);
                }
                if split.is_some() {
                    params["splitStroke"] = json!(true);
                    params["tolerance"] = json!(cx.tol(LINE_TOL_PX));
                }
                if !self.opts.open_filled_as_closed {
                    params["openFilledAsClosed"] = json!(false);
                }
                if self.opts.swatches {
                    params["pickColorFrom"] = json!("swatches");
                }
                vec![Action::Exec("shapeBuilder.merge".into(), params)]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, cx: &ToolContext) -> Vec<Overlay> {
        let mut out = vec![];
        let map = &self.cache.map;
        let color = if self.alt { HIGHLIGHT_RED } else { HATCH };
        let show: Vec<Hit> = if self.dragging {
            let t = &self.touched;
            let regions = t.regions.iter().map(|&i| Hit::Region(i));
            regions.chain(t.lines.iter().map(|&i| Hit::Line(i))).chain(map.erased_edges(t).into_iter().map(Hit::Edge)).collect()
        } else {
            self.hover.into_iter().collect()
        };
        for hit in show {
            match hit {
                // Highlight Fill off: merge mode leaves the regions as they are.
                Hit::Region(_) if !self.alt && !self.opts.highlight_fill => {}
                Hit::Region(i) => {
                    if let Some((_, bp, _)) = map.regions.get(i) {
                        out.push(Overlay::Path { path: hatch(bp, cx.tol(4.0)), color, width: 1.0, dashed: false });
                        out.push(Overlay::Path { path: bp.clone(), color, width: 1.0, dashed: false });
                    }
                }
                Hit::Line(i) | Hit::Edge(i) => {
                    let pieces = if matches!(hit, Hit::Line(_)) { &map.lines } else { &map.edges };
                    if let Some((l, _)) = pieces.get(i) {
                        out.push(Overlay::Path { path: l.path.to_bezpath(), color: HIGHLIGHT_RED, width: 3.0, dashed: false });
                    }
                }
            }
        }
        if self.dragging
            && let Some((a, b)) = self.marquee.filter(|(a, b)| a != b)
        {
            out.push(Overlay::Marquee(Rect::from_points(a, b)));
        }
        if self.dragging && self.points.len() > 1 {
            let mut bp = BezPath::new();
            bp.move_to(self.points[0]);
            for q in &self.points[1..] {
                bp.line_to(*q);
            }
            out.push(Overlay::Path { path: bp, color, width: 1.0, dashed: false });
        }
        if self.opts.swatches && self.opts.cursor_swatch_preview && !self.dragging {
            out.extend(self.swatch_chips(cx));
        }
        // Highlight Stroke When Editable draws the stroke a click would split.
        if let Some(run) = self.stroke.as_ref().filter(|_| self.opts.highlight_stroke && !self.dragging) {
            let mut path = BezPath::new();
            run.to_bezpath_into(&mut path);
            out.push(Overlay::Path { path, color: self.opts.stroke_color, width: 3.0, dashed: false });
        }
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, m: Mods) -> Cursor {
        // A drag keeps the mode it started in; hovering follows the key.
        let erase = if self.dragging { self.alt } else { m.alt };
        if erase {
            Cursor::ShapeBuilderErase
        } else if self.stroke.is_some() && !self.dragging {
            // Over a stroke a click splits.
            Cursor::Scissors
        } else {
            Cursor::ShapeBuilder
        }
    }

    fn busy(&self) -> bool {
        self.dragging
    }

    fn waiting(&self) -> bool {
        self.cache.waiting()
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) {
        if !self.cache.waiting() || self.dragging {
            return;
        }
        // The map that arrives may be for an earlier revision; the hover then starts the current
        // one's, which `wait` waits for too.
        loop {
            self.cache.settle(wait);
            if let Some(p) = self.at {
                self.hover_at(cx, p);
            }
            if !wait || !self.cache.waiting() {
                break;
            }
        }
    }

    /// With Pick Color From Color Swatches, the Left and Right arrow keys pick the color swatch
    /// before or after the current fill.
    fn claims_key(&self, _cx: &ToolContext, key: ToolKey) -> bool {
        self.opts.swatches && !self.dragging && matches!(key, ToolKey::Left | ToolKey::Right)
    }

    fn key(&mut self, cx: &ToolContext, key: ToolKey, _mods: Mods) -> Vec<Action> {
        if !self.claims_key(cx, key) {
            return vec![];
        }
        let list = color_swatches(cx.doc);
        let Some((i, s)) = step_swatch(&list, &cx.paint.fill, self.picked, key == ToolKey::Right).and_then(|i| Some((i, list.get(i)?))) else {
            return vec![];
        };
        self.picked = Some(i);
        // The current fill only (no `ids`): the selected art keeps its own.
        vec![Action::Exec("paint.setFill".into(), json!({ "swatch": s.name, "ids": [] }))]
    }

    fn options(&self) -> Value {
        let o = &self.opts;
        let [r, g, b] = o.stroke_color;
        json!({
            "openFilledAsClosed": o.open_filled_as_closed,
            "pickColorFrom": if o.swatches { "swatches" } else { "artwork" },
            "cursorSwatchPreview": o.cursor_swatch_preview,
            "selection": if o.straight_line { "straightLine" } else { "freeform" },
            "highlightFill": o.highlight_fill,
            "gapDetection": o.gap_detection,
            "gapLength": o.gap_length,
            "clickStrokeSplitsPath": o.click_stroke_splits,
            "highlightStroke": o.highlight_stroke,
            "highlightColor": format!("#{r:02x}{g:02x}{b:02x}"),
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        let o = &mut self.opts;
        match key {
            "openFilledAsClosed" => o.open_filled_as_closed = v.as_bool().unwrap_or(o.open_filled_as_closed),
            "pickColorFrom" => o.swatches = v.as_str().map_or(o.swatches, |s| s == "swatches"),
            "cursorSwatchPreview" => o.cursor_swatch_preview = v.as_bool().unwrap_or(o.cursor_swatch_preview),
            "selection" => o.straight_line = v.as_str().map_or(o.straight_line, |s| s == "straightLine"),
            "highlightFill" => o.highlight_fill = v.as_bool().unwrap_or(o.highlight_fill),
            "gapDetection" => o.gap_detection = v.as_bool().unwrap_or(o.gap_detection),
            "gapLength" => {
                if let Some(n) = v.as_f64().filter(|n| n.is_finite()) {
                    o.gap_length = n.clamp(0.0, SHAPE_BUILDER_MAX_GAP);
                }
            }
            "clickStrokeSplitsPath" => o.click_stroke_splits = v.as_bool().unwrap_or(o.click_stroke_splits),
            "highlightStroke" => o.highlight_stroke = v.as_bool().unwrap_or(o.highlight_stroke),
            "highlightColor" => o.stroke_color = lp_color(v).unwrap_or(o.stroke_color),
            _ => {}
        }
        self.stroke = None;
        self.cache.clear();
        self.cache.gap = self.gap();
    }
}

// ---------- Live Paint Bucket ----------

/// What the Live Paint tools highlight under the cursor.
#[derive(Clone, Debug, Default, PartialEq)]
enum LpHover {
    #[default]
    None,
    /// A face (or edge) of an existing Live Paint group.
    Part(BezPath),
    /// A region of the (not yet live) selection.
    Region(usize),
}

/// The Live Paint group and part the Live Paint tools hover and click at `p`. Inside a visible face
/// of a visible Live Paint group ([`live_paint_at`]) it is that group with its visible face under
/// `p`, or with `edge` its visible edge nearest `p` within `tol` (None when there is none). Outside
/// every visible face, with `edge`, it is the visible edge nearest `p` within `tol` among the
/// visible Live Paint groups (the front-most group's on a tie). None when no group is found.
fn lp_part_at(doc: &Document, p: Point, tol: f64, edge: bool) -> Option<(NodeId, Option<&Arc<Node>>)> {
    if let Some(g) = live_paint_at(doc, p).and_then(|id| doc.node(id)) {
        return Some((g.id, if edge { visible_edge_near(g, p, tol) } else { visible_face_at(g, p) }));
    }
    if !edge {
        return None;
    }
    let mut best: Option<(f64, NodeId, &Arc<Node>)> = None;
    doc.walk(|n| {
        if is_live_paint(n)
            && let Some(e) = visible_edge_near(n, p, tol)
            && let Some((.., d)) = e.path_data().and_then(|pd| pd.nearest(p))
            && best.is_none_or(|b| d <= b.0)
            && doc.is_visible(n.id)
        {
            best = Some((d, n.id, e));
        }
    });
    best.map(|(_, g, e)| (g, Some(e)))
}

/// What the Live Paint tools highlight at `p`: the part of a Live Paint group ([`lp_part_at`]), else
/// with `cache` the region of the selection under `p`. A large selection's regions are found on
/// another thread ([`RegionCache::ready`]), and until they are, no region is highlighted.
fn lp_hover(cx: &ToolContext, cache: Option<&mut RegionCache>, p: Point, edge: bool) -> LpHover {
    if let Some((_, part)) = lp_part_at(cx.doc, p, cx.tol(4.0), edge) {
        return part.and_then(|n| n.path_data()).map(|pd| LpHover::Part(pd.to_bezpath())).unwrap_or_default();
    }
    let sel_is_live = cx.selection.objects.iter().any(|id| cx.doc.node(*id).is_some_and(is_live_paint));
    if !sel_is_live && let Some(i) = cache.and_then(|c| c.ready(cx, true)).and_then(|m| m.region_at(p)) {
        return LpHover::Region(i);
    }
    LpHover::None
}

/// The highlight of `h`, `width` screen pixels wide in `color`.
fn lp_overlays(h: &LpHover, cache: &RegionCache, color: [u8; 3], width: f32) -> Vec<Overlay> {
    match h {
        LpHover::None => vec![],
        LpHover::Part(bp) => vec![Overlay::Path { path: bp.clone(), color, width, dashed: false }],
        LpHover::Region(i) => {
            cache.map.regions.get(*i).map(|(_, bp, _)| vec![Overlay::Path { path: bp.clone(), color, width, dashed: false }]).unwrap_or_default()
        }
    }
}

/// Live Paint Bucket: click fills the face under the cursor with the current fill; Shift-click
/// paints the nearest edge with the current stroke. Clicking selected ordinary paths makes them a
/// Live Paint group first.
pub struct LivePaintBucketTool {
    cache: RegionCache,
    hover: LpHover,
    /// Where the pointer last moved, and whether Shift was held.
    at: Option<(Point, bool)>,
}

impl Default for LivePaintBucketTool {
    fn default() -> Self {
        Self { cache: RegionCache::live_paint(), hover: LpHover::None, at: None }
    }
}

impl Tool for LivePaintBucketTool {
    fn id(&self) -> &'static str {
        "livePaintBucket"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.at = Some((p, ev.mods.shift));
                self.hover = lp_hover(cx, Some(&mut self.cache), p, ev.mods.shift);
                vec![]
            }
            PointerKind::Down => {
                self.hover = LpHover::None;
                let point = json!([p.x, p.y]);
                if ev.mods.shift {
                    return match lp_part_at(cx.doc, p, cx.tol(4.0), true) {
                        Some((g, Some(_))) => {
                            vec![Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": point, "tolerance": cx.tol(4.0) }))]
                        }
                        _ => vec![],
                    };
                }
                if let Some(g) = live_paint_at(cx.doc, p) {
                    return vec![Action::Exec("livePaint.fill".into(), json!({ "group": g.0, "point": point }))];
                }
                if self.cache.at(cx, p).is_some() {
                    let roots = sorted_roots(cx.doc, &cx.selection.objects);
                    return vec![Action::Exec("livePaint.fill".into(), json!({ "ids": json_ids(&roots), "point": point }))];
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        lp_overlays(&self.hover, &self.cache, HIGHLIGHT_RED, 4.0)
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::LivePaintBucket
    }

    fn waiting(&self) -> bool {
        self.cache.waiting()
    }

    fn poll(&mut self, cx: &ToolContext, wait: bool) {
        if !self.cache.waiting() {
            return;
        }
        // As the Shape Builder's: the map that arrives may be for an earlier revision. Over a part
        // of a Live Paint group the hover takes no map.
        loop {
            self.cache.settle(wait);
            if let Some((p, shift)) = self.at {
                self.hover = lp_hover(cx, Some(&mut self.cache), p, shift);
            }
            if !wait || !self.cache.waiting() {
                break;
            }
        }
    }
}

// ---------- Live Paint Selection ----------

/// The Live Paint Selection tool's highlight width (screen pixels) by default.
pub const LP_SELECT_WIDTH: f64 = 4.0;
/// The least and greatest highlight width (screen pixels) the Live Paint Selection tool takes.
pub const LP_SELECT_WIDTH_RANGE: (f64, f64) = (1.0, 20.0);

/// Live Paint Selection: click selects a face (Shift adds); Alt-click selects an edge. Its Tool
/// Options (double-click the tool) are `selectFills` and `selectStrokes` (both on), the highlight
/// `color` (`#rrggbb`, red by default) and its `width` ([`LP_SELECT_WIDTH`] pixels). With Select
/// Strokes off, Alt-click selects the face under the pointer as a click does; with Select Fills
/// off, a click selects the edge nearest the pointer as Alt-click does; with both off, a click
/// selects no face or edge.
pub struct LivePaintSelectionTool {
    cache: RegionCache,
    hover: LpHover,
    fills: bool,
    strokes: bool,
    color: [u8; 3],
    width: f64,
}

impl Default for LivePaintSelectionTool {
    fn default() -> Self {
        Self { cache: RegionCache::live_paint(), hover: LpHover::None, fills: true, strokes: true, color: HIGHLIGHT_RED, width: LP_SELECT_WIDTH }
    }
}

impl LivePaintSelectionTool {
    /// What a click picks, Alt held or not: Some(true) an edge, Some(false) a face, None nothing.
    fn picks_edge(&self, alt: bool) -> Option<bool> {
        match (self.fills, self.strokes) {
            (true, true) => Some(alt),
            (true, false) => Some(false),
            (false, true) => Some(true),
            (false, false) => None,
        }
    }
}

/// A highlight color option: `#rrggbb` or a layer color's name (any case).
fn lp_color(v: &Value) -> Option<[u8; 3]> {
    let s = v.as_str()?.trim();
    if let Some((_, c)) = vectorcraft_doc::LAYER_COLORS.iter().find(|(name, _)| name.eq_ignore_ascii_case(s)) {
        return Some(*c);
    }
    let [r, g, b, _] = vectorcraft_color::Color::from_hex(s)?.to_rgba8(1.0);
    Some([r, g, b])
}

impl Tool for LivePaintSelectionTool {
    fn id(&self) -> &'static str {
        "livePaintSelection"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        let edge = self.picks_edge(ev.mods.alt);
        match ev.kind {
            PointerKind::Move => {
                // Only the parts of Live Paint groups highlight, so no map of the selection is built.
                self.hover = match edge.map(|e| lp_hover(cx, None, p, e)) {
                    Some(h @ LpHover::Part(_)) => h,
                    _ => LpHover::None,
                };
                vec![]
            }
            PointerKind::Down => {
                let hit = edge.and_then(|e| lp_part_at(cx.doc, p, cx.tol(4.0), e)).and_then(|(_, part)| part).map(|n| n.id);
                match (hit, ev.mods.shift) {
                    (Some(id), true) => vec![Action::Exec("select.toggle".into(), json!({ "id": id.0 }))],
                    (Some(id), false) => vec![Action::Exec("select.set".into(), json!({ "ids": [id.0] }))],
                    (None, true) => vec![],
                    (None, false) => vec![Action::Exec("select.none".into(), json!({}))],
                }
            }
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        lp_overlays(&self.hover, &self.cache, self.color, self.width as f32)
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Arrow
    }

    fn options(&self) -> Value {
        let [r, g, b] = self.color;
        json!({
            "selectFills": self.fills,
            "selectStrokes": self.strokes,
            "color": format!("#{r:02x}{g:02x}{b:02x}"),
            "width": self.width,
        })
    }

    fn set_option(&mut self, key: &str, v: &Value) {
        match key {
            "selectFills" => self.fills = v.as_bool().unwrap_or(self.fills),
            "selectStrokes" => self.strokes = v.as_bool().unwrap_or(self.strokes),
            "color" => self.color = lp_color(v).unwrap_or(self.color),
            "width" => {
                let (min, max) = LP_SELECT_WIDTH_RANGE;
                self.width = v.as_f64().filter(|w| w.is_finite()).map_or(self.width, |w| w.clamp(min, max));
            }
            _ => return,
        }
        self.hover = LpHover::None;
    }
}

/// Create a builder-family tool by id (None = not one of ours).
pub fn create(id: &str) -> Option<Box<dyn Tool>> {
    Some(match id {
        "shapeBuilder" => Box::new(ShapeBuilderTool::default()),
        "livePaintBucket" => Box::new(LivePaintBucketTool::default()),
        "livePaintSelection" => Box::new(LivePaintSelectionTool::default()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::*;
    use vectorcraft_doc::{Appearance, Selection};
    use vectorcraft_geom::shapes;

    fn two_rects() -> (Document, NodeId, NodeId) {
        two_rects_at(0.0)
    }

    /// Squares A (`x`..`x`+100) and B (`x`+50..`x`+150), 100 pt tall, overlapping by half.
    fn two_rects_at(x: f64) -> (Document, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0)), Appearance::default_art())).unwrap();
        let b = d.alloc_id();
        d.insert(Some(l), 1, Node::path(b, shapes::rectangle(Rect::new(x + 50.0, 0.0, x + 150.0, 100.0)), Appearance::default_art())).unwrap();
        (d, a, b)
    }

    fn ev(kind: PointerKind, x: f64, y: f64) -> PointerEvent {
        PointerEvent::new(kind, x, y)
    }

    #[test]
    fn shape_builder_hover_highlights_region() {
        let (d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([a, b]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        assert!(t.pointer(&cx, &ev(PointerKind::Move, 75.0, 50.0)).is_empty());
        let ov = t.overlays(&cx);
        assert_eq!(ov.len(), 2);
        let Overlay::Path { path, .. } = &ov[1] else { panic!() };
        let bb = path.bounding_box();
        assert!((bb.x0 - 50.0).abs() < 1e-6 && (bb.x1 - 100.0).abs() < 1e-6, "{bb:?}");
        t.pointer(&cx, &ev(PointerKind::Move, 400.0, 400.0));
        assert!(t.overlays(&cx).is_empty());
    }

    #[test]
    fn shape_builder_drag_emits_merge() {
        let (d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([b, a]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 25.0, 50.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 50.0));
        assert_eq!(t.touched.regions.len(), 2);
        assert!(t.busy());
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 75.0, 50.0));
        let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!(id, "shapeBuilder.merge");
        assert_eq!(v["erase"], false);
        assert_eq!(v["ids"], json!([a.0, b.0]), "sorted back to front");
        assert!(v["points"].as_array().unwrap().len() >= 2);
    }

    #[test]
    fn shape_builder_rebuilds_its_map_for_a_new_revision_at_the_same_address() {
        let (mut d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([a, b]);
        let p = paint();
        let mut t = ShapeBuilderTool::default();
        let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 50.0));
        // A later revision of the document, with the squares 300 pt to the right, at the same address.
        d = two_rects_at(300.0).0;
        let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
        t.pointer(&cx2, &ev(PointerKind::Down, 325.0, 50.0));
        t.pointer(&cx2, &ev(PointerKind::Drag, 375.0, 50.0));
        let acts = t.pointer(&cx2, &ev(PointerKind::Up, 375.0, 50.0));
        assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "shapeBuilder.merge"), "{acts:?}");
    }

    #[test]
    fn shape_builder_builds_its_map_once_for_hover_alt_hover_and_a_merge() {
        let (d, s) = corner_rects();
        let p = paint();
        let mut t = ShapeBuilderTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        t.pointer(&cx1, &ev(PointerKind::Move, 101.0, 75.0));
        assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
        // Alt hovering over an edge uses the same map.
        t.pointer(&cx1, &ev(PointerKind::Move, 101.0, 75.0).with_mods(alt));
        assert!(matches!(t.hover, Some(Hit::Edge(_))), "{:?}", t.hover);
        assert_eq!(t.cache.builds, 1, "a hover then an Alt hover");
        t.pointer(&cx1, &ev(PointerKind::Down, 25.0, 25.0));
        t.pointer(&cx1, &ev(PointerKind::Drag, 75.0, 75.0));
        let acts = t.pointer(&cx1, &ev(PointerKind::Up, 75.0, 75.0));
        assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "shapeBuilder.merge"), "{acts:?}");
        // The merge left the document as it was (it failed, say): the map still fits it.
        t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 75.0));
        assert_eq!(t.cache.builds, 1, "a hover after a merge that changed nothing");
        let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
        t.pointer(&cx2, &ev(PointerKind::Move, 75.0, 75.0));
        assert_eq!(t.cache.builds, 2, "a hover on a new revision");
    }

    /// A row of `n` selected squares 20 pt wide, each overlapping the next by half: 4 segments each.
    fn square_row(n: usize) -> (Document, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let mut s = Selection::default();
        for k in 0..n {
            let x = k as f64 * 10.0;
            let id = d.alloc_id();
            d.insert(Some(l), k, Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), Appearance::default_art())).unwrap();
            s.add(id);
        }
        (d, s)
    }

    /// Polls `t`, as the desktop app does each frame, until it stops waiting.
    fn poll_until_ready(t: &mut dyn Tool, cx: &ToolContext) {
        let start = std::time::Instant::now();
        while t.waiting() {
            assert!(start.elapsed() < std::time::Duration::from_secs(60), "the tool is still waiting");
            std::thread::sleep(std::time::Duration::from_millis(2));
            t.poll(cx, false);
        }
    }

    #[test]
    fn shape_builder_hover_builds_a_large_map_on_another_thread() {
        let (d, s) = square_row(40);
        let p = paint();
        let mut t = ShapeBuilderTool::default();
        let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        let hover = ev(PointerKind::Move, 15.0, 10.0);
        t.pointer(&cx1, &hover);
        assert_eq!(t.cache.builds, 0, "the first hover returns before the map is built");
        assert!(t.waiting() && t.hover.is_none() && t.overlays(&cx1).is_empty(), "{:?}", t.hover);
        poll_until_ready(&mut t, &cx1);
        assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
        assert_eq!((t.cache.builds, t.cache.background), (1, 1));
        assert_eq!(t.overlays(&cx1).len(), 2, "the region's hatch and outline");
        // A new revision while the map of the one before it is being built: only the newer one's
        // map is used.
        let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
        let cx3 = ToolContext { revision: (1, 3), ..cx(&d, &s, &p) };
        t.pointer(&cx2, &hover);
        t.pointer(&cx3, &hover);
        poll_until_ready(&mut t, &cx3);
        assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
        assert_eq!((t.cache.builds, t.cache.background), (3, 3), "the earlier revision's map, then the newer one's");
        assert_eq!(t.cache.key.as_ref().map(|k| k.1), Some((1, 3)));
        // A host without frames waits for the map.
        let cx4 = ToolContext { revision: (1, 4), ..cx(&d, &s, &p) };
        t.pointer(&cx4, &hover);
        assert!(t.waiting() && t.hover.is_none());
        t.poll(&cx4, true);
        assert!(!t.waiting());
        assert!(matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
        assert_eq!((t.cache.builds, t.cache.background), (4, 4));
        // A press while the map is being built waits for it, and the drag merges.
        let cx5 = ToolContext { revision: (1, 5), ..cx(&d, &s, &p) };
        t.pointer(&cx5, &hover);
        t.pointer(&cx5, &ev(PointerKind::Down, 15.0, 10.0));
        t.pointer(&cx5, &ev(PointerKind::Drag, 35.0, 10.0));
        let acts = t.pointer(&cx5, &ev(PointerKind::Up, 35.0, 10.0));
        assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "shapeBuilder.merge"), "{acts:?}");
        assert_eq!((t.cache.builds, t.cache.background), (5, 5));
        assert!(!t.waiting());
        // Changing an option forgets a map being built with the old one.
        let cx6 = ToolContext { revision: (1, 6), ..cx(&d, &s, &p) };
        t.pointer(&cx6, &hover);
        t.set_option("gapDetection", &json!(true));
        assert!(!t.waiting());
    }

    #[test]
    fn shape_builder_hover_builds_a_small_map_at_once() {
        let (d, s) = square_row(3);
        let p = paint();
        let mut t = ShapeBuilderTool::default();
        let cx = cx(&d, &s, &p);
        t.pointer(&cx, &ev(PointerKind::Move, 15.0, 10.0));
        assert!(!t.waiting() && matches!(t.hover, Some(Hit::Region(_))), "{:?}", t.hover);
        assert_eq!((t.cache.builds, t.cache.background), (1, 0));
    }

    #[test]
    fn shape_builder_alt_drag_erases_and_empty_drag_does_nothing() {
        let (d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([a, b]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 125.0, 50.0).with_mods(alt));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 125.0, 50.0).with_mods(alt));
        let Action::Exec(_, v) = &acts[0] else { panic!() };
        assert_eq!(v["erase"], true);
        t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, 300.0, 300.0)).is_empty());
    }

    /// Squares A (0..100) and B (50..150 both ways), overlapping at a corner, selected.
    fn corner_rects() -> (Document, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let mut s = Selection::default();
        for (k, r) in [Rect::new(0.0, 0.0, 100.0, 100.0), Rect::new(50.0, 50.0, 150.0, 150.0)].into_iter().enumerate() {
            let id = d.alloc_id();
            d.insert(Some(l), k, Node::path(id, shapes::rectangle(r), Appearance::default_art())).unwrap();
            s.add(id);
        }
        (d, s)
    }

    fn colors(ov: &[Overlay]) -> Vec<[u8; 3]> {
        ov.iter().map(|o| if let Overlay::Path { color, .. } = o { *color } else { panic!("{o:?}") }).collect()
    }

    #[test]
    fn shape_builder_alt_switches_to_erase_mode_while_held() {
        let (d, s) = corner_rects();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        let at = Point::new(75.0, 75.0);
        // Merge mode: a grey mesh and the plus pointer.
        t.pointer(&cx, &ev(PointerKind::Move, at.x, at.y));
        assert_eq!(colors(&t.overlays(&cx)), [HATCH; 2]);
        assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::ShapeBuilder);
        // Alt held: what would go highlights red, and the pointer shows a minus.
        t.pointer(&cx, &ev(PointerKind::Move, at.x, at.y).with_mods(alt));
        assert_eq!(colors(&t.overlays(&cx)), [HIGHLIGHT_RED; 2]);
        assert_eq!(t.cursor(&cx, at, alt), Cursor::ShapeBuilderErase);
        // Released: merge mode again.
        t.pointer(&cx, &ev(PointerKind::Move, at.x, at.y));
        assert_eq!(colors(&t.overlays(&cx)), [HATCH; 2]);
        assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::ShapeBuilder);
        // A drag keeps the mode it started in.
        t.pointer(&cx, &ev(PointerKind::Down, at.x, at.y).with_mods(alt));
        assert_eq!(t.cursor(&cx, at, Mods::default()), Cursor::ShapeBuilderErase);
        let acts = t.pointer(&cx, &ev(PointerKind::Up, at.x, at.y));
        let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!(v["erase"], true);
    }

    #[test]
    fn shape_builder_alt_on_an_edge_highlights_and_deletes_it() {
        let (d, s) = corner_rects();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        // A's side inside B, between the points where B's outline crosses it.
        t.pointer(&cx, &ev(PointerKind::Move, 101.0, 75.0).with_mods(alt));
        assert!(matches!(t.hover, Some(Hit::Edge(_))), "{:?}", t.hover);
        let ov = t.overlays(&cx);
        let [Overlay::Path { path, color: HIGHLIGHT_RED, width: 3.0, .. }] = ov.as_slice() else { panic!("{ov:?}") };
        assert_eq!(path.bounding_box(), Rect::new(50.0, 50.0, 100.0, 100.0));
        // Without Alt it's the region under the pointer.
        t.pointer(&cx, &ev(PointerKind::Move, 101.0, 75.0));
        assert!(matches!(t.hover, Some(Hit::Region(_))));
        t.pointer(&cx, &ev(PointerKind::Down, 101.0, 75.0).with_mods(alt));
        assert_eq!(t.touched.edges.len(), 1);
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 101.0, 75.0).with_mods(alt));
        let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!((id.as_str(), &v["erase"], &v["tolerance"]), ("shapeBuilder.merge", &json!(true), &json!(4.0)));
    }

    #[test]
    fn shape_builder_erase_drag_into_regions_shows_the_regions_not_the_edges_it_crosses() {
        let (d, s) = corner_rects();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let alt = Mods { alt: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 25.0, 25.0).with_mods(alt));
        t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 75.0).with_mods(alt));
        assert_eq!(t.touched.regions.len(), 2);
        assert!(!t.touched.edges.is_empty(), "the drag crossed B's outline");
        assert!(t.cache.map.erased_edges(&t.touched).is_empty(), "which goes with the regions");
        // Two regions (mesh + outline each) and the drag's trail, all red.
        assert_eq!(colors(&t.overlays(&cx)), [HIGHLIGHT_RED; 5]);
    }

    /// A 3×3 grid of touching 100 pt squares from (0, 0) to (300, 300), selected.
    fn grid() -> (Document, Selection) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let mut s = Selection::default();
        for k in 0..9 {
            let (x, y) = ((k % 3) as f64 * 100.0, (k / 3) as f64 * 100.0);
            let id = d.alloc_id();
            d.insert(Some(l), k, Node::path(id, shapes::rectangle(Rect::new(x, y, x + 100.0, y + 100.0)), Appearance::default_art())).unwrap();
            s.add(id);
        }
        (d, s)
    }

    /// The paths of an overlay list (a shaded region draws two: its mesh and its outline), and its
    /// marquees.
    fn paths_and_marquees(ov: &[Overlay]) -> (usize, Vec<Rect>) {
        let paths = ov.iter().filter(|o| matches!(o, Overlay::Path { .. })).count();
        (paths, ov.iter().filter_map(|o| if let Overlay::Marquee(r) = o { Some(*r) } else { None }).collect())
    }

    #[test]
    fn shape_builder_shift_drag_highlights_and_merges_every_region_in_its_marquee() {
        let (d, s) = grid();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let shift = Mods { shift: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 250.0, 250.0).with_mods(shift));
        // Shift may be let go once the drag is on.
        t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 150.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 50.0, 50.0));
        assert_eq!(t.touched.regions.len(), 9, "{:?}", t.touched);
        assert_eq!(t.touched.regions[0], t.cache.map.region_at(Point::new(250.0, 250.0)).unwrap(), "where the drag started comes first");
        assert_eq!(paths_and_marquees(&t.overlays(&cx)), (18, vec![Rect::new(50.0, 50.0, 250.0, 250.0)]), "nine regions, no trail");
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0));
        let Action::Exec(id, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!((id.as_str(), &v["rect"], &v["erase"]), ("shapeBuilder.merge", &json!([250.0, 250.0, -200.0, -200.0]), &json!(false)));
        assert!(v.get("points").is_none(), "{v}");
        assert!(t.overlays(&cx).iter().all(|o| !matches!(o, Overlay::Marquee(_))), "the marquee goes with the drag");
    }

    #[test]
    fn shape_builder_alt_shift_drag_marks_the_regions_in_its_marquee_for_deleting() {
        let (d, s) = grid();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let alt_shift = Mods { alt: true, shift: true, ..Default::default() };
        // A narrow marquee down the middle column.
        t.pointer(&cx, &ev(PointerKind::Down, 140.0, 50.0).with_mods(alt_shift));
        t.pointer(&cx, &ev(PointerKind::Drag, 160.0, 250.0).with_mods(alt_shift));
        assert_eq!(t.touched.regions.len(), 3);
        let ov = t.overlays(&cx);
        assert!(ov.iter().all(|o| matches!(o, Overlay::Marquee(_) | Overlay::Path { color: HIGHLIGHT_RED, .. })), "{ov:?}");
        assert_eq!(paths_and_marquees(&ov).0, 6, "three regions");
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 160.0, 250.0).with_mods(alt_shift));
        let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!((&v["rect"], &v["erase"]), (&json!([140.0, 50.0, 20.0, 200.0]), &json!(true)));
    }

    /// Without Shift a drag still touches what the pointer passes over; a Shift-click that does not
    /// move is a click.
    #[test]
    fn shape_builder_drag_without_shift_and_shift_click_send_points() {
        let (d, s) = grid();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        t.pointer(&cx, &ev(PointerKind::Down, 250.0, 250.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 150.0, 150.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 50.0, 50.0));
        assert_eq!(t.touched.regions.len(), 3, "the diagonal");
        let (paths, marquees) = paths_and_marquees(&t.overlays(&cx));
        assert_eq!((paths, marquees.len()), (7, 0), "three regions and the trail");
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 50.0, 50.0));
        let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!(v["points"], json!([[250.0, 250.0], [150.0, 150.0], [50.0, 50.0]]));
        assert!(v.get("rect").is_none(), "{v}");
        let shift = Mods { shift: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Down, 150.0, 150.0).with_mods(shift));
        assert_eq!(t.touched.regions.len(), 1);
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 150.0, 150.0).with_mods(shift));
        let Action::Exec(_, v) = &acts[0] else { panic!("{acts:?}") };
        assert_eq!((&v["points"], v.get("rect")), (&json!([[150.0, 150.0]]), None));
    }

    #[test]
    fn marquee_regions_touch_crossed_outlines_held_anchors_and_the_region_around_it() {
        let (d, s) = grid();
        let (map, _) = BuilderMap::new(&d, &sorted_roots(&d, &s.objects), false, true);
        let at = |x: f64, y: f64| map.region_at(Point::new(x, y)).unwrap();
        // Wholly inside the middle square, then crossing into the one to its right.
        assert_eq!(map.regions_in_marquee(Point::new(120.0, 120.0), Point::new(180.0, 180.0)), [at(150.0, 150.0)]);
        assert_eq!(map.regions_in_marquee(Point::new(120.0, 120.0), Point::new(220.0, 180.0)), [at(150.0, 150.0), at(250.0, 150.0)]);
        // Ending on a shared side does not reach the square past it.
        assert_eq!(map.regions_in_marquee(Point::new(120.0, 120.0), Point::new(200.0, 180.0)), [at(150.0, 150.0)]);
        // Around the whole grid, from outside it: every square, with no region under either corner.
        assert_eq!(map.regions_in_marquee(Point::new(-10.0, -10.0), Point::new(310.0, 310.0)).len(), 9);
        assert!(map.regions_in_marquee(Point::new(400.0, 400.0), Point::new(450.0, 450.0)).is_empty());
        // Where the drag started first, then where it ends, whichever way it runs.
        for (a, b) in [((50.0, 50.0), (250.0, 250.0)), ((250.0, 250.0), (50.0, 50.0)), ((250.0, 50.0), (50.0, 250.0))] {
            let got = map.regions_in_marquee(a.into(), b.into());
            assert_eq!((got.len(), got[0], got[1]), (9, at(a.0, a.1), at(b.0, b.1)), "{a:?} to {b:?}");
        }
    }

    /// With In Merge Mode, Clicking Stroke Splits the Path on, a Shift-drag that starts on a stroke
    /// sends its marquee and no split, however little it moves; a Shift-click there splits the
    /// stroke as a click does.
    #[test]
    fn shape_builder_shift_drag_on_a_stroke_merges_and_a_shift_click_splits() {
        let (d, s) = corner_rects();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        t.set_option("clickStrokeSplitsPath", &json!(true));
        let shift = Mods { shift: true, ..Default::default() };
        // 1 pt outside A's left side, a move of less than 3 pixels, and one across the art.
        for (to, rect) in [((0.5, 26.0), json!([-1.0, 25.0, 1.5, 1.0])), ((75.0, 75.0), json!([-1.0, 25.0, 76.0, 50.0]))] {
            t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
            t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0).with_mods(shift));
            t.pointer(&cx, &ev(PointerKind::Drag, to.0, to.1).with_mods(shift));
            let acts = t.pointer(&cx, &ev(PointerKind::Up, to.0, to.1).with_mods(shift));
            let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
            assert_eq!((&v["rect"], v.get("points"), v.get("splitStroke")), (&rect, None, None), "{v}");
        }
        t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0).with_mods(shift));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, -1.0, 25.0).with_mods(shift));
        let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
        assert_eq!((&v["splitStroke"], &v["points"], v.get("rect")), (&json!(true), &json!([[-1.0, 25.0]]), None), "{v}");
    }

    /// While a Shift-drag that started over a stroke a click would split draws its marquee, the
    /// pointer shows the Shape Builder, not the scissors, and the stroke is not highlighted.
    #[test]
    fn shape_builder_shift_drag_shows_no_scissors_or_stroke_highlight() {
        let (d, s) = corner_rects();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        t.set_option("clickStrokeSplitsPath", &json!(true));
        let shift = Mods { shift: true, ..Default::default() };
        let highlighted = |ov: &[Overlay]| ov.iter().any(|o| matches!(o, Overlay::Path { color: STROKE_HIGHLIGHT, width: 3.0, .. }));
        t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
        assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), shift), Cursor::Scissors);
        assert!(highlighted(&t.overlays(&cx)));
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0).with_mods(shift));
        t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 75.0).with_mods(shift));
        let ov = t.overlays(&cx);
        assert!(ov.iter().any(|o| matches!(o, Overlay::Marquee(_))) && !highlighted(&ov), "{ov:?}");
        assert_eq!(t.cursor(&cx, Point::new(75.0, 75.0), shift), Cursor::ShapeBuilder);
    }

    #[test]
    fn shape_builder_stroke_options_round_trip_and_pick_the_side_under_the_pointer() {
        let (d, s) = corner_rects();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = ShapeBuilderTool::default();
        let o = t.options();
        assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(false), &json!(true), &json!("#ff4f4f")));
        // With the option off, the pointer 1 pt outside A's left side highlights nothing.
        t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
        assert!(t.overlays(&cx).is_empty());
        assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), Mods::default()), Cursor::ShapeBuilder);
        t.set_option("clickStrokeSplitsPath", &json!(true));
        t.set_option("highlightColor", &json!("#00ff00"));
        // A value that is not a color or not a switch is ignored.
        t.set_option("highlightColor", &json!("nope"));
        t.set_option("highlightStroke", &json!("off"));
        let o = t.options();
        assert_eq!((&o["clickStrokeSplitsPath"], &o["highlightStroke"], &o["highlightColor"]), (&json!(true), &json!(true), &json!("#00ff00")));
        // On, the tool draws A's left side from corner to corner in the highlight color and shows the scissors.
        t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0));
        let ov = t.overlays(&cx);
        let [Overlay::Path { path, color: [0, 0xff, 0], width: 3.0, .. }] = ov.as_slice() else { panic!("{ov:?}") };
        assert_eq!(path.bounding_box(), Rect::new(0.0, 0.0, 0.0, 100.0));
        assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), Mods::default()), Cursor::Scissors);
        // With Alt the tool is in erase mode, which deletes edges and does not split them.
        let alt = Mods { alt: true, ..Default::default() };
        t.pointer(&cx, &ev(PointerKind::Move, -1.0, 25.0).with_mods(alt));
        assert_eq!(t.cursor(&cx, Point::new(-1.0, 25.0), alt), Cursor::ShapeBuilderErase);
        assert!(t.stroke.is_none());
        // A click splits; the command gets the point, the switch and the tolerance.
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, -1.0, 25.0));
        let [Action::Exec(id, v)] = acts.as_slice() else { panic!("{acts:?}") };
        assert_eq!((id.as_str(), &v["splitStroke"], &v["erase"], &v["tolerance"]), ("shapeBuilder.merge", &json!(true), &json!(false), &json!(4.0)));
        // A release up to 3 pixels from the press is still a click, which splits where the press was.
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
        t.pointer(&cx, &ev(PointerKind::Drag, -1.5, 26.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 0.5, 27.0));
        let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
        assert_eq!((&v["splitStroke"], &v["points"]), (&json!(true), &json!([[-1.0, 25.0]])));
        // A release farther away is a drag, which splits nothing.
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
        assert!(t.pointer(&cx, &ev(PointerKind::Up, -1.0, 30.0)).is_empty());
        // So is a Straight Line drag that goes away and ends back near the press.
        t.set_option("selection", &json!("straightLine"));
        t.pointer(&cx, &ev(PointerKind::Down, -1.0, 25.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 30.0, 25.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, -0.5, 25.5));
        assert!(!acts.iter().any(|a| matches!(a, Action::Exec(_, v) if v.get("splitStroke").is_some())), "{acts:?}");
        t.set_option("selection", &json!("freeform"));
        // A drag merges as before and sends no switch.
        t.pointer(&cx, &ev(PointerKind::Down, 25.0, 25.0));
        t.pointer(&cx, &ev(PointerKind::Drag, 75.0, 75.0));
        let acts = t.pointer(&cx, &ev(PointerKind::Up, 75.0, 75.0));
        let [Action::Exec(_, v)] = acts.as_slice() else { panic!("{acts:?}") };
        assert!(v.get("splitStroke").is_none(), "{v}");
    }

    #[test]
    fn hatch_stays_inside_region() {
        let bp = shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)).to_bezpath();
        let h = hatch(&bp, 4.0);
        let bb = h.bounding_box();
        assert!(bb.x0 >= -1e-6 && bb.y0 >= -1e-6 && bb.x1 <= 40.0 + 1e-6 && bb.y1 <= 20.0 + 1e-6);
        assert!(h.elements().len() > 10);
    }

    /// An empty hidden sources group for a Live Paint group made in a test.
    fn empty_sources(d: &mut Document) -> Arc<Node> {
        let mut g = Node::group(d.alloc_id(), vec![]);
        g.name = Some(SOURCES_NAME.into());
        g.visible = false;
        Arc::new(g)
    }

    fn live_group_doc() -> (Document, NodeId, NodeId, NodeId) {
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let f1 = d.alloc_id();
        let f2 = d.alloc_id();
        let e = d.alloc_id();
        let mut face1 = Node::path(f1, shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 50.0)), Appearance::default());
        face1.name = Some(FACE_NAME.into());
        let mut face2 = Node::path(f2, shapes::rectangle(Rect::new(50.0, 0.0, 100.0, 50.0)), Appearance::default());
        face2.name = Some(FACE_NAME.into());
        let mut edge = Node::path(e, shapes::line(Point::new(50.0, 0.0), Point::new(50.0, 50.0)), Appearance::default());
        edge.name = Some(EDGE_NAME.into());
        let gid = d.alloc_id();
        let mut g = Node::group(gid, vec![empty_sources(&mut d), Arc::new(face1), Arc::new(face2), Arc::new(edge)]);
        g.name = Some(LIVE_PAINT_NAME.into());
        d.insert(Some(l), 0, g).unwrap();
        (d, gid, f2, e)
    }

    /// Adds a Live Paint group of one face `r` with its four sides as edges (left, top, right,
    /// bottom) in front of the layer's art; returns the group and the edges.
    fn live_square(d: &mut Document, r: Rect) -> (NodeId, [NodeId; 4]) {
        let l = d.layers[0].id;
        let mut face = Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::default());
        face.name = Some(FACE_NAME.into());
        let (a, b, c, e) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
        let mut kids = vec![empty_sources(d), Arc::new(face)];
        let mut ids = [NodeId(0); 4];
        for (k, (from, to)) in [(e, a), (a, b), (b, c), (c, e)].into_iter().enumerate() {
            ids[k] = d.alloc_id();
            let mut edge = Node::path(ids[k], shapes::line(from, to), Appearance::default());
            edge.name = Some(EDGE_NAME.into());
            kids.push(Arc::new(edge));
        }
        let gid = d.alloc_id();
        let mut g = Node::group(gid, kids);
        g.name = Some(LIVE_PAINT_NAME.into());
        let at = d.layers[0].children().map_or(0, |c| c.len());
        d.insert(Some(l), at, g).unwrap();
        (gid, ids)
    }

    #[test]
    fn bucket_click_fills_face_and_shift_click_strokes_edge() {
        let (d, gid, f2, _e) = live_group_doc();
        assert_eq!(live_paint_at(&d, Point::new(75.0, 25.0)), Some(gid));
        assert_eq!(face_at(d.node(gid).unwrap(), Point::new(75.0, 25.0)).unwrap().id, f2);
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = LivePaintBucketTool::default();
        t.pointer(&cx, &ev(PointerKind::Move, 75.0, 25.0));
        let ov = t.overlays(&cx);
        assert!(matches!(&ov[0], Overlay::Path { color: HIGHLIGHT_RED, .. }));
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0));
        let Action::Exec(id, v) = &acts[0] else { panic!() };
        assert_eq!(id, "livePaint.fill");
        assert_eq!(v["group"], gid.0);
        let shift = Mods { shift: true, ..Default::default() };
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(shift));
        let Action::Exec(id, _) = &acts[0] else { panic!() };
        assert_eq!(id, "livePaint.strokeEdge");
    }

    #[test]
    fn bucket_on_plain_selection_makes_live_paint() {
        let (d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([a, b]);
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = LivePaintBucketTool::default();
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 75.0, 50.0));
        let Action::Exec(id, v) = &acts[0] else { panic!() };
        assert_eq!(id, "livePaint.fill");
        assert_eq!(v["ids"], json!([a.0, b.0]));
    }

    #[test]
    fn bucket_rebuilds_its_map_for_a_new_revision_at_the_same_address() {
        let (mut d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([a, b]);
        let p = paint();
        let mut t = LivePaintBucketTool::default();
        let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 50.0));
        // A later revision of the document, with the squares 300 pt to the right, at the same address.
        d = two_rects_at(300.0).0;
        let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
        let acts = t.pointer(&cx2, &ev(PointerKind::Down, 375.0, 50.0));
        assert_eq!(acts, vec![Action::Exec("livePaint.fill".into(), json!({ "ids": [a.0, b.0], "point": [375.0, 50.0] }))]);
    }

    #[test]
    fn bucket_keeps_its_map_while_the_document_is_unchanged() {
        let (d, a, b) = two_rects();
        let mut s = Selection::default();
        s.set([a, b]);
        let p = paint();
        let mut t = LivePaintBucketTool::default();
        let cx1 = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        t.pointer(&cx1, &ev(PointerKind::Move, 75.0, 50.0));
        let acts = t.pointer(&cx1, &ev(PointerKind::Down, 75.0, 50.0));
        assert!(matches!(acts.as_slice(), [Action::Exec(id, _)] if id == "livePaint.fill"), "{acts:?}");
        // The fill left the document as it was: the map still fits it.
        t.pointer(&cx1, &ev(PointerKind::Move, 25.0, 50.0));
        assert_eq!(t.cache.builds, 1, "a hover after a fill that changed nothing");
        let cx2 = ToolContext { revision: (1, 2), ..cx(&d, &s, &p) };
        t.pointer(&cx2, &ev(PointerKind::Move, 25.0, 50.0));
        assert_eq!(t.cache.builds, 2, "a hover on a new revision");
    }

    #[test]
    fn bucket_hover_builds_a_large_map_on_another_thread() {
        let (d, s) = square_row(40);
        let p = paint();
        let mut t = LivePaintBucketTool::default();
        let cx = ToolContext { revision: (1, 1), ..cx(&d, &s, &p) };
        let hover = ev(PointerKind::Move, 15.0, 10.0);
        t.pointer(&cx, &hover);
        assert_eq!(t.cache.builds, 0, "the first hover returns before the map is built");
        assert!(t.waiting() && t.hover == LpHover::None, "{:?}", t.hover);
        poll_until_ready(&mut t, &cx);
        assert!(matches!(t.hover, LpHover::Region(_)), "{:?}", t.hover);
        assert_eq!((t.cache.builds, t.cache.background), (1, 1));
        assert_eq!(t.overlays(&cx).len(), 1);
    }

    #[test]
    fn bucket_takes_the_map_it_waits_for_while_hovering_a_live_paint_group() {
        let (mut d, s) = square_row(40);
        live_square(&mut d, Rect::new(100.0, 100.0, 200.0, 200.0));
        let p = paint();
        let mut t = LivePaintBucketTool::default();
        let cx = cx(&d, &s, &p);
        t.pointer(&cx, &ev(PointerKind::Move, 15.0, 10.0));
        assert!(t.waiting());
        t.pointer(&cx, &ev(PointerKind::Move, 150.0, 150.0));
        assert!(matches!(t.hover, LpHover::Part(_)), "{:?}", t.hover);
        poll_until_ready(&mut t, &cx);
        assert!(matches!(t.hover, LpHover::Part(_)), "{:?}", t.hover);
        assert_eq!((t.cache.builds, t.cache.background), (1, 1));
    }

    #[test]
    fn live_paint_selection_hover_builds_no_map_of_the_selection() {
        let (d, s) = square_row(3);
        let p = paint();
        let mut t = LivePaintSelectionTool::default();
        let cx = cx(&d, &s, &p);
        t.pointer(&cx, &ev(PointerKind::Move, 15.0, 10.0));
        assert_eq!(t.hover, LpHover::None);
        assert_eq!(t.cache.builds, 0);
        assert!(!t.waiting());
    }

    #[test]
    fn live_paint_selection_selects_faces() {
        let (d, _gid, f2, e) = live_group_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = LivePaintSelectionTool::default();
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0));
        assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [f2.0] }))]);
        let alt = Mods { alt: true, ..Default::default() };
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(alt));
        assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [e.0] }))]);
        let acts = t.pointer(&cx, &ev(PointerKind::Down, 300.0, 300.0));
        assert_eq!(acts, vec![Action::Exec("select.none".into(), json!({}))]);
    }

    #[test]
    fn live_paint_selection_options_choose_what_a_click_selects() {
        let (d, _gid, f2, e) = live_group_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let set = |ids: &[NodeId]| vec![Action::Exec("select.set".into(), json!({ "ids": ids.iter().map(|i| i.0).collect::<Vec<_>>() }))];
        let none = vec![Action::Exec("select.none".into(), json!({}))];
        let mut t = LivePaintSelectionTool::default();
        assert_eq!(t.options(), json!({"selectFills": true, "selectStrokes": true, "color": "#ff2a2a", "width": 4.0}));
        // Select Fills off: a click inside a face selects nothing, and a click by an edge selects it.
        t.set_option("selectFills", &json!(false));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0)), none);
        t.pointer(&cx, &ev(PointerKind::Move, 75.0, 25.0));
        assert!(t.overlays(&cx).is_empty());
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0)), set(&[e]));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(alt)), set(&[e]));
        // Select Strokes off: Alt-click by an edge selects the face under the pointer.
        t.set_option("selectFills", &json!(true));
        t.set_option("selectStrokes", &json!(false));
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(alt)), set(&[f2]));
        t.pointer(&cx, &ev(PointerKind::Move, 51.0, 25.0).with_mods(alt));
        let ov = t.overlays(&cx);
        let [Overlay::Path { path, .. }] = ov.as_slice() else { panic!("{ov:?}") };
        assert_eq!(path.bounding_box(), Rect::new(50.0, 0.0, 100.0, 50.0), "the face, not the edge");
        // Both off: nothing is selected or highlighted.
        t.set_option("selectFills", &json!(false));
        for m in [Mods::default(), alt] {
            assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 51.0, 25.0).with_mods(m)), none);
            t.pointer(&cx, &ev(PointerKind::Move, 51.0, 25.0).with_mods(m));
            assert!(t.overlays(&cx).is_empty());
        }
        let shift = Mods { shift: true, ..Default::default() };
        assert!(t.pointer(&cx, &ev(PointerKind::Down, 75.0, 25.0).with_mods(shift)).is_empty());
    }

    #[test]
    fn live_paint_selection_click_outside_the_faces_selects_the_edge_it_highlights() {
        let mut d = Document::new(500.0, 500.0);
        let (_, [_, _, right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let mut t = LivePaintSelectionTool::default();
        // Alt held, and Select Fills off without Alt: 2 pt right of the square, the pointer
        // highlights the square's right side, and a click selects that side (Shift-click adds it).
        for (fills, m) in [(true, alt), (false, Mods::default())] {
            t.set_option("selectFills", &json!(fills));
            t.pointer(&cx, &ev(PointerKind::Move, 102.0, 50.0).with_mods(m));
            let ov = t.overlays(&cx);
            let [Overlay::Path { path, .. }] = ov.as_slice() else { panic!("{ov:?}") };
            assert_eq!(path.bounding_box(), Rect::new(100.0, 0.0, 100.0, 100.0));
            let acts = t.pointer(&cx, &ev(PointerKind::Down, 102.0, 50.0).with_mods(m));
            assert_eq!(acts, vec![Action::Exec("select.set".into(), json!({ "ids": [right.0] }))]);
            let acts = t.pointer(&cx, &ev(PointerKind::Down, 102.0, 50.0).with_mods(Mods { shift: true, ..m }));
            assert_eq!(acts, vec![Action::Exec("select.toggle".into(), json!({ "id": right.0 }))]);
        }
        // Farther than 4 pixels from every edge: no highlight, and a click selects nothing.
        t.pointer(&cx, &ev(PointerKind::Move, 106.0, 50.0));
        assert!(t.overlays(&cx).is_empty());
        assert_eq!(t.pointer(&cx, &ev(PointerKind::Down, 106.0, 50.0)), vec![Action::Exec("select.none".into(), json!({}))]);
    }

    #[test]
    fn live_paint_edges_outside_the_faces_come_from_the_nearest_visible_group() {
        let mut d = Document::new(500.0, 500.0);
        let (back, [_, _, back_right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
        let (front, [front_left, ..]) = live_square(&mut d, Rect::new(104.0, 0.0, 204.0, 100.0));
        let s = Selection::default();
        let p = paint();
        let alt = Mods { alt: true, ..Default::default() };
        let shift = Mods { shift: true, ..Default::default() };
        // 1.5 pt from the back group's right side and 2.5 pt from the front group's left side.
        let (x, y) = (101.5, 50.0);
        let edge_x = |ov: &[Overlay]| match ov {
            [] => None,
            [Overlay::Path { path, .. }] => Some(path.bounding_box().x0),
            _ => panic!("{ov:?}"),
        };
        // Live Paint Selection (Alt) and Live Paint Bucket (Shift) highlight and pick the same edge.
        let check = |d: &Document, want: Option<(NodeId, NodeId, f64)>| {
            let cx = cx(d, &s, &p);
            let mut sel = LivePaintSelectionTool::default();
            sel.pointer(&cx, &ev(PointerKind::Move, x, y).with_mods(alt));
            assert_eq!(edge_x(&sel.overlays(&cx)), want.map(|w| w.2));
            let picked = match want {
                Some((_, e, _)) => Action::Exec("select.set".into(), json!({ "ids": [e.0] })),
                None => Action::Exec("select.none".into(), json!({})),
            };
            assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, x, y).with_mods(alt)), vec![picked]);
            let mut bucket = LivePaintBucketTool::default();
            bucket.pointer(&cx, &ev(PointerKind::Move, x, y).with_mods(shift));
            assert_eq!(edge_x(&bucket.overlays(&cx)), want.map(|w| w.2));
            let stroked = want.map(|(g, ..)| Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": [x, y], "tolerance": 4.0 })));
            assert_eq!(bucket.pointer(&cx, &ev(PointerKind::Down, x, y).with_mods(shift)), stroked.into_iter().collect::<Vec<_>>());
        };
        check(&d, Some((back, back_right, 100.0)));
        // A hidden group's edges are not highlighted or picked.
        d.node_mut(back).unwrap().visible = false;
        check(&d, Some((front, front_left, 104.0)));
        d.node_mut(front).unwrap().visible = false;
        check(&d, None);
    }

    #[test]
    fn live_paint_tools_skip_hidden_edges() {
        let mut d = Document::new(500.0, 500.0);
        let (_, [_, _, back_right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
        let (front, [front_left, ..]) = live_square(&mut d, Rect::new(104.0, 0.0, 204.0, 100.0));
        // The back group's right side is hidden on its own; both groups stay visible.
        d.node_mut(back_right).unwrap().visible = false;
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let shift = Mods { shift: true, ..Default::default() };
        let edge_x = |ov: &[Overlay]| match ov {
            [] => None,
            [Overlay::Path { path, .. }] => Some(path.bounding_box().x0),
            _ => panic!("{ov:?}"),
        };
        // Outside the faces, 1.5 pt from the hidden side and 2.5 pt from the front group's left
        // side, both tools highlight and pick the visible side. Inside the back face, 1.5 pt from
        // the hidden side, no edge is in reach, so neither tool highlights or picks one.
        for (x, want) in [(101.5, Some((front, front_left, 104.0))), (98.5, None)] {
            let mut sel = LivePaintSelectionTool::default();
            sel.pointer(&cx, &ev(PointerKind::Move, x, 50.0).with_mods(alt));
            assert_eq!(edge_x(&sel.overlays(&cx)), want.map(|w| w.2), "x {x}");
            let picked = match want {
                Some((_, e, _)) => Action::Exec("select.set".into(), json!({ "ids": [e.0] })),
                None => Action::Exec("select.none".into(), json!({})),
            };
            assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, x, 50.0).with_mods(alt)), vec![picked], "x {x}");
            let mut bucket = LivePaintBucketTool::default();
            bucket.pointer(&cx, &ev(PointerKind::Move, x, 50.0).with_mods(shift));
            assert_eq!(edge_x(&bucket.overlays(&cx)), want.map(|w| w.2), "x {x}");
            let stroked =
                want.map(|(g, ..)| Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": [x, 50.0], "tolerance": 4.0 })));
            assert_eq!(bucket.pointer(&cx, &ev(PointerKind::Down, x, 50.0).with_mods(shift)), stroked.into_iter().collect::<Vec<_>>(), "x {x}");
        }
    }

    #[test]
    fn live_paint_tools_skip_hidden_faces() {
        let mut d = Document::new(500.0, 500.0);
        let (back, [_, _, back_right, _]) = live_square(&mut d, Rect::new(0.0, 0.0, 100.0, 100.0));
        let (front, _) = live_square(&mut d, Rect::new(50.0, 0.0, 150.0, 100.0));
        let face = |g: NodeId| faces(d.node(g).unwrap())[0].id;
        let (back_face, hidden) = (face(back), face(front));
        // The front group's face is hidden on its own; both groups stay visible.
        d.node_mut(hidden).unwrap().visible = false;
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let alt = Mods { alt: true, ..Default::default() };
        let part = |ov: &[Overlay]| match ov {
            [] => None,
            [Overlay::Path { path, .. }] => Some(path.bounding_box()),
            _ => panic!("{ov:?}"),
        };
        // Where the faces overlap, both tools take the back group's face under the hidden one.
        let mut bucket = LivePaintBucketTool::default();
        bucket.pointer(&cx, &ev(PointerKind::Move, 75.0, 50.0));
        assert_eq!(part(&bucket.overlays(&cx)), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
        let fill = Action::Exec("livePaint.fill".into(), json!({ "group": back.0, "point": [75.0, 50.0] }));
        assert_eq!(bucket.pointer(&cx, &ev(PointerKind::Down, 75.0, 50.0)), vec![fill]);
        let mut sel = LivePaintSelectionTool::default();
        sel.pointer(&cx, &ev(PointerKind::Move, 75.0, 50.0));
        assert_eq!(part(&sel.overlays(&cx)), Some(Rect::new(0.0, 0.0, 100.0, 100.0)));
        assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, 75.0, 50.0)), vec![Action::Exec("select.set".into(), json!({ "ids": [back_face.0] }))]);
        // Over the hidden face alone, neither tool highlights or picks a face.
        bucket.pointer(&cx, &ev(PointerKind::Move, 125.0, 50.0));
        assert_eq!(part(&bucket.overlays(&cx)), None);
        assert!(bucket.pointer(&cx, &ev(PointerKind::Down, 125.0, 50.0)).is_empty());
        sel.pointer(&cx, &ev(PointerKind::Move, 125.0, 50.0));
        assert_eq!(part(&sel.overlays(&cx)), None);
        assert_eq!(sel.pointer(&cx, &ev(PointerKind::Down, 125.0, 50.0)), vec![Action::Exec("select.none".into(), json!({}))]);
        // Inside the hidden face, 1.5 pt from the back group's right side, Alt-click takes that side
        // as it does outside every face.
        sel.pointer(&cx, &ev(PointerKind::Move, 101.5, 50.0).with_mods(alt));
        assert_eq!(part(&sel.overlays(&cx)), Some(Rect::new(100.0, 0.0, 100.0, 100.0)));
        let picked = sel.pointer(&cx, &ev(PointerKind::Down, 101.5, 50.0).with_mods(alt));
        assert_eq!(picked, vec![Action::Exec("select.set".into(), json!({ "ids": [back_right.0] }))]);
    }

    #[test]
    fn live_paint_selection_options_set_the_highlight() {
        let (d, ..) = live_group_doc();
        let s = Selection::default();
        let p = paint();
        let cx = cx(&d, &s, &p);
        let mut t = LivePaintSelectionTool::default();
        let highlight = |t: &mut LivePaintSelectionTool| {
            t.pointer(&cx, &ev(PointerKind::Move, 75.0, 25.0));
            match t.overlays(&cx).as_slice() {
                [Overlay::Path { color, width, .. }] => (*color, *width),
                ov => panic!("{ov:?}"),
            }
        };
        assert_eq!(highlight(&mut t), (HIGHLIGHT_RED, 4.0));
        t.set_option("color", &json!("#00FF00"));
        t.set_option("width", &json!(9));
        assert_eq!(highlight(&mut t), ([0, 0xff, 0], 9.0));
        // A layer color's name; a value that is not a color, or a width that is not a number, is ignored.
        t.set_option("color", &json!("light blue"));
        t.set_option("width", &json!("wide"));
        assert_eq!(highlight(&mut t), ([0x4f, 0x80, 0xff], 9.0));
        t.set_option("color", &json!("nope"));
        assert_eq!(t.options()["color"], "#4f80ff");
        // The width stays within its range.
        t.set_option("width", &json!(100));
        assert_eq!(t.options()["width"], json!(LP_SELECT_WIDTH_RANGE.1));
        t.set_option("width", &json!(0));
        assert_eq!(t.options()["width"], json!(LP_SELECT_WIDTH_RANGE.0));
    }

    #[test]
    fn create_builds_the_three_tools() {
        for id in ["shapeBuilder", "livePaintBucket", "livePaintSelection"] {
            assert_eq!(create(id).unwrap().id(), id);
        }
        assert!(create("pen").is_none());
    }
}
