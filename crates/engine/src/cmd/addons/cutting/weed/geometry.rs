//! The geometry of Weed Box and Lines: the selected art as the outlines a cutter cuts ([`Region`]),
//! and the weeding lines across a box that stop at those outlines or at a gap from them
//! ([`lines`]).
//!
//! A region is measured along one straight line at a time. A closed outline encloses its area by
//! its fill rule, an open one is a line with no area, a group is the union of its members and a
//! clip group is what its members draw inside its clipping path. Along a weeding line the region
//! gives the stretches inside its area ([`Region::spans`]), and its outline pieces
//! ([`Region::outline`]) give the stretches within the gap of an outline.

use std::cell::Cell;

use vectorcraft_doc::{Document, Node, NodeId, NodeKind};
use vectorcraft_geom::{BezPath, FillRule, PathEl, Point, Rect};

use super::super::super::super::pathops::node_path;

/// How closely curves are followed, in points.
const TOLERANCE: f64 = 0.02;
/// The most points the outlines of one selection are measured by, once curves are followed in
/// straight segments (each subpath's points, its first one included).
pub(super) const MAX_POINTS: usize = 200_000;
/// The most tests of a segment against another one measuring a clip group's outlines takes.
const MAX_PAIRS: usize = 40_000_000;
/// The deepest groups, symbol instances and live objects are opened inside one another: as deep
/// as any object is drawn.
pub(super) const MAX_DEPTH: usize = vectorcraft_doc::depth::MAX_DRAWN_DEPTH;
/// The most pairs of an outline segment and a weeding line it comes near that one run measures.
const MAX_NEAR: usize = 10_000_000;
/// Pieces of weeding line shorter than this, in points, are left out.
const MIN_PIECE: f64 = 0.01;

/// A straight segment.
pub(super) type Segment = (Point, Point);

/// One subpath followed in straight segments.
#[derive(Clone, Debug)]
pub(super) struct Poly {
    points: Vec<Point>,
    closed: bool,
    /// The box around `points`.
    bounds: Rect,
}

impl Poly {
    /// The segments, with the closing one of a closed subpath.
    fn segments(&self) -> impl Iterator<Item = Segment> + '_ {
        let closing = self.closed.then(|| (self.points.last().copied(), self.points.first().copied()));
        self.points.windows(2).map(|w| (w[0], w[1])).chain(closing.and_then(|(a, b)| Some((a?, b?))))
    }
}

/// The selected art as the outlines a cutter cuts.
#[derive(Clone, Debug)]
pub(super) enum Region {
    /// A path, compound path or type: its subpaths with the rule its closed ones fill by.
    Shape { polys: Vec<Poly>, rule: FillRule },
    /// Everything its members draw.
    Union(Vec<Region>),
    /// What `content` draws inside `clip`.
    Clip { clip: Box<Region>, content: Box<Region> },
}

/// Reads the selected objects into a [`Region`], keeping count of the outline points.
pub(super) struct Reader<'a> {
    doc: &'a Document,
    points: usize,
}

impl<'a> Reader<'a> {
    pub(super) fn new(doc: &'a Document) -> Self {
        Self { doc, points: 0 }
    }

    /// The region of `nodes` together, `None` when they hold no outlines.
    pub(super) fn read(&mut self, nodes: &[&Node]) -> Result<Option<Region>, String> {
        let mut parts = vec![];
        for n in nodes {
            parts.extend(self.node(n, 0)?);
        }
        Ok(union(parts))
    }

    fn node(&mut self, n: &Node, depth: usize) -> Result<Option<Region>, String> {
        if depth > MAX_DEPTH {
            return Err(format!("the selected art is nested more than {MAX_DEPTH} levels deep"));
        }
        if !n.visible {
            return Ok(None);
        }
        match &n.kind {
            NodeKind::Path { guide: true, .. } | NodeKind::RulerGuide { .. } => Ok(None),
            NodeKind::Path { .. } | NodeKind::Compound { .. } | NodeKind::Text(_) => self.shape(n),
            NodeKind::Group { clip, .. } | NodeKind::Layer { clip, .. } => {
                let members = n.editing_children();
                if *clip {
                    let Some((first, rest)) = members.split_first() else { return Ok(None) };
                    let Some(clip) = self.clip_region(first)? else { return Ok(None) };
                    let mut parts = vec![];
                    for c in rest {
                        parts.extend(self.node(c, depth + 1)?);
                    }
                    Ok(union(parts).map(|content| Region::Clip { clip: Box::new(clip), content: Box::new(content) }))
                } else {
                    let mut parts = vec![];
                    for c in members {
                        parts.extend(self.node(c, depth + 1)?);
                    }
                    Ok(union(parts))
                }
            }
            NodeKind::Image(im) => {
                let (w, h) = (f64::from(im.width), f64::from(im.height));
                let corners = [Point::new(0.0, 0.0), Point::new(w, 0.0), Point::new(w, h), Point::new(0.0, h)].map(|p| im.xf * p);
                self.polygon(&corners)
            }
            // An instance of a symbol the document doesn't have draws nothing.
            NodeKind::SymbolInstance { .. } => match vectorcraft_render::effects::outline_object(Some(self.doc), n) {
                Some(art) => self.node(&art, depth + 1),
                None => Ok(None),
            },
            NodeKind::PlacedDocument(p) => {
                // The art of the file it shows; a file that can't be read shows its preview in its
                // box, which may be rotated.
                if vectorcraft_doc::placed_document::read_cached(p).is_none() {
                    return self.polygon(&p.corners());
                }
                let art = vectorcraft_render::effects::expand_live_deep(Some(self.doc), n);
                if std::mem::discriminant(&art.kind) == std::mem::discriminant(&n.kind) {
                    self.polygon(&p.corners())
                } else {
                    self.node(&art, depth + 1)
                }
            }
            NodeKind::Blend { .. } | NodeKind::Envelope { .. } | NodeKind::Mesh(_) | NodeKind::Repeat(_) | NodeKind::Intertwine { .. } => {
                // The art a live object draws, as the exporters write it; an object that stays
                // as it is is measured by its box.
                let art = vectorcraft_render::effects::expand_live_deep(Some(self.doc), n);
                if std::mem::discriminant(&art.kind) == std::mem::discriminant(&n.kind) { self.frame(n) } else { self.node(&art, depth + 1) }
            }
        }
    }

    /// The region a clipping path `first` clips to, whatever its own visibility: the region the
    /// renderer clips to (a path, a compound path, type by its glyphs, a group's visible members,
    /// an image's frame). `None` when it clips to nothing, and the clipped art is hidden.
    fn clip_region(&mut self, first: &Node) -> Result<Option<Region>, String> {
        let Some((outline, rule)) = vectorcraft_render::effects::clip_outline(first) else { return Ok(None) };
        let mut polys = self.polys(outline)?;
        // A region is filled: an open subpath clips as if closed.
        polys.iter_mut().for_each(|p| p.closed = true);
        Ok((!polys.is_empty()).then_some(Region::Shape { polys, rule }))
    }

    /// The region of the objects `ids` together, each within the clipping paths of the clip groups
    /// and layers around it (an object selected inside a clip group shows only within its clipping
    /// path, and not at all inside a hidden group or layer; a clipping path is measured as its clip
    /// group), `None` when they show no outlines.
    pub(super) fn read_ids(&mut self, ids: &[NodeId]) -> Result<Option<Region>, String> {
        let doc = self.doc;
        let clips = |n: &Node| matches!(n.kind, NodeKind::Group { clip: true, .. } | NodeKind::Layer { clip: true, .. });
        // Every object's place in the tree, found in one walk.
        let places = doc.index_paths(ids.iter().copied());
        let mut paths = vec![];
        for id in ids {
            let Some(mut path) = places.get(id).cloned() else { continue };
            // A clipping path selected on its own stands for the art it shows: its clip group.
            while let (Some(n), Some(up)) = (doc.node_at(&path), path.split_last().map(|(_, up)| up).and_then(|up| doc.node_at(up))) {
                if !(clips(up) && up.editing_children().first().is_some_and(|f| f.id == n.id)) {
                    break;
                }
                path.pop();
            }
            paths.push(path);
        }
        // An object selected twice, or inside another selected object, is measured once, with it.
        paths.sort();
        paths.dedup();
        let mut kept: Vec<Vec<usize>> = vec![];
        for path in paths {
            if !kept.last().is_some_and(|k| path.starts_with(k)) {
                kept.push(path);
            }
        }
        let mut parts = vec![];
        'ids: for path in &kept {
            let Some(n) = doc.node_at(path) else { continue };
            // Art in a hidden group or layer isn't shown, and isn't measured.
            if (1..path.len()).any(|depth| path.get(..depth).and_then(|p| doc.node_at(p)).is_some_and(|c| !c.visible)) {
                continue;
            }
            let Some(mut region) = self.read(&[n])? else { continue };
            // The containers around the object, innermost first.
            let mut child = n.id;
            for depth in (1..path.len()).rev() {
                let Some(container) = path.get(..depth).and_then(|p| doc.node_at(p)) else { break };
                if clips(container)
                    && let Some(first) = container.editing_children().first().filter(|f| f.id != child)
                {
                    let Some(clip) = self.clip_region(first)? else { continue 'ids };
                    region = Region::Clip { clip: Box::new(clip), content: Box::new(region) };
                }
                child = container.id;
            }
            parts.push(region);
        }
        Ok(union(parts))
    }

    /// A path, compound path or type as its outlines.
    fn shape(&mut self, n: &Node) -> Result<Option<Region>, String> {
        // Each anchor is at least one point, so a path past the budget errs before it is copied.
        if anchors(n) > MAX_POINTS.saturating_sub(self.points) {
            return Err(too_many());
        }
        let Some((path, rule)) = node_path(n) else { return Ok(None) };
        let polys = self.polys(path.to_bezpath())?;
        Ok((!polys.is_empty()).then_some(Region::Shape { polys, rule }))
    }

    /// The box of an object whose outlines can't be read.
    fn frame(&mut self, n: &Node) -> Result<Option<Region>, String> {
        match n.geometric_bounds_with(&self.doc.symbol_boxes()) {
            Some(r) => self.polygon(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)]),
            None => Ok(None),
        }
    }

    /// A closed polygon through `points`.
    fn polygon(&mut self, points: &[Point]) -> Result<Option<Region>, String> {
        let poly = poly(points.to_vec(), true);
        self.count(poly.iter().map(|p| p.points.len()).sum())?;
        Ok(poly.map(|p| Region::Shape { polys: vec![p], rule: FillRule::NonZero }))
    }

    fn count(&mut self, n: usize) -> Result<(), String> {
        self.points = self.points.saturating_add(n);
        if self.points > MAX_POINTS { Err(too_many()) } else { Ok(()) }
    }

    /// The subpaths of `path` followed in straight segments.
    fn polys(&mut self, path: BezPath) -> Result<Vec<Poly>, String> {
        let (mut out, mut current, points) = (vec![], vec![], Cell::new(0usize));
        let left = MAX_POINTS.saturating_sub(self.points);
        // Past the budget no further curve is followed (one curve gives at most
        // `MAX_FLATTEN_PER_CURVE` points), and the read fails below.
        let elements = path.into_iter().take_while(|_| points.get() <= left);
        vectorcraft_geom::flatten(elements, TOLERANCE, |el| {
            points.set(points.get() + 1);
            if points.get() > left {
                return;
            }
            match el {
                PathEl::MoveTo(p) => {
                    out.extend(poly(std::mem::take(&mut current), false));
                    current.push(p);
                }
                PathEl::LineTo(p) => current.push(p),
                PathEl::ClosePath => out.extend(poly(std::mem::take(&mut current), true)),
                PathEl::QuadTo(..) | PathEl::CurveTo(..) => {}
            }
        });
        if points.get() > left {
            return Err(too_many());
        }
        out.extend(poly(current, false));
        self.count(out.iter().map(|p: &Poly| p.points.len()).sum())?;
        Ok(out)
    }
}

fn too_many() -> String {
    format!("the selected art has too many outline points to measure (more than {MAX_POINTS})")
}

/// The anchors of a path or compound path node, read without copying its path; 0 for type.
fn anchors(n: &Node) -> usize {
    match &n.kind {
        NodeKind::Path { path, .. } => path.anchor_count(),
        NodeKind::Compound { children, .. } => children.iter().fold(0, |sum, c| sum.saturating_add(anchors(c))),
        _ => 0,
    }
}

/// A subpath through `points`, `None` for fewer than two points or one that isn't finite.
fn poly(points: Vec<Point>, closed: bool) -> Option<Poly> {
    if points.len() < 2 || points.iter().any(|p| !p.x.is_finite() || !p.y.is_finite()) {
        return None;
    }
    let first = *points.first()?;
    let bounds = points.iter().fold(Rect::from_points(first, first), |r, p| r.union_pt(*p));
    Some(Poly { points, closed, bounds })
}

/// One region of `parts`, `None` when there are none.
fn union(mut parts: Vec<Region>) -> Option<Region> {
    match parts.len() {
        0 => None,
        1 => parts.pop(),
        _ => Some(Region::Union(parts)),
    }
}

/// `p` as (along, across) a weeding line: horizontal lines run along x, vertical ones along y.
fn uv(p: Point, vertical: bool) -> (f64, f64) {
    if vertical { (p.y, p.x) } else { (p.x, p.y) }
}

/// Sorts `spans` and joins those that overlap or touch.
fn merged(mut spans: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    spans.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(spans.len());
    for (a, b) in spans {
        match out.last_mut() {
            Some(last) if a <= last.1 => last.1 = last.1.max(b),
            _ => out.push((a, b)),
        }
    }
    out
}

/// Where sorted, joined spans `a` and `b` overlap.
fn intersected(a: &[(f64, f64)], b: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let (mut i, mut j, mut out) = (0, 0, vec![]);
    while let (Some(&(a0, a1)), Some(&(b0, b1))) = (a.get(i), b.get(j)) {
        let (lo, hi) = (a0.max(b0), a1.min(b1));
        if lo <= hi {
            out.push((lo, hi));
        }
        if a1 < b1 {
            i += 1;
        } else {
            j += 1;
        }
    }
    out
}

/// Where segment a→b crosses the line `across` = `c`, along it, and which way (1 when `across`
/// grows from a to b), `None` when it doesn't. Each crossing counts once: an end exactly on the
/// line counts as past it.
fn crosses(a: Point, b: Point, vertical: bool, c: f64) -> Option<(f64, i32)> {
    let ((u0, v0), (u1, v1)) = (uv(a, vertical), uv(b, vertical));
    ((v0 <= c && v1 > c) || (v1 <= c && v0 > c)).then(|| (u0 + (c - v0) * (u1 - u0) / (v1 - v0), if v1 > v0 { 1 } else { -1 }))
}

/// The stretches inside an area filled by `rule`, from where its outlines cross a line, sorted
/// along it.
fn filled(crossings: impl IntoIterator<Item = (f64, i32)>, rule: FillRule) -> Vec<(f64, f64)> {
    let (mut winding, mut start, mut out) = (0, 0.0, vec![]);
    let inside = |w: i32| match rule {
        FillRule::NonZero => w != 0,
        FillRule::EvenOdd => w % 2 != 0,
    };
    for (u, dir) in crossings {
        let was = inside(winding);
        winding += dir;
        match (was, inside(winding)) {
            (false, true) => start = u,
            (true, false) => out.push((start, u)),
            _ => {}
        }
    }
    merged(out)
}

/// Where the closed outlines of a region's shapes cross a set of weeding lines: for each line, the
/// crossings as (shape, place along the line, direction), sorted by shape and then by place. Shapes
/// are numbered in the order [`Region::shapes`] lists them.
pub(super) type Crossings = Vec<Vec<(usize, f64, i32)>>;

/// Where the shapes of each part of a region are numbered among its [`Crossings`].
#[derive(Debug)]
pub(super) enum Layout {
    Shape,
    /// The number of each part's first shape, from the union's first, and the parts' layouts.
    Union {
        starts: Vec<usize>,
        parts: Vec<Layout>,
    },
    /// The clipping path's shapes come first.
    Clip {
        clip_shapes: usize,
        clip: Box<Layout>,
        content: Box<Layout>,
    },
}

impl Layout {
    /// The layout of `region` and how many shapes it has.
    pub(super) fn of(region: &Region) -> (Layout, usize) {
        match region {
            Region::Shape { .. } => (Layout::Shape, 1),
            Region::Union(parts) => {
                let (mut starts, mut layouts, mut n) = (vec![], vec![], 0);
                for r in parts {
                    let (l, k) = Layout::of(r);
                    starts.push(n);
                    layouts.push(l);
                    n += k;
                }
                (Layout::Union { starts, parts: layouts }, n)
            }
            Region::Clip { clip, content } => {
                let ((lc, a), (lt, b)) = (Layout::of(clip), Layout::of(content));
                (Layout::Clip { clip_shapes: a, clip: Box::new(lc), content: Box::new(lt) }, a + b)
            }
        }
    }
}

/// The [`Crossings`] of `region` with the lines `across` = v0 + (k + 1)·step, k < `count`. Errs
/// past [`MAX_NEAR`] crossings in all.
pub(super) fn crossings(region: &Region, vertical: bool, v0: f64, step: f64, count: usize) -> Result<Crossings, String> {
    let mut shapes = vec![];
    region.shapes(&mut shapes);
    let at = |k: usize| v0 + step * (k as f64 + 1.0);
    // The lines a segment may cross: those from just below its lower end to its higher end.
    let candidates = |(a, b): Segment| {
        let (va, vb) = (uv(a, vertical).1, uv(b, vertical).1);
        if step <= 0.0 || va == vb {
            return 0..0;
        }
        let first = (((va.min(vb) - v0) / step).floor() - 1.0).clamp(0.0, count as f64);
        let end = ((va.max(vb) - v0) / step).ceil().clamp(0.0, count as f64);
        (first as usize)..(end as usize)
    };
    let closed = |polys: &[Poly]| polys.iter().filter(|p| p.closed).flat_map(Poly::segments).collect::<Vec<_>>();
    let total = shapes.iter().flat_map(|(polys, _)| closed(polys)).fold(0usize, |n, s| n.saturating_add(candidates(s).len()));
    if total > MAX_NEAR {
        return Err("the weeding lines cross the art's outlines too many times to measure; use fewer lines".into());
    }
    let mut out: Crossings = vec![vec![]; count];
    for (i, (polys, _)) in shapes.iter().enumerate() {
        for (a, b) in closed(polys) {
            for k in candidates((a, b)) {
                if let (Some((u, dir)), Some(line)) = (crosses(a, b, vertical, at(k)), out.get_mut(k)) {
                    line.push((i, u, dir));
                }
            }
        }
    }
    for line in &mut out {
        line.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
    }
    Ok(out)
}

impl Region {
    /// The stretches of the line `across` = `c` (horizontal: y = c, along x; vertical: x = c,
    /// along y) inside the region's area, sorted and joined.
    pub(super) fn spans(&self, vertical: bool, c: f64) -> Vec<(f64, f64)> {
        match self {
            Region::Shape { polys, rule } => {
                let mut crossings: Vec<(f64, i32)> = vec![];
                for p in polys.iter().filter(|p| p.closed) {
                    let (lo, hi) = if vertical { (p.bounds.x0, p.bounds.x1) } else { (p.bounds.y0, p.bounds.y1) };
                    if c >= lo && c <= hi {
                        crossings.extend(p.segments().filter_map(|(a, b)| crosses(a, b, vertical, c)));
                    }
                }
                crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
                filled(crossings, *rule)
            }
            Region::Union(parts) => merged(parts.iter().flat_map(|r| r.spans(vertical, c)).collect()),
            Region::Clip { clip, content } => intersected(&clip.spans(vertical, c), &content.spans(vertical, c)),
        }
    }

    /// The region's shapes with their fill rules, in a fixed order.
    fn shapes<'a>(&'a self, out: &mut Vec<(&'a [Poly], FillRule)>) {
        match self {
            Region::Shape { polys, rule } => out.push((polys, *rule)),
            Region::Union(parts) => parts.iter().for_each(|r| r.shapes(out)),
            Region::Clip { clip, content } => {
                clip.shapes(out);
                content.shapes(out);
            }
        }
    }

    /// [`Region::spans`] on a weeding line, from `line`, the line's [`Crossings`] with the
    /// region's shapes, numbered from `base` (`layout` is the region's [`Layout`]). Only the parts
    /// the line crosses are visited: a part it doesn't cross has no stretch inside its area.
    pub(super) fn spans_on(&self, layout: &Layout, line: &[(usize, f64, i32)], base: usize) -> Vec<(f64, f64)> {
        if line.is_empty() {
            return vec![];
        }
        match (self, layout) {
            (Region::Shape { rule, .. }, Layout::Shape) => filled(line.iter().map(|&(_, u, dir)| (u, dir)), *rule),
            (Region::Union(parts), Layout::Union { starts, parts: layouts }) => {
                let (mut out, mut rest) = (vec![], line);
                while let Some(&(id, ..)) = rest.first() {
                    // The part whose shapes the next crossing is on, and its crossings.
                    let j = starts.partition_point(|&s| base + s <= id).saturating_sub(1);
                    let (Some(r), Some(l), Some(&start)) = (parts.get(j), layouts.get(j), starts.get(j)) else { break };
                    let end = starts.get(j + 1).map_or(usize::MAX, |&s| base + s);
                    let n = rest.partition_point(|c| c.0 < end);
                    if n == 0 {
                        break;
                    }
                    let (mine, after) = rest.split_at(n);
                    out.extend(r.spans_on(l, mine, base + start));
                    rest = after;
                }
                merged(out)
            }
            (Region::Clip { clip, content }, Layout::Clip { clip_shapes, clip: lc, content: lt }) => {
                let (a, b) = line.split_at(line.partition_point(|c| c.0 < base + clip_shapes));
                if a.is_empty() || b.is_empty() {
                    return vec![];
                }
                intersected(&clip.spans_on(lc, a, base), &content.spans_on(lt, b, base + clip_shapes))
            }
            _ => vec![],
        }
    }

    /// Is `p` inside the region's area or on one of `segments`, the outlines it shows?
    fn contains(&self, p: Point, segments: &[Segment]) -> bool {
        const EPS: f64 = 1e-7;
        self.spans(false, p.y).iter().any(|&(a, b)| p.x >= a - EPS && p.x <= b + EPS) || segments.iter().any(|&(a, b)| distance(p, a, b) <= EPS)
    }

    /// The pieces of outline the region shows: every outline of a shape and of a group's members,
    /// and in a clip group the members' outlines inside the clipping path and the clipping path's
    /// outline where the members' area or outlines are. Errs past [`MAX_PAIRS`], counted over
    /// every clip group in the region together.
    pub(super) fn outline(&self) -> Result<Vec<Segment>, String> {
        self.outline_counted(&mut 0)
    }

    /// [`Region::outline`], adding the measure's work to `work`.
    fn outline_counted(&self, work: &mut usize) -> Result<Vec<Segment>, String> {
        match self {
            Region::Shape { polys, .. } => Ok(polys.iter().flat_map(Poly::segments).collect()),
            Region::Union(parts) => {
                let mut out = vec![];
                for r in parts {
                    out.extend(r.outline_counted(work)?);
                }
                Ok(out)
            }
            Region::Clip { clip, content } => {
                // Each side's area is bounded by the outlines it shows, so the pieces are cut and
                // tested against those, never against art a nested clip hides.
                let (clip_shown, content_shown) = (clip.outline_counted(work)?, content.outline_counted(work)?);
                let mut out = within(&content_shown, clip, &clip_shown, work)?;
                out.extend(within(&clip_shown, content, &content_shown, work)?);
                Ok(out)
            }
        }
    }

    /// The points of the region's closed outlines, those a nested clip hides included: what one
    /// [`Region::spans`] looks at, at most.
    fn closed_points(&self) -> usize {
        match self {
            Region::Shape { polys, .. } => polys.iter().filter(|p| p.closed).map(|p| p.points.len()).sum(),
            Region::Union(parts) => parts.iter().map(Region::closed_points).sum(),
            Region::Clip { clip, content } => clip.closed_points().saturating_add(content.closed_points()),
        }
    }
}

fn too_complex_clip() -> String {
    "a clip group in the selection has too many outline segments to measure".into()
}

/// The pieces of `segments` inside `region` (or on its outlines), cut where they cross
/// `region_segments`, the outlines the region shows. Each segment, and each piece of one tested
/// for being inside, adds the region's closed outline points (hidden ones included) and its
/// shown segment count to `work`, and the measure errs past [`MAX_PAIRS`].
fn within(segments: &[Segment], region: &Region, region_segments: &[Segment], work: &mut usize) -> Result<Vec<Segment>, String> {
    let mut out = vec![];
    let cost = region.closed_points().saturating_add(region_segments.len()).max(1);
    let charge = |work: &mut usize| {
        *work = work.saturating_add(cost);
        if *work > MAX_PAIRS { Err(too_complex_clip()) } else { Ok(()) }
    };
    for &(a, b) in segments {
        charge(work)?;
        let mut cuts = vec![0.0, 1.0];
        let seg_box = Rect::from_points(a, b);
        for &(c, d) in region_segments {
            if touches(seg_box, Rect::from_points(c, d)) {
                cuts.extend(crossing(a, b, c, d));
            }
        }
        cuts.sort_by(f64::total_cmp);
        cuts.dedup_by(|x, y| (*x - *y).abs() < 1e-12);
        let at = |t: f64| a + (b - a) * t;
        for w in cuts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 < 1e-12 {
                continue;
            }
            charge(work)?;
            if region.contains(at((t0 + t1) / 2.0), region_segments) {
                out.push((at(t0), at(t1)));
            }
        }
    }
    Ok(out)
}

/// Do boxes `a` and `b` overlap or touch?
fn touches(a: Rect, b: Rect) -> bool {
    a.x0 <= b.x1 && b.x0 <= a.x1 && a.y0 <= b.y1 && b.y0 <= a.y1
}

/// Where along a→b (0 to 1) segment c→d crosses or touches it: one place, or the ends of the
/// shared stretch when the two lie on one line.
fn crossing(a: Point, b: Point, c: Point, d: Point) -> Vec<f64> {
    let (r, s) = (b - a, d - c);
    let denom = r.cross(s);
    let qp = c - a;
    let len2 = r.hypot2();
    if len2 == 0.0 {
        return vec![];
    }
    if denom.abs() <= 1e-12 * len2.sqrt() * s.hypot().max(1e-300) {
        // Parallel: the ends of c→d on a→b when the two lie on one line.
        if qp.cross(r).abs() > 1e-9 * len2.sqrt() {
            return vec![];
        }
        return [c, d].iter().map(|p| (*p - a).dot(r) / len2).filter(|t| (0.0..=1.0).contains(t)).collect();
    }
    let t = qp.cross(s) / denom;
    let u = qp.cross(r) / denom;
    if (0.0..=1.0).contains(&t) && (0.0..=1.0).contains(&u) { vec![t] } else { vec![] }
}

/// The distance from `p` to segment a→b.
fn distance(p: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let len2 = ab.hypot2();
    let t = if len2 > 0.0 { ((p - a).dot(ab) / len2).clamp(0.0, 1.0) } else { 0.0 };
    (p - (a + ab * t)).hypot()
}

/// The stretch of the line `across` = `c` within `gap` of segment a→b, `None` when the line
/// passes farther away. With no gap it is where the segment meets the line.
fn near(a: Point, b: Point, vertical: bool, c: f64, gap: f64) -> Option<(f64, f64)> {
    let ((u0, v0), (u1, v1)) = (uv(a, vertical), uv(b, vertical));
    if gap <= 0.0 {
        if v0 == v1 {
            return (v0 == c).then_some((u0.min(u1), u0.max(u1)));
        }
        if c < v0.min(v1) || c > v0.max(v1) {
            return None;
        }
        let u = u0 + (c - v0) * (u1 - u0) / (v1 - v0);
        return Some((u, u));
    }
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    let mut take = |x: f64, y: f64| {
        lo = lo.min(x);
        hi = hi.max(y);
    };
    // The round ends.
    for (u, v) in [(u0, v0), (u1, v1)] {
        let dv = (c - v).abs();
        if dv <= gap {
            let h = (gap * gap - dv * dv).max(0.0).sqrt();
            take(u - h, u + h);
        }
    }
    // The band along the segment: points whose foot falls on it, within the gap of it.
    let (du, dv) = (u1 - u0, v1 - v0);
    let len = du.hypot(dv);
    if len > 0.0 {
        // Each condition is linear in the position x along the line: k·x + m ≥ 0.
        let mut range = (f64::NEG_INFINITY, f64::INFINITY);
        let mut keep = |k: f64, m: f64| {
            if k == 0.0 {
                if m < 0.0 {
                    range = (1.0, 0.0);
                }
            } else if k > 0.0 {
                range.0 = range.0.max(-m / k);
            } else {
                range.1 = range.1.min(-m / k);
            }
        };
        // The foot at or after a: (x - u0)·du + (c - v0)·dv ≥ 0.
        keep(du, -u0 * du + (c - v0) * dv);
        // The foot at or before b.
        keep(-du, u0 * du - (c - v0) * dv + len * len);
        // Within the gap on either side: |(x - u0)·dv - (c - v0)·du| ≤ gap·len.
        let cross0 = -u0 * dv - (c - v0) * du;
        keep(-dv, -cross0 + gap * len);
        keep(dv, cross0 + gap * len);
        if range.0 <= range.1 {
            take(range.0, range.1);
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// How many weeding lines run each way.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Counts {
    pub horizontal: usize,
    pub vertical: usize,
}

/// The weeding lines across `frame`: `n.horizontal` horizontal and `n.vertical` vertical lines that
/// divide it evenly, each from one side of the frame to the other, left out where it is inside the
/// region's area or within `gap` of one of its outlines (`outline`). Pieces are in order along each
/// line, the horizontal lines first, top to bottom. Errs past [`MAX_NEAR`] pairs of an outline
/// segment and a line it comes within `gap` of, past [`MAX_NEAR`] crossings of the region's
/// outlines with the lines one way, or past `max_pieces` pieces.
pub(super) fn lines(region: &Region, outline: &[Segment], frame: Rect, n: Counts, gap: f64, max_pieces: usize) -> Result<Vec<Segment>, String> {
    let mut out = vec![];
    let (layout, _) = Layout::of(region);
    for (vertical, count) in [(false, n.horizontal), (true, n.vertical)] {
        let ((u0, v0), (u1, v1)) = (uv(Point::new(frame.x0, frame.y0), vertical), uv(Point::new(frame.x1, frame.y1), vertical));
        let step = (v1 - v0) / (count as f64 + 1.0);
        // A frame with no height (for horizontal lines) or no width has no waste to divide.
        if step <= 0.0 {
            continue;
        }
        // The lines (0-based) each outline segment comes within `gap` of: lines k (1-based) lie at
        // v0 + k·step. The range is widened by a small tolerance so that a line whose position
        // rounds just past a segment's end is still measured; `near` drops lines the segment misses.
        let reach = |&(a, b): &Segment| {
            let (va, vb) = (uv(a, vertical).1, uv(b, vertical).1);
            let (lo, hi) = (va.min(vb) - gap, va.max(vb) + gap);
            let first = ((lo - v0) / step - 1e-9).ceil().max(1.0);
            let last = ((hi - v0) / step + 1e-9).floor().min(count as f64);
            if step > 0.0 && first <= last { (first as usize - 1)..(last as usize) } else { 0..0 }
        };
        // The outlines near each line, found once for all of them, by index into `outline`.
        let pairs = outline.iter().fold(0usize, |sum, s| sum.saturating_add(reach(s).len()));
        if pairs > MAX_NEAR {
            return Err("the weeding lines pass too close to too many outline segments to measure; use fewer lines or a smaller gap".into());
        }
        let mut near_line: Vec<Vec<usize>> = vec![vec![]; count];
        for (j, s) in outline.iter().enumerate() {
            for k in reach(s) {
                if let Some(list) = near_line.get_mut(k) {
                    list.push(j);
                }
            }
        }
        let crossed = crossings(region, vertical, v0, step, count)?;
        for (i, segments) in near_line.iter().enumerate() {
            let c = v0 + step * (i as f64 + 1.0);
            let mut blocked = region.spans_on(&layout, crossed.get(i).map(Vec::as_slice).unwrap_or_default(), 0);
            blocked.extend(segments.iter().filter_map(|&j| outline.get(j)).filter_map(|&(a, b)| near(a, b, vertical, c, gap)));
            let mut from = u0;
            let point = |u: f64| if vertical { Point::new(c, u) } else { Point::new(u, c) };
            for (a, b) in merged(blocked) {
                if a - from >= MIN_PIECE && a <= u1 {
                    out.push((point(from), point(a.min(u1))));
                }
                from = from.max(b);
            }
            if u1 - from >= MIN_PIECE {
                out.push((point(from), point(u1)));
            }
            if out.len() > max_pieces {
                return Err(format!("the weeding lines would be more than {max_pieces} pieces"));
            }
        }
    }
    Ok(out)
}
