//! Illustrator's Pathfinder panel and Shape Builder regions, over an ordered stack of shapes
//! (index 0 = back-most, last = front-most).

use std::collections::HashMap;

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, Point, Shape as _};
use linesweeper::topology::ContourIdx;
use vectorcraft_geom::{FillRule, PathData};

use crate::boolean::{Arrangement, Multi, Seg, all_contours_to_path, contours_to_path, fill_bezpath, normalize_bez, segs_to_subpath, unite_all};

/// A filled shape in a Pathfinder stack. `key` identifies its paint (e.g. a hashed fill colour);
/// results carry the key of the object whose paint they keep.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub path: PathData,
    pub rule: FillRule,
    pub key: u64,
}

impl Shape {
    pub fn new(path: PathData, rule: FillRule, key: u64) -> Self {
        Self { path, rule, key }
    }
}

/// The ten Pathfinder panel operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathfinderOp {
    /// Shape mode: union of everything; result keeps the front-most key.
    Unite,
    /// Shape mode: back-most minus everything in front; keeps the back-most key.
    MinusFront,
    /// Shape mode: area covered by *all* shapes; keeps the front-most key.
    Intersect,
    /// Shape mode: area covered by an odd number of shapes; keeps the front-most key.
    Exclude,
    /// Split into every non-overlapping face; each face keeps the key of the front-most shape
    /// covering it.
    Divide,
    /// Remove hidden parts; one result per shape that remains visible (no merging).
    Trim,
    /// Trim, then merge touching/overlapping visible parts with the same key.
    Merge,
    /// Keep the visible parts of lower shapes that lie inside the front-most shape (which is removed).
    Crop,
    /// Split every edge at intersections; returns *open* paths keyed by the shape they bound.
    Outline,
    /// Front-most minus everything behind; keeps the front-most key.
    MinusBack,
}

/// One face of the planar arrangement of a set of shapes (Shape Builder / Live Paint face).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Face outline (outer contour plus holes, consistently oriented).
    pub path: PathData,
    /// Indices of the input shapes covering this face, ascending.
    pub sources: Vec<usize>,
}

impl Region {
    /// Front-most covering shape (`None` only for an uncovered face, which regions never are).
    pub fn top(&self) -> Option<usize> {
        self.sources.last().copied()
    }
    /// Does the face contain `p`?
    pub fn contains(&self, p: Point) -> bool {
        self.path.to_bezpath().winding(p) != 0
    }
}

fn arrangement(shapes: &[Shape]) -> Option<Arrangement> {
    let bps: Vec<(BezPath, FillRule)> = shapes.iter().map(|s| (fill_bezpath(&s.path), s.rule)).collect();
    Arrangement::new(bps).ok()
}

fn topmost(m: &[bool]) -> Option<usize> {
    m.iter().rposition(|&b| b)
}

fn one(path: PathData, key: u64) -> Vec<Shape> {
    if path.is_empty() { Vec::new() } else { vec![Shape::new(path, FillRule::NonZero, key)] }
}

/// Run a Pathfinder operation over `shapes` (back → front).
pub fn pathfinder(op: PathfinderOp, shapes: &[Shape]) -> Vec<Shape> {
    let n = shapes.len();
    if n == 0 {
        return Vec::new();
    }
    let front_key = shapes[n - 1].key;
    let unite = || {
        let v: Vec<(&PathData, FillRule)> = shapes.iter().map(|s| (&s.path, s.rule)).collect();
        one(unite_all(&v), front_key)
    };
    // Unite doesn't need the arrangement.
    if op == PathfinderOp::Unite {
        return unite();
    }
    let Some(arr) = arrangement(shapes) else { return Vec::new() };
    let p = &arr.tidy;
    match op {
        PathfinderOp::Unite => unite(),
        PathfinderOp::MinusFront => one(all_contours_to_path(&arr.contours(|m| m[0] && !m[1..].iter().any(|&b| b)), p), shapes[0].key),
        PathfinderOp::MinusBack => one(all_contours_to_path(&arr.contours(|m| m[n - 1] && !m[..n - 1].iter().any(|&b| b)), p), front_key),
        PathfinderOp::Intersect => one(all_contours_to_path(&arr.contours(|m| m.iter().all(|&b| b)), p), front_key),
        PathfinderOp::Exclude => one(all_contours_to_path(&arr.contours(|m| m.iter().filter(|&&b| b).count() % 2 == 1), p), front_key),
        PathfinderOp::Divide => regions_of(&arr)
            .into_iter()
            .filter_map(|r| {
                let key = shapes.get(r.top()?)?.key;
                Some(Shape::new(r.path, FillRule::NonZero, key))
            })
            .collect(),
        PathfinderOp::Trim => (0..n).flat_map(|i| one(all_contours_to_path(&arr.contours(|m| topmost(m) == Some(i)), p), shapes[i].key)).collect(),
        PathfinderOp::Merge => {
            let mut keys: Vec<u64> = Vec::new();
            for s in shapes {
                if !keys.contains(&s.key) {
                    keys.push(s.key);
                }
            }
            let mut out = Vec::new();
            for k in keys {
                let c = arr.contours(|m| topmost(m).is_some_and(|t| shapes[t].key == k));
                for g in c.grouped() {
                    out.extend(one(contours_to_path(&c, g, p), k));
                }
            }
            out
        }
        PathfinderOp::Crop => {
            if n < 2 {
                return Vec::new();
            }
            (0..n - 1)
                .flat_map(|i| {
                    let c = arr.contours(|m| m[n - 1] && topmost(&m[..n - 1]) == Some(i));
                    one(all_contours_to_path(&c, p), shapes[i].key)
                })
                .collect()
        }
        PathfinderOp::Outline => outline_edges(&arr, shapes),
    }
}

/// The faces of `arr`, each distinct coverage mask's in turn (sorted), each face grouped as
/// `arr.contours(|m| m == mask).grouped()` groups it.
///
/// A contour pass walks the whole arrangement, so one pass per mask would cost masks × edges
/// (two blends crossing make thousands of each). Instead the masks go into batches whose faces
/// share no vertex (so no edge either): one pass per batch walks each face's contours exactly as
/// its own mask's pass would, and the mask a contour bounds is the one of its batch at its first
/// point.
fn regions_of(arr: &Arrangement) -> Vec<Region> {
    let masks = arr.distinct_masks();
    let ids: HashMap<&[bool], usize> = masks.iter().enumerate().map(|(i, m)| (m.as_slice(), i)).collect();
    let id = |w: &Multi| ids.get(arr.mask(w).as_slice()).copied();
    let top = &arr.top;
    // The masks of the faces around each vertex.
    let mut around: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
    for s in top.segment_indices() {
        let h = s.first_half();
        let sides = [id(top.winding_clockwise(h)), id(top.winding_counter_clockwise(h))];
        for end in [h, s.second_half()] {
            let at = around.entry(vertex_key(top.point(end).to_kurbo())).or_default();
            for m in sides.into_iter().flatten() {
                if !at.contains(&m) {
                    at.push(m);
                }
            }
        }
    }
    let mut near = vec![Vec::new(); masks.len()];
    for at in around.values() {
        for &a in at {
            near[a].extend(at.iter().copied().filter(|&b| b != a));
        }
    }
    // Greedy colouring: each mask takes the first batch none of its neighbours is in.
    let mut batch = vec![usize::MAX; masks.len()];
    let mut batches = 0;
    for m in 0..masks.len() {
        let mut taken = vec![false; batches];
        for &n in &near[m] {
            if let Some(t) = taken.get_mut(batch[n]) {
                *t = true;
            }
        }
        batch[m] = taken.iter().position(|t| !t).unwrap_or(batches);
        batches = batches.max(batch[m] + 1);
    }
    let mut faces: Vec<Vec<PathData>> = vec![Vec::new(); masks.len()];
    for b in 0..batches {
        let c = top.contours(|w| id(w).is_some_and(|m| batch[m] == b));
        let owner: Option<Vec<usize>> = c
            .contours()
            .map(|ct| {
                let Some(kurbo::PathEl::MoveTo(p)) = ct.path.elements().first() else { return None };
                around.get(&vertex_key(*p))?.iter().copied().find(|&m| batch[m] == b)
            })
            .collect();
        let Some(owner) = owner else {
            // Not expected: every contour starts at a vertex of the arrangement. Fall back to a
            // pass per mask for this batch.
            for (m, mask) in masks.iter().enumerate().filter(|(m, _)| batch[*m] == b) {
                let c = arr.contours(|k| k == mask.as_slice());
                faces[m].extend(c.grouped().into_iter().map(|g| contours_to_path(&c, g, &arr.tidy)));
            }
            continue;
        };
        // A contour belongs under the nearest enclosing contour of the same mask, as in that
        // mask's own pass.
        let mut children = vec![Vec::new(); owner.len()];
        let mut roots = Vec::new();
        for (i, &m) in owner.iter().enumerate() {
            let mut up = c[ContourIdx(i)].parent;
            while let Some(p) = up.filter(|p| owner.get(p.0) != Some(&m)) {
                up = c[p].parent;
            }
            match up {
                Some(p) => children[p.0].push(i),
                None => roots.push(i),
            }
        }
        for r in roots {
            let mut group = Vec::new();
            let mut stack = vec![r];
            while let Some(i) = stack.pop() {
                group.push(ContourIdx(i));
                stack.extend(children[i].iter().rev());
            }
            faces[owner[r]].push(contours_to_path(&c, group, &arr.tidy));
        }
    }
    let mut out = Vec::new();
    for (mask, paths) in masks.iter().zip(faces) {
        let sources: Vec<usize> = mask.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect();
        out.extend(paths.into_iter().filter(|p| !p.is_empty()).map(|path| Region { path, sources: sources.clone() }));
    }
    out
}

/// A point as a map key (the arrangement's vertices are exact).
fn vertex_key(p: Point) -> (u64, u64) {
    (p.x.to_bits(), p.y.to_bits())
}

/// All faces of the planar arrangement of `shapes` (every area covered by at least one shape,
/// split wherever coverage changes).
pub fn regions(shapes: &[Shape]) -> Vec<Region> {
    arrangement(shapes).map(|a| regions_of(&a)).unwrap_or_default()
}

/// The face under `point`, if any (Shape Builder hover/click).
pub fn region_at(shapes: &[Shape], point: Point) -> Option<Region> {
    regions(shapes).into_iter().find(|r| r.contains(point))
}

/// Merge Shape Builder regions into one path (their union).
pub fn merge_regions(regions: &[&Region]) -> PathData {
    let v: Vec<(&PathData, FillRule)> = regions.iter().map(|r| (&r.path, FillRule::NonZero)).collect();
    unite_all(&v)
}

/// Merges faces of the planar arrangement of a set of shapes (as [`regions`] gives them) into one
/// path, from the arrangement itself, made once for every merge of the set: the faces' contours
/// share their edges exactly, so the edges between merged faces vanish. Uniting the faces' tidied
/// outlines ([`merge_regions`]) could leave those edges in, as a hairline gap or a spur, where
/// neighbouring faces' outlines had been refit apart. Shapes with open paths (faces lines cut, as
/// [`shape_builder`](crate::shape_builder) gives them) merge in their planar map the same way
/// (#893). Either falls back to [`merge_regions`] when it can't place a face.
pub struct FaceMerger {
    /// The arrangement (`None`: open paths, or the sweep failed) and the number of shapes.
    arr: Option<Arrangement>,
    shapes: usize,
    /// With open paths, the shapes, and their planar map once a merge has needed it.
    lines: Option<(Vec<Shape>, std::cell::OnceCell<Option<crate::planar::Map>>)>,
}

impl FaceMerger {
    pub fn new(shapes: &[Shape]) -> Self {
        let has_open = shapes.iter().flat_map(|s| &s.path.subpaths).any(|sp| !sp.closed && sp.anchors.len() >= 2);
        // The Shape Builder's faces come from the planar map only up to its cap.
        let segments: usize = shapes.iter().flat_map(|s| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
        let lines = (has_open && segments <= crate::SHAPE_BUILDER_MAX_SEGMENTS).then(|| (shapes.to_vec(), std::cell::OnceCell::new()));
        Self { arr: if has_open { None } else { arrangement(shapes) }, shapes: shapes.len(), lines }
    }

    /// The union of `faces`.
    pub fn merge(&self, faces: &[&Region]) -> PathData {
        self.exact(faces).or_else(|| self.planar(faces)).filter(|p| !p.is_empty()).unwrap_or_else(|| merge_regions(faces))
    }

    /// `faces` merged in the planar map of shapes with open paths: each face found again by a
    /// point inside it, and the result checked to cover what they cover.
    fn planar(&self, faces: &[&Region]) -> Option<PathData> {
        let (shapes, map) = self.lines.as_ref()?;
        let map = map.get_or_init(|| crate::planar::map(shapes)).as_ref()?;
        let inside: Vec<Point> = faces.iter().map(|r| crate::planar::interior_point(&r.path)).collect::<Option<_>>()?;
        let merged = map.merge(&inside)?;
        // A face not found as itself (two points in one face of the map) shows in the area.
        let want: f64 = faces.iter().map(|r| crate::area(&r.path, FillRule::NonZero)).sum();
        let outlines: f64 = faces.iter().map(|r| r.path.to_bezpath().perimeter(1e-6)).sum();
        ((crate::area(&merged, FillRule::NonZero) - want).abs() <= 1e-9 * want.max(1.0) + crate::DEFAULT_PRECISION * outlines).then_some(merged)
    }

    fn exact(&self, faces: &[&Region]) -> Option<PathData> {
        let arr = self.arr.as_ref()?;
        // Each face by its coverage and a point inside it.
        let mut picks: Vec<(Vec<bool>, Point)> = Vec::with_capacity(faces.len());
        for r in faces {
            let mut mask = vec![false; self.shapes];
            for &s in &r.sources {
                *mask.get_mut(s)? = true;
            }
            picks.push((mask, crate::planar::interior_point(&r.path)?));
        }
        let mut raw = BezPath::new();
        let mut done: Vec<&[bool]> = vec![];
        for (mask, _) in &picks {
            if done.contains(&mask.as_slice()) {
                continue;
            }
            done.push(mask);
            let c = arr.contours(|k| k == mask.as_slice());
            for group in c.grouped() {
                let mut face = BezPath::new();
                for i in group {
                    face.extend(c[i].path.iter());
                }
                if picks.iter().any(|(m, p)| m == mask && face.winding(*p) != 0) {
                    raw.extend(face.iter());
                }
            }
        }
        let merged = normalize_bez(&raw, FillRule::NonZero).ok()?;
        Some(all_contours_to_path(&merged, &arr.tidy))
    }
}

/// Pathfinder Outline: split every shape's boundary at junctions of the arrangement.
fn outline_edges(arr: &Arrangement, shapes: &[Shape]) -> Vec<Shape> {
    let junctions = arr.junctions();
    let scale = shapes.iter().filter_map(|s| s.path.control_bounds()).map(|r| r.width().max(r.height())).fold(1.0, f64::max);
    let tol = scale * 1e-7;
    // (chain, key, source index)
    let mut chains: Vec<(Vec<Seg>, u64)> = Vec::new();
    for s in shapes {
        for sp in &s.path.subpaths {
            if sp.anchors.len() < 2 {
                continue;
            }
            let mut sp = sp.clone();
            sp.closed = true;
            // Split each segment at junctions; record which piece starts are breaks.
            let mut pieces: Vec<(Seg, bool)> = Vec::new();
            for i in 0..sp.segment_count() {
                let c = sp.segment(i);
                let line = sp.segment_is_line(i);
                let mut ts: Vec<f64> = junctions
                    .iter()
                    .filter_map(|&j| {
                        let nr = c.nearest(j, 1e-9);
                        (nr.distance_sq.sqrt() <= tol.max(1e-6)).then_some(nr.t)
                    })
                    .collect();
                ts.sort_by(f64::total_cmp);
                let start_is_junction = ts.first().is_some_and(|&t| t < 1e-9);
                let mut cuts: Vec<f64> = vec![0.0];
                cuts.extend(ts.into_iter().filter(|&t| t > 1e-9 && t < 1.0 - 1e-9));
                cuts.push(1.0);
                cuts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
                for (k, w) in cuts.windows(2).enumerate() {
                    let sub = c.subsegment(w[0]..w[1]);
                    let seg = if line { Seg::line(sub.p0, sub.p3) } else { Seg { c: sub, line: false } };
                    pieces.push((seg, if k == 0 { start_is_junction } else { true }));
                }
            }
            // Chain pieces between breaks.
            let first_break = pieces.iter().position(|(_, b)| *b);
            match first_break {
                None => {
                    let segs: Vec<Seg> = pieces.into_iter().map(|(s, _)| s).collect();
                    chains.push((segs, s.key));
                }
                Some(k) => {
                    pieces.rotate_left(k);
                    let mut cur: Vec<Seg> = Vec::new();
                    for (seg, brk) in pieces {
                        if brk && !cur.is_empty() {
                            chains.push((std::mem::take(&mut cur), s.key));
                        }
                        cur.push(seg);
                    }
                    if !cur.is_empty() {
                        chains.push((cur, s.key));
                    }
                }
            }
        }
    }
    // De-duplicate coincident chains, keeping the front-most key (later shapes overwrite).
    let mut out: Vec<(Vec<Seg>, u64)> = Vec::new();
    let sig = |c: &[Seg]| -> (Point, Point, Point) {
        let a = c[0].c.p0;
        let b = c[c.len() - 1].c.p3;
        let mid = mid_point(c);
        (a, b, mid)
    };
    for (chain, key) in chains {
        let (a, b, m) = sig(&chain);
        let near = |p: Point, q: Point| p.distance(q) <= tol.max(1e-6) * 10.0;
        if let Some(e) = out.iter_mut().find(|(c, _)| {
            let (a2, b2, m2) = sig(c);
            near(m, m2) && ((near(a, a2) && near(b, b2)) || (near(a, b2) && near(b, a2)))
        }) {
            e.1 = key;
        } else {
            out.push((chain, key));
        }
    }
    out.into_iter()
        .filter_map(|(chain, key)| {
            // Chain joints are the shapes' own anchors, so there is nothing to re-merge.
            let sp = segs_to_subpath(&chain, false)?;
            Some(Shape::new(PathData::single(sp), FillRule::NonZero, key))
        })
        .collect()
}

fn mid_point(c: &[Seg]) -> Point {
    let lens: Vec<f64> = c.iter().map(|s| kurbo::ParamCurveArclen::arclen(&s.c, 1e-6)).collect();
    let total: f64 = lens.iter().sum();
    let mut acc = 0.0;
    for (s, l) in c.iter().zip(&lens) {
        if acc + l >= total / 2.0 && *l > 0.0 {
            let t = kurbo::ParamCurveArclen::inv_arclen(&s.c, total / 2.0 - acc, 1e-6);
            return s.c.eval(t);
        }
        acc += l;
    }
    c[0].c.p0
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use vectorcraft_geom::{Rect, SubPath, shapes};

    /// The faces one contour pass per mask finds: what [`regions_of`] must match exactly.
    fn regions_one_mask_at_a_time(arr: &Arrangement) -> Vec<Region> {
        let mut out = Vec::new();
        for mask in arr.distinct_masks() {
            let c = arr.contours(|m| m == mask.as_slice());
            let sources: Vec<usize> = mask.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect();
            for g in c.grouped() {
                let path = contours_to_path(&c, g, &arr.tidy);
                if !path.is_empty() {
                    out.push(Region { path, sources: sources.clone() });
                }
            }
        }
        out
    }

    fn same_as_one_mask_at_a_time(shapes: &[Shape]) {
        let arr = arrangement(shapes).expect("arrangement");
        assert_eq!(regions_of(&arr), regions_one_mask_at_a_time(&arr));
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PathData {
        shapes::rectangle(Rect::new(x0, y0, x1, y1))
    }

    /// Shapes on a coarse grid, so that edges and corners coincide: rectangles, rectangles with
    /// a hole, ellipses and polygons that may cross themselves, under either fill rule.
    fn arb_shape() -> impl Strategy<Value = Shape> {
        let c = || (0..12i32).prop_map(|v| f64::from(v) * 10.0);
        let r = (c(), c(), 1..6i32, 1..6i32).prop_map(|(x, y, w, h)| (x, y, x + f64::from(w) * 10.0, y + f64::from(h) * 10.0));
        let path = prop_oneof![
            r.clone().prop_map(|(x0, y0, x1, y1)| rect(x0, y0, x1, y1)),
            r.clone().prop_map(|(x0, y0, x1, y1)| {
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let mut p = rect(x0 - 10.0, y0 - 10.0, x1 + 10.0, y1 + 10.0);
                p.subpaths.extend(rect(x0, y0, cx.max(x0 + 5.0), cy.max(y0 + 5.0)).subpaths);
                p
            }),
            r.prop_map(|(x0, y0, x1, y1)| shapes::ellipse(Rect::new(x0, y0, x1, y1))),
            prop::collection::vec((c(), c()), 3..7).prop_map(|pts| {
                let pts: Vec<Point> = pts.into_iter().map(|(x, y)| Point::new(x, y)).collect();
                PathData::single(SubPath::polyline(&pts, true))
            }),
        ];
        (path, any::<bool>(), any::<u64>())
            .prop_map(|(p, even_odd, key)| Shape::new(p, if even_odd { FillRule::EvenOdd } else { FillRule::NonZero }, key))
    }

    #[test]
    fn crossing_strips_match_one_mask_at_a_time() {
        let mut v = Vec::new();
        for i in 0..12 {
            let x = 10.0 + 20.0 * f64::from(i);
            v.push(Shape::new(rect(x, 0.0, x + 4.0, 250.0), FillRule::NonZero, 1));
            v.push(Shape::new(rect(0.0, x, 250.0, x + 4.0), FillRule::NonZero, 2));
        }
        same_as_one_mask_at_a_time(&v);
        // Every crossing, and each strip in 13 pieces.
        assert_eq!(regions(&v).len(), 12 * 12 + 2 * 12 * 13);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, rng_seed: proptest::test_runner::RngSeed::Fixed(0x5eed_9a7f), ..ProptestConfig::default() })]

        /// Batching the masks changes nothing: the same regions, in the same order, with the same
        /// contours in each.
        #[test]
        fn batched_regions_match_one_mask_at_a_time(v in prop::collection::vec(arb_shape(), 1..8)) {
            same_as_one_mask_at_a_time(&v);
        }
    }
}
