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

/// The greatest distance (pt) between the ends of two Live Paint edges that the Live Paint tools take
/// as one point. Every edge comes from one planar map, so their ends meet this closely.
pub const LP_JOIN_TOL: f64 = 1e-4;

/// The greatest distance (pt) between the middle of a Live Paint face's outline segment and an edge
/// it runs along. The planar map refits outlines to [`vectorcraft_pathops::DEFAULT_PRECISION`],
/// keeping each refit segment's ends, and with Gap Detection on, the faces and the edges come from
/// two maps, so an outline can lie that far off its edges between its ends.
const LP_FACE_EDGE_TOL: f64 = vectorcraft_pathops::DEFAULT_PRECISION;

/// The cell [`LP_JOIN_TOL`] wide that holds `p`, for finding the points near it; two points within
/// [`LP_JOIN_TOL`] of each other lie in the same or neighboring cells. The cast saturates, so a
/// coordinate too large or not finite does not overflow.
fn lp_cell(p: Point) -> (i64, i64) {
    ((p.x / LP_JOIN_TOL).round() as i64, (p.y / LP_JOIN_TOL).round() as i64)
}

/// The cell that holds `p` ([`lp_cell`]) and the eight cells around it.
fn lp_cells_around(p: Point) -> impl Iterator<Item = (i64, i64)> {
    let (x, y) = lp_cell(p);
    (-1..=1).flat_map(move |dx: i64| (-1..=1).map(move |dy: i64| (x.saturating_add(dx), y.saturating_add(dy))))
}

/// The stroke paint Live Paint edge `edge` draws with: the paint of its topmost stroke that is shown
/// with a paint, a weight above 0 and an opacity above 0, or [`Paint::None`] when the edge is fully
/// transparent or has no such stroke.
pub(super) fn lp_stroke_paint(edge: &Node) -> Paint {
    if edge.opacity <= 0.0 {
        return Paint::None;
    }
    edge.appearance
        .items
        .iter()
        .rev()
        .find_map(|i| match i {
            vectorcraft_doc::appearance::AppearanceItem::Stroke(s) if s.visible && !s.paint.is_none() && s.width > 0.0 && s.opacity > 0.0 => {
                Some(s.paint.clone())
            }
            _ => None,
        })
        .unwrap_or(Paint::None)
}

/// Whether Live Paint edge `edge` draws no stroke ([`lp_stroke_paint`] is [`Paint::None`]).
fn unstroked(edge: &Node) -> bool {
    lp_stroke_paint(edge).is_none()
}

/// The segments of `pd`.
fn lp_segments(pd: &PathData) -> Vec<kurbo::CubicBez> {
    pd.subpaths.iter().flat_map(|sp| (0..sp.segment_count()).map(|i| sp.segment(i))).collect()
}

/// The faces of Live Paint group `group` that a flood from face `start` reaches, `start` first:
/// `start`, and each face that shares with a face already reached an edge that is hidden or draws
/// no stroke. Hidden faces are not reached and do not pass the flood on. A face runs along the parts
/// of the edge segments its outline segments lie on ([`lp_lies_on`]) or overlap ([`lp_overlap`]).
/// Two faces share an edge where those parts overlap along more than [`LP_FACE_EDGE_TOL`], or, where
/// both outlines lie within [`LP_JOIN_TOL`] of the edge, along more than half the shorter part. So
/// two faces that a Gap Detection gap parts on the same side of an edge, whose parts only touch, do
/// not share it, and two faces whose shared edge is shorter than [`LP_FACE_EDGE_TOL`] do. Empty when
/// `start` is not a face of `group` that is not hidden.
pub fn flood_faces(group: &Node, start: NodeId) -> Vec<NodeId> {
    use kurbo::{ParamCurve, ParamCurveArclen};
    use std::collections::{HashMap, HashSet};
    let shown: Vec<&Arc<Node>> = faces(group).into_iter().filter(|f| f.visible).collect();
    if !shown.iter().any(|f| f.id == start) {
        return vec![];
    }
    // Each edge segment, and whether a flood crosses it.
    let edge_segs: Vec<(kurbo::CubicBez, bool)> = edges(group)
        .into_iter()
        .flat_map(|e| {
            let cross = !e.visible || unstroked(e);
            e.path_data().map(lp_segments).unwrap_or_default().into_iter().map(move |s| (s, cross))
        })
        .collect();
    // The faces along each edge segment a flood crosses, each with the part of the segment it runs
    // along (from and to, in the segment's own parameter). A face outline segment usually lies on
    // one edge segment, stroked or not ([`lp_lies_on`]); one that does not (a piece of an edge
    // segment where a Gap Detection line meets it, the line itself, or an outline segment that spans
    // several edge segments) is measured against the edge segments a flood crosses that its bounds
    // meet ([`lp_overlap`]). Each part also records whether its outline segment lies within
    // [`LP_JOIN_TOL`] of it ([`lp_close`]).
    let all: Vec<(kurbo::CubicBez, bool)> = edge_segs.iter().map(|&(s, _)| (s, true)).collect();
    let all_grid = LpEdgeGrid::new(&all);
    let mut along: HashMap<usize, Vec<(NodeId, f64, f64, bool)>> = HashMap::new();
    let mut pieces: Vec<(kurbo::CubicBez, NodeId)> = vec![];
    for f in &shown {
        for piece in f.path_data().map(lp_segments).unwrap_or_default() {
            let m = piece.eval(0.5);
            let bounds = kurbo::ParamCurveExtrema::bounding_box(&piece).inflate(LP_FACE_EDGE_TOL, LP_FACE_EDGE_TOL);
            // The edge segment the outline segment lies on whose middle is nearest its own.
            let on = all_grid
                .near_box(bounds)
                .into_iter()
                .filter_map(|k| edge_segs.get(k).and_then(|(s, cross)| lp_lies_on(s, &piece).map(|part| (k, s, *cross, part))))
                .min_by(|x, y| (x.1.eval(0.5) - m).hypot().total_cmp(&(y.1.eval(0.5) - m).hypot()));
            match on {
                Some((k, s, cross, (from, to))) => {
                    if cross {
                        along.entry(k).or_default().push((f.id, from, to, lp_close(s, from, to, &piece)));
                    }
                }
                None => pieces.push((piece, f.id)),
            }
        }
    }
    if !pieces.is_empty() {
        let grid = LpEdgeGrid::new(&edge_segs);
        for (piece, id) in pieces {
            let bounds = kurbo::ParamCurveExtrema::bounding_box(&piece).inflate(LP_FACE_EDGE_TOL, LP_FACE_EDGE_TOL);
            for k in grid.near_box(bounds) {
                if let Some((s, true)) = edge_segs.get(k)
                    && let Some((a, c)) = lp_overlap(s, &piece)
                {
                    along.entry(k).or_default().push((id, a, c, lp_close(s, a, c, &piece)));
                }
            }
        }
    }
    let mut next: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    // Sorted by where each part starts, a part is compared only with the parts after it that start
    // before it ends. Two parts that overlap along [`LP_FACE_EDGE_TOL`] or less join only when both
    // lie within [`LP_JOIN_TOL`] of the edge and the overlap is more than half the shorter part: an
    // edge that short is shared, while an outline that only crosses it lies off it.
    for (k, on) in &mut along {
        let Some((s, _)) = edge_segs.get(*k) else { continue };
        on.sort_by(|x, y| x.1.total_cmp(&y.1));
        for (i, &(a, a0, a1, a_close)) in on.iter().enumerate() {
            for &(c, c0, c1, c_close) in on.iter().skip(i + 1).take_while(|x| x.1 < a1) {
                let (from, to) = (a0.max(c0), a1.min(c1));
                let len = |x: f64, y: f64| s.subsegment(x..y).arclen(1e-6);
                let shared = if a != c && from < to { len(from, to) } else { 0.0 };
                let short_shared = a_close && c_close && shared > 0.5 * len(a0, a1).min(len(c0, c1));
                if shared > 0.0 && (shared > LP_FACE_EDGE_TOL || short_shared) {
                    next.entry(a).or_default().push(c);
                    next.entry(c).or_default().push(a);
                }
            }
        }
    }
    let mut reached = vec![start];
    let mut seen = HashSet::from([start]);
    let mut i = 0;
    while let Some(&f) = reached.get(i) {
        i += 1;
        for &n in next.get(&f).map(Vec::as_slice).unwrap_or_default() {
            if seen.insert(n) {
                reached.push(n);
            }
        }
    }
    reached
}

/// The part of edge segment `s` that outline segment `piece` runs along, as (from, to) in `s`'s
/// own parameter, or None when they share no length. The part runs between the ends of `s` that lie
/// within [`LP_FACE_EDGE_TOL`] of `piece` and the ends of `piece` that lie within [`LP_JOIN_TOL`] of
/// `s` (the map keeps the ends of an outline segment it refits), and its middle lies within
/// [`LP_FACE_EDGE_TOL`] of `piece`, where the two run along each other ([`lp_runs_along`]). When no
/// end of `piece` lies on `s`, `piece` must lie within [`LP_JOIN_TOL`] of the part ([`lp_close`]). So
/// two segments that only cross or touch, or that only lie near each other, share nothing.
fn lp_overlap(s: &kurbo::CubicBez, piece: &kurbo::CubicBez) -> Option<(f64, f64)> {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let on_piece = |p: Point| piece.nearest(p, 1e-9).distance_sq.sqrt() <= LP_FACE_EDGE_TOL;
    let mut ts: Vec<f64> = [(s.p0, 0.0), (s.p3, 1.0)].into_iter().filter(|(p, _)| on_piece(*p)).map(|(_, t)| t).collect();
    let mut piece_end_on = false;
    for p in [piece.p0, piece.p3] {
        let n = s.nearest(p, 1e-9);
        if n.distance_sq.sqrt() <= LP_JOIN_TOL {
            ts.push(n.t);
            piece_end_on = true;
        }
    }
    let from = ts.iter().copied().fold(f64::INFINITY, f64::min);
    let to = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if from >= to {
        return None;
    }
    let mid = (from + to) / 2.0;
    let n = piece.nearest(s.eval(mid), 1e-9);
    let along = if piece_end_on { lp_runs_along(s, from, to, piece, mid, n.t) } else { lp_close(s, from, to, piece) };
    (n.distance_sq.sqrt() <= LP_FACE_EDGE_TOL && along).then_some((from, to))
}

/// The part of edge segment `s` that outline segment `piece` lies on as a whole, as (from, to) in
/// `s`'s own parameter: both ends of `piece` lie within [`LP_JOIN_TOL`] of `s`, its middle lies
/// within [`LP_FACE_EDGE_TOL`] of `s`, and the two run along each other ([`lp_runs_along`]).
fn lp_lies_on(s: &kurbo::CubicBez, piece: &kurbo::CubicBez) -> Option<(f64, f64)> {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let (a, c, m) = (s.nearest(piece.p0, 1e-9), s.nearest(piece.p3, 1e-9), s.nearest(piece.eval(0.5), 1e-9));
    let (from, to) = (a.t.min(c.t), a.t.max(c.t));
    let near = a.distance_sq.sqrt() <= LP_JOIN_TOL && c.distance_sq.sqrt() <= LP_JOIN_TOL && m.distance_sq.sqrt() <= LP_FACE_EDGE_TOL;
    (near && lp_runs_along(s, from, to, piece, m.t, 0.5)).then_some((from, to))
}

/// Whether outline segment `piece` runs along the part of edge segment `s` from `from` to `to`
/// rather than crossing it: it lies within [`LP_JOIN_TOL`] of that part ([`lp_close`]), or the two
/// run the same way or opposite ways at `ts` on `s` and `tp` on `piece` ([`lp_parallel`]).
fn lp_runs_along(s: &kurbo::CubicBez, from: f64, to: f64, piece: &kurbo::CubicBez, ts: f64, tp: f64) -> bool {
    lp_close(s, from, to, piece) || lp_parallel(s, ts, piece, tp)
}

/// Whether outline segment `piece` lies within [`LP_JOIN_TOL`] of edge segment `s` at the ends and
/// the middle of the part of `s` from `from` to `to`.
fn lp_close(s: &kurbo::CubicBez, from: f64, to: f64, piece: &kurbo::CubicBez) -> bool {
    use kurbo::{ParamCurve, ParamCurveNearest};
    [from, (from + to) / 2.0, to].into_iter().all(|t| piece.nearest(s.eval(t), 1e-9).distance_sq.sqrt() <= LP_JOIN_TOL)
}

/// Whether segment `s` at parameter `ts` and segment `piece` at parameter `tp` run the same way or
/// opposite ways: the sine of the angle between their directions there is at most 0.2. False where
/// either has no direction.
fn lp_parallel(s: &kurbo::CubicBez, ts: f64, piece: &kurbo::CubicBez, tp: f64) -> bool {
    use kurbo::{ParamCurve, ParamCurveDeriv};
    let (u, v) = (s.deriv().eval(ts).to_vec2(), piece.deriv().eval(tp).to_vec2());
    let n = u.hypot() * v.hypot();
    n > 0.0 && u.cross(v).abs() <= 0.2 * n
}

/// Most grid cells one edge segment is listed in by [`LpEdgeGrid`]; a longer one is tried for
/// every point.
const LP_GRID_SPAN: i64 = 1024;

/// The farthest cell from the grid's origin, on each axis, that [`LpEdgeGrid`] numbers a point in.
const LP_GRID_LIMIT: f64 = 1_048_576.0;

/// The edge segments a flood crosses, by the cells of a square grid (about one cell per segment)
/// that their bounds, widened by [`LP_FACE_EDGE_TOL`], cover.
struct LpEdgeGrid {
    origin: Point,
    cell: f64,
    /// Segment indices by cell.
    cells: std::collections::HashMap<(i64, i64), Vec<usize>>,
    /// Segments whose bounds cover more than [`LP_GRID_SPAN`] cells, near every point.
    wide: Vec<usize>,
}

impl LpEdgeGrid {
    /// The grid of the segments of `edges` that a flood crosses (those marked true).
    fn new(edges: &[(kurbo::CubicBez, bool)]) -> Self {
        let boxes: Vec<(usize, Rect)> = edges
            .iter()
            .enumerate()
            .filter(|(_, e)| e.1)
            .map(|(k, (s, _))| (k, kurbo::ParamCurveExtrema::bounding_box(s).inflate(LP_FACE_EDGE_TOL, LP_FACE_EDGE_TOL)))
            .collect();
        let all = boxes.iter().map(|b| b.1).reduce(|a, c| a.union(c)).unwrap_or_default();
        let side = (boxes.len() as f64).sqrt().ceil().max(1.0);
        let cell = all.width().max(all.height()) / side;
        let cell = if cell.is_finite() && cell > LP_FACE_EDGE_TOL { cell } else { LP_FACE_EDGE_TOL };
        let mut grid = Self { origin: all.origin(), cell, cells: std::collections::HashMap::new(), wide: vec![] };
        for (k, b) in boxes {
            let ((x0, y0), (x1, y1)) = (grid.cell_of(Point::new(b.x0, b.y0)), grid.cell_of(Point::new(b.x1, b.y1)));
            if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > LP_GRID_SPAN {
                grid.wide.push(k);
                continue;
            }
            for x in x0..=x1 {
                for y in y0..=y1 {
                    grid.cells.entry((x, y)).or_default().push(k);
                }
            }
        }
        grid
    }

    /// The cell `p` is in, counted from the origin (at most [`LP_GRID_LIMIT`] cells away on each
    /// axis).
    fn cell_of(&self, p: Point) -> (i64, i64) {
        let at = |v: f64, o: f64| ((v - o) / self.cell).floor().clamp(-LP_GRID_LIMIT, LP_GRID_LIMIT) as i64;
        (at(p.x, self.origin.x), at(p.y, self.origin.y))
    }

    /// The segments whose widened bounds may meet `r`, each once, in ascending order. When `r`
    /// covers more than [`LP_GRID_SPAN`] cells, every listed segment is returned.
    fn near_box(&self, r: Rect) -> Vec<usize> {
        let ((x0, y0), (x1, y1)) = (self.cell_of(Point::new(r.x0, r.y0)), self.cell_of(Point::new(r.x1, r.y1)));
        let mut out: Vec<usize> = self.wide.clone();
        if (x1 - x0 + 1).saturating_mul(y1 - y0 + 1) > LP_GRID_SPAN {
            out.extend(self.cells.values().flatten());
        } else {
            for x in x0..=x1 {
                for y in y0..=y1 {
                    out.extend(self.cells.get(&(x, y)).into_iter().flatten());
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

/// The edges of Live Paint group `group` that are not hidden and are joined to edge `start` through
/// edges that draw with stroke paint `paint` ([`lp_stroke_paint`]), `start` first: `start`, and each
/// edge that draws with that stroke paint and has an end (the first or last point of one of its subpaths) within [`LP_JOIN_TOL`] of an
/// end of an edge already reached.
/// Empty when `start` is not an edge of `group` that is not hidden.
pub fn connected_edges(group: &Node, start: NodeId, paint: &Paint) -> Vec<NodeId> {
    use std::collections::{HashMap, HashSet};
    let es: Vec<&Arc<Node>> = edges(group).into_iter().filter(|e| e.visible && (e.id == start || lp_stroke_paint(e) == *paint)).collect();
    // The first and last points of each subpath of each edge.
    let ends: Vec<Vec<Point>> = es
        .iter()
        .map(|e| {
            let Some(pd) = e.path_data() else { return vec![] };
            pd.subpaths
                .iter()
                .filter_map(|sp| {
                    let n = sp.segment_count();
                    (n > 0).then(|| [sp.segment(0).p0, sp.segment(n - 1).p3])
                })
                .flatten()
                .collect()
        })
        .collect();
    let mut at: HashMap<(i64, i64), Vec<(Point, usize)>> = HashMap::new();
    for (i, pts) in ends.iter().enumerate() {
        for &p in pts {
            at.entry(lp_cell(p)).or_default().push((p, i));
        }
    }
    let Some(first) = es.iter().position(|e| e.id == start) else { return vec![] };
    let mut reached = vec![first];
    let mut seen = HashSet::from([first]);
    let mut k = 0;
    while let Some(&i) = reached.get(k) {
        k += 1;
        for &p in ends.get(i).map(Vec::as_slice).unwrap_or_default() {
            for c in lp_cells_around(p) {
                for &(q, j) in at.get(&c).map(Vec::as_slice).unwrap_or_default() {
                    if (q - p).hypot() <= LP_JOIN_TOL && seen.insert(j) {
                        reached.push(j);
                    }
                }
            }
        }
    }
    reached.into_iter().filter_map(|i| es.get(i).map(|e| e.id)).collect()
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
    /// The Shape Builder merges computed on this thread ([`merges_computed`]).
    pub(super) static MERGES_COMPUTED: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
