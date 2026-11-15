//! Geometric stroke contours are a separate target channel: their fitted
//! vertices are never anchors, intersections, tangent or perpendicular guides.

use super::{Affine, Line, ParamCurve, ParamCurveNearest, PathSeg, Point, Rect, Shape, Vec2};
use super::{HashSet, ToolContext};
use vectorcraft_doc::{AppearanceItem, Document, Node, NodeId, NodeKind, QueryPaintKind};
use vectorcraft_geom::kurbo::ParamCurveExtrema;
use vectorcraft_geom::{FillRule, PathData};

#[derive(Default)]
pub(super) struct Contours {
    contours: Vec<Contour>,
    segments: usize,
    exhausted: bool,
}

struct Contour {
    bounds: Rect,
    segments: Vec<PathSeg>,
}

impl Contours {
    pub(super) fn is_empty(&self) -> bool {
        self.contours.is_empty()
    }
    pub(super) fn push(&mut self, path: &PathData, visible: Option<Rect>) {
        if self.exhausted {
            return;
        }
        let path = path.to_bezpath();
        let bounds = path.bounding_box();
        if !bounds.is_finite() || visible.is_some_and(|v| !overlap(v, bounds)) {
            return;
        }
        let remaining = super::targets::BUDGET.saturating_sub(self.segments);
        let segments: Vec<_> = path.segments().take(remaining + 1).collect();
        if segments.len() > remaining {
            self.exhausted = true;
            return;
        }
        if !segments.is_empty() {
            self.segments += segments.len();
            self.contours.push(Contour { bounds, segments });
            self.exhausted = self.segments == super::targets::BUDGET;
        }
    }

    pub(super) fn nearest(&self, p: Point, tol: f64) -> Option<Point> {
        if !finite_point(p) || !tol.is_finite() || tol < 0.0 {
            return None;
        }
        self.contours
            .iter()
            .filter(|c| c.bounds.inflate(tol, tol).contains(p))
            .flat_map(|c| &c.segments)
            .map(|s| (s, s.nearest(p, 1e-6)))
            .filter(|(_, n)| n.distance_sq <= tol * tol)
            .min_by(|a, b| a.1.distance_sq.total_cmp(&b.1.distance_sq))
            .map(|(s, n)| s.eval(n.t))
    }

    /// Actual intersections with the permitted trajectory, never a projection
    /// of a free nearest-point delta back onto the constraint.
    pub(super) fn on_line(&self, at: Point, dir: Vec2, near: Point, tol: f64, ray: bool) -> Option<Point> {
        let len = dir.hypot();
        if !finite_point(at) || !finite_point(near) || !len.is_finite() || len == 0.0 || !tol.is_finite() || tol < 0.0 {
            return None;
        }
        let u = dir / len;
        let center = at + u * (near - at).dot(u);
        let line = Line::new(center - u * tol, center + u * tol);
        self.contours
            .iter()
            .filter(|c| c.bounds.inflate(tol, tol).contains(near))
            .flat_map(|c| &c.segments)
            .flat_map(|s| intersections(*s, line))
            .filter(|q| q.distance(near) <= tol && (!ray || (*q - at).dot(u) >= 0.0))
            .min_by(|a, b| a.distance_squared(near).total_cmp(&b.distance_squared(near)))
    }

    /// The nearest contact of a rectangle's finite boundary and a contour.
    /// Point/line bounds use the same geometric query without zero-length
    /// segment division in kurbo's segment-pair solver.
    pub(super) fn rect_delta(&self, rects: &[Rect], tol: f64, along: Option<Vec2>) -> Option<(Vec2, Point)> {
        if !tol.is_finite() || tol < 0.0 {
            return None;
        }
        let along = match along {
            Some(v) if v.hypot().is_finite() && v.hypot() > 0.0 => Some(v / v.hypot()),
            Some(_) => return None,
            None => None,
        };
        let mut best: Option<(Vec2, Point)> = None;
        let mut consider = |delta: Vec2, q: Point| {
            let d = delta.hypot2();
            if finite_point(q) && d.is_finite() && d <= tol * tol && best.is_none_or(|b| d < b.0.hypot2()) {
                best = Some((delta, q));
            }
        };
        for r in rects.iter().filter(|r| r.is_finite()) {
            let points = [Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)];
            for c in self.contours.iter().filter(|c| overlap(c.bounds.inflate(tol, tol), *r)) {
                for i in 0..4 {
                    let side = Line::new(points[i], points[(i + 1) % 4]);
                    for seg in &c.segments {
                        if let Some(u) = along {
                            constrained_side(side, *seg, u, tol, &mut consider);
                        } else if let PathSeg::Line(line) = seg {
                            line_contact(side, *line, &mut consider);
                        } else if side.p0 == side.p1 {
                            let n = seg.nearest(side.p0, 1e-6);
                            let q = seg.eval(n.t);
                            consider(q - side.p0, q);
                        } else {
                            let hit = PathSeg::Line(side).min_dist(*seg, 1e-6);
                            let (p, q) = (side.eval(hit.t1), seg.eval(hit.t2));
                            consider(q - p, q);
                        }
                    }
                }
            }
        }
        best
    }
}

/// The exact finite line-segment case avoids the iterative curve-pair solver's
/// tangential parameter drift. A closest pair is an intersection, or an endpoint
/// projected onto the opposite finite segment. Strict candidate ties retain the
/// caller's document/side order.
fn line_contact(a: Line, b: Line, emit: &mut impl FnMut(Vec2, Point)) {
    if !a.is_finite() || !b.is_finite() {
        return;
    }
    if let Some(q) = a.crossing_point(b)
        && finite_point(q)
        && within_segment(a, q)
        && within_segment(b, q)
    {
        emit(Vec2::ZERO, q);
        return;
    }
    for p in [a.p0, a.p1] {
        let q = project_segment(b, p);
        emit(q - p, q);
    }
    for q in [b.p0, b.p1] {
        let p = project_segment(a, q);
        emit(q - p, q);
    }
}

fn within_segment(line: Line, p: Point) -> bool {
    let d = line.p1 - line.p0;
    let length = d.hypot2();
    if length == 0.0 {
        return p == line.p0;
    }
    let t = (p - line.p0).dot(d) / length;
    (0.0..=1.0).contains(&t)
}

fn project_segment(line: Line, p: Point) -> Point {
    // Preserve the common coordinate exactly for axis-aligned sides. In
    // particular an overlapping parallel stroke edge never nudges the body
    // tangentially merely because an iterative parameter was approximate.
    if line.p0.x == line.p1.x {
        return Point::new(line.p0.x, p.y.clamp(line.p0.y.min(line.p1.y), line.p0.y.max(line.p1.y)));
    }
    if line.p0.y == line.p1.y {
        return Point::new(p.x.clamp(line.p0.x.min(line.p1.x), line.p0.x.max(line.p1.x)), line.p0.y);
    }
    line.eval(line.nearest(p, 0.0).t)
}

fn overlap(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

fn finite_point(p: Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

fn intersections(seg: PathSeg, line: Line) -> Vec<Point> {
    if line.p0 == line.p1 {
        let nearest = seg.nearest(line.p0, 1e-6);
        return (nearest.distance_sq <= 1e-18).then_some(line.p0).into_iter().collect();
    }
    let v = line.p1 - line.p0;
    let normal = Vec2::new(-v.y, v.x);
    if controls(seg).iter().all(|p| normal.dot(*p - line.p0) == 0.0) {
        let l2 = v.hypot2();
        let ts: Vec<_> = projected_extrema(seg, v).into_iter().map(|t| (seg.eval(t) - line.p0).dot(v) / l2).collect();
        let lo = ts.iter().copied().fold(f64::INFINITY, f64::min).max(0.0);
        let hi = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max).min(1.0);
        return if lo <= hi { vec![line.eval(0.5_f64.clamp(lo, hi)), line.eval(lo), line.eval(hi)] } else { vec![] };
    }
    seg.intersect_line(line)
        .into_iter()
        .filter(|x| (0.0..=1.0).contains(&x.line_t) && (0.0..=1.0).contains(&x.segment_t))
        .map(|x| seg.eval(x.segment_t))
        .filter(|q| finite_point(*q))
        .collect()
}

fn projected_extrema(seg: PathSeg, normal: Vec2) -> Vec<f64> {
    let projected = Affine::new([normal.x, 0.0, normal.y, 0.0, 0.0, 0.0]) * seg;
    std::iter::once(0.0).chain(projected.extrema()).chain([1.0]).collect()
}

fn controls(seg: PathSeg) -> Vec<Point> {
    match seg {
        PathSeg::Line(l) => vec![l.p0, l.p1],
        PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
        PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
    }
}

/// Minimise |lambda| with q(t)=p0+s*v+lambda*u and 0<=s<=1.
/// For a nonparallel side, lambda(t) is the target's projection onto
/// the SOURCE SIDE NORMAL divided by n.u. The finite candidate set is:
/// current intersections (lambda=0), target endpoints/normal extrema,
/// and the side endpoints' trajectories (the feasible-interval boundary).
fn constrained_side(side: Line, seg: PathSeg, u: Vec2, tol: f64, emit: &mut impl FnMut(Vec2, Point)) {
    let v = side.p1 - side.p0;
    if v.hypot2() == 0.0 {
        endpoint_contacts(side.p0, seg, u, tol, emit);
        return;
    }
    for q in intersections(seg, side) {
        emit(Vec2::ZERO, q);
    }
    let n = Vec2::new(-v.y, v.x);
    let denominator = n.dot(u);
    if denominator != 0.0 {
        nonparallel_side(side, seg, u, tol, emit);
    } else {
        parallel_side(side, seg, u, tol, emit);
    }
}

fn nonparallel_side(side: Line, seg: PathSeg, u: Vec2, tol: f64, emit: &mut impl FnMut(Vec2, Point)) {
    let v = side.p1 - side.p0;
    let n = Vec2::new(-v.y, v.x);
    let denominator = n.dot(u);
    // Target endpoints and source-side-normal extrema still require the
    // finite source-side feasibility check below.
    for t in projected_extrema(seg, n).into_iter().filter(|t| (0.0..=1.0).contains(t)) {
        let q = seg.eval(t);
        let lambda = n.dot(q - side.p0) / denominator;
        let p = q - u * lambda;
        let s = (p - side.p0).dot(v) / v.hypot2();
        if lambda.is_finite() && (0.0..=1.0).contains(&s) && lambda.abs() <= tol {
            emit(u * lambda, q);
        }
    }
    // These roots already lie on a KNOWN finite source endpoint's permitted
    // trajectory. Reconstructing its side parameter from curve.eval(root)
    // can round s just outside[0,1] or divide by a near-zero normal.u.
    for endpoint in [side.p0, side.p1] {
        endpoint_contacts(endpoint, seg, u, tol, emit);
    }
}

/// Keep the finite trajectory parameter returned by the intersection solver.
/// This preserves the actual constraint direction, including its cos/sin
/// residual; it is not a projection of a free nearest-point delta.
fn endpoint_contacts(endpoint: Point, seg: PathSeg, u: Vec2, tol: f64, emit: &mut impl FnMut(Vec2, Point)) {
    let line = Line::new(endpoint - u * tol, endpoint + u * tol);
    if line.p0 == line.p1 {
        for q in intersections(seg, line) {
            emit(Vec2::ZERO, q);
        }
        return;
    }
    let v = line.p1 - line.p0;
    let normal = Vec2::new(-v.y, v.x);
    // Preserve the existing coincident-trajectory behavior for a degenerate
    // point box too. These points are already on the permitted finite line.
    if controls(seg).iter().all(|p| normal.dot(*p - line.p0) == 0.0) {
        for q in intersections(seg, line) {
            let lambda = (q - endpoint).dot(u);
            emit(u * lambda, endpoint + u * lambda);
        }
        return;
    }
    for hit in seg.intersect_line(line).into_iter().filter(|h| (0.0..=1.0).contains(&h.line_t) && (0.0..=1.0).contains(&h.segment_t)) {
        let lambda = (2.0 * hit.line_t - 1.0) * tol;
        emit(u * lambda, endpoint + u * lambda);
    }
}

fn parallel_side(side: Line, seg: PathSeg, _u: Vec2, tol: f64, emit: &mut impl FnMut(Vec2, Point)) {
    let v = side.p1 - side.p0;
    let n = Vec2::new(-v.y, v.x);
    // The side slides along its own supporting line. Contact is possible
    // only where the curve lies on that line. A collinear curve contributes
    // its complete projected interval, including a contact at zero shift.
    let length = v.hypot();
    let w = v / length;
    let line = Line::new(side.p0 - w * tol, side.p1 + w * tol);
    let collinear = controls(seg).iter().all(|p| n.dot(*p - side.p0) == 0.0);
    let hits = if collinear {
        let extrema = projected_extrema(seg, w);
        let points: Vec<_> = extrema.into_iter().map(|t| seg.eval(t)).collect();
        let lo = points.iter().min_by(|a, b| ((**a - side.p0).dot(w)).total_cmp(&((**b - side.p0).dot(w)))).copied();
        let hi = points.iter().max_by(|a, b| ((**a - side.p0).dot(w)).total_cmp(&((**b - side.p0).dot(w)))).copied();
        if let (Some(a), Some(b)) = (lo, hi) {
            let (x, y) = ((a - side.p0).dot(w), (b - side.p0).dot(w));
            if x <= length && y >= 0.0 {
                let q = side.p0 + w * x.max(0.0);
                emit(Vec2::ZERO, q);
            }
        }
        points
    } else {
        intersections(seg, line)
    };
    for q in hits {
        let at = (q - side.p0).dot(w);
        let shift = if at < 0.0 {
            at
        } else if at > length {
            at - length
        } else {
            0.0
        };
        let delta = w * shift;
        if delta.hypot() <= tol {
            emit(delta, q);
        }
    }
}

/// Gather once per existing target-cache/gesture. OFF leaves before cloning or
/// evaluating any art. Root-limited uses still evaluate ancestor effects/clips,
/// but only the requested source owners can emit targets.
pub(super) fn gather(cx: &ToolContext, exclude: &[NodeId], visible: Option<Rect>, roots: Option<&[NodeId]>) -> Contours {
    if !cx.smart_guides || !cx.snap_to_stroke_edges {
        return Contours::default();
    }
    let mut blocked: HashSet<NodeId> = exclude.iter().copied().collect();
    for id in exclude {
        if let Some(n) = cx.doc.node(*id) {
            n.walk(&mut |n| {
                blocked.insert(n.id);
            });
        }
        // A compound owns its members' stroke. Container paint/reference-bound
        // geometry also changes when a selected descendant moves.
        if let Some(parents) = cx.doc.ancestry(*id) {
            blocked.extend(parents);
        }
    }
    let allowed = roots.map(|roots| {
        let mut ids = HashSet::new();
        for id in roots {
            if let Some(n) = cx.doc.node(*id) {
                n.walk(&mut |n| {
                    ids.insert(n.id);
                });
            }
        }
        ids
    });
    let view = vectorcraft_effects::stroke_paint_view(cx.doc);
    let mut out = Contours::default();
    let eligibility = Eligibility { blocked: &blocked, allowed: allowed.as_ref() };
    for layer in &view.layers {
        gather_node(&view, layer, &[], &eligibility, visible, &mut out, 0);
    }
    out
}

struct Eligibility<'a> {
    blocked: &'a HashSet<NodeId>,
    allowed: Option<&'a HashSet<NodeId>>,
}

impl Eligibility<'_> {
    fn accepts(&self, n: &Node) -> bool {
        n.extras().query_origin.0.is_some_and(|o| {
            !self.blocked.contains(&o.paint_owner)
                && !self.blocked.contains(&o.geometry_source)
                && self.allowed.is_none_or(|ids| ids.contains(&o.paint_owner))
        })
    }
}

type Clips = Vec<(PathData, FillRule)>;

fn clip_region(mut path: PathData, mut rule: FillRule, clips: &[(PathData, FillRule)]) -> PathData {
    for (clip, clip_rule) in clips {
        path = vectorcraft_pathops::boolean(&path, rule, clip, *clip_rule, vectorcraft_pathops::BoolOp::Intersect);
        rule = FillRule::NonZero;
        if path.is_empty() {
            break;
        }
    }
    // Normalize even-odd too before unions/contour extraction.
    if rule == FillRule::EvenOdd {
        path = vectorcraft_pathops::boolean(&path, rule, &PathData::default(), FillRule::NonZero, vectorcraft_pathops::BoolOp::Union);
    }
    path
}

fn union_regions(regions: Vec<PathData>) -> PathData {
    regions.into_iter().fold(PathData::default(), |a, b| {
        if a.is_empty() { b } else { vectorcraft_pathops::boolean(&a, FillRule::NonZero, &b, FillRule::NonZero, vectorcraft_pathops::BoolOp::Union) }
    })
}

fn expanded(doc: &Document, n: &Node) -> Option<Node> {
    if let Some(m) = vectorcraft_effects::evaluate_container(Some(doc), n) {
        return Some(m);
    }
    if let Some(m) = vectorcraft_effects::reshape(Some(doc), n, None) {
        return Some(m);
    }
    match &n.kind {
        NodeKind::Text(_) | NodeKind::SymbolInstance { .. } => vectorcraft_effects::outline_object(Some(doc), n),
        NodeKind::Blend { .. }
        | NodeKind::Envelope { .. }
        | NodeKind::Mesh(_)
        | NodeKind::Repeat(_)
        | NodeKind::Intertwine { .. }
        | NodeKind::PlacedDocument(_) => Some(vectorcraft_effects::expanded_live_group(Some(doc), n)),
        _ => None,
    }
}

fn gather_node(
    doc: &Document,
    n: &Node,
    clips: &[(PathData, FillRule)],
    eligibility: &Eligibility,
    visible: Option<Rect>,
    out: &mut Contours,
    depth: usize,
) {
    if out.exhausted || !n.visible || n.opacity <= 0.0 || depth >= vectorcraft_doc::depth::MAX_DRAWN_DEPTH {
        return;
    }
    let origin = n.extras().query_origin.0;
    if origin.is_some_and(|o| o.kind == QueryPaintKind::Fill) {
        return;
    }
    if origin.is_some_and(|o| o.kind == QueryPaintKind::Stroke) {
        if eligibility.accepts(n) {
            for region in painted_layers(doc, n, clips, depth) {
                out.push(&region, visible);
            }
        }
        return;
    }
    if let Some(m) = expanded(doc, n) {
        gather_node(doc, &m, clips, eligibility, visible, out, depth);
        return;
    }
    if matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) {
        if !eligibility.accepts(n) {
            return;
        }
        for paint in vectorcraft_effects::resolved_stroke_paints(n) {
            if let AppearanceItem::Stroke(st) = paint.item {
                let region = stroke_region(doc, &paint.path, paint.rule, &st, clips, depth);
                out.push(&region, visible);
            }
        }
        return;
    }
    let Some(children) = n.children() else {
        return;
    };
    let mut inner: Clips = clips.to_vec();
    let first = usize::from(n.clips());
    if first == 1 {
        let Some(clip) = children.first() else {
            return;
        };
        // The clipping object's own stroke paints above the mask, under only
        // the outer clip context. Its original fill never enters this channel.
        gather_node(doc, clip, clips, eligibility, visible, out, depth + 1);
        let Some((path, rule)) = vectorcraft_effects::clip_outline(clip) else {
            return;
        };
        inner.push((PathData::from_bezpath(&path), rule));
    }
    for child in children.iter().skip(first.max(usize::from(n.extras().shaper.is_some()))) {
        gather_node(doc, child, &inner, eligibility, visible, out, depth + 1);
    }
}

/// One stroke's pieces are united, but explicit Transform-copy wrappers
/// partition that source paint into separate contour layers.
fn painted_layers(doc: &Document, n: &Node, clips: &[(PathData, FillRule)], depth: usize) -> Vec<PathData> {
    let walk = PaintWalk { clips: clips.to_vec(), path: vec![], partition: vec![], split_copies: true, depth };
    let mut layers = std::collections::BTreeMap::<Vec<usize>, Vec<PathData>>::new();
    for (key, path) in painted_in(doc, n, &walk) {
        layers.entry(key).or_default().push(path);
    }
    layers.into_values().map(union_regions).collect()
}

/// Brush art is all part of its source stroke paint, including repeated brush
/// pieces. No fitted vertices, colour-piece seams or brush-internal copies.
fn painted_regions(doc: &Document, n: &Node, clips: &[(PathData, FillRule)], depth: usize) -> Vec<PathData> {
    let walk = PaintWalk { clips: clips.to_vec(), path: vec![], partition: vec![], split_copies: false, depth };
    painted_in(doc, n, &walk).into_iter().map(|(_, path)| path).collect()
}

struct PaintWalk {
    clips: Clips,
    path: Vec<usize>,
    partition: Vec<usize>,
    split_copies: bool,
    depth: usize,
}

impl PaintWalk {
    fn child(&self, index: usize, clips: &[(PathData, FillRule)], partition: bool) -> Self {
        let mut path = self.path.clone();
        path.push(index);
        Self {
            clips: clips.to_vec(),
            partition: if partition { path.clone() } else { self.partition.clone() },
            path,
            split_copies: self.split_copies,
            depth: self.depth + 1,
        }
    }
}

type PaintedRegions = Vec<(Vec<usize>, PathData)>;

fn painted_in(doc: &Document, n: &Node, walk: &PaintWalk) -> PaintedRegions {
    if !n.visible || n.opacity <= 0.0 || walk.depth >= vectorcraft_doc::depth::MAX_DRAWN_DEPTH {
        return vec![];
    }
    if let Some(m) = expanded(doc, n) {
        return painted_in(doc, &m, walk);
    }
    if matches!(n.kind, NodeKind::Path { guide: false, .. } | NodeKind::Compound { .. }) {
        return vectorcraft_effects::resolved_stroke_paints(n)
            .into_iter()
            .map(|p| {
                let region = match p.item {
                    AppearanceItem::Fill(_) => clip_region(p.path, p.rule, &walk.clips),
                    AppearanceItem::Stroke(st) => stroke_region(doc, &p.path, p.rule, &st, &walk.clips, walk.depth),
                };
                (walk.partition.clone(), region)
            })
            .collect();
    }
    if let NodeKind::Image(im) = &n.kind {
        let frame = vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, im.width as f64, im.height as f64)).transformed(im.xf);
        return vec![(walk.partition.clone(), clip_region(frame, FillRule::NonZero, &walk.clips))];
    }
    let Some(children) = n.children() else {
        return vec![];
    };
    let partition = walk.split_copies && n.extras().query_origin.0.is_some_and(|o| o.kind == QueryPaintKind::Geometry);
    let first = usize::from(n.clips());
    let mut regions = vec![];
    let mut inner = walk.clips.clone();
    if first == 1 {
        let Some(clip) = children.first() else {
            return regions;
        };
        let paint = clip.clip_paint();
        if let Some(stroke) = paint.stroke {
            regions.extend(painted_in(doc, &stroke, &walk.child(0, &walk.clips, partition)));
        }
        let Some((path, rule)) = vectorcraft_effects::clip_outline(clip) else {
            return regions;
        };
        inner.push((PathData::from_bezpath(&path), rule));
        if let Some(fill) = paint.fill {
            regions.extend(painted_in(doc, &fill, &walk.child(1, &inner, partition)));
        }
    }
    for (index, child) in children.iter().enumerate().skip(first.max(usize::from(n.extras().shaper.is_some()))) {
        regions.extend(painted_in(doc, child, &walk.child(index + 2, &inner, partition)));
    }
    regions
}

fn stroke_region(
    doc: &Document,
    path: &PathData,
    rule: FillRule,
    stroke: &vectorcraft_doc::StrokeLayer,
    clips: &[(PathData, FillRule)],
    depth: usize,
) -> PathData {
    if let Some(name) = stroke.brush.as_deref()
        && let Some(brush) = vectorcraft_brush::find(doc, name)
    {
        let regions = vectorcraft_brush::stroke_pieces(doc, &brush, &path.to_bezpath(), stroke)
            .iter()
            .flat_map(|piece| painted_regions(doc, piece, clips, depth + 1))
            .collect();
        return union_regions(regions);
    }
    // Missing brush names follow the renderer's ordinary-stroke fallback.
    clip_region(vectorcraft_effects::stroke::outline_region(path, rule, stroke), FillRule::NonZero, clips)
}

#[cfg(test)]
mod constrained_endpoint_tests {
    use super::*;

    #[test]
    fn derived_curve_endpoint_contact_preserves_real_constraint() {
        // Original asymmetric-profile outline segment from the actual public
        // Shift-body probe, not an adversarial or enlarged numeric fixture.
        let curve = PathSeg::Cubic(vectorcraft_geom::kurbo::CubicBez::new(
            (128.45052114776985, 190.28101068323193),
            (108.2940972295255, 174.686473997535),
            (94.99623809803211, 151.98943264693503),
            (87.32982187797965, 127.98475829585894),
        ));
        let endpoint = Point::new(110.32236828168163, 175.44113071748868);
        let u = Vec2::new(-1.0, 1.2246467991473532e-16);
        let closest = |side: Line| {
            let mut best: Option<(Vec2, Point)> = None;
            constrained_side(side, curve, u, 4.0, &mut |d, q| {
                if d.hypot2() <= 16.0 && best.is_none_or(|old| d.hypot2() < old.0.hypot2()) {
                    best = Some((d, q));
                }
            });
            best
        };
        let (delta, q) =
            closest(Line::new(endpoint, endpoint + Vec2::new(0.0, 40.0))).expect("the independently proven endpoint trajectory is feasible");
        assert!(delta.x > 2.5 && delta.x < 2.7 && delta.hypot() <= 4.0);
        assert!(delta.y != 0.0 && delta.cross(u).abs() < 1e-15, "retain the real Shift direction, not bitexact horizontal");
        assert_eq!(q, endpoint + delta);
        assert!(curve.nearest(q, 1e-9).distance_sq < 1e-18, "the contact must lie on the actual curved contour");
        // Same endpoint trajectory also serves a point-sized source box.
        assert!(closest(Line::new(endpoint, endpoint)).is_some());
        // Its infinite supporting line is not permission to contact a curve
        // outside this finite source side's y interval.
        let blocked_x = 128.45052114776985 - 2.0; // endpoint x is within reach; y is not.
        assert!(closest(Line::new((blocked_x, 230.0), (blocked_x, 260.0))).is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_contacts_are_finite_and_keep_parallel_slide_exact() {
        let closest = |a: Line, b: Line| {
            let mut best: Option<(Vec2, Point)> = None;
            line_contact(a, b, &mut |d, q| {
                if best.is_none_or(|old| d.hypot2() < old.0.hypot2()) {
                    best = Some((d, q));
                }
            });
            best.unwrap()
        };
        // The ordinary body's y=140 must remain exactly140 beside a vertical
        // stroke edge; no relaxed public drag/Undo guard is needed.
        assert_eq!(
            closest(Line::new((207.0, 140.0), (207.0, 160.0)), Line::new((205.0, 100.0), (205.0, 200.0))),
            (Vec2::new(-2.0, 0.0), Point::new(205.0, 140.0))
        );
        // Finite endpoints: the target's supporting infinite line is closer,
        // but that point is outside the painted segment.
        assert_eq!(closest(Line::new((4.0, 0.0), (7.0, 0.0)), Line::new((0.0, 3.0), (2.0, 3.0))), (Vec2::new(-2.0, 3.0), Point::new(2.0, 3.0)));
        assert_eq!(closest(Line::new((0.0, 0.0), (10.0, 0.0)), Line::new((5.0, -5.0), (5.0, 5.0))), (Vec2::ZERO, Point::new(5.0, 0.0)));
    }
}
