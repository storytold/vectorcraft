use super::lookup::{Face, MAPS_BUILT, faces_of, outline_snap, shape_builder_shapes};
use super::*;

/// Shape Builder hover hatch colour (neutral grey) and Live Paint highlight red.
pub const HATCH: [u8; 3] = [0x80, 0x80, 0x80];

pub const HIGHLIGHT_RED: [u8; 3] = [0xff, 0x2a, 0x2a];

/// How many Shape Builder maps ([`BuilderMap::of_shapes`]) this thread has built, for tests that
/// count them.
pub fn maps_built() -> usize {
    MAPS_BUILT.with(std::cell::Cell::get)
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

    /// The map of `shapes` (the inputs, back → front, as [`shape_builder_shapes`] gives them), with
    /// the outline edges only `with_edges`, closing gaps up to `gap`.
    pub fn of_shapes(shapes: Vec<po::Shape>, with_edges: bool, gap: f64) -> Self {
        if shapes.is_empty() {
            return Self::default();
        }
        MAPS_BUILT.with(|n| n.set(n.get() + 1));
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
        self.piece_hit(p, piece_tol).or_else(|| self.region_at(p).map(Hit::Region))
    }

    /// The line piece or edge within `piece_tol` of `p` that [`Self::hit`] gives (None without
    /// `piece_tol`).
    fn piece_hit(&self, p: Point, piece_tol: Option<f64>) -> Option<Hit> {
        piece_tol.and_then(|t| match (piece_near(&self.lines, p, t), piece_near(&self.edges, p, t)) {
            (Some(l), Some(e)) if e.0 < l.0 => Some(Hit::Edge(e.1)),
            (Some(l), _) => Some(Hit::Line(l.1)),
            (None, e) => e.map(|e| Hit::Edge(e.1)),
        })
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

    /// What a Shape Builder gesture at `p` touches: [`Self::hit`], except that while erasing, a
    /// point in a region that is no more than `2 * piece_tol` across there (see
    /// [`Self::narrow_at`]) touches the region and not the line piece or edge beside it.
    pub fn builder_hit(&self, p: Point, piece_tol: Option<f64>) -> Option<Hit> {
        match (piece_tol, self.piece_hit(p, piece_tol)) {
            (Some(tol), Some(piece)) => Some(match self.region_at(p) {
                Some(r) if self.narrow_at(r, p, tol) => Hit::Region(r),
                _ => piece,
            }),
            (_, piece) => piece.or_else(|| self.region_at(p).map(Hit::Region)),
        }
    }

    /// Is region `r` no more than `2 * tol` across at `p`, measured from a point of its outline
    /// within `tol` of `p`, through `p`, to its outline on the other side? Every point on that line
    /// is then within `tol` of the outline. The points tried are the point of the outline nearest
    /// `p` and each point of a segment where the segment is square to the line to `p`, so a
    /// point near the end of a thin bar measures across the bar from its sides as well as along it
    /// from the end, and splitting a segment in two does not change the answer. False when `p` is
    /// on the outline or more than `tol` from it.
    fn narrow_at(&self, r: usize, p: Point, tol: f64) -> bool {
        use kurbo::{ParamCurve as _, ParamCurveNearest as _};
        let Some((_, bp, _)) = self.regions.get(r) else { return false };
        // Each segment with the parameter, point and distance of its point nearest `p`.
        let near: Vec<(kurbo::PathSeg, f64, Point, f64)> = bp
            .segments()
            .map(|seg| {
                let n = seg.nearest(p, 1e-6);
                (seg, n.t, seg.eval(n.t), n.distance_sq.sqrt())
            })
            .collect();
        if !near.iter().all(|&(.., d)| d.is_finite() && d > 1e-9) {
            return false;
        }
        let closest = near.iter().map(|&(.., d)| d).fold(f64::INFINITY, f64::min);
        // A line from `p` no longer than `2 * tol` only meets segments that close to `p`.
        let reach: Vec<kurbo::PathSeg> = near.iter().filter(|&&(.., d)| d <= 2.0 * tol).map(|&(seg, ..)| seg).collect();
        // A segment's end is its nearest point when the segment turns away from `p` or stops
        // short of the point square to it; there, only the nearest point of all the outline and
        // an end where the segment is square to the line to `p` count.
        let square = |seg: kurbo::PathSeg, t: f64, n: Point| {
            use kurbo::ParamCurveDeriv as _;
            let dir = match seg {
                kurbo::PathSeg::Line(l) => l.p1 - l.p0,
                kurbo::PathSeg::Quad(q) => q.deriv().eval(t).to_vec2(),
                kurbo::PathSeg::Cubic(c) => c.deriv().eval(t).to_vec2(),
            };
            let v = p - n;
            dir.hypot() > 1e-12 && dir.dot(v).abs() <= 1e-6 * dir.hypot() * v.hypot()
        };
        let from =
            |&&(seg, t, n, d): &&(kurbo::PathSeg, f64, Point, f64)| d <= tol && (d == closest || (t > 1e-9 && t < 1.0 - 1e-9) || square(seg, t, n));
        near.iter().filter(from).any(|&(_, _, n, d)| {
            let line = kurbo::Line::new(p, p + (p - n) * ((2.0 * tol - d) / d));
            reach.iter().any(|seg| seg.intersect_line(line).iter().any(|x| x.line_t > 1e-9 && x.line_t <= 1.0))
        })
    }

    /// What a gesture along the polyline `pts` touches, in order: [`Self::hit`] at each vertex and
    /// at the points [`Self::probes`] picks between them, so it touches every region it crosses,
    /// however narrow.
    pub fn hits_along(&self, pts: &[Point], piece_tol: Option<f64>) -> Touched {
        self.along(pts, piece_tol, Self::hit)
    }

    /// What a Shape Builder gesture along the polyline `pts` touches: [`Self::hits_along`] with
    /// [`Self::builder_hit`], so an erasing gesture also touches each region it goes into where the
    /// region is no more than `2 * piece_tol` across.
    pub fn builder_hits_along(&self, pts: &[Point], piece_tol: Option<f64>) -> Touched {
        self.along(pts, piece_tol, Self::builder_hit)
    }

    fn along(&self, pts: &[Point], piece_tol: Option<f64>, hit: fn(&Self, Point, Option<f64>) -> Option<Hit>) -> Touched {
        let mut touched = Touched::default();
        let mut last: Option<Point> = None;
        for &p in pts {
            if let Some(a) = last {
                for q in self.probes(a, p, piece_tol) {
                    touched.add(hit(self, q, piece_tol));
                }
            }
            touched.add(hit(self, p, piece_tol));
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
