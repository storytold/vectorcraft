//! "VectorCraft Generic CMYK (SWOP-like)": our own parametric press model.
//!
//! We can't redistribute Adobe's or ECI's characterisation profiles, so the built-in CMYK space is
//! a small physical model instead of an ICC LUT. It is *ours* and only SWOP-*like*: the numbers
//! are rounded, typical values for coated web offset, not a measured dataset.
//!
//! * **Forward (CMYK → XYZ):** each ink's nominal dot area is widened by a parabolic dot-gain
//!   curve (`a + 4·g·a·(1−a)`, i.e. `g` gain at 50%); the effective areas mix the eight CMY
//!   Neugebauer primaries with Demichel weights; black is mixed in the same way against
//!   "primary + K" colours whose optical densities add but saturate towards a maximum density
//!   (normalised so K on paper is exact), so rich blacks don't multiply to an unrealistic depth.
//! * **Inverse (Lab → CMYK):** damped, bounded Gauss–Newton on the forward model solves CMY for a
//!   fixed K. Black generation is GCR: solve CMY with K = 0, take the grey component
//!   `min(C, M, Y)`, map it through the black-generation curve to K, re-solve CMY. Under-colour
//!   removal enforces the total area coverage (TAC) limit by adding K and re-solving, then scaling
//!   CMY as a last resort.
//! * **Rendering intents:** colorimetric intents map the source white to paper (relative) or not
//!   (absolute) and clip; perceptual applies black-point compensation plus soft chroma compression
//!   against a gamut boundary descriptor (segment maxima of the forward model); saturation keeps
//!   chroma by pulling out-of-gamut colours towards the hue's cusp lightness.

use std::sync::OnceLock;

use super::Intent;
use super::lab::{D50, Lab, delta_e2000, lab_to_xyz, srgb_to_lab, xyz_to_lab, xyz_to_srgb};

/// Black generation strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BlackGeneration {
    /// Grey component below which no black is generated.
    pub start: f32,
    /// Maximum black.
    pub max_k: f32,
    /// Curve exponent (>1 = lighter black in the midtones).
    pub gamma: f32,
}

/// Parameters of the parametric press model.
#[derive(Clone, Debug)]
pub struct GenericCmykParams {
    pub name: &'static str,
    pub paper: Lab,
    /// Solid overprints, Demichel order: C, M, CM, Y, CY, MY, CMY.
    pub solids: [Lab; 7],
    pub black: Lab,
    /// Dot gain at 50% per ink (C, M, Y, K).
    pub dot_gain: [f32; 4],
    /// Total area coverage limit (sum of inks, 1.0 = 100%).
    pub tac: f32,
    pub bg: BlackGeneration,
}

/// Rounded, typical SWOP-like values (not measured data).
pub fn swop_like() -> GenericCmykParams {
    GenericCmykParams {
        name: super::GENERIC_CMYK,
        paper: Lab::new(93.0, -0.5, 2.5),
        solids: [
            Lab::new(55.0, -37.0, -50.0), // C
            Lab::new(48.0, 74.0, -3.0),   // M
            Lab::new(24.0, 22.0, -46.0),  // CM (blue)
            Lab::new(89.0, -5.0, 93.0),   // Y
            Lab::new(50.0, -65.0, 27.0),  // CY (green)
            Lab::new(47.0, 68.0, 48.0),   // MY (red)
            Lab::new(24.0, 1.0, 0.0),     // CMY
        ],
        black: Lab::new(19.0, 0.5, 0.5),
        dot_gain: [0.20, 0.20, 0.20, 0.22],
        tac: 3.0,
        bg: BlackGeneration { start: 0.1, max_k: 0.95, gamma: 1.25 },
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
    /// XYZ of the 8 CMY primaries (index = c | m<<1 | y<<2), without and with K.
    prim: [[f32; 3]; 8],
    prim_k: [[f32; 3]; 8],
    paper_xyz: [f32; 3],
    /// Media-relative XYZ of the darkest printable black (for black-point compensation).
    black_rel: [f32; 3],
    gbd: OnceLock<Gbd>,
}

impl GenericCmyk {
    pub fn new(params: GenericCmykParams) -> Self {
        let paper_xyz = lab_to_xyz(params.paper);
        let mut prim = [[0.0f32; 3]; 8];
        prim[0] = paper_xyz;
        for (p, lab) in prim[1..].iter_mut().zip(params.solids.iter()) {
            *p = lab_to_xyz(*lab);
        }
        let k_xyz = lab_to_xyz(params.black);
        // Black over other inks: optical densities add, but saturate towards a maximum density
        // (ink trapping / surface reflection), normalised so that K on paper is exact.
        const D_MAX: f32 = 2.0;
        let sat = |d: f32| D_MAX * (1.0 - (-d / D_MAX).exp());
        let mut prim_k = [[0.0f32; 3]; 8];
        for i in 0..8 {
            for ch in 0..3 {
                let di = -(prim[i][ch] / paper_xyz[ch]).max(1e-6).log10();
                let dk = -(k_xyz[ch] / paper_xyz[ch]).max(1e-6).log10();
                let d = sat(di + dk) * dk / sat(dk).max(1e-6);
                prim_k[i][ch] = paper_xyz[ch] * 10f32.powf(-d.max(di));
            }
        }
        let mut g = Self { params, prim, prim_k, paper_xyz, black_rel: [0.0; 3], gbd: OnceLock::new() };
        // Darkest black within TAC: search a few rich blacks.
        let mut best = [1.0f32; 3];
        for k in [0.8f32, 0.9, 1.0] {
            let rest = (g.params.tac - k).max(0.0) / 3.0;
            let cmy = rest.min(1.0);
            let x = g.to_xyz_rel([cmy, cmy, cmy, k]);
            if x[1] < best[1] {
                best = x;
            }
        }
        g.black_rel = best;
        g
    }

    pub fn name(&self) -> &str {
        self.params.name
    }

    fn gain(a: f32, g: f32) -> f32 {
        let a = a.clamp(0.0, 1.0);
        (a + 4.0 * g * a * (1.0 - a)).clamp(0.0, 1.0)
    }

    /// CMYK (0..1) → absolute XYZ (D50).
    pub fn to_xyz(&self, cmyk: [f32; 4]) -> [f32; 3] {
        let dg = self.params.dot_gain;
        let c = Self::gain(cmyk[0], dg[0]);
        let m = Self::gain(cmyk[1], dg[1]);
        let y = Self::gain(cmyk[2], dg[2]);
        let k = Self::gain(cmyk[3], dg[3]);
        let mut out = [0.0f32; 3];
        for i in 0..8 {
            let w = (if i & 1 != 0 { c } else { 1.0 - c }) * (if i & 2 != 0 { m } else { 1.0 - m }) * (if i & 4 != 0 { y } else { 1.0 - y });
            if w == 0.0 {
                continue;
            }
            for (ch, o) in out.iter_mut().enumerate() {
                *o += w * ((1.0 - k) * self.prim[i][ch] + k * self.prim_k[i][ch]);
            }
        }
        out
    }

    /// Media-relative XYZ (paper = D50 white).
    pub fn to_xyz_rel(&self, cmyk: [f32; 4]) -> [f32; 3] {
        let x = self.to_xyz(cmyk);
        [x[0] * D50[0] / self.paper_xyz[0], x[1] * D50[1] / self.paper_xyz[1], x[2] * D50[2] / self.paper_xyz[2]]
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
    /// darkest rich black shows as black again.
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

    /// Lab of the darkest printable black (media-relative).
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

    fn black_for_grey(&self, grey: f32) -> f32 {
        let bg = self.params.bg;
        if grey <= bg.start {
            return 0.0;
        }
        (((grey - bg.start) / (1.0 - bg.start)).clamp(0.0, 1.0).powf(bg.gamma) * bg.max_k).clamp(0.0, 1.0)
    }

    /// Separate a media-relative Lab (already gamut-mapped) into CMYK with GCR/UCR and the TAC limit.
    pub fn separate(&self, target: Lab) -> [f32; 4] {
        if target.l >= 99.99 && target.a.abs() < 0.01 && target.b.abs() < 0.01 {
            return [0.0; 4];
        }
        let cmy0 = self.solve_cmy(target, 0.0, Self::initial_cmy(target, 0.0));
        let grey = cmy0[0].min(cmy0[1]).min(cmy0[2]);
        let mut k = self.black_for_grey(grey);
        // If CMY alone can't get dark enough, black must carry the rest.
        let l0 = self.to_lab_rel([cmy0[0], cmy0[1], cmy0[2], 0.0]).l;
        if l0 > target.l + 1.0 {
            k = k.max(((l0 - target.l) / l0.max(1.0)).clamp(0.0, 1.0));
        }
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
