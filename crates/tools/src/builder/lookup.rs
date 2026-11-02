use super::*;

/// Names starting with this mark a Live Paint group, together with its sources group ([`SOURCES_NAME`]).
pub const LIVE_PAINT_PREFIX: &str = "Live Paint";

/// Default name of a new Live Paint group.
pub const LIVE_PAINT_NAME: &str = "Live Paint Group";

/// The hidden child group holding the source paths.
pub const SOURCES_NAME: &str = "Live Paint Sources";

pub const FACE_NAME: &str = "Face";

pub const EDGE_NAME: &str = "Edge";

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
    live_paint_at_where(doc, p, |id| doc.is_visible(id))
}

/// [`live_paint_at`] among the Live Paint groups for which `takes` is true.
pub(super) fn live_paint_at_where(doc: &Document, p: Point, takes: impl Fn(NodeId) -> bool) -> Option<NodeId> {
    let mut hit = None;
    doc.walk(|n| {
        if is_live_paint(n) && visible_face_at(n, p).is_some() && takes(n.id) {
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

/// [`leaf_shapes`]; with `builder` (the Shape Builder's inputs), a Live Paint group contributes
/// only its filled faces and its stroked edges, and locked objects contribute nothing.
fn collect_leaves<'a>(n: &'a Node, builder: bool, out: &mut Vec<&'a Node>) {
    collect_members(n, builder, out);
    if builder {
        out.retain(|m| m.visible && !m.locked);
    }
}

/// The leaves the Shape Builder takes from `n` ([`collect_leaves`]), in paint order, with each
/// hidden or locked object it skips in its place.
pub fn builder_members(n: &Node) -> Vec<&Node> {
    let mut out = vec![];
    collect_members(n, true, &mut out);
    out
}

/// [`collect_leaves`] with, for `builder`, each hidden or locked object in place of what it holds.
fn collect_members<'a>(n: &'a Node, builder: bool, out: &mut Vec<&'a Node>) {
    if !n.visible && !builder {
        return;
    }
    if builder && (!n.visible || n.locked) {
        out.push(n);
        return;
    }
    if n.extras().shaper.is_some() {
        for c in n.children().into_iter().flatten().skip(1) {
            collect_members(c, builder, out);
        }
        return;
    }
    match &n.kind {
        NodeKind::Path { guide: true, .. } | NodeKind::Path { clipping: true, .. } => {}
        NodeKind::Path { .. } | NodeKind::Compound { .. } => out.push(n),
        NodeKind::Group { .. } if builder && is_live_paint(n) => {
            out.extend(faces(n).into_iter().filter(|f| !f.visible || f.locked || !f.appearance.fill_paint().is_none()).map(|f| &**f));
            out.extend(edges(n).into_iter().filter(|e| !e.visible || e.locked || !e.appearance.stroke_paint().is_none()).map(|e| &**e));
        }
        NodeKind::Group { children, .. } | NodeKind::Layer { children, .. } => {
            for c in children {
                collect_members(c, builder, out);
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

/// [`shapes_for`]; with `builder`, the leaves [`collect_leaves`] takes for the Shape Builder, and
/// nothing from a root in a locked or hidden group or layer.
fn shapes_of<'a>(doc: &'a Document, roots: &[NodeId], builder: bool) -> (Vec<po::Shape>, Vec<&'a Node>) {
    let mut lv = vec![];
    for id in roots {
        if let Some(n) = doc.node(*id)
            && (!builder || doc.is_editable(*id))
        {
            collect_leaves(n, builder, &mut lv);
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
/// and its edges without a stroke are left out. Locked objects, and objects in a locked group or
/// layer, give nothing.
pub fn shape_builder_shapes<'a>(doc: &'a Document, roots: &[NodeId], open_filled_as_closed: bool) -> (Vec<po::Shape>, Vec<&'a Node>) {
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

pub(super) fn faces_of(regions: Vec<po::Region>) -> Vec<Face> {
    regions
        .into_iter()
        .filter_map(|r| {
            let bp = r.path.to_bezpath();
            let b = r.path.bounds()?;
            Some((r, bp, b))
        })
        .collect()
}

thread_local! {
    /// The Shape Builder maps built on this thread ([`maps_built`]).
    pub(super) static MAPS_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
