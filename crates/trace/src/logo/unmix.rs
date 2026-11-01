//! Soft coverage: how much of each flat colour (and of transparency) every pixel contains.
//!
//! An edge pixel of an anti-aliased logo is a blend of the colours that meet there. Reading it as
//! "the nearest colour" throws away exactly the sub-pixel information that says where the edge
//! runs, so each pixel is instead explained as a mix of at most three classes, whichever mix
//! reproduces the pixel best.

use std::collections::{HashMap, HashSet};

/// Premultiplied RGBA in `0..=1`.
pub(crate) type Px = [f32; 4];

const LAMBDA: f64 = 30.0;
/// Pixels this close (in 0..1 premultiplied RGBA) to a class are taken as exactly that class.
const PURE: f64 = 4.0 / 255.0;

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

/// How many distinct pixel values are not (nearly) exactly one class: the ones `unmix` has to work
/// on. A flat area whose color is near but not exactly a class counts once, however large it is.
pub(crate) fn count_ambiguous(pix: &[Px], classes: &[[f64; 4]]) -> usize {
    let mut seen = HashSet::new();
    for px in pix {
        let p = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2]), f64::from(px[3])];
        if !classes.iter().any(|cv| (0..4).map(|j| (p[j] - cv[j]).powi(2)).sum::<f64>().sqrt() < PURE) {
            seen.insert(px.map(f32::to_bits));
        }
    }
    seen.len()
}

/// Coverage planes, one `width × height` plane per class (each pixel's weights sum to 1).
pub(crate) fn unmix(pix: &[Px], width: usize, height: usize, classes: &[[f64; 4]]) -> (Vec<Vec<f32>>, Vec<f32>) {
    let k = classes.len();
    let n = width.saturating_mul(height);
    let mut planes = vec![vec![0.0f32; n]; k];
    // Squared error left over by the best mix: large where no mix of the classes explains the pixel.
    let mut residual = vec![0.0f32; n];
    let subs = subsets(classes);
    // Each distinct pixel value is mixed once: a flat area that is near but not exactly a class
    // repeats one value.
    let mut mixed: HashMap<[u32; 4], Mix> = HashMap::new();
    for (i, px) in pix.iter().take(n).enumerate() {
        let p = [f64::from(px[0]), f64::from(px[1]), f64::from(px[2]), f64::from(px[3])];
        // Fast path: a pixel that is (nearly) exactly one class.
        let mut near = (f64::MAX, 0usize);
        for (c, cv) in classes.iter().enumerate() {
            let d: f64 = (0..4).map(|j| (p[j] - cv[j]).powi(2)).sum();
            if d < near.0 {
                near = (d, c);
            }
        }
        if near.0.sqrt() < PURE {
            if let Some(v) = planes.get_mut(near.1).and_then(|pl| pl.get_mut(i)) {
                *v = 1.0;
            }
            continue;
        }
        let (res, weights) = mixed.entry(px.map(f32::to_bits)).or_insert_with(|| best_mix(p, near, &subs));
        if let Some(r) = residual.get_mut(i) {
            *r = *res;
        }
        for &(c, wv) in weights.iter() {
            if let Some(v) = planes.get_mut(c).and_then(|pl| pl.get_mut(i)) {
                *v = wv;
            }
        }
    }
    (planes, residual)
}

/// A pixel's mix: the squared error it leaves and each class's weight.
type Mix = (f32, Vec<(usize, f32)>);

/// The mix of at most three classes that reproduces `p` best, starting from the nearest single
/// class `near` (squared distance, class).
fn best_mix(p: [f64; 4], near: (f64, usize), subs: &[Subset]) -> Mix {
    // Best single class is the starting point; try every two- and three-class mix.
    let mut best = (near.0, vec![(near.1, 1.0f64)]);
    let mut best_penalty = 0.0;
    for s in subs {
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
        if res < best.0 {
            best = (res, (0..s.n).map(|j| (s.idx[j], w[j])).collect());
            best_penalty = s.penalty;
        }
    }
    ((best.0 - best_penalty).max(0.0) as f32, best.1.into_iter().map(|(c, w)| (c, w as f32)).collect())
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
        let (pl, _) = unmix(&pix, 2, 1, &classes());
        assert_eq!((pl[0][0], pl[2][1]), (1.0, 1.0));
    }

    #[test]
    fn a_blend_of_two_colours_recovers_its_proportions() {
        // 30% red over 70% blue.
        let pix = vec![[0.3, 0.0, 0.7, 1.0]];
        let (pl, _) = unmix(&pix, 1, 1, &classes());
        assert!((pl[0][0] - 0.3).abs() < 0.02, "red {}", pl[0][0]);
        assert!((pl[1][0] - 0.7).abs() < 0.02, "blue {}", pl[1][0]);
    }

    #[test]
    fn a_half_transparent_edge_is_half_covered() {
        // Red at 40% coverage over nothing: premultiplied (0.4, 0, 0, 0.4).
        let pix = vec![[0.4, 0.0, 0.0, 0.4]];
        let (pl, _) = unmix(&pix, 1, 1, &classes());
        assert!((pl[0][0] - 0.4).abs() < 0.02 && (pl[2][0] - 0.6).abs() < 0.02, "{:?}", (pl[0][0], pl[2][0]));
    }

    #[test]
    fn a_colour_no_class_can_explain_leaves_a_large_residual() {
        // Green is none of red, blue or transparent, nor any blend of them.
        let pix = vec![[0.0, 0.8, 0.0, 1.0], [0.3, 0.0, 0.7, 1.0]];
        let (_, res) = unmix(&pix, 2, 1, &classes());
        assert!(res[0] > 0.1, "unexplained pixel residual {}", res[0]);
        assert!(res[1] < 1e-3, "an explained blend residual {}", res[1]);
    }

    #[test]
    fn a_repeated_color_near_a_class_is_counted_once_and_mixed_alike() {
        // A flat area a few levels off red, a blend, and pure red and transparent pixels.
        let off = [0.97, 0.02, 0.02, 1.0];
        let mut pix = vec![off; 1000];
        pix.extend([[0.3, 0.0, 0.7, 1.0], [1.0, 0.0, 0.0, 1.0], [0.0, 0.0, 0.0, 0.0]]);
        assert_eq!(count_ambiguous(&pix, &classes()), 2);
        let (pl, res) = unmix(&pix, pix.len(), 1, &classes());
        assert!(pl[0][0] > 0.95, "red {}", pl[0][0]);
        for i in 1..1000 {
            assert_eq!((pl[0][i], pl[1][i], pl[2][i], res[i]), (pl[0][0], pl[1][0], pl[2][0], res[0]), "pixel {i}");
        }
        assert!((pl[0][1000] - 0.3).abs() < 0.02 && (pl[1][1000] - 0.7).abs() < 0.02);
    }

    #[test]
    fn weights_sum_to_one() {
        let pix = vec![[0.2, 0.1, 0.5, 0.8], [0.9, 0.0, 0.05, 0.95]];
        let (pl, _) = unmix(&pix, 2, 1, &classes());
        for i in 0..2 {
            let s: f32 = pl.iter().map(|p| p[i]).sum();
            assert!((s - 1.0).abs() < 1e-4, "pixel {i} sums to {s}");
        }
    }
}
