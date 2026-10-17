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

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Paint, Swatch};
use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathData, Point, Rect, Shape as _};
use vectorcraft_pathops as po;

use crate::{Action, Cursor, Mods, Overlay, PointerEvent, PointerKind, Tool, ToolContext, ToolKey, json_ids};

/// Names starting with this mark a Live Paint group.
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

/// Is `n` a Live Paint group?
pub fn is_live_paint(n: &Node) -> bool {
    matches!(n.kind, NodeKind::Group { clip: false, .. }) && n.name.as_deref().is_some_and(|s| s.starts_with(LIVE_PAINT_PREFIX))
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
    faces(group).into_iter().rev().find(|f| f.path_data().is_some_and(|pd| path_contains(pd, FillRule::NonZero, p)))
}

/// The edge of `group` nearest to `p` within `tol`.
pub fn edge_near(group: &Node, p: Point, tol: f64) -> Option<&Arc<Node>> {
    let mut best: Option<(f64, &Arc<Node>)> = None;
    for e in edges(group) {
        if let Some((.., d)) = e.path_data().and_then(|pd| pd.nearest(p))
            && d <= tol
            && best.is_none_or(|b| d < b.0)
        {
            best = Some((d, e));
        }
    }
    best.map(|b| b.1)
}

/// The visible Live Paint group with a face under `p` (front-most in paint order).
pub fn live_paint_at(doc: &Document, p: Point) -> Option<NodeId> {
    let mut hit = None;
    doc.walk(|n| {
        if is_live_paint(n) && doc.is_visible(n.id) && face_at(n, p).is_some() {
            hit = Some(n.id);
        }
    });
    hit
}

/// Paths, compound paths (as one shape) of a subtree in paint order; guides, clipping paths and
/// hidden objects are skipped. Live Paint groups contribute their faces.
pub fn leaf_shapes<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
    if !n.visible {
        return;
    }
    if n.extras().shaper.is_some() {
        for c in n.children().into_iter().flatten().skip(1) {
            leaf_shapes(c, out);
        }
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } => out.push(n),
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                leaf_shapes(c, out);
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

/// Pathfinder shapes for the given roots (back → front by the order given), keyed by leaf index,
/// with the leaves they came from.
pub fn shapes_for<'a>(doc: &'a Document, roots: &[NodeId]) -> (Vec<po::Shape>, Vec<&'a Node>) {
    let mut lv = vec![];
    for id in roots {
        if let Some(n) = doc.node(*id) {
            leaf_shapes(n, &mut lv);
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
/// they cross, and so do filled open paths without `open_filled_as_closed`.
fn shape_builder_shapes<'a>(doc: &'a Document, roots: &[NodeId], open_filled_as_closed: bool) -> (Vec<po::Shape>, Vec<&'a Node>) {
    let (mut shapes, leaves) = shapes_for(doc, roots);
    for (s, l) in shapes.iter_mut().zip(&leaves) {
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
    /// edges only `with_edges` (they cost a planar map of their own when no open path is in). With
    /// `open_filled_as_closed` (Consider Open Filled Paths as Closed), a filled open path bounds a
    /// region as if an invisible edge closed it; without it, the path cuts like a line.
    pub fn new<'a>(doc: &'a Document, roots: &[NodeId], with_edges: bool, open_filled_as_closed: bool) -> (Self, Vec<&'a Node>) {
        let (shapes, leaves) = shape_builder_shapes(doc, roots, open_filled_as_closed);
        if shapes.is_empty() {
            return (Self::default(), leaves);
        }
        let arr = po::shape_builder(&shapes, with_edges);
        let bounded = |pieces: Vec<po::Shape>| {
            pieces
                .into_iter()
                .filter_map(|l| {
                    let b = l.path.bounds()?;
                    Some((l, b))
                })
                .collect()
        };
        (Self { shapes, regions: faces_of(arr.regions), lines: bounded(arr.lines), edges: bounded(arr.edges) }, leaves)
    }

    /// The region containing `p`.
    pub fn region_at(&self, p: Point) -> Option<usize> {
        self.regions.iter().position(|(_, bp, b)| b.contains(p) && bp.winding(p) != 0)
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

    /// Does line piece `i` run between two of `regions` (an edge that merging or deleting them
    /// removes)?
    pub fn line_between(&self, i: usize, regions: &[usize]) -> bool {
        self.lines.get(i).is_some_and(|(l, _)| self.sides(l).iter().all(|r| r.is_some_and(|r| regions.contains(&r))))
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

/// The planar map of the current selection, cached by document identity, selection and whether
/// filled open paths count as closed.
#[derive(Default)]
struct RegionCache {
    key: Option<(usize, Vec<NodeId>, bool)>,
    map: BuilderMap,
    /// Live Paint faces (every path an edge, filled or not) rather than Shape Builder regions.
    live: bool,
    /// The map has the Shape Builder's outline edges.
    edges: bool,
}

impl RegionCache {
    fn live_paint() -> Self {
        Self { live: true, ..Default::default() }
    }
    /// The map, with the outline edges when `edges` (built the first time they're wanted), and
    /// filled open paths closed when `open_filled_as_closed` (Shape Builder only).
    fn get(&mut self, cx: &ToolContext, edges: bool, open_filled_as_closed: bool) -> &BuilderMap {
        let key = (cx.doc as *const Document as usize, cx.selection.objects.clone(), open_filled_as_closed);
        if self.key.as_ref() != Some(&key) || (edges && !self.edges) {
            let roots = sorted_roots(cx.doc, &cx.selection.objects);
            self.map = if self.live {
                let (shapes, _) = shapes_for(cx.doc, &roots);
                let regions = if shapes.is_empty() { vec![] } else { faces_of(po::live_paint(&shapes).0) };
                BuilderMap { regions, ..Default::default() }
            } else {
                BuilderMap::new(cx.doc, &roots, edges, open_filled_as_closed).0
            };
            self.key = Some(key);
            self.edges = edges;
        }
        &self.map
    }
    fn at(&mut self, cx: &ToolContext, p: Point) -> Option<usize> {
        self.get(cx, false, true).region_at(p)
    }
}

// ---------- Shape Builder ----------

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
    /// art it is merged from.
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
}

impl Default for ShapeBuilderOptions {
    fn default() -> Self {
        Self { open_filled_as_closed: true, swatches: false, cursor_swatch_preview: false, straight_line: false, highlight_fill: true }
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
/// (outline pieces between the points where paths meet) it passes over. Its options are
/// [`ShapeBuilderOptions`].
#[derive(Default)]
pub struct ShapeBuilderTool {
    cache: RegionCache,
    hover: Option<Hit>,
    points: Vec<Point>,
    touched: Touched,
    dragging: bool,
    /// Erase mode: Alt held while hovering, or when the drag started.
    alt: bool,
    /// Where the pointer last moved (the cursor swatch preview sits above it).
    at: Option<Point>,
    /// The index in [`color_swatches`] of the swatch the arrow keys picked last.
    picked: Option<usize>,
    opts: ShapeBuilderOptions,
}

/// How close (screen px) the pointer must come to a line piece or an edge to delete it.
const LINE_TOL_PX: f64 = 4.0;

impl ShapeBuilderTool {
    fn line_tol(cx: &ToolContext, alt: bool) -> Option<f64> {
        alt.then(|| cx.tol(LINE_TOL_PX))
    }

    fn touch(&mut self, cx: &ToolContext, from: Option<Point>, to: Point) {
        let pts = match from {
            Some(a) => sample_polyline(&[a, to], cx.tol(2.0)),
            None => vec![to],
        };
        let tol = Self::line_tol(cx, self.alt);
        let map = self.cache.get(cx, self.alt, self.opts.open_filled_as_closed);
        for p in pts {
            self.touched.add(map.hit(p, tol));
        }
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
                self.hover = self.cache.get(cx, self.alt, self.opts.open_filled_as_closed).hit(p, Self::line_tol(cx, self.alt));
                vec![]
            }
            PointerKind::Down => {
                if cx.selection.is_empty() {
                    return vec![];
                }
                self.dragging = true;
                self.alt = ev.mods.alt;
                self.points = vec![p];
                self.touched = Touched::default();
                self.touch(cx, None, p);
                vec![]
            }
            PointerKind::Drag if self.dragging => {
                self.at = Some(p);
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
                if self.opts.straight_line {
                    self.straight_to(cx, p);
                } else if self.points.last().is_none_or(|q| *q != p) {
                    let last = self.points.last().copied();
                    self.touch(cx, last, p);
                    self.points.push(p);
                }
                let points = std::mem::take(&mut self.points);
                if std::mem::take(&mut self.touched).is_empty() {
                    return vec![];
                }
                self.cache.key = None;
                let roots = sorted_roots(cx.doc, &cx.selection.objects);
                let pts: Vec<Value> = points.iter().map(|q| json!([q.x, q.y])).collect();
                // The mode the drag started in, which its highlight showed.
                let erase = self.alt;
                let mut params = json!({ "ids": json_ids(&roots), "points": pts, "erase": erase });
                if let Some(t) = Self::line_tol(cx, erase) {
                    params["tolerance"] = json!(t);
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
        out
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, m: Mods) -> Cursor {
        // A drag keeps the mode it started in; hovering follows the key.
        let erase = if self.dragging { self.alt } else { m.alt };
        if erase { Cursor::ShapeBuilderErase } else { Cursor::ShapeBuilder }
    }

    fn busy(&self) -> bool {
        self.dragging
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
        json!({
            "openFilledAsClosed": o.open_filled_as_closed,
            "pickColorFrom": if o.swatches { "swatches" } else { "artwork" },
            "cursorSwatchPreview": o.cursor_swatch_preview,
            "selection": if o.straight_line { "straightLine" } else { "freeform" },
            "highlightFill": o.highlight_fill,
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
            _ => {}
        }
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

fn lp_hover(cx: &ToolContext, cache: &mut RegionCache, p: Point, edge: bool) -> LpHover {
    if let Some(g) = live_paint_at(cx.doc, p).and_then(|id| cx.doc.node(id)) {
        let part = if edge { edge_near(g, p, cx.tol(4.0)) } else { face_at(g, p) };
        return part.and_then(|n| n.path_data()).map(|pd| LpHover::Part(pd.to_bezpath())).unwrap_or_default();
    }
    if edge {
        // Edges of every Live Paint group near the pointer.
        let mut found = None;
        cx.doc.walk(|n| {
            if is_live_paint(n)
                && let Some(e) = edge_near(n, p, cx.tol(4.0))
            {
                found = e.path_data().map(|pd| pd.to_bezpath());
            }
        });
        if let Some(bp) = found {
            return LpHover::Part(bp);
        }
    }
    let sel_is_live = cx.selection.objects.iter().any(|id| cx.doc.node(*id).is_some_and(is_live_paint));
    if !sel_is_live && let Some(i) = cache.at(cx, p) {
        return LpHover::Region(i);
    }
    LpHover::None
}

fn lp_overlays(h: &LpHover, cache: &RegionCache) -> Vec<Overlay> {
    match h {
        LpHover::None => vec![],
        LpHover::Part(bp) => vec![Overlay::Path { path: bp.clone(), color: HIGHLIGHT_RED, width: 4.0, dashed: false }],
        LpHover::Region(i) => cache
            .map
            .regions
            .get(*i)
            .map(|(_, bp, _)| vec![Overlay::Path { path: bp.clone(), color: HIGHLIGHT_RED, width: 4.0, dashed: false }])
            .unwrap_or_default(),
    }
}

/// Live Paint Bucket: click fills the face under the cursor with the current fill; Shift-click
/// paints the nearest edge with the current stroke. Clicking selected ordinary paths makes them a
/// Live Paint group first.
pub struct LivePaintBucketTool {
    cache: RegionCache,
    hover: LpHover,
}

impl Default for LivePaintBucketTool {
    fn default() -> Self {
        Self { cache: RegionCache::live_paint(), hover: LpHover::None }
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
                self.hover = lp_hover(cx, &mut self.cache, p, ev.mods.shift);
                vec![]
            }
            PointerKind::Down => {
                self.hover = LpHover::None;
                let point = json!([p.x, p.y]);
                if ev.mods.shift {
                    let mut group = live_paint_at(cx.doc, p);
                    if group.is_none() {
                        cx.doc.walk(|n| {
                            if is_live_paint(n) && edge_near(n, p, cx.tol(4.0)).is_some() {
                                group = Some(n.id);
                            }
                        });
                    }
                    return match group {
                        Some(g) => {
                            vec![Action::Exec("livePaint.strokeEdge".into(), json!({ "group": g.0, "point": point, "tolerance": cx.tol(4.0) }))]
                        }
                        None => vec![],
                    };
                }
                if let Some(g) = live_paint_at(cx.doc, p) {
                    return vec![Action::Exec("livePaint.fill".into(), json!({ "group": g.0, "point": point }))];
                }
                if self.cache.at(cx, p).is_some() {
                    self.cache.key = None;
                    let roots = sorted_roots(cx.doc, &cx.selection.objects);
                    return vec![Action::Exec("livePaint.fill".into(), json!({ "ids": json_ids(&roots), "point": point }))];
                }
                vec![]
            }
            _ => vec![],
        }
    }

    fn overlays(&self, _cx: &ToolContext) -> Vec<Overlay> {
        lp_overlays(&self.hover, &self.cache)
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::LivePaintBucket
    }
}

// ---------- Live Paint Selection ----------

/// Live Paint Selection: click selects a face (Shift adds); Alt-click selects an edge.
pub struct LivePaintSelectionTool {
    cache: RegionCache,
    hover: LpHover,
}

impl Default for LivePaintSelectionTool {
    fn default() -> Self {
        Self { cache: RegionCache::live_paint(), hover: LpHover::None }
    }
}

impl Tool for LivePaintSelectionTool {
    fn id(&self) -> &'static str {
        "livePaintSelection"
    }

    fn pointer(&mut self, cx: &ToolContext, ev: &PointerEvent) -> Vec<Action> {
        let p = ev.pos;
        match ev.kind {
            PointerKind::Move => {
                self.hover = match lp_hover(cx, &mut self.cache, p, ev.mods.alt) {
                    h @ LpHover::Part(_) => h,
                    _ => LpHover::None,
                };
                vec![]
            }
            PointerKind::Down => {
                let mut hit = None;
                if let Some(g) = live_paint_at(cx.doc, p).and_then(|id| cx.doc.node(id)) {
                    let part = if ev.mods.alt { edge_near(g, p, cx.tol(4.0)) } else { face_at(g, p) };
                    hit = part.map(|n| n.id);
                }
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
        lp_overlays(&self.hover, &self.cache)
    }

    fn cursor(&self, _cx: &ToolContext, _p: Point, _m: Mods) -> Cursor {
        Cursor::Arrow
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
        let mut d = Document::new(500.0, 500.0);
        let l = d.layers[0].id;
        let a = d.alloc_id();
        d.insert(Some(l), 0, Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default_art())).unwrap();
        let b = d.alloc_id();
        d.insert(Some(l), 1, Node::path(b, shapes::rectangle(Rect::new(50.0, 0.0, 150.0, 100.0)), Appearance::default_art())).unwrap();
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

    #[test]
    fn hatch_stays_inside_region() {
        let bp = shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 20.0)).to_bezpath();
        let h = hatch(&bp, 4.0);
        let bb = h.bounding_box();
        assert!(bb.x0 >= -1e-6 && bb.y0 >= -1e-6 && bb.x1 <= 40.0 + 1e-6 && bb.y1 <= 20.0 + 1e-6);
        assert!(h.elements().len() > 10);
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
        let mut g = Node::group(gid, vec![Arc::new(face1), Arc::new(face2), Arc::new(edge)]);
        g.name = Some(LIVE_PAINT_NAME.into());
        d.insert(Some(l), 0, g).unwrap();
        (d, gid, f2, e)
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
    fn create_builds_the_three_tools() {
        for id in ["shapeBuilder", "livePaintBucket", "livePaintSelection"] {
            assert_eq!(create(id).unwrap().id(), id);
        }
        assert!(create("pen").is_none());
    }
}
