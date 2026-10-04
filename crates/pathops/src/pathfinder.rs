//! Illustrator's Pathfinder panel and Shape Builder regions, over an ordered stack of shapes
//! (index 0 = back-most, last = front-most).

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, Point, Shape as _};
use vectorcraft_geom::{FillRule, PathData};

use crate::boolean::{Arrangement, Seg, all_contours_to_path, contours_to_path, fill_bezpath, segs_to_subpath, unite_all};

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
    /// Front-most covering shape.
    /// (Regions always have a source; an uncovered one would report the back-most shape.)
    pub fn top(&self) -> usize {
        self.sources.last().copied().unwrap_or(0)
    }
    /// Does the face contain `p`?
    pub fn contains(&self, p: Point) -> bool {
        self.path.to_bezpath().winding(p) != 0
    }
}

fn arrangement(shapes: &[Shape]) -> Option<Arrangement> {
    let bps: Vec<(BezPath, FillRule)> = shapes.iter().map(|s| (fill_bezpath(&s.path), s.rule)).collect();
    Arrangement::new(&bps).ok()
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
            .map(|r| {
                let key = shapes[r.top()].key;
                Shape::new(r.path, FillRule::NonZero, key)
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

fn regions_of(arr: &Arrangement) -> Vec<Region> {
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
