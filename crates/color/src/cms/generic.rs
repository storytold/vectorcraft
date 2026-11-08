//! "VectorCraft Generic CMYK (SWOP-like)": our own parametric press model.
//!
//! We can't redistribute Adobe's or ECI's characterisation profiles, so the built-in CMYK space is
//! a small physical model instead of an ICC LUT. It is *ours* and only SWOP-*like*. Two of its
//! figures are published values for SWOP printing: the tone value increase of each ink at 40% and
//! 50% from CGATS TR 001 (<https://registry.color.org/cmyk-registry/cgats_tr_001>), and the 300%
//! total ink limit (<https://en.wikipedia.org/wiki/Specifications_for_Web_Offset_Publications>).
//! The paper, solid and overprint colors and the black-generation curve are VectorCraft's own values.
//!
//! * **Forward (CMYK → XYZ):** each ink's nominal dot area `a` is widened by a tone curve
//!   `a + 4·g·a·(1−a)·(1 + s·(2a−1))` that passes through the TR 001 tone value increase at 50%
//!   (`g`) and 40% (which sets `s`). The effective areas mix the 16 Neugebauer primaries (paper and
//!   every solid and overprint of C, M, Y and K, media-relative) with Demichel weights, linearly in
//!   XYZ, so a single ink's colorimetric tone value increase (Murray–Davies) is the TR 001 value.
//! * **Inverse (Lab → CMYK):** damped, bounded Gauss–Newton on the forward model solves CMY for a
//!   fixed K. Black generation is GCR: solve CMY with K = 0, take the gray component
//!   `min(C, M, Y)`, add [`BlackGeneration::shortfall`] for each L* point by which that solve stays
//!   lighter than the target, map the sum through the black curve to K, re-solve CMY. Under-color
//!   removal enforces the total area coverage (TAC) limit by adding K and re-solving, then scaling
//!   CMY as a last resort. The black point, which source black separates to and which displays as
//!   black with black-point compensation, is [`BlackGeneration::black_point_k`] black with neutral
//!   CMY filling the TAC. With compensation, a build darker than it in every XYZ channel also
//!   displays as black (see [`GenericCmyk::display_xyz`]).
//! * **Rendering intents:** colorimetric intents map the source white to paper (relative) or not
//!   (absolute) and clip; perceptual applies black-point compensation plus soft chroma compression
//!   against a gamut boundary descriptor (segment maxima of the forward model); saturation keeps
//!   chroma by pulling out-of-gamut colours towards the hue's cusp lightness.

use std::sync::OnceLock;

use super::Intent;
use super::lab::{D50, Lab, delta_e2000, lab_to_xyz, srgb_to_lab, xyz_to_lab, xyz_to_srgb};

/// Black generation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlackGeneration {
    /// The black curve as `(x, K)` knots with rising `x`, read by linear interpolation. `x` is the
    /// gray component of the solve without black plus [`Self::shortfall`] per L* point by which
    /// that solve stays lighter than the target.
    pub curve: [(f32, f32); 13],
    /// Added to the gray component for each L* point the black-free solve stays too light.
    pub shortfall: f32,
    /// Black of the black point, whose CMY is neutral and fills the TAC.
    pub black_point_k: f32,
}

/// Parameters of the parametric press model.
#[derive(Clone, Debug)]
pub struct GenericCmykParams {
    pub name: &'static str,
    /// Paper color (absolute Lab, D50).
    pub paper: Lab,
    /// Media-relative Lab of the solids and overprints, Demichel order (index `c | m<<1 | y<<2 |
    /// k<<3`, paper left out): C, M, CM, Y, CY, MY, CMY, then K, CK, MK, CMK, YK, CYK, MYK, CMYK.
    pub primaries: [Lab; 15],
    /// Tone value increase at 50% per ink (C, M, Y, K).
    pub tvi50: [f32; 4],
    /// Tone value increase at 40% per ink (C, M, Y, K).
    pub tvi40: [f32; 4],
    /// Total area coverage limit (sum of inks, 1.0 = 100%).
    pub tac: f32,
    pub bg: BlackGeneration,
}

/// The built-in SWOP-like press: TR 001 tone value increase, a 300% ink limit, and VectorCraft's
/// own paper, primaries and black curve.
pub fn swop_like() -> GenericCmykParams {
    GenericCmykParams {
        name: super::GENERIC_CMYK,
        paper: Lab::new(93.0, -0.5, 2.5),
        primaries: [
            Lab::new(65.7, -42.6, -47.2), // C
            Lab::new(55.5, 76.2, -9.4),   // M
            Lab::new(31.1, 20.3, -49.7),  // CM (blue)
            Lab::new(96.2, -7.0, 90.1),   // Y
            Lab::new(61.0, -68.4, 30.1),  // CY (green)
            Lab::new(55.0, 69.8, 45.0),   // MY (red)
            Lab::new(28.3, 0.7, -0.5),    // CMY
            Lab::new(21.5, 0.8, -0.9),    // K
            Lab::new(15.9, -8.4, -10.0),  // CK
            Lab::new(14.7, 15.4, -1.0),   // MK
            Lab::new(11.2, 8.9, -6.6),    // CMK
            Lab::new(20.2, -2.4, 12.0),   // YK
            Lab::new(15.7, -14.6, 4.6),   // CYK
            Lab::new(14.5, 11.2, 5.7),    // MYK
            Lab::new(11.0, -0.7, 0.1),    // CMYK
        ],
        tvi50: [0.200, 0.176, 0.168, 0.216],
        tvi40: [0.200, 0.173, 0.167, 0.219],
        tac: 3.0,
        bg: BlackGeneration {
            curve: [
                (0.0, 0.0),
                (0.25, 0.0),
                (0.35, 0.006),
                (0.45, 0.059),
                (0.55, 0.139),
                (0.65, 0.23),
                (0.75, 0.322),
                (0.85, 0.41),
                (0.95, 0.511),
                (1.05, 0.6),
                (1.2, 0.733),
                (1.35, 0.858),
                (1.5, 1.0),
            ],
            shortfall: 0.03,
            black_point_k: 0.9,
        },
    }
}

/// Gamut boundary descriptor: max chroma per (L, hue) cell in media-relative Lab.
struct Gbd {
    cmax: Vec<f32>,
    /// Lightness of maximum chroma per hue bin.
    cusp_l: Vec<f32>,
}

const GBD_L: usize = 51; // L 0..100, step 2
const GBD_H: usize = 72; // 5° bins

pub struct GenericCmyk {
    pub params: GenericCmykParams,
    /// Media-relative XYZ of the 16 primaries (index = c | m<<1 | y<<2 | k<<3; 0 is paper).
    prim: [[f32; 3]; 16],
    /// Shape `s` of each ink's tone curve, from its 40% and 50% tone value increase.
    skew: [f32; 4],
    paper_xyz: [f32; 3],
    /// Media-relative XYZ of the black point (for black-point compensation).
    black_rel: [f32; 3],
    gbd: OnceLock<Gbd>,
}

impl GenericCmyk {
    pub fn new(params: GenericCmykParams) -> Self {
        let paper_xyz = lab_to_xyz(params.paper);
        let mut prim = [D50; 16];
        for (p, lab) in prim[1..].iter_mut().zip(params.primaries.iter()) {
            *p = lab_to_xyz(*lab).map(|v| v.max(1e-6));
        }
        // The curve's increase at 40% is 0.96·g·(1 − 0.2·s), so `s` follows from the 40% value.
        let skew = [0, 1, 2, 3].map(|i| {
            let g = params.tvi50[i];
            if g > 1e-6 { (1.0 - params.tvi40[i] / (0.96 * g)) / 0.2 } else { 0.0 }
        });
        let mut g = Self { params, prim, skew, paper_xyz, black_rel: [0.0; 3], gbd: OnceLock::new() };
        g.black_rel = g.to_xyz_rel(g.black_point_build());
        g
    }

    pub fn name(&self) -> &str {
        self.params.name
    }

    /// The black point: [`BlackGeneration::black_point_k`] black, with C, M and Y
    /// chosen so that they fill the TAC and the color is neutral (a* = b* = 0).
    pub fn black_point_build(&self) -> [f32; 4] {
        let k = self.params.bg.black_point_k.clamp(0.0, 1.0);
        let rest = (self.params.tac - k).clamp(0.0, 3.0);
        let build = |c: f32, m: f32| [c, m, (rest - c - m).clamp(0.0, 1.0), k];
        let ab = |c: f32, m: f32| {
            let l = self.to_lab_rel(build(c, m));
            [l.a, l.b]
        };
        let lo = (rest - 2.0).max(0.0);
        let hi = rest.min(1.0);
        let (mut c, mut m) = ((rest / 3.0).clamp(lo, hi), (rest / 3.0).clamp(lo, hi));
        for _ in 0..30 {
            let r = ab(c, m);
            if r[0].abs() + r[1].abs() < 1e-4 {
                break;
            }
            let h = 1e-3;
            let rc = ab(c - h, m);
            let rm = ab(c, m - h);
            let j = [[(r[0] - rc[0]) / h, (r[0] - rm[0]) / h], [(r[1] - rc[1]) / h, (r[1] - rm[1]) / h]];
            let det = j[0][0] * j[1][1] - j[0][1] * j[1][0];
            if det.abs() < 1e-9 {
                break;
            }
            let dc = (-r[0] * j[1][1] + r[1] * j[0][1]) / det;
            let dm = (-r[1] * j[0][0] + r[0] * j[1][0]) / det;
            c = (c + dc.clamp(-0.1, 0.1)).clamp(lo, hi);
            m = (m + dm.clamp(-0.1, 0.1)).clamp(lo, hi);
            // Yellow takes the rest, so keep it within 0..1 as well.
            let y = rest - c - m;
            if y > 1.0 {
                c += (y - 1.0) / 2.0;
                m += (y - 1.0) / 2.0;
            } else if y < 0.0 {
                c += y / 2.0;
                m += y / 2.0;
            }
            c = c.clamp(lo, hi);
            m = m.clamp(lo, hi);
        }
        build(c.clamp(lo, hi), m.clamp(lo, hi))
    }

    /// Effective dot area of ink `i` at nominal area `a`: the TR 001-style tone curve.
    fn gain(&self, i: usize, a: f32) -> f32 {
        let a = if a.is_finite() { a.clamp(0.0, 1.0) } else { 0.0 };
        let g = self.params.tvi50[i];
        (a + 4.0 * g * a * (1.0 - a) * (1.0 + self.skew[i] * (2.0 * a - 1.0))).clamp(0.0, 1.0)
    }

    /// CMYK (0..1) → absolute XYZ (D50).
    pub fn to_xyz(&self, cmyk: [f32; 4]) -> [f32; 3] {
        let x = self.to_xyz_rel(cmyk);
        [x[0] * self.paper_xyz[0] / D50[0], x[1] * self.paper_xyz[1] / D50[1], x[2] * self.paper_xyz[2] / D50[2]]
    }

    /// Media-relative XYZ (paper = D50 white).
    pub fn to_xyz_rel(&self, cmyk: [f32; 4]) -> [f32; 3] {
        let e = [0, 1, 2, 3].map(|i| self.gain(i, cmyk[i]));
        let mut out = [0.0f32; 3];
        for (i, p) in self.prim.iter().enumerate() {
            let mut w = 1.0;
            for (bit, a) in e.iter().enumerate() {
                w *= if i & (1 << bit) != 0 { *a } else { 1.0 - a };
            }
            if w == 0.0 {
                continue;
            }
            for (o, v) in out.iter_mut().zip(p) {
                *o += w * v;
            }
        }
        out
    }

    pub fn to_lab_rel(&self, cmyk: [f32; 4]) -> Lab {
        xyz_to_lab(self.to_xyz_rel(cmyk))
    }
    pub fn to_lab_abs(&self, cmyk: [f32; 4]) -> Lab {
        xyz_to_lab(self.to_xyz(cmyk))
    }

    /// Display XYZ: media-relative (paper shows as white) or absolute (simulate paper colour).
    /// With `bpc` (media-relative only) the press black displays as black, undoing the
    /// compensation [`Self::gamut_map`] applies on the way in, so sRGB black separated to the
    /// black point shows as black again. Each XYZ channel is clamped at 0 on its own, so a build
    /// darker than the black point in every channel shows as black, and one darker in only some
    /// channels shows as a colored near-black.
    pub fn display_xyz(&self, cmyk: [f32; 4], absolute: bool, bpc: bool) -> [f32; 3] {
        if absolute {
            return self.to_xyz(cmyk);
        }
        let x = self.to_xyz_rel(cmyk);
        if !bpc {
            return x;
        }
        let bp = self.black_rel;
        [0, 1, 2].map(|i| ((x[i] - bp[i]) / (1.0 - bp[i] / D50[i])).max(0.0))
    }

    /// Display sRGB, see [`Self::display_xyz`].
    pub fn to_srgb(&self, cmyk: [f32; 4], absolute: bool, bpc: bool) -> [f32; 3] {
        xyz_to_srgb(self.display_xyz(cmyk, absolute, bpc))
    }

    /// Lab of the black point (media-relative), see [`Self::black_point_build`].
    pub fn black_point(&self) -> Lab {
        xyz_to_lab(self.black_rel)
    }

    /// Solve CMY for a fixed K so that the media-relative Lab matches `target` (bounded).
    fn solve_cmy(&self, target: Lab, k: f32, init: [f32; 3]) -> [f32; 3] {
        let mut x = init.map(|v| v.clamp(0.0, 1.0));
        let f = |x: [f32; 3]| {
            let l = self.to_lab_rel([x[0], x[1], x[2], k]);
            [l.l - target.l, l.a - target.a, l.b - target.b]
        };
        let mut r = f(x);
        let mut err = r.iter().map(|v| v * v).sum::<f32>();
        let mut lambda = 1e-3f32;
        for _ in 0..40 {
            if err < 1e-4 {
                break;
            }
            // Numeric Jacobian (one-sided, pointing into the box).
            let mut j = [[0.0f32; 3]; 3];
            for col in 0..3 {
                let h = if x[col] > 0.5 { -1e-3 } else { 1e-3 };
                let mut xp = x;
                xp[col] += h;
                let rp = f(xp);
                for row in 0..3 {
                    j[row][col] = (rp[row] - r[row]) / h;
                }
            }
            // Levenberg–Marquardt step: (JᵀJ + λ·diag) δ = −Jᵀr.
            let mut a = [[0.0f32; 3]; 3];
            let mut g = [0.0f32; 3];
            for p in 0..3 {
                for q in 0..3 {
                    a[p][q] = (0..3).map(|i| j[i][p] * j[i][q]).sum();
                }
                g[p] = -(0..3).map(|i| j[i][p] * r[i]).sum::<f32>();
            }
            // An ink at a bound whose descent direction points out of the box stays where it is; the step moves the other inks.
            for p in 0..3 {
                if (x[p] <= 0.0 && g[p] < 0.0) || (x[p] >= 1.0 && g[p] > 0.0) {
                    a[p] = [0.0; 3];
                    for row in &mut a {
                        row[p] = 0.0;
                    }
                    a[p][p] = 1.0;
                    g[p] = 0.0;
                }
            }
            // Every ink is held at a bound, or the gradient is zero: no step can lower the error.
            if g == [0.0; 3] {
                break;
            }
            let mut improved = false;
            for _ in 0..8 {
                let mut al = a;
                for (p, row) in al.iter_mut().enumerate() {
                    row[p] += lambda * (a[p][p] + 1e-3);
                }
                let Some(d) = solve3(al, g) else {
                    lambda *= 10.0;
                    continue;
                };
                let xn = [(x[0] + d[0]).clamp(0.0, 1.0), (x[1] + d[1]).clamp(0.0, 1.0), (x[2] + d[2]).clamp(0.0, 1.0)];
                let rn = f(xn);
                let en = rn.iter().map(|v| v * v).sum::<f32>();
                if en < err {
                    let step = (0..3).map(|i| (xn[i] - x[i]).abs()).fold(0.0, f32::max);
                    x = xn;
                    r = rn;
                    err = en;
                    lambda = (lambda * 0.3).max(1e-7);
                    improved = step > 1e-6;
                    break;
                }
                lambda *= 10.0;
            }
            if !improved {
                break;
            }
        }
        x
    }

    fn initial_cmy(target: Lab, k: f32) -> [f32; 3] {
        let rgb = super::lab::lab_to_srgb(target);
        let d = (1.0 - k).max(0.05);
        rgb.map(|v| ((1.0 - v - k) / d).clamp(0.0, 1.0))
    }

    /// Black for `x` on the black curve (see [`BlackGeneration::curve`]).
    fn black_for(&self, x: f32) -> f32 {
        let curve = &self.params.bg.curve;
        let mut prev = curve[0];
        if x <= prev.0 {
            return prev.1.clamp(0.0, 1.0);
        }
        for &(kx, kk) in &curve[1..] {
            if x <= kx {
                let t = if kx > prev.0 { (x - prev.0) / (kx - prev.0) } else { 1.0 };
                return (prev.1 + (kk - prev.1) * t).clamp(0.0, 1.0);
            }
            prev = (kx, kk);
        }
        prev.1.clamp(0.0, 1.0)
    }

    /// Separate a media-relative Lab (already gamut-mapped) into CMYK with GCR/UCR and the TAC limit.
    pub fn separate(&self, target: Lab) -> [f32; 4] {
        if target.l >= 99.99 && target.a.abs() < 0.01 && target.b.abs() < 0.01 {
            return [0.0; 4];
        }
        let cmy0 = self.solve_cmy(target, 0.0, Self::initial_cmy(target, 0.0));
        let gray = cmy0[0].min(cmy0[1]).min(cmy0[2]);
        // When CMY alone can't get dark enough, black carries the rest.
        let l0 = self.to_lab_rel([cmy0[0], cmy0[1], cmy0[2], 0.0]).l;
        let mut k = self.black_for(gray + self.params.bg.shortfall * (l0 - target.l).max(0.0));
        let mut cmy = if k > 0.0 { self.solve_cmy(target, k, Self::initial_cmy(target, k)) } else { cmy0 };
        // UCR: trade CMY for K until within the TAC limit.
        for _ in 0..6 {
            let sum = cmy.iter().sum::<f32>() + k;
            if sum <= self.params.tac + 1e-3 || k >= 1.0 {
                break;
            }
            k = (k + (sum - self.params.tac) * 0.5).min(1.0);
            cmy = self.solve_cmy(target, k, cmy);
        }
        let sum = cmy.iter().sum::<f32>() + k;
        if sum > self.params.tac {
            let s = ((self.params.tac - k) / cmy.iter().sum::<f32>().max(1e-6)).clamp(0.0, 1.0);
            cmy = cmy.map(|v| v * s);
        }
        [cmy[0], cmy[1], cmy[2], k].map(|v| if v < 1e-4 { 0.0 } else { v.min(1.0) })
    }

    fn gbd(&self) -> &Gbd {
        self.gbd.get_or_init(|| {
            let mut cmax = vec![0.0f32; GBD_L * GBD_H];
            let mut cusp = vec![(0.0f32, 50.0f32); GBD_H];
            let n = 16;
            for ki in 0..=5 {
                let k = ki as f32 / 5.0;
                for ci in 0..=n {
                    for mi in 0..=n {
                        for yi in 0..=n {
                            let (c, m, y) = (ci as f32 / n as f32, mi as f32 / n as f32, yi as f32 / n as f32);
                            if c + m + y + k > self.params.tac + 1e-3 {
                                continue;
                            }
                            let lab = self.to_lab_rel([c, m, y, k]);
                            let li = ((lab.l / 2.0).round() as usize).min(GBD_L - 1);
                            let hi = ((lab.hue() / 5.0) as usize).min(GBD_H - 1);
                            let ch = lab.chroma();
                            let cell = &mut cmax[li * GBD_H + hi];
                            *cell = cell.max(ch);
                            if ch > cusp[hi].0 {
                                cusp[hi] = (ch, lab.l);
                            }
                        }
                    }
                }
            }
            // Fill holes: take the max over neighbouring hue bins, then interpolate along L.
            let mut smooth = cmax.clone();
            for li in 0..GBD_L {
                for hi in 0..GBD_H {
                    let get = |h: usize| cmax[li * GBD_H + h % GBD_H];
                    smooth[li * GBD_H + hi] = get(hi).max(get(hi + 1)).max(get(hi + GBD_H - 1));
                }
            }
            for hi in 0..GBD_H {
                let col: Vec<f32> = (0..GBD_L).map(|li| smooth[li * GBD_H + hi]).collect();
                let known: Vec<usize> = (0..GBD_L).filter(|&li| col[li] > 0.0).collect();
                for li in 0..GBD_L {
                    if col[li] > 0.0 {
                        continue;
                    }
                    let below = known.iter().rev().find(|&&k| k < li);
                    let above = known.iter().find(|&&k| k > li);
                    smooth[li * GBD_H + hi] = match (below, above) {
                        (Some(&b), Some(&a)) => col[b] + (col[a] - col[b]) * (li - b) as f32 / (a - b) as f32,
                        _ => 0.0,
                    };
                }
            }
            Gbd { cmax: smooth, cusp_l: cusp.into_iter().map(|c| c.1).collect() }
        })
    }

    /// Maximum printable chroma at (L, hue), media-relative.
    pub fn max_chroma(&self, l: f32, hue: f32) -> f32 {
        let g = self.gbd();
        let lf = (l.clamp(0.0, 100.0) / 2.0).min((GBD_L - 1) as f32);
        let hf = hue.rem_euclid(360.0) / 5.0;
        let (l0, h0) = (lf.floor() as usize, hf.floor() as usize);
        let (l1, h1) = ((l0 + 1).min(GBD_L - 1), (h0 + 1) % GBD_H);
        let (tl, th) = (lf - l0 as f32, hf - h0 as f32);
        let at = |li: usize, hi: usize| g.cmax[li * GBD_H + hi % GBD_H];
        let a = at(l0, h0) * (1.0 - th) + at(l0, h1) * th;
        let b = at(l1, h0) * (1.0 - th) + at(l1, h1) * th;
        a * (1.0 - tl) + b * tl
    }

    fn cusp_l(&self, hue: f32) -> f32 {
        self.gbd().cusp_l[((hue.rem_euclid(360.0) / 5.0) as usize).min(GBD_H - 1)]
    }

    /// Map an absolute source Lab (D50; sRGB white = L 100) to the media-relative target for `intent`.
    pub fn gamut_map(&self, src: Lab, intent: Intent, bpc: bool) -> Lab {
        if intent == Intent::AbsoluteColorimetric {
            // Keep absolute colorimetry: express in media-relative terms without white mapping.
            let x = lab_to_xyz(src);
            return xyz_to_lab([x[0] * D50[0] / self.paper_xyz[0], x[1] * D50[1] / self.paper_xyz[1], x[2] * D50[2] / self.paper_xyz[2]]);
        }
        let mut lab = src;
        if bpc || matches!(intent, Intent::Perceptual | Intent::Saturation) {
            // Black-point compensation in XYZ: source black (0) → printable black.
            let x = lab_to_xyz(lab);
            let bp = self.black_rel;
            let y = [0, 1, 2].map(|i| bp[i] + x[i] * (1.0 - bp[i] / D50[i]));
            lab = xyz_to_lab(y);
        }
        match intent {
            Intent::Perceptual => {
                let c = lab.chroma();
                if c > 1e-3 {
                    // Cusp-directed: very out-of-gamut colours also move towards the lightness
                    // where the press reaches its maximum chroma for this hue.
                    let h = lab.hue();
                    let cmax0 = self.max_chroma(lab.l, h).max(1.0);
                    if c > cmax0 {
                        let w = ((c - cmax0) / c).clamp(0.0, 1.0) * 0.9;
                        lab.l += (self.cusp_l(h) - lab.l) * w;
                    }
                    let cmax = self.max_chroma(lab.l, h).max(1.0);
                    let r = c / cmax;
                    const KNEE: f32 = 0.75;
                    let r2 = if r > KNEE { KNEE + (1.0 - KNEE) * (1.0 - (-(r - KNEE) / (1.0 - KNEE)).exp()) } else { r };
                    lab = Lab::from_lch(lab.l, r2 * cmax * 0.995, lab.hue());
                }
                lab
            }
            Intent::Saturation => {
                let (c, h) = (lab.chroma(), lab.hue());
                if c > 1e-3 {
                    let mut l = lab.l;
                    let cmax = self.max_chroma(l, h);
                    if c > cmax {
                        let cl = self.cusp_l(h);
                        l += (cl - l) * ((c - cmax) / c.max(1.0)).clamp(0.0, 1.0);
                    }
                    let cmax = self.max_chroma(l, h);
                    lab = Lab::from_lch(l, (c * 1.1).min(cmax * 0.995), h);
                }
                lab
            }
            _ => lab,
        }
    }

    /// Convert an absolute Lab to CMYK with `intent`.
    pub fn from_lab(&self, src: Lab, intent: Intent, bpc: bool) -> [f32; 4] {
        self.separate(self.gamut_map(src, intent, bpc))
    }

    pub fn from_srgb(&self, rgb: [f32; 3], intent: Intent, bpc: bool) -> [f32; 4] {
        self.from_lab(srgb_to_lab(rgb), intent, bpc)
    }

    /// ΔE2000 between a colour and its best (relative colorimetric) reproduction as it displays
    /// (with `bpc`, the press black showing as black).
    pub fn gamut_error(&self, src: Lab, bpc: bool) -> f32 {
        let target = self.gamut_map(src, Intent::RelativeColorimetric, bpc);
        let cmyk = self.separate(target);
        delta_e2000(xyz_to_lab(self.display_xyz(cmyk, false, bpc)), src)
    }
}

fn solve3(a: [[f32; 3]; 3], b: [f32; 3]) -> Option<[f32; 3]> {
    let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
    if det.abs() < 1e-12 {
        return None;
    }
    let mut out = [0.0f32; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let mut m = a;
        for r in 0..3 {
            m[r][i] = b[r];
        }
        *o = (m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
            + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]))
            / det;
    }
    Some(out)
}
