//! Colour reduction and noise removal.

use crate::{Mode, Palette, Raster, TraceParams};

/// Label of a transparent (untraced) pixel.
pub const TRANSPARENT: u16 = u16::MAX;

/// A palette-indexed image.
#[derive(Clone, Debug, PartialEq)]
pub struct Quantized {
    /// One palette index per pixel (row-major), or [`TRANSPARENT`].
    pub labels: Vec<u16>,
    pub palette: Vec<[u8; 3]>,
}

fn luma(p: [u8; 4]) -> u8 {
    ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114 + 500) / 1000) as u8
}

/// Reduce `img` to a palette according to `params.mode`.
pub fn quantize(img: &Raster, params: &TraceParams) -> Quantized {
    let px = img.rgba.as_chunks::<4>().0.iter().map(|c| [c[0], c[1], c[2], c[3]]);
    match params.mode {
        Mode::BlackAndWhite => {
            let t = params.threshold;
            let labels = px
                .map(|p| {
                    if p[3] < 128 {
                        TRANSPARENT
                    } else if luma(p) < t {
                        1
                    } else {
                        0
                    }
                })
                .collect();
            Quantized { labels, palette: vec![[255, 255, 255], [0, 0, 0]] }
        }
        Mode::Grayscale => {
            let mut hist = [0u64; 256];
            for p in img.rgba.as_chunks::<4>().0 {
                if p[3] >= 128 {
                    hist[luma([p[0], p[1], p[2], p[3]]) as usize] += 1;
                }
            }
            let levels = kmeans_1d(&hist, params.colors.clamp(2, 256) as usize);
            let mut lut = [0u16; 256];
            for (v, slot) in lut.iter_mut().enumerate() {
                *slot = nearest_1d(&levels, v as f64) as u16;
            }
            let labels = px.map(|p| if p[3] < 128 { TRANSPARENT } else { lut[luma(p) as usize] }).collect();
            let palette = levels.iter().map(|&l| {
                let g = l.round().clamp(0.0, 255.0) as u8;
                [g, g, g]
            });
            compact(labels, palette.collect())
        }
        // Logo mode traces without quantising; asked directly, it quantises like Color.
        Mode::Color | Mode::Logo => {
            // 15-bit colour histogram with per-bin sums (exact bin means).
            let mut bins: Vec<Bin> = vec![Bin::default(); 1 << 15];
            for p in img.rgba.as_chunks::<4>().0 {
                if p[3] >= 128 {
                    let b = &mut bins[key(p[0], p[1], p[2])];
                    b.n += 1;
                    b.sum[0] += p[0] as u64;
                    b.sum[1] += p[1] as u64;
                    b.sum[2] += p[2] as u64;
                }
            }
            let used: Vec<Used> = bins.iter().enumerate().filter(|(_, b)| b.n > 0).map(|(i, b)| (i, b.mean(), b.n as f64)).collect();
            let centers = color_palette(&used, params);
            let mut lut = vec![0u16; 1 << 15];
            for (i, c, _) in &used {
                lut[*i] = nearest_rgb(&centers, *c) as u16;
            }
            let labels = px.map(|p| if p[3] < 128 { TRANSPARENT } else { lut[key(p[0], p[1], p[2])] }).collect();
            let palette = centers.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]).collect();
            compact(labels, palette)
        }
    }
}

/// Drop unused palette entries and merge duplicates.
fn compact(mut labels: Vec<u16>, palette: Vec<[u8; 3]>) -> Quantized {
    let mut used = vec![false; palette.len()];
    for &l in &labels {
        if l != TRANSPARENT {
            used[l as usize] = true;
        }
    }
    let mut remap = vec![0u16; palette.len()];
    let mut out: Vec<[u8; 3]> = vec![];
    for (i, c) in palette.iter().enumerate() {
        if !used[i] {
            continue;
        }
        remap[i] = match out.iter().position(|o| o == c) {
            Some(j) => j as u16,
            None => {
                out.push(*c);
                (out.len() - 1) as u16
            }
        };
    }
    for l in &mut labels {
        if *l != TRANSPARENT {
            *l = remap[*l as usize];
        }
    }
    Quantized { labels, palette: out }
}

#[derive(Clone, Copy, Default)]
struct Bin {
    n: u64,
    sum: [u64; 3],
}

impl Bin {
    fn mean(&self) -> [f64; 3] {
        let n = self.n.max(1) as f64;
        [self.sum[0] as f64 / n, self.sum[1] as f64 / n, self.sum[2] as f64 / n]
    }
}

fn key(r: u8, g: u8, b: u8) -> usize {
    ((r as usize >> 3) << 10) | ((g as usize >> 3) << 5) | (b as usize >> 3)
}

fn nearest_1d(levels: &[f64], v: f64) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (i, &l) in levels.iter().enumerate() {
        let d = (l - v).abs();
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// 1-D k-means over a 256-bin histogram (Lloyd iterations, evenly spaced start).
fn kmeans_1d(hist: &[u64; 256], k: usize) -> Vec<f64> {
    let lo = hist.iter().position(|&n| n > 0).unwrap_or(0) as f64;
    let hi = hist.iter().rposition(|&n| n > 0).unwrap_or(255) as f64;
    let mut levels: Vec<f64> = (0..k).map(|i| if k == 1 { lo } else { lo + (hi - lo) * i as f64 / (k - 1) as f64 }).collect();
    for _ in 0..20 {
        let mut sum = vec![0.0; k];
        let mut cnt = vec![0.0; k];
        for (v, &n) in hist.iter().enumerate() {
            if n > 0 {
                let j = nearest_1d(&levels, v as f64);
                sum[j] += v as f64 * n as f64;
                cnt[j] += n as f64;
            }
        }
        let mut moved = false;
        for j in 0..k {
            if cnt[j] > 0.0 {
                let m = sum[j] / cnt[j];
                moved |= (m - levels[j]).abs() > 0.01;
                levels[j] = m;
            }
        }
        if !moved {
            break;
        }
    }
    levels
}

fn dist2(a: [f64; 3], b: [f64; 3]) -> f64 {
    // Slightly perceptual weights.
    let (dr, dg, db) = (a[0] - b[0], a[1] - b[1], a[2] - b[2]);
    2.0 * dr * dr + 4.0 * dg * dg + 3.0 * db * db
}

fn nearest_rgb(centers: &[[f64; 3]], c: [f64; 3]) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (i, &m) in centers.iter().enumerate() {
        let d = dist2(m, c);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// The most colours Full Tone (and Automatic on continuous-tone images) makes.
pub const FULL_TONE_MAX: usize = 256;

/// A histogram bin in use: its index, mean colour and pixel count.
type Used = (usize, [f64; 3], f64);

/// Color mode's palette for the histogram bins `used`, as `params.palette` asks.
fn color_palette(used: &[Used], params: &TraceParams) -> Vec<[f64; 3]> {
    let k = params.colors.clamp(2, 256) as usize;
    let detail = (params.color_detail / 100.0).clamp(0.0, 1.0);
    // Full Tone: split until no colour box is wider than 96 (few colours) … 8 (many) levels per
    // channel, which is one histogram bin.
    let full_tone = || refine(used, centers(&median_cut(used, FULL_TONE_MAX, 96.0 - 88.0 * detail)));
    match params.palette {
        Palette::Limited => kmeans_rgb(used, k),
        Palette::FullTone => full_tone(),
        Palette::Automatic => match flat_colors(used, detail) {
            Some(n) => kmeans_rgb(used, n),
            None => full_tone(),
        },
        Palette::DocumentLibrary if !params.swatches.is_empty() => library_colors(used, &params.swatches, k),
        // No library colours to use: as Limited.
        Palette::DocumentLibrary => kmeans_rgb(used, k),
    }
}

/// Automatic: how many colours flat art is made of, or `None` for continuous tone (photos,
/// gradients). Flat art has most of its pixels in a few tight colours (anti-aliased edges and
/// noise aside): the bins holding 90% of the pixels are few, and each colour's pixels sit mostly
/// in one bin. `detail` (0–1) lets smaller colour areas count as colours of their own.
fn flat_colors(used: &[Used], detail: f64) -> Option<usize> {
    let total: f64 = used.iter().map(|u| u.2).sum();
    if total <= 0.0 {
        return Some(2);
    }
    let mut by_count: Vec<&Used> = used.iter().collect();
    by_count.sort_by(|a, b| b.2.total_cmp(&a.2));
    let mut covered = 0.0;
    let main = by_count
        .iter()
        .take_while(|u| {
            let before = covered;
            covered += u.2;
            before < 0.9 * total
        })
        .count();
    if main > FLAT_BINS {
        return None;
    }
    let boxes = median_cut(used, 64, 24.0);
    let min_share = (0.04 - 0.035 * detail) * total;
    let colours: Vec<&ColorBox> = boxes.iter().filter(|b| b.weight >= min_share).collect();
    let weight: f64 = colours.iter().map(|b| b.weight).sum();
    let peaks: f64 = colours.iter().map(|b| b.peak).sum();
    (weight >= 0.9 * total && peaks >= 0.5 * weight && colours.len() <= 32).then_some(colours.len().max(2))
}

/// The most histogram bins flat art's main colours spread over (a few bins per colour).
const FLAT_BINS: usize = 96;

/// Document Library: the `k` swatches (at most) that the most pixels are nearest to.
fn library_colors(used: &[Used], swatches: &[[u8; 3]], k: usize) -> Vec<[f64; 3]> {
    let mut all: Vec<[f64; 3]> = vec![];
    for s in swatches {
        let c = s.map(f64::from);
        if !all.contains(&c) {
            all.push(c);
        }
    }
    let mut weight = vec![0.0; all.len()];
    for (_, c, n) in used {
        if let Some(w) = weight.get_mut(nearest_rgb(&all, *c)) {
            *w += n;
        }
    }
    let mut order: Vec<usize> = (0..all.len()).collect();
    order.sort_by(|a, b| weight[*b].total_cmp(&weight[*a]).then(a.cmp(b)));
    order.into_iter().take(k.max(1)).filter_map(|i| all.get(i).copied()).collect()
}

/// A median-cut box: its weighted mean colour, pixel count and the pixel count of its heaviest bin.
struct ColorBox {
    mean: [f64; 3],
    weight: f64,
    peak: f64,
}

fn centers(boxes: &[ColorBox]) -> Vec<[f64; 3]> {
    boxes.iter().map(|b| b.mean).collect()
}

/// Median cut to `k` colours, then weighted k-means refinement over histogram bins.
fn kmeans_rgb(used: &[Used], k: usize) -> Vec<[f64; 3]> {
    refine(used, centers(&median_cut(used, k, 0.0)))
}

/// Median cut: repeatedly split the box with the largest weighted extent, while there are fewer
/// than `k` boxes and some box spans more than `spread` levels of a channel.
fn median_cut(used: &[Used], k: usize, spread: f64) -> Vec<ColorBox> {
    if used.is_empty() {
        return vec![ColorBox { mean: [255.0; 3], weight: 0.0, peak: 0.0 }];
    }
    let mut boxes: Vec<Vec<usize>> = vec![(0..used.len()).collect()];
    while boxes.len() < k {
        let mut best: Option<(f64, usize, usize)> = None;
        for (bi, b) in boxes.iter().enumerate() {
            if b.len() < 2 {
                continue;
            }
            let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
            let mut wsum = 0.0;
            for &i in b {
                for ch in 0..3 {
                    lo[ch] = lo[ch].min(used[i].1[ch]);
                    hi[ch] = hi[ch].max(used[i].1[ch]);
                }
                wsum += used[i].2;
            }
            let (ch, range) = (0..3).map(|c| (c, hi[c] - lo[c])).fold((0, -1.0), |a, b| if b.1 > a.1 { b } else { a });
            if range <= spread.max(0.0) {
                continue;
            }
            let score = range * wsum.sqrt();
            if best.is_none_or(|x| score > x.0) {
                best = Some((score, bi, ch));
            }
        }
        let Some((_, bi, ch)) = best else { break };
        let mut b = boxes.swap_remove(bi);
        b.sort_by(|x, y| used[*x].1[ch].total_cmp(&used[*y].1[ch]));
        let total: f64 = b.iter().map(|&i| used[i].2).sum();
        let mut acc = 0.0;
        let mut cut = 1;
        for (j, &i) in b.iter().enumerate() {
            acc += used[i].2;
            if acc >= total / 2.0 {
                cut = (j + 1).clamp(1, b.len() - 1);
                break;
            }
        }
        let tail = b.split_off(cut);
        boxes.push(b);
        boxes.push(tail);
    }
    boxes
        .iter()
        .map(|b| {
            let mut s = [0.0; 3];
            let (mut w, mut peak) = (0.0, 0.0f64);
            for &i in b {
                for (acc, v) in s.iter_mut().zip(used[i].1) {
                    *acc += v * used[i].2;
                }
                w += used[i].2;
                peak = peak.max(used[i].2);
            }
            ColorBox { mean: [s[0] / w, s[1] / w, s[2] / w], weight: w, peak }
        })
        .collect()
}

/// Lloyd refinement of `centers` over the bins `used`.
fn refine(used: &[Used], mut centers: Vec<[f64; 3]>) -> Vec<[f64; 3]> {
    for _ in 0..12 {
        let mut sum = vec![[0.0; 3]; centers.len()];
        let mut cnt = vec![0.0; centers.len()];
        for (_, c, n) in used {
            let j = nearest_rgb(&centers, *c);
            for ch in 0..3 {
                sum[j][ch] += c[ch] * n;
            }
            cnt[j] += n;
        }
        let mut moved = 0.0f64;
        for j in 0..centers.len() {
            if cnt[j] > 0.0 {
                let m = [sum[j][0] / cnt[j], sum[j][1] / cnt[j], sum[j][2] / cnt[j]];
                moved = moved.max(dist2(m, centers[j]));
                centers[j] = m;
            }
        }
        if moved < 0.25 {
            break;
        }
    }
    centers
}

/// Merge 4-connected same-label components smaller than `min_area` pixels into their most common
/// neighbouring label. Transparent pixels are never relabelled.
///
/// A pass can merge a speck into a small region that it merges away later, leaving the speck behind
/// as a new island of that region's old label (#819: more noise removal gave more paths). Passes
/// repeat until one changes nothing: each merge joins two components, so they always end.
pub fn denoise(labels: &mut [u16], w: usize, h: usize, min_area: usize) {
    if min_area <= 1 || w == 0 || h == 0 || labels.len() < w.saturating_mul(h) {
        return;
    }
    // A backstop only: real images settle in a few passes.
    const MAX_PASSES: usize = 64;
    for _ in 0..MAX_PASSES {
        if !denoise_pass(labels, w, h, min_area) {
            break;
        }
    }
}

/// One [`denoise`] pass over every component; true when it relabelled any.
fn denoise_pass(labels: &mut [u16], w: usize, h: usize, min_area: usize) -> bool {
    let mut changed = false;
    let mut seen = vec![false; w * h];
    let mut stack: Vec<usize> = Vec::new();
    let mut comp: Vec<usize> = Vec::new();
    let mut neigh: Vec<(u16, u32)> = Vec::new();
    for start in 0..w * h {
        if seen[start] || labels[start] == TRANSPARENT {
            continue;
        }
        let l = labels[start];
        comp.clear();
        neigh.clear();
        stack.push(start);
        seen[start] = true;
        let mut small = true;
        while let Some(i) = stack.pop() {
            if small {
                comp.push(i);
                if comp.len() >= min_area {
                    small = false;
                }
            }
            let (x, y) = (i % w, i / w);
            let mut visit = |j: usize| {
                let lj = labels[j];
                if lj == l {
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                } else if lj != TRANSPARENT {
                    match neigh.iter_mut().find(|(n, _)| *n == lj) {
                        Some(e) => e.1 += 1,
                        None => neigh.push((lj, 1)),
                    }
                }
            };
            if x > 0 {
                visit(i - 1);
            }
            if x + 1 < w {
                visit(i + 1);
            }
            if y > 0 {
                visit(i - w);
            }
            if y + 1 < h {
                visit(i + w);
            }
        }
        if small && let Some(&(to, _)) = neigh.iter().max_by_key(|(n, c)| (*c, u16::MAX - *n)) {
            for &i in &comp {
                labels[i] = to;
            }
            changed = true;
        }
    }
    changed
}
