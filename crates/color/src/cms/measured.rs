//! CMYK working spaces computed from published characterization data.
//!
//! Fogra publishes the measured colors of its reference printing conditions
//! (<https://fogra.org/en/downloads/work-tools/characterisation-data>) and allows the data to be
//! redistributed unmodified, with Fogra named as the source. VectorCraft embeds the FOGRA51 and
//! FOGRA52 files byte for byte (`crates/color/data/`) and computes each press model from them the
//! first time it is used:
//!
//! * The paper is the measured blank patch. The 15 solids and overprints of the
//!   [`super::generic`] model are the measured patches of those builds, and each ink's tone curve
//!   is fitted to its single-ink ramp.
//! * What that model still misses at each patch (in media-relative Lab) is fitted with a smoothed
//!   lattice over CMYK ([`Lattice`]), which [`GenericCmyk::to_xyz_rel`] adds to the model. The
//!   smoothing carries the fit into regions the patch set does not sample, such as dark builds
//!   with more than 60% black.
//!
//! The total ink limit and the black generation are VectorCraft's own values.

use super::generic::{GenericCmyk, GenericCmykParams, swop_like};
use super::lab::{D50, Lab, lab_to_xyz, xyz_to_lab};

/// A printing condition with published characterization data.
#[derive(Clone, Copy, Debug)]
pub struct Condition {
    /// The working space's name.
    pub name: &'static str,
    /// The reference name of the characterization data, which a PDF/X output intent uses as
    /// its `/OutputConditionIdentifier`.
    pub reference: &'static str,
    /// The characterization data file, as published.
    pub data: &'static [u8],
    /// Total area coverage limit (sum of inks, 1.0 = 100%): VectorCraft's own value.
    pub tac: f32,
}

/// The registry that the reference names of [`CONDITIONS`] belong to
/// (<https://registry.color.org/cmyk-registry/>).
pub const REGISTRY: &str = "http://www.color.org";

/// The printing conditions VectorCraft computes working spaces for.
pub const CONDITIONS: [Condition; 2] = [
    Condition { name: super::FOGRA51_CMYK, reference: "FOGRA51", data: include_bytes!("../../data/FOGRA51.txt"), tac: 3.0 },
    Condition { name: super::FOGRA52_CMYK, reference: "FOGRA52", data: include_bytes!("../../data/FOGRA52.txt"), tac: 3.0 },
];

/// The condition a working space of [`CONDITIONS`] is computed for.
pub fn condition(name: &str) -> Option<&'static Condition> {
    CONDITIONS.iter().find(|c| c.name == name)
}

/// One measured patch: nominal inks (0..1) and absolute Lab (D50).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    pub cmyk: [f32; 4],
    pub lab: Lab,
}

/// The most patches [`parse`] reads.
const MAX_PATCHES: usize = 20_000;

/// The patches of a CGATS / ISO 28178 characterization data file with `CMYK_C`, `CMYK_M`,
/// `CMYK_Y`, `CMYK_K`, `LAB_L`, `LAB_A` and `LAB_B` fields. `None` when a field is missing, a
/// row is short or holds a value that is not a finite number, an ink is outside 0 to 100, the
/// file has no rows, the rows do not end with `END_DATA`, or a `NUMBER_OF_SETS` in the header is
/// not the number of rows.
pub fn parse(data: &[u8]) -> Option<Vec<Patch>> {
    let text = std::str::from_utf8(data).ok()?;
    let sets = text.lines().map(str::trim).take_while(|l| *l != "BEGIN_DATA").find_map(|l| {
        let mut words = l.split_whitespace();
        (words.next() == Some("NUMBER_OF_SETS")).then(|| words.next().map(|n| n.trim_matches('"')))
    });
    let sets = match sets {
        Some(n) => Some(n?.parse::<usize>().ok()?),
        None => None,
    };
    let mut lines = text.lines().map(str::trim);
    lines.by_ref().find(|l| *l == "BEGIN_DATA_FORMAT")?;
    let mut fields = vec![];
    for line in lines.by_ref() {
        if line == "END_DATA_FORMAT" {
            break;
        }
        fields.extend(line.split_whitespace());
    }
    let at = |f: &str| fields.iter().position(|x| *x == f);
    let cols = [at("CMYK_C")?, at("CMYK_M")?, at("CMYK_Y")?, at("CMYK_K")?, at("LAB_L")?, at("LAB_A")?, at("LAB_B")?];
    lines.by_ref().find(|l| *l == "BEGIN_DATA")?;
    let mut out = vec![];
    let mut ended = false;
    for line in lines {
        if line == "END_DATA" {
            ended = true;
            break;
        }
        if line.is_empty() {
            continue;
        }
        if out.len() >= MAX_PATCHES {
            return None;
        }
        let row: Vec<&str> = line.split_whitespace().collect();
        let mut v = [0f32; 7];
        for (slot, &col) in v.iter_mut().zip(&cols) {
            *slot = row.get(col)?.parse::<f32>().ok().filter(|x| x.is_finite())?;
        }
        if v[..4].iter().any(|x| !(0.0..=100.0).contains(x)) {
            return None;
        }
        out.push(Patch { cmyk: [v[0], v[1], v[2], v[3]].map(|x| x / 100.0), lab: Lab::new(v[4], v[5], v[6]) });
    }
    (ended && !out.is_empty() && sets.is_none_or(|n| n == out.len())).then_some(out)
}

/// Absolute Lab → media-relative Lab (`paper` maps to the D50 white).
fn relative(lab: Lab, paper: [f32; 3]) -> Lab {
    let x = lab_to_xyz(lab);
    xyz_to_lab([0, 1, 2].map(|i| x[i] * D50[i] / paper[i]))
}

/// The mean of the Lab values of the patches whose inks are `inks`, in XYZ.
fn mean_lab(patches: &[Patch], inks: [f32; 4]) -> Option<Lab> {
    let mut sum = [0f32; 3];
    let mut n = 0;
    for p in patches.iter().filter(|p| p.cmyk.iter().zip(&inks).all(|(a, b)| (a - b).abs() < 1e-4)) {
        let x = lab_to_xyz(p.lab);
        for i in 0..3 {
            sum[i] += x[i];
        }
        n += 1;
    }
    (n > 0).then(|| xyz_to_lab(sum.map(|v| v / n as f32)))
}

/// The tone value increase at 50% and 40% of ink `ink`, fitted to its single-ink ramp with the
/// curve of [`super::generic`]: the effective area is `a + u·4a(1−a) + v·4a(1−a)(2a−1)`, so the
/// 50% increase is `u` and the 40% increase is `0.96·u − 0.192·v`. Each patch's effective area
/// comes from the Murray–Davies formula in the media-relative XYZ channel the ink darkens most
/// (`paper` is the paper's absolute XYZ, `solid` the solid's media-relative XYZ).
fn tone_increase(patches: &[Patch], ink: usize, paper: [f32; 3], solid: [f32; 3]) -> Option<(f32, f32)> {
    let ch = (0..3).max_by(|&a, &b| (D50[a] - solid[a]).abs().total_cmp(&(D50[b] - solid[b]).abs()))?;
    let span = D50[ch] - solid[ch];
    if span.abs() < 1e-4 {
        return None;
    }
    let (mut s11, mut s12, mut s22, mut r1, mut r2) = (0f64, 0f64, 0f64, 0f64, 0f64);
    for p in patches {
        let a = p.cmyk[ink];
        let alone = p.cmyk.iter().enumerate().all(|(i, v)| i == ink || *v == 0.0);
        if !alone || a <= 0.0 || a >= 1.0 {
            continue;
        }
        let e = (D50[ch] - relative_xyz(p.lab, paper)[ch]) / span;
        let (b1, b2) = (f64::from(4.0 * a * (1.0 - a)), f64::from(4.0 * a * (1.0 - a) * (2.0 * a - 1.0)));
        let d = f64::from(e - a);
        s11 += b1 * b1;
        s12 += b1 * b2;
        s22 += b2 * b2;
        r1 += b1 * d;
        r2 += b2 * d;
    }
    let det = s11 * s22 - s12 * s12;
    if det.abs() < 1e-12 {
        return None;
    }
    let u = ((r1 * s22 - r2 * s12) / det) as f32;
    let v = ((s11 * r2 - s12 * r1) / det) as f32;
    let tvi50 = u.clamp(0.0, 0.45);
    let tvi40 = (0.96 * u - 0.192 * v).clamp(0.0, 0.45);
    Some((tvi50, tvi40))
}

/// Absolute Lab → media-relative XYZ.
fn relative_xyz(lab: Lab, paper: [f32; 3]) -> [f32; 3] {
    let x = lab_to_xyz(lab);
    [0, 1, 2].map(|i| x[i] * D50[i] / paper[i])
}

/// The press model of `condition`, computed from its data. `None` when the data does not parse,
/// lacks the paper, a solid or overprint, or a single-ink ramp, or gives a paper whose XYZ is not
/// positive and finite.
pub fn model(condition: &Condition) -> Option<GenericCmyk> {
    fit(condition, &parse(condition.data)?, SMOOTHING)
}

/// The press model of `condition` fitted to `patches`, with the lattice's `smoothing` (`None` in
/// the cases [`model`] lists).
pub(crate) fn fit(condition: &Condition, patches: &[Patch], smoothing: f64) -> Option<GenericCmyk> {
    let paper = mean_lab(patches, [0.0; 4])?;
    let paper_xyz = lab_to_xyz(paper);
    if paper_xyz.iter().any(|v| !v.is_finite() || *v <= 0.0) {
        return None;
    }
    let mut primaries = [Lab::new(0.0, 0.0, 0.0); 15];
    for (i, slot) in primaries.iter_mut().enumerate() {
        let bits = i + 1;
        let inks = [0, 1, 2, 3].map(|b| if bits & (1 << b) != 0 { 1.0 } else { 0.0 });
        *slot = relative(mean_lab(patches, inks)?, paper_xyz);
    }
    let mut tvi50 = [0f32; 4];
    let mut tvi40 = [0f32; 4];
    for ink in 0..4 {
        let solid = lab_to_xyz(primaries[(1 << ink) - 1]);
        (tvi50[ink], tvi40[ink]) = tone_increase(patches, ink, paper_xyz, solid)?;
    }
    let params = GenericCmykParams { name: condition.name, paper, primaries, tvi50, tvi40, tac: condition.tac, bg: swop_like().bg };
    let base = GenericCmyk::new(params.clone());
    let points: Vec<([f32; 4], [f32; 3])> = patches
        .iter()
        .map(|p| {
            let want = relative(p.lab, paper_xyz);
            let got = base.to_lab_rel(p.cmyk);
            (p.cmyk, [want.l - got.l, want.a - got.a, want.b - got.b])
        })
        .collect();
    Some(GenericCmyk::with_correction(params, Lattice::fit(&points, smoothing)))
}

/// Weight of the lattice's smoothness against its fit to the patches.
pub(crate) const SMOOTHING: f64 = 0.02;

/// Knots of each ink's axis of the [`Lattice`]: the steps of the published single-ink ramps.
const KNOTS: [f32; 9] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.55, 0.7, 0.85, 1.0];
const N: usize = KNOTS.len();
const NODES: usize = N * N * N * N;

/// A Lab correction over CMYK, read by multilinear interpolation between nodes at [`KNOTS`].
#[derive(Clone, Debug)]
pub struct Lattice {
    values: Vec<[f32; 3]>,
}

/// The cell of `v` along an axis and its position in it.
fn locate(v: f32) -> (usize, f32) {
    let v = if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    let i = KNOTS.iter().skip(1).position(|k| v <= *k).unwrap_or(N - 2).min(N - 2);
    let (lo, hi) = (KNOTS[i], KNOTS[i + 1]);
    (i, ((v - lo) / (hi - lo)).clamp(0.0, 1.0))
}

/// The 16 nodes around `cmyk` with their interpolation weights.
fn corners(cmyk: [f32; 4]) -> [(usize, f32); 16] {
    let cells = cmyk.map(locate);
    let mut out = [(0, 0.0); 16];
    for (corner, slot) in out.iter_mut().enumerate() {
        let mut index = 0;
        let mut w = 1.0;
        for (axis, (cell, t)) in cells.iter().enumerate() {
            let up = corner & (1 << axis) != 0;
            index = index * N + cell + usize::from(up);
            w *= if up { *t } else { 1.0 - t };
        }
        *slot = (index, w);
    }
    out
}

/// Index strides of the four axes (C is the slowest).
const STRIDES: [usize; 4] = [N * N * N, N * N, N, 1];

impl Lattice {
    /// The correction at `cmyk`.
    pub fn at(&self, cmyk: [f32; 4]) -> [f32; 3] {
        let mut out = [0f32; 3];
        for (i, w) in corners(cmyk) {
            if w == 0.0 {
                continue;
            }
            let v = self.values.get(i).copied().unwrap_or_default();
            for c in 0..3 {
                out[c] += w * v[c];
            }
        }
        out
    }

    /// The lattice that best fits `points` (inks, correction) in the least-squares sense, with
    /// `smoothing` times the squared second differences of the nodes along each axis added to the
    /// error. The node at no ink stays 0, so the paper keeps the model's color (white, media-
    /// relative). Solved by conjugate gradients, one Lab channel at a time.
    pub fn fit(points: &[([f32; 4], [f32; 3])], smoothing: f64) -> Lattice {
        let weights: Vec<[(usize, f32); 16]> = points.iter().map(|(cmyk, _)| corners(*cmyk)).collect();
        // The divided second difference at interior knot j, scaled to a unit spacing of 1/8.
        let curvature: Vec<[f64; 3]> = (1..N - 1)
            .map(|j| {
                let (h1, h2) = (f64::from(KNOTS[j] - KNOTS[j - 1]), f64::from(KNOTS[j + 1] - KNOTS[j]));
                let s = 0.125f64 * 0.125;
                [2.0 / (h1 * (h1 + h2)) * s, -2.0 / (h1 * h2) * s, 2.0 / (h2 * (h1 + h2)) * s]
            })
            .collect();
        let apply = |x: &[f64], out: &mut [f64]| {
            out.fill(0.0);
            for w in &weights {
                let v: f64 = w.iter().map(|(i, w)| f64::from(*w) * x.get(*i).copied().unwrap_or(0.0)).sum();
                for (i, w) in w {
                    if let Some(o) = out.get_mut(*i) {
                        *o += f64::from(*w) * v;
                    }
                }
            }
            for node in 0..NODES {
                let mut rest = node;
                for stride in STRIDES {
                    let j = rest / stride;
                    rest %= stride;
                    // Only interior knots (1 to N − 2) have a second difference.
                    let Some(c) = j.checked_sub(1).and_then(|k| curvature.get(k)) else { continue };
                    let idx = [node - stride, node, node + stride];
                    let d: f64 = idx.iter().zip(c).map(|(i, c)| c * x.get(*i).copied().unwrap_or(0.0)).sum();
                    for (i, c) in idx.iter().zip(c) {
                        if let Some(o) = out.get_mut(*i) {
                            *o += smoothing * c * d;
                        }
                    }
                }
            }
            for (o, v) in out.iter_mut().zip(x) {
                *o += 1e-9 * v;
            }
            // The paper's node is held: with its right-hand side 0 it stays 0 in every iterate.
            if let (Some(o), Some(v)) = (out.first_mut(), x.first()) {
                *o = *v;
            }
        };
        let mut values = vec![[0f32; 3]; NODES];
        for ch in 0..3 {
            let mut b = vec![0f64; NODES];
            for (w, (_, r)) in weights.iter().zip(points) {
                for (i, w) in w {
                    if let Some(o) = b.get_mut(*i) {
                        *o += f64::from(*w) * f64::from(r[ch]);
                    }
                }
            }
            if let Some(o) = b.first_mut() {
                *o = 0.0;
            }
            let x = conjugate_gradients(&apply, &b, 4000);
            for (v, x) in values.iter_mut().zip(&x) {
                v[ch] = *x as f32;
            }
        }
        Lattice { values }
    }
}

/// Solve `A x = b` for the symmetric positive definite `A` that `apply` multiplies by.
fn conjugate_gradients(apply: &dyn Fn(&[f64], &mut [f64]), b: &[f64], max_iterations: usize) -> Vec<f64> {
    let n = b.len();
    let mut x = vec![0f64; n];
    let mut r = b.to_vec();
    let mut p = r.clone();
    let mut ap = vec![0f64; n];
    let dot = |a: &[f64], b: &[f64]| a.iter().zip(b).map(|(a, b)| a * b).sum::<f64>();
    let mut rr = dot(&r, &r);
    let stop = rr * 1e-20;
    for _ in 0..max_iterations {
        if rr <= stop || rr == 0.0 {
            break;
        }
        apply(&p, &mut ap);
        let pap = dot(&p, &ap);
        if pap <= 0.0 || !pap.is_finite() {
            break;
        }
        let alpha = rr / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        let next = dot(&r, &r);
        let beta = next / rr;
        rr = next;
        for i in 0..n {
            p[i] = r[i] + beta * p[i];
        }
    }
    x
}
