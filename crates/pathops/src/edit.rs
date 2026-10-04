//! Object → Path commands: Simplify, Smooth, Remove redundant points, Add Anchor Points, Average,
//! Join, Split Into Grid.

use kurbo::{ParamCurveArclen, Point, Rect, Vec2};
use vectorcraft_geom::{Anchor, PathData, SubPath};

use crate::boolean::{Seg, segs_to_subpath};
use crate::fit::{end_tangent, fit_cubics, fit_single, is_straight, sample, start_tangent};

/// Options for [`simplify_with`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SimplifyOptions {
    /// Maximum distance (document points) between the original and simplified path.
    pub tolerance: f64,
    /// Anchors where the path turns by more than this many degrees stay corners.
    pub corner_angle_deg: f64,
    /// Produce only straight segments (Illustrator's "Convert to Straight Lines").
    pub straight_lines: bool,
}

impl Default for SimplifyOptions {
    fn default() -> Self {
        Self { tolerance: 0.5, corner_angle_deg: 30.0, straight_lines: false }
    }
}

/// Simplify with the default corner threshold (30°).
pub fn simplify(path: &PathData, tolerance: f64) -> PathData {
    simplify_with(path, &SimplifyOptions { tolerance, ..Default::default() })
}

/// Simplify never adds anchors: keep whichever subpath is smaller.
fn no_worse(original: &SubPath, fitted: SubPath) -> SubPath {
    if fitted.anchors.len() > original.anchors.len() { original.clone() } else { fitted }
}

fn subpath_segs(sp: &SubPath) -> Vec<Seg> {
    (0..sp.segment_count()).map(|i| Seg { c: sp.segment(i), line: sp.segment_is_line(i) }).collect()
}

fn turn_deg(a: &Seg, b: &Seg) -> f64 {
    let d = end_tangent(&a.c).dot(start_tangent(&b.c)).clamp(-1.0, 1.0);
    d.acos().to_degrees()
}

/// Least-squares Bézier refit of every subpath, keeping corners sharper than the threshold.
pub fn simplify_with(path: &PathData, opts: &SimplifyOptions) -> PathData {
    let tol = opts.tolerance.max(1e-6);
    let mut out = Vec::new();
    for sp in &path.subpaths {
        let mut segs = subpath_segs(sp);
        let n = segs.len();
        if n == 0 {
            out.push(sp.clone());
            continue;
        }
        let corner_at = |segs: &[Seg], i: usize| -> bool {
            if !sp.closed && (i == 0 || i == n) {
                return true;
            }
            turn_deg(&segs[(i + n - 1) % n], &segs[i % n]) > opts.corner_angle_deg
        };
        let mut corners: Vec<usize> = (0..n).filter(|&i| corner_at(&segs, i)).collect();
        if sp.closed {
            if let Some(&k) = corners.first() {
                segs.rotate_left(k);
                corners = corners.iter().map(|&c| c - k).collect();
            } else {
                corners = vec![0];
            }
            corners.push(n);
        } else if corners.last() != Some(&n) {
            corners.push(n);
        }
        let mut result: Vec<Seg> = Vec::new();
        for w in corners.windows(2) {
            let run = &segs[w[0]..w[1]];
            if run.is_empty() {
                continue;
            }
            let mut pts = Vec::new();
            for (k, s) in run.iter().enumerate() {
                let len = s.c.arclen(1e-6);
                let m = ((len / (tol * 0.5)).ceil() as usize).clamp(4, 256);
                sample(&s.c, m, &mut pts, k == 0);
            }
            if opts.straight_lines {
                let keep = rdp(&pts, tol);
                result.extend(keep.windows(2).map(|p| Seg::line(p[0], p[1])));
                continue;
            }
            let t0 = start_tangent(&run[0].c);
            let t1 = end_tangent(&run[run.len() - 1].c);
            for c in fit_cubics(&pts, t0, t1, tol) {
                let line = is_straight(&c, tol * 0.02);
                result.push(if line { Seg::line(c.p0, c.p3) } else { Seg { c, line } });
            }
        }
        if let Some(s) = segs_to_subpath(&result, sp.closed) {
            out.push(if opts.straight_lines { s } else { no_worse(sp, s) });
        }
    }
    PathData::new(out)
}

/// Ramer–Douglas–Peucker polyline reduction.
fn rdp(pts: &[Point], tol: f64) -> Vec<Point> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut keep = vec![false; pts.len()];
    keep[0] = true;
    keep[pts.len() - 1] = true;
    let mut stack = vec![(0, pts.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        let (pa, pb) = (pts[a], pts[b]);
        let chord = pb - pa;
        let len = chord.hypot();
        let mut best = (0.0, a);
        for (i, &p) in pts.iter().enumerate().take(b).skip(a + 1) {
            let d = if len < 1e-12 { p.distance(pa) } else { (p - pa).cross(chord).abs() / len };
            if d > best.0 {
                best = (d, i);
            }
        }
        if best.0 > tol {
            keep[best.1] = true;
            stack.push((a, best.1));
            stack.push((best.1, b));
        }
    }
    pts.iter().zip(keep).filter(|(_, k)| *k).map(|(p, _)| *p).collect()
}

/// Object → Path → Smooth: pull handles towards Catmull-Rom-like tangents. `amount` ∈ [0, 1]
/// (0 = unchanged, 1 = fully smooth). Endpoints of open paths keep their handles.
pub fn smooth(path: &PathData, amount: f64) -> PathData {
    let t = amount.clamp(0.0, 1.0);
    let mut out = path.clone();
    for sp in &mut out.subpaths {
        let n = sp.anchors.len();
        if n < 3 {
            continue;
        }
        let orig = sp.anchors.clone();
        for i in 0..n {
            if !sp.closed && (i == 0 || i == n - 1) {
                continue;
            }
            let prev = orig[(i + n - 1) % n].p;
            let next = orig[(i + 1) % n].p;
            let p = orig[i].p;
            let Some(dir) = crate::fit::unit(next - prev) else { continue };
            let tin = p - dir * (p.distance(prev) / 3.0);
            let tout = p + dir * (p.distance(next) / 3.0);
            let a = &orig[i];
            let h_in = a.h_in.lerp(tin, t);
            let h_out = a.h_out.lerp(tout, t);
            sp.anchors[i] = Anchor::with_handles(p, h_in, h_out);
        }
    }
    out
}

/// Remove anchors that can be deleted without changing the shape by more than `tolerance`
/// (collinear corner points, duplicate points, and curve joints that one cubic can replace).
pub fn remove_redundant_points(path: &PathData, tolerance: f64) -> PathData {
    let tol = tolerance.max(1e-9);
    let mut out = Vec::new();
    for sp in &path.subpaths {
        let mut segs = subpath_segs(sp);
        if segs.is_empty() {
            out.push(sp.clone());
            continue;
        }
        // Drop zero-length segments.
        segs.retain(|s| !(s.c.p0.distance(s.c.p3) < 1e-9 && s.c.p1.distance(s.c.p0) < 1e-9 && s.c.p2.distance(s.c.p0) < 1e-9));
        let min_segs = if sp.closed { 2 } else { 1 };
        let mut changed = true;
        while changed && segs.len() > min_segs {
            changed = false;
            let n = segs.len();
            let joints = if sp.closed { n } else { n - 1 };
            for j in 0..joints {
                let (i0, i1) = if sp.closed { ((j + n - 1) % n, j) } else { (j, j + 1) };
                if let Some(m) = merge_pair(&segs[i0], &segs[i1], tol) {
                    segs[i0] = m;
                    segs.remove(i1);
                    if sp.closed && i1 == 0 {
                        // The merged segment now starts the loop.
                        if let Some(last) = segs.pop() {
                            segs.insert(0, last);
                        }
                    }
                    changed = true;
                    break;
                }
            }
        }
        if let Some(s) = segs_to_subpath(&segs, sp.closed) {
            out.push(s);
        }
    }
    PathData::new(out)
}

fn merge_pair(a: &Seg, b: &Seg, tol: f64) -> Option<Seg> {
    if a.line && b.line {
        let chord = b.c.p3 - a.c.p0;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let mid = a.c.p3 - a.c.p0;
        let along = mid.dot(chord) / len;
        return ((mid.cross(chord) / len).abs() <= tol && along >= 0.0 && along <= len).then(|| Seg::line(a.c.p0, b.c.p3));
    }
    // Only merge across (near-)smooth joints.
    if end_tangent(&a.c).dot(start_tangent(&b.c)) < 0.99 {
        return None;
    }
    let mut pts = Vec::new();
    sample(&a.c, 16, &mut pts, true);
    sample(&b.c, 16, &mut pts, false);
    let (c, err, _) = fit_single(&pts, start_tangent(&a.c), end_tangent(&b.c));
    (err <= tol).then_some(Seg { c, line: false })
}

/// Object → Path → Add Anchor Points: one new anchor at the middle (t = 0.5) of every segment.
pub fn add_anchor_points(path: &PathData) -> PathData {
    let mut out = path.clone();
    for sp in &mut out.subpaths {
        for seg in (0..sp.segment_count()).rev() {
            sp.insert_anchor(seg, 0.5);
        }
    }
    out
}

/// Axis for [`average`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AverageAxis {
    /// Align on a common horizontal line (same y).
    Horizontal,
    /// Align on a common vertical line (same x).
    Vertical,
    /// Collapse to the common centroid.
    Both,
}

/// Object → Path → Average: move the selected anchors `(subpath, anchor)` (with their handles)
/// to their average position along `axis`.
pub fn average(path: &PathData, selection: &[(usize, usize)], axis: AverageAxis) -> PathData {
    let mut out = path.clone();
    let pts: Vec<Point> = selection.iter().filter_map(|&(s, a)| path.subpaths.get(s).and_then(|sp| sp.anchors.get(a)).map(|a| a.p)).collect();
    if pts.is_empty() {
        return out;
    }
    let c = pts.iter().fold(Vec2::ZERO, |acc, p| acc + p.to_vec2()) / pts.len() as f64;
    for &(s, a) in selection {
        if let Some(an) = out.anchor_mut(s, a) {
            let d = match axis {
                AverageAxis::Horizontal => Vec2::new(0.0, c.y - an.p.y),
                AverageAxis::Vertical => Vec2::new(c.x - an.p.x, 0.0),
                AverageAxis::Both => c - an.p.to_vec2(),
            };
            an.translate(d);
        }
    }
    out
}

/// Object → Path → Join. Open subpaths of all inputs are joined nearest-endpoint-first into one
/// open path (endpoints closer than `tolerance` are merged into a single anchor, otherwise a
/// straight segment connects them). A single open subpath is closed instead. Closed subpaths
/// pass through unchanged.
pub fn join(paths: &[PathData], tolerance: f64) -> PathData {
    let mut closed: Vec<SubPath> = Vec::new();
    let mut open: Vec<SubPath> = Vec::new();
    for p in paths {
        for sp in &p.subpaths {
            if sp.closed || sp.anchors.len() < 2 {
                if !sp.anchors.is_empty() {
                    closed.push(sp.clone());
                }
            } else {
                open.push(sp.clone());
            }
        }
    }
    if open.len() == 1
        && let Some(mut sp) = open.pop()
    {
        let n = sp.anchors.len();
        if n > 2
            && sp.anchors[0].p.distance(sp.anchors[n - 1].p) <= tolerance
            && let Some(last) = sp.anchors.pop()
        {
            let f = &mut sp.anchors[0];
            *f = Anchor::with_handles(f.p, last.h_in, f.h_out);
        } else {
            let f = &mut sp.anchors[0];
            f.h_in = f.p;
            if let Some(l) = sp.anchors.last_mut() {
                l.h_out = l.p;
            }
        }
        sp.closed = true;
        closed.push(sp);
        return PathData::new(closed);
    }
    while open.len() > 1 {
        // Find the closest pair of endpoints on different subpaths.
        let mut best = (f64::INFINITY, 0, 0, false, false);
        for i in 0..open.len() {
            for j in (i + 1)..open.len() {
                for ei in [false, true] {
                    for ej in [false, true] {
                        let ai = if ei { open[i].anchors.last() } else { open[i].anchors.first() };
                        let aj = if ej { open[j].anchors.last() } else { open[j].anchors.first() };
                        let (Some(ai), Some(aj)) = (ai, aj) else { continue };
                        let d = ai.p.distance(aj.p);
                        if d < best.0 {
                            best = (d, i, j, ei, ej);
                        }
                    }
                }
            }
        }
        let (d, i, j, ei, ej) = best;
        let mut b = open.remove(j);
        let a = &mut open[i];
        if !ei {
            a.reverse();
        }
        if ej {
            b.reverse();
        }
        if d <= tolerance {
            let first = b.anchors.remove(0);
            if let Some(l) = a.anchors.last_mut() {
                *l = Anchor::with_handles(l.p, l.h_in, first.h_out);
            }
        } else {
            if let Some(l) = a.anchors.last_mut() {
                l.h_out = l.p;
            }
            b.anchors[0].h_in = b.anchors[0].p;
        }
        a.anchors.extend(b.anchors);
    }
    closed.extend(open);
    PathData::new(closed)
}

/// Object → Path → Split Into Grid: `rows × cols` rectangles filling `rect` with `gutter` spacing
/// (row-major, top-left first).
pub fn split_into_grid(rect: Rect, rows: usize, cols: usize, gutter: f64) -> Vec<PathData> {
    if rows == 0 || cols == 0 {
        return Vec::new();
    }
    let w = (rect.width() - gutter * (cols as f64 - 1.0)) / cols as f64;
    let h = (rect.height() - gutter * (rows as f64 - 1.0)) / rows as f64;
    if w <= 0.0 || h <= 0.0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let x = rect.x0 + c as f64 * (w + gutter);
            let y = rect.y0 + r as f64 * (h + gutter);
            out.push(PathData::single(SubPath::polyline(
                &[Point::new(x, y), Point::new(x + w, y), Point::new(x + w, y + h), Point::new(x, y + h)],
                true,
            )));
        }
    }
    out
}
