//! Soft coverage: how much of each flat colour (and of transparency) every pixel contains.
//!
//! An edge pixel of an anti-aliased logo is a blend of the colours that meet there. Reading it as
//! "the nearest colour" throws away exactly the sub-pixel information that says where the edge
//! runs, so each pixel is instead explained as a mix of at most three classes, whichever mix
//! reproduces the pixel best. When another mix also reproduces it (with a gray between black and
//! white among the classes, a black-to-white edge pixel is a mix of black and gray as well as of
//! black and white), the pixel takes the mix whose classes lie nearest it in the flat areas around it,
//! when its neighbors show an edge between those classes.

use std::collections::{HashMap, HashSet};

/// Premultiplied RGBA in `0..=1`.
pub(crate) type Px = [f32; 4];

const LAMBDA: f64 = 30.0;
/// Pixels this close (in 0..1 premultiplied RGBA) to a class are taken as exactly that class.
const PURE: f64 = 4.0 / 255.0;
/// A mix whose squared error is at most this reproduces the pixel: it comes within [`PURE`] of it,
/// as a pixel taken as exactly one class does.
const REPRODUCES: f64 = PURE * PURE;
/// How far (in pixels, each way) a class is looked for around a pixel that two mixes reproduce.
const NEARBY: usize = 6;
/// A pixel whose neighbors are all this close to it (in 0..1 premultiplied RGBA) lies in a flat area.
const FLAT: f64 = 1.5 / 255.0;
/// How many joined pixels within [`PURE`] of one class are taken as art of that class, such as a
/// one-pixel line. The edge pixels of a curve that come within `PURE` of a class join in far fewer.
/// A straight edge whose anti-aliased pixels come within `PURE` of
/// a class gives the same pixels as a line of that class, and keeps the class too. A one-pixel line
/// between two classes that is shorter, falls between pixels or is blurred is read as a blend of
/// those two classes.
const LINE: usize = 2 * NEARBY + 1;

struct Subset {
    idx: [usize; 3],
    n: usize,
    /// `(AᵀA + λ²·11ᵀ)⁻¹`, row-major `n × n`.
    ninv: [f64; 9],
    cols: [[f64; 4]; 3],
    penalty: f64,
}

fn inv2(m: [f64; 4]) -> Option<[f64; 4]> {
    let det = m[0] * m[3] - m[1] * m[2];
    (det.abs() > 1e-9).then(|| [m[3] / det, -m[1] / det, -m[2] / det, m[0] / det])
}

fn inv3(m: [f64; 9]) -> Option<[f64; 9]> {
    let c = [
        m[4] * m[8] - m[5] * m[7],
        m[2] * m[7] - m[1] * m[8],
        m[1] * m[5] - m[2] * m[4],
        m[5] * m[6] - m[3] * m[8],
        m[0] * m[8] - m[2] * m[6],
        m[2] * m[3] - m[0] * m[5],
        m[3] * m[7] - m[4] * m[6],
        m[1] * m[6] - m[0] * m[7],
        m[0] * m[4] - m[1] * m[3],
    ];
    let det = m[0] * c[0] + m[1] * c[3] + m[2] * c[6];
    (det.abs() > 1e-9).then(|| {
        let d = 1.0 / det;
        [c[0] * d, c[1] * d, c[2] * d, c[3] * d, c[4] * d, c[5] * d, c[6] * d, c[7] * d, c[8] * d]
    })
}

fn subsets(classes: &[[f64; 4]]) -> Vec<Subset> {
    let k = classes.len();
    let mut out = Vec::new();
    let mut push = |idx: [usize; 3], n: usize, penalty: f64| {
        let mut cols = [[0.0; 4]; 3];
        for (slot, &i) in cols.iter_mut().zip(idx.iter()).take(n) {
            *slot = classes.get(i).copied().unwrap_or([0.0; 4]);
        }
        let mut nm = [0.0f64; 9];
        for a in 0..n {
            for b in 0..n {
                let dot: f64 = (0..4).map(|c| cols[a][c] * cols[b][c]).sum();
                nm[a * n + b] = dot + LAMBDA * LAMBDA;
            }
        }
        let ninv = match n {
            2 => inv2([nm[0], nm[1], nm[2], nm[3]]).map(|m| {
                let mut o = [0.0; 9];
                o[..4].copy_from_slice(&m);
                o
            }),
            3 => inv3(nm),
            _ => None,
        };
        if let Some(ninv) = ninv {
            out.push(Subset { idx, n, ninv, cols, penalty });
        }
    };
    for a in 0..k {
        for b in (a + 1)..k {
            push([a, b, 0], 2, 4e-4);
        }
    }
    for a in 0..k {
        for b in (a + 1)..k {
            for c in (b + 1)..k {
                push([a, b, c], 3, 1.2e-3);
            }
        }
    }
    out
}

/// How many distinct pixel values `unmix` has to work on: the values not (nearly) exactly one class,
/// and the values that are, at pixels outside a flat area ([`in_flat_area`]). A flat area whose color
/// is near but not exactly a class counts once, however large it is.
pub(crate) fn count_ambiguous(pix: &[Px], width: usize, height: usize, classes: &[[f64; 4]]) -> usize {
    let mut seen = HashSet::new();
    for (i, px) in pix.iter().take(width.saturating_mul(height)).enumerate() {
        let p = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2]), f64::from(px[3])];
        let key = px.map(f32::to_bits);
        if seen.contains(&key) {
            continue;
        }
        if !classes.iter().any(|cv| (0..4).map(|j| (p[j] - cv[j]).powi(2)).sum::<f64>().sqrt() < PURE)
            || !in_flat_area(pix, width, height, i % width, i / width)
        {
            seen.insert(key);
        }
    }
    seen.len()
}

/// The pixels that took another mix than their best one, in pixel order, each with every class's
/// weight before it did.
pub(crate) type Switched = Vec<(usize, Vec<f32>)>;

/// Coverage planes, one `width × height` plane per class (each pixel's weights sum to 1), the
/// squared error each pixel's best mix leaves, and the pixels that took another mix ([`Switched`]).
pub(crate) fn unmix(pix: &[Px], width: usize, height: usize, classes: &[[f64; 4]]) -> (Vec<Vec<f32>>, Vec<f32>, Switched) {
    let k = classes.len();
    let n = width.saturating_mul(height);
    let mut planes = vec![vec![0.0f32; n]; k];
    // Squared error left over by the best mix: large where no mix of the classes explains the pixel.
    let mut residual = vec![0.0f32; n];
    let subs = subsets(classes);
    // Each distinct pixel value is mixed once: a flat area that is near but not exactly a class
    // repeats one value.
    let mut mixed: HashMap<[u32; 4], Mixes> = HashMap::new();
    // The class each pixel stands for (the class nearest it, when every other class is at least
    // twice as far).
    let mut stands_for: Vec<Option<usize>> = vec![None; n];
    // The class each pixel comes within `PURE` of, where it does.
    let mut pure: Vec<Option<usize>> = vec![None; n];
    // The pixels outside a flat area that another mix also reproduces, with their values.
    let mut tied: Vec<(usize, [u32; 4])> = Vec::new();
    for (i, px) in pix.iter().take(n).enumerate() {
        let p = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2]), f64::from(px[3])];
        // Fast path: a pixel that is (nearly) exactly one class.
        let mut near = (f64::MAX, 0usize);
        let mut second = f64::MAX;
        for (c, cv) in classes.iter().enumerate() {
            let d: f64 = (0..4).map(|j| (p[j] - cv[j]).powi(2)).sum();
            if d < near.0 {
                second = near.0;
                near = (d, c);
            } else if d < second {
                second = d;
            }
        }
        if let Some(q) = stands_for.get_mut(i) {
            // (Squared distances: twice as far is four times the square.)
            *q = (4.0 * near.0 <= second).then_some(near.1);
        }
        let key = px.map(f32::to_bits);
        if near.0.sqrt() < PURE {
            if let Some(v) = planes.get_mut(near.1).and_then(|pl| pl.get_mut(i)) {
                *v = 1.0;
            }
            if let Some(q) = pure.get_mut(i) {
                *q = Some(near.1);
            }
            // (An edge pixel can come within `PURE` of a class it holds none of: a black-to-white
            // edge pixel as light as a gray class.)
            if !in_flat_area(pix, width, height, i % width, i / width)
                && !mixed.entry(key).or_insert_with(|| best_mixes(p, near, &subs)).tied.is_empty()
            {
                tied.push((i, key));
            }
            continue;
        }
        let mixes = mixed.entry(key).or_insert_with(|| best_mixes(p, near, &subs));
        if let Some(r) = residual.get_mut(i) {
            *r = mixes.residual;
        }
        set_weights(&mut planes, i, &mixes.best, 1.0);
        // (A pixel in a flat area has no neighbor far enough from it for `across_edge`.)
        if !mixes.tied.is_empty() && !in_flat_area(pix, width, height, i % width, i / width) {
            tied.push((i, key));
        }
    }
    let mut switched = Vec::new();
    if tied.is_empty() {
        return (planes, residual, switched);
    }
    // A pixel two mixes reproduce takes another mix of two classes whose farthest class lies nearer
    // it than the best mix's does, in the flat areas around it ([`flat_classes`], [`RowDistances::around`]),
    // when the pixel lies on an edge between that mix's classes ([`across_edge`]). For the best mix,
    // the classes held by flat areas between two classes count as well ([`between_classes`]). A
    // pixel within `PURE` of a class that is joined to at least [`LINE`] such pixels of that class
    // (itself included) keeps it ([`pure_runs`]), as a one-pixel line of gray between black and white.
    let flat = flat_classes(pix, &stands_for, width, height);
    let between = between_classes(pix, &flat, &mixed, width, height);
    let runs = pure_runs(&pure, width, height);
    let mut flat_rows = RowDistances::new(|j| [flat.get(j).copied().flatten(), None], width, height, k);
    let mut between_rows = (!between.is_empty()).then(|| RowDistances::new(|j| between.get(j).copied().unwrap_or([None; 2]), width, height, k));
    let mut dist = vec![NEARBY + 1; k];
    let mut kept = vec![NEARBY + 1; k];
    for (i, key) in tied {
        if pure.get(i).copied().flatten().is_some() && runs.get(i).is_some_and(|&r| r >= LINE) {
            continue;
        }
        let Some(mixes) = mixed.get(&key) else { continue };
        let edges: Vec<&Weights> = mixes.tied.iter().filter(|m| across_edge(pix, width, height, i, classes, &mixes.best, m)).collect();
        if edges.is_empty() {
            continue;
        }
        flat_rows.around(i, &mut dist);
        kept.copy_from_slice(&dist);
        if let Some(rows) = &mut between_rows {
            rows.around(i, &mut kept);
            for (h, &d) in kept.iter_mut().zip(&dist) {
                *h = (*h).min(d);
            }
        }
        let now = farthest(&mixes.best, &kept);
        let other = edges.into_iter().filter(|m| farthest(m, &dist) < now).min_by_key(|m| farthest(m, &dist));
        if let Some(other) = other {
            switched.push((i, planes.iter().map(|pl| pl.get(i).copied().unwrap_or(0.0)).collect()));
            // (`residual` keeps the best mix's error; the mix taken comes within `REPRODUCES` of the
            // pixel as well.)
            set_weights(&mut planes, i, &mixes.best, 0.0);
            set_weights(&mut planes, i, other, 1.0);
        }
    }
    (planes, residual, switched)
}

/// Whether pixel `i` lies on an edge between the two classes of `other`, rather than in an area of
/// a class of `best` that `other` leaves out: for each such class, the pixel has a neighbor beyond
/// it toward each class of `other`, along the line between them. (A black-to-white edge pixel as
/// light as a gray has a darker neighbor and a lighter one; a gray stroke on white has no neighbor
/// darker than the gray.)
fn across_edge(pix: &[Px], width: usize, height: usize, i: usize, classes: &[[f64; 4]], best: &Weights, other: &Weights) -> bool {
    let [(a, _), (b, _)] = other.as_slice() else { return false };
    let (Some(ca), Some(cb)) = (classes.get(*a), classes.get(*b)) else { return false };
    let ab: [f64; 4] = std::array::from_fn(|j| cb[j] - ca[j]);
    let len2: f64 = ab.iter().map(|v| v * v).sum();
    if len2 <= 1e-12 {
        return false;
    }
    let len = len2.sqrt();
    // Where `v` lies along the line from `a` to `b` (0 at `a`, 1 at `b`), and how far off it.
    let place = |v: [f64; 4]| {
        let t = (0..4).map(|j| (v[j] - ca[j]) * ab[j]).sum::<f64>() / len2;
        let off = (0..4).map(|j| (v[j] - ca[j] - t * ab[j]).powi(2)).sum::<f64>().sqrt();
        (t, off)
    };
    let (x, y) = (i % width.max(1), i / width.max(1));
    let neighbors: Vec<f64> = (y.saturating_sub(1)..(y + 2).min(height))
        .flat_map(|yy| (x.saturating_sub(1)..(x + 2).min(width)).map(move |xx| yy * width + xx))
        .filter(|&j| j != i)
        .filter_map(|j| pix.get(j))
        .map(|q| place([f64::from(q[0]), f64::from(q[1]), f64::from(q[2]), f64::from(q[3])]))
        .filter(|&(_, off)| off <= 2.0 * PURE)
        .map(|(t, _)| t)
        .collect();
    let left_out: Vec<usize> = best.iter().filter(|&&(c, w)| w > 0.0 && c != *a && c != *b).map(|&(c, _)| c).collect();
    !left_out.is_empty()
        && left_out.iter().all(|&c| {
            let Some(&cv) = classes.get(c) else { return false };
            let (tc, _) = place(cv);
            neighbors.iter().any(|&t| (tc - t) * len > PURE) && neighbors.iter().any(|&t| (t - tc) * len > PURE)
        })
}

/// How far the farthest class of `mix` lies, by the distances `dist` ([`RowDistances::around`]).
fn farthest(mix: &Weights, dist: &[usize]) -> usize {
    mix.iter().filter(|&&(_, w)| w > 0.0).map(|&(c, _)| dist.get(c).copied().unwrap_or(NEARBY + 1)).max().unwrap_or(0)
}

/// Sets pixel `i` of each class's plane in `weights` to the class's weight times `scale`.
fn set_weights(planes: &mut [Vec<f32>], i: usize, weights: &[(usize, f32)], scale: f32) {
    for &(c, wv) in weights {
        if let Some(v) = planes.get_mut(c).and_then(|pl| pl.get_mut(i)) {
            *v = wv * scale;
        }
    }
}

/// A class's weight in the mix of a flat area that stands for no class, from which the area
/// counts as holding that class.
const HOLDS: f32 = 0.25;

/// The class each pixel stands for (`stands_for`) where the pixel lies in a flat area
/// ([`in_flat_area`]), and `None` elsewhere.
fn flat_classes(pix: &[Px], stands_for: &[Option<usize>], width: usize, height: usize) -> Vec<Option<usize>> {
    (0..width.saturating_mul(height))
        .map(|i| stands_for.get(i).copied().flatten().filter(|_| in_flat_area(pix, width, height, i % width, i / width)))
        .collect()
}

/// The classes held by flat areas that stand for no class (a flat color between two classes, as
/// with colors named some levels off): the classes of at least [`HOLDS`] weight in the area's mix
/// (at most two, the heaviest), held also by the pixels of exactly its color joined to it, flat or
/// not (its rows beside another color, where a soft edge or a shape close by leaves no flat pixel).
/// Empty when there is no such area.
fn between_classes(pix: &[Px], flat: &[Option<usize>], mixed: &HashMap<[u32; 4], Mixes>, width: usize, height: usize) -> Vec<[Option<usize>; 2]> {
    let mut held = Vec::new();
    // Pixels holding classes, from which the pixels of their color are reached.
    let mut reach: Vec<usize> = Vec::new();
    for (i, f) in flat.iter().enumerate() {
        if f.is_some() || !in_flat_area(pix, width, height, i % width, i / width) {
            continue;
        }
        let Some(mixes) = pix.get(i).and_then(|px| mixed.get(&px.map(f32::to_bits))) else { continue };
        let mut heavy: Vec<(usize, f32)> = mixes.best.iter().copied().filter(|&(_, w)| w >= HOLDS).collect();
        heavy.sort_by(|a, b| b.1.total_cmp(&a.1));
        if heavy.is_empty() {
            continue;
        }
        if held.is_empty() {
            held = vec![[None; 2]; width.saturating_mul(height)];
        }
        if let Some(slot) = held.get_mut(i) {
            *slot = [heavy.first().map(|&(c, _)| c), heavy.get(1).map(|&(c, _)| c)];
            reach.push(i);
        }
    }
    while let Some(i) = reach.pop() {
        let (Some(&classes), Some(key)) = (held.get(i), pix.get(i).map(|px| px.map(f32::to_bits))) else { continue };
        let (x, y) = (i % width, i / width);
        for yy in y.saturating_sub(1)..(y + 2).min(height) {
            for xx in x.saturating_sub(1)..(x + 2).min(width) {
                let j = yy * width + xx;
                if let (Some(slot @ [None, None]), Some(q)) = (held.get_mut(j), pix.get(j))
                    && q.map(f32::to_bits) == key
                {
                    *slot = classes;
                    reach.push(j);
                }
            }
        }
    }
    held
}

/// For each of `k` classes, how far along each row the nearest pixel holding that class (`at`, the
/// classes pixel `j` holds) lies from each pixel, up to `NEARBY + 1` when none lies within
/// [`NEARBY`]. A row is worked out when it is first asked for, so a few pixels cost a few rows.
struct RowDistances<F> {
    at: F,
    width: usize,
    k: usize,
    /// Per row, once worked out: the distance for pixel `x` and class `c` at `x * k + c`.
    rows: Vec<Option<Vec<u8>>>,
}

impl<F: Fn(usize) -> [Option<usize>; 2]> RowDistances<F> {
    const NONE: u8 = (NEARBY + 1) as u8;

    fn new(at: F, width: usize, height: usize, k: usize) -> Self {
        Self { at, width, k, rows: vec![None; height] }
    }

    fn row(&mut self, y: usize) -> &[u8] {
        if self.rows.get(y).is_some_and(Option::is_none) {
            let built = self.build(y);
            if let Some(slot) = self.rows.get_mut(y) {
                *slot = Some(built);
            }
        }
        self.rows.get(y).and_then(Option::as_deref).unwrap_or(&[])
    }

    fn build(&self, y: usize) -> Vec<u8> {
        let (width, k) = (self.width, self.k);
        let mut row = vec![Self::NONE; width.saturating_mul(k)];
        // Toward the right, then toward the left, each from the last pixel holding the class.
        let mut last = vec![Self::NONE; k];
        for x in 0..width {
            let held = (self.at)(y * width + x);
            for (c, d) in last.iter_mut().enumerate() {
                *d = if held.contains(&Some(c)) { 0 } else { d.saturating_add(1).min(Self::NONE) };
                if let Some(r) = row.get_mut(x * k + c) {
                    *r = *d;
                }
            }
        }
        last.fill(Self::NONE);
        for x in (0..width).rev() {
            for (c, d) in last.iter_mut().enumerate() {
                let Some(r) = row.get_mut(x * k + c) else { continue };
                *d = if *r == 0 { 0 } else { d.saturating_add(1).min(Self::NONE) };
                *r = (*r).min(*d);
            }
        }
        row
    }

    /// For each class, how far (in pixels, the larger of the two axes) the nearest pixel holding
    /// that class lies from pixel `i`, into `dist` (one entry per class); `NEARBY + 1` when none
    /// lies within [`NEARBY`].
    fn around(&mut self, i: usize, dist: &mut [usize]) {
        let (width, k, height) = (self.width.max(1), self.k, self.rows.len());
        let (x, y) = (i % width, i / width);
        dist.fill(NEARBY + 1);
        for yy in y.saturating_sub(NEARBY)..(y + NEARBY + 1).min(height) {
            let dy = yy.abs_diff(y);
            let row = self.row(yy);
            for (c, d) in dist.iter_mut().enumerate().take(k) {
                if let Some(&r) = row.get(x * k + c) {
                    *d = (*d).min(usize::from(r).max(dy));
                }
            }
        }
    }
}

/// For each pixel within [`PURE`] of a class (`pure`), how many pixels within `PURE` of the same
/// class are joined to it (itself included, through the eight neighbors); 0 for other pixels.
fn pure_runs(pure: &[Option<usize>], width: usize, height: usize) -> Vec<usize> {
    let n = width.saturating_mul(height);
    let mut size = vec![0usize; n];
    let mut seen = vec![false; n];
    let mut stack = Vec::new();
    let mut members = Vec::new();
    for start in 0..n {
        let Some(c) = pure.get(start).copied().flatten() else { continue };
        if seen.get(start).copied().unwrap_or(true) {
            continue;
        }
        if let Some(s) = seen.get_mut(start) {
            *s = true;
        }
        stack.push(start);
        members.clear();
        while let Some(i) = stack.pop() {
            members.push(i);
            let (x, y) = (i % width, i / width);
            for yy in y.saturating_sub(1)..(y + 2).min(height) {
                for xx in x.saturating_sub(1)..(x + 2).min(width) {
                    let j = yy * width + xx;
                    if pure.get(j).copied().flatten() == Some(c)
                        && let Some(s) = seen.get_mut(j)
                        && !*s
                    {
                        *s = true;
                        stack.push(j);
                    }
                }
            }
        }
        for &i in &members {
            if let Some(s) = size.get_mut(i) {
                *s = members.len();
            }
        }
    }
    size
}

/// Whether every neighbor of the pixel at (`x`, `y`) inside the image lies within [`FLAT`] of it.
fn in_flat_area(pix: &[Px], width: usize, height: usize, x: usize, y: usize) -> bool {
    let Some(p) = pix.get(y * width + x) else { return false };
    (y.saturating_sub(1)..(y + 2).min(height)).all(|yy| {
        (x.saturating_sub(1)..(x + 2).min(width)).all(|xx| {
            pix.get(yy * width + xx).is_some_and(|q| (0..4).map(|j| (f64::from(p[j]) - f64::from(q[j])).powi(2)).sum::<f64>().sqrt() <= FLAT)
        })
    })
}

/// Each class's weight in a pixel's mix.
type Weights = Vec<(usize, f32)>;

/// A mix tried for a pixel: its squared error with its penalty, the penalty, and its classes with
/// their weights (the first `n`).
#[derive(Clone, Copy)]
struct Candidate {
    res: f64,
    penalty: f64,
    idx: [usize; 3],
    w: [f64; 3],
    n: usize,
}

impl Candidate {
    fn weights(&self) -> Weights {
        self.idx.iter().zip(self.w).take(self.n).map(|(&c, w)| (c, w as f32)).collect()
    }
}

/// How a pixel value is mixed: the squared error the best mix leaves, each class's weight in it,
/// and the other mixes of two classes that reproduce the value ([`REPRODUCES`]).
struct Mixes {
    residual: f32,
    best: Weights,
    tied: Vec<Weights>,
}

/// The mix of at most three classes that reproduces `p` best, starting from the nearest single
/// class `near` (squared distance, class), and the other mixes that also reproduce it.
fn best_mixes(p: [f64; 4], near: (f64, usize), subs: &[Subset]) -> Mixes {
    let reproduces = |m: &Candidate| m.res - m.penalty <= REPRODUCES;
    // Best single class is the starting point; try every two- and three-class mix.
    let mut top = Candidate { res: near.0, penalty: 0.0, idx: [near.1, 0, 0], w: [1.0, 0.0, 0.0], n: 1 };
    // The other mixes of two classes that reproduce the pixel, also when the best mix does not (a
    // single class carries no penalty and can come nearer than an exact mix of two): only a mix of
    // two is switched to ([`across_edge`]). A mix holding the nearest class when that class alone
    // reproduces the pixel adds a class of next to no weight to it, and is left out. (At most one
    // candidate per pair of classes, so the list stays small with many classes.) Each is kept with
    // its subset's index, and the best mix found is left out of them at the end.
    let single = reproduces(&top).then_some(near.1);
    let mut top_at = None;
    let mut pairs: Vec<(usize, Candidate)> = Vec::new();
    for (at, s) in subs.iter().enumerate() {
        let mut rhs = [0.0f64; 3];
        for (j, r) in rhs.iter_mut().enumerate().take(s.n) {
            *r = (0..4).map(|c| s.cols[j][c] * p[c]).sum::<f64>() + LAMBDA * LAMBDA;
        }
        let mut w = [0.0f64; 3];
        for (a, wa) in w.iter_mut().enumerate().take(s.n) {
            *wa = (0..s.n).map(|b| s.ninv[a * s.n + b] * rhs[b]).sum();
        }
        if w.iter().take(s.n).any(|&x| x < -0.02) {
            continue;
        }
        let mut sum = 0.0;
        for wa in w.iter_mut().take(s.n) {
            *wa = wa.clamp(0.0, 1.0);
            sum += *wa;
        }
        if sum < 1e-9 {
            continue;
        }
        for wa in w.iter_mut().take(s.n) {
            *wa /= sum;
        }
        let mut res = s.penalty;
        for (c, &pc) in p.iter().enumerate() {
            let m: f64 = (0..s.n).map(|j| w[j] * s.cols[j][c]).sum();
            res += (pc - m).powi(2);
        }
        let m = Candidate { res, penalty: s.penalty, idx: s.idx, w, n: s.n };
        if m.n == 2 && reproduces(&m) && single.is_none_or(|c| m.idx[0] != c && m.idx[1] != c) {
            pairs.push((at, m));
        }
        if res < top.res {
            top = m;
            top_at = Some(at);
        }
    }
    let tied = pairs.iter().filter(|&&(at, _)| Some(at) != top_at).map(|(_, m)| m.weights()).collect();
    Mixes { residual: (top.res - top.penalty).max(0.0) as f32, best: top.weights(), tied }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes() -> Vec<[f64; 4]> {
        vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]]
    }

    #[test]
    fn pure_pixels_are_exact() {
        let pix = vec![[1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.0]];
        let (pl, _, _) = unmix(&pix, 2, 1, &classes());
        assert_eq!((pl[0][0], pl[2][1]), (1.0, 1.0));
    }

    #[test]
    fn a_blend_of_two_colours_recovers_its_proportions() {
        // 30% red over 70% blue.
        let pix = vec![[0.3, 0.0, 0.7, 1.0]];
        let (pl, _, _) = unmix(&pix, 1, 1, &classes());
        assert!((pl[0][0] - 0.3).abs() < 0.02, "red {}", pl[0][0]);
        assert!((pl[1][0] - 0.7).abs() < 0.02, "blue {}", pl[1][0]);
    }

    #[test]
    fn a_half_transparent_edge_is_half_covered() {
        // Red at 40% coverage over nothing: premultiplied (0.4, 0, 0, 0.4).
        let pix = vec![[0.4, 0.0, 0.0, 0.4]];
        let (pl, _, _) = unmix(&pix, 1, 1, &classes());
        assert!((pl[0][0] - 0.4).abs() < 0.02 && (pl[2][0] - 0.6).abs() < 0.02, "{:?}", (pl[0][0], pl[2][0]));
    }

    #[test]
    fn a_colour_no_class_can_explain_leaves_a_large_residual() {
        // Green is none of red, blue or transparent, nor any blend of them.
        let pix = vec![[0.0, 0.8, 0.0, 1.0], [0.3, 0.0, 0.7, 1.0]];
        let (_, res, _) = unmix(&pix, 2, 1, &classes());
        assert!(res[0] > 0.1, "unexplained pixel residual {}", res[0]);
        assert!(res[1] < 1e-3, "an explained blend residual {}", res[1]);
    }

    #[test]
    fn a_repeated_color_near_a_class_is_counted_once_and_mixed_alike() {
        // A flat area a few levels off red, a blend, and pure red and transparent pixels.
        let off = [0.97, 0.02, 0.02, 1.0];
        let mut pix = vec![off; 1000];
        pix.extend([[0.3, 0.0, 0.7, 1.0], [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.0]]);
        // (The pure red and transparent pixels lie outside a flat area, so they count too: `unmix`
        // checks whether another mix reproduces them.)
        assert_eq!(count_ambiguous(&pix, pix.len(), 1, &classes()), 4);
        // Pure pixels in a flat area do not count.
        assert_eq!(count_ambiguous(&[[1.0, 0.0, 0.0, 1.0]; 9], 3, 3, &classes()), 0);
        let (pl, res, _) = unmix(&pix, pix.len(), 1, &classes());
        assert!(pl[0][0] > 0.95, "red {}", pl[0][0]);
        for i in 1..1000 {
            assert_eq!((pl[0][i], pl[1][i], pl[2][i], res[i]), (pl[0][0], pl[1][0], pl[2][0], res[0]), "pixel {i}");
        }
        assert!((pl[0][1000] - 0.3).abs() < 0.02 && (pl[1][1000] - 0.7).abs() < 0.02);
    }

    #[test]
    fn an_edge_pixel_two_mixes_reproduce_takes_the_classes_beside_it() {
        // A light gray lies on the line from black to white. A row of flat black, one edge pixel,
        // flat white: the edge pixel is a blend of black and white, with no gray in it, also when
        // it comes within `PURE` of the gray (0.905), and when the gray alone, which carries no
        // penalty, comes nearer it than any mix of two does without reproducing it (0.91).
        let classes = vec![[0.9, 0.9, 0.9, 1.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]];
        let row = |left: [f32; 4], v: f32| {
            let mut pix = vec![left; 5];
            pix.push([v, v, v, 1.0]);
            pix.extend([[1.0f32, 1.0, 1.0, 1.0]; 5]);
            pix
        };
        for v in [0.1f32, 0.3, 0.7, 0.85, 0.905, 0.91, 0.95, 0.98] {
            let (pl, _, _) = unmix(&row([0.0, 0.0, 0.0, 1.0], v), 11, 1, &classes);
            assert_eq!(pl[0][5], 0.0, "gray in a {v} edge pixel");
            assert!((pl[2][5] - v).abs() < 0.02 && (pl[1][5] - (1.0 - v)).abs() < 0.02, "{v}: black {} white {}", pl[1][5], pl[2][5]);
        }
        // Beside flat gray, a gray-to-white edge pixel keeps its gray.
        let (pl, _, _) = unmix(&row([0.9, 0.9, 0.9, 1.0], 0.95), 11, 1, &classes);
        assert!((pl[0][5] - 0.5).abs() < 0.02 && pl[1][5] == 0.0, "gray {} black {}", pl[0][5], pl[1][5]);
        // A gray pixel between white ones, flat black four pixels off: a gray stroke, kept.
        let mut pix = vec![[0.0f32, 0.0, 0.0, 1.0]; 4];
        pix.extend([[1.0f32, 1.0, 1.0, 1.0]; 2]);
        pix.push([0.9, 0.9, 0.9, 1.0]);
        pix.extend([[1.0f32, 1.0, 1.0, 1.0]; 5]);
        let (pl, _, _) = unmix(&pix, 12, 1, &classes);
        assert_eq!((pl[0][6], pl[1][6]), (1.0, 0.0), "the stroke");
    }

    #[test]
    fn a_column_of_pixels_as_light_as_a_class_keeps_it() {
        // Flat black, one column at the gray's value, flat white. In `LINE` rows or more the column
        // is gray art (a one-pixel line, or a straight edge giving the same pixels); in fewer rows it
        // is an edge between black and white.
        let classes = vec![[0.9, 0.9, 0.9, 1.0], [0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 0.0]];
        for (rows, gray) in [(LINE, 1.0), (LINE + 10, 1.0), (LINE - 1, 0.0), (3, 0.0)] {
            let mut pix = Vec::new();
            for _ in 0..rows {
                pix.extend([[0.0f32, 0.0, 0.0, 1.0]; 5]);
                pix.push([0.9, 0.9, 0.9, 1.0]);
                pix.extend([[1.0f32, 1.0, 1.0, 1.0]; 5]);
            }
            let (pl, _, _) = unmix(&pix, 11, rows, &classes);
            for y in 0..rows {
                assert_eq!(pl[0][y * 11 + 5], gray, "{rows} rows: gray in row {y}");
            }
        }
    }

    #[test]
    fn row_distances_match_a_scan_of_the_window_around_each_pixel() {
        // Three classes scattered over 23 x 17 pixels, some pixels holding two, most none.
        let (w, h, k) = (23usize, 17usize, 3usize);
        let at = |j: usize| -> [Option<usize>; 2] {
            match (j * 7919) % 97 {
                0..=2 => [Some(0), None],
                3 => [Some(1), Some(2)],
                4 | 5 => [None, Some(2)],
                _ => [None; 2],
            }
        };
        let mut rows = RowDistances::new(at, w, h, k);
        let mut dist = vec![0; k];
        for i in 0..w * h {
            let (x, y) = (i % w, i / w);
            let mut want = vec![NEARBY + 1; k];
            for yy in y.saturating_sub(NEARBY)..(y + NEARBY + 1).min(h) {
                for xx in x.saturating_sub(NEARBY)..(x + NEARBY + 1).min(w) {
                    for c in at(yy * w + xx).into_iter().flatten() {
                        want[c] = want[c].min(xx.abs_diff(x).max(yy.abs_diff(y)));
                    }
                }
            }
            rows.around(i, &mut dist);
            assert_eq!(dist, want, "pixel ({x}, {y})");
        }
    }

    #[test]
    fn weights_sum_to_one() {
        let pix = vec![[0.2, 0.1, 0.5, 0.8], [0.9, 0.0, 0.05, 0.95]];
        let (pl, _, _) = unmix(&pix, 2, 1, &classes());
        for i in 0..2 {
            let s: f32 = pl.iter().map(|p| p[i]).sum();
            assert!((s - 1.0).abs() < 1e-4, "pixel {i} sums to {s}");
        }
    }
}
