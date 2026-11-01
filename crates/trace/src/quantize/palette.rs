use super::{Palette, Raster, TraceParams, oklab};

#[derive(Clone, Copy, Default)]
pub(super) struct Bin {
    pub(super) n: u64,
    sum: [u64; 3],
}

impl Bin {
    pub(super) fn add(&mut self, p: &[u8; 4]) {
        self.n += 1;
        for (sum, v) in self.sum.iter_mut().zip(p) {
            *sum += u64::from(*v);
        }
    }
    pub(super) fn mean(&self) -> [f64; 3] {
        let n = self.n.max(1) as f64;
        [self.sum[0] as f64 / n, self.sum[1] as f64 / n, self.sum[2] as f64 / n]
    }
}

/// The colour histogram's bin index: 5 bits per channel over that channel's range in the image.
/// A channel spanning at most 32 values keeps every value apart; a full-range one keeps the
/// plain top 5 bits.
pub(super) struct HistKey {
    lo: [u8; 3],
    shift: [u8; 3],
}

impl HistKey {
    /// Fit the bins to the opaque pixels (alpha >= 128) of `img`.
    pub(super) fn new(img: &Raster) -> Self {
        let (mut lo, mut hi) = ([u8::MAX; 3], [0u8; 3]);
        for p in img.rgba.as_chunks::<4>().0 {
            if p[3] >= 128 {
                for ch in 0..3 {
                    lo[ch] = lo[ch].min(p[ch]);
                    hi[ch] = hi[ch].max(p[ch]);
                }
            }
        }
        let mut shift = [0u8; 3];
        for ch in 0..3 {
            if lo[ch] > hi[ch] {
                // No opaque pixel: nothing is looked up.
                lo[ch] = 0;
                continue;
            }
            let range = hi[ch] - lo[ch];
            while shift[ch] < 3 && (range >> shift[ch]) > 31 {
                shift[ch] += 1;
            }
        }
        Self { lo, shift }
    }

    pub(super) fn key(&self, r: u8, g: u8, b: u8) -> usize {
        let c = |v: u8, ch: usize| (v.saturating_sub(self.lo[ch]) >> self.shift[ch]).min(31) as usize;
        (c(r, 0) << 10) | (c(g, 1) << 5) | c(b, 2)
    }
}

pub(super) fn nearest_1d(levels: &[f64], v: f64) -> usize {
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
pub(super) fn kmeans_1d(hist: &[u64; 256], k: usize) -> Vec<f64> {
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

/// The index of the colour of `labs` (Oklab) nearest to `c` (Oklab): the one that looks most like it.
pub(super) fn nearest_lab(labs: &[[f64; 3]], c: [f64; 3]) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (i, m) in labs.iter().enumerate() {
        let d = (m[0] - c[0]).powi(2) + (m[1] - c[1]).powi(2) + (m[2] - c[2]).powi(2);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

/// How much more a difference in hue and chroma (Oklab a and b) counts than one in lightness when
/// a pixel takes a library color.
const LIBRARY_CHROMA_WEIGHT: f64 = 3.0;

/// The index of the library color of `labs` (Oklab) nearest to `c` (Oklab), with differences in
/// hue and chroma counted [`LIBRARY_CHROMA_WEIGHT`] times: a neutral or muted tone takes a neutral
/// library color of another lightness rather than a saturated one of its own lightness.
pub(super) fn nearest_library(labs: &[[f64; 3]], c: [f64; 3]) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (i, m) in labs.iter().enumerate() {
        let d = (m[0] - c[0]).powi(2) + (LIBRARY_CHROMA_WEIGHT * (m[1] - c[1])).powi(2) + (LIBRARY_CHROMA_WEIGHT * (m[2] - c[2])).powi(2);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

pub(super) fn labs(colors: &[[f64; 3]]) -> Vec<[f64; 3]> {
    colors.iter().map(|c| oklab(*c)).collect()
}

/// The most colours Full Tone (and Automatic on continuous-tone images) makes.
pub const FULL_TONE_MAX: usize = 256;

/// A histogram bin in use: its index, mean colour and pixel count.
pub(super) type Used = (usize, [f64; 3], f64);

/// A color that is no palette color, the index of the rare palette color it traces as, and the
/// index of the palette color on the other side of it (see [`rare_colours`]).
pub(super) type TracedAs = ([f64; 3], usize, usize);

/// Color mode's palette for the histogram bins `used`, as `params.palette` asks.
pub(super) fn color_palette(used: &[Used], params: &TraceParams) -> Vec<[f64; 3]> {
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
pub(super) fn flat_colors(used: &[Used], detail: f64) -> Option<usize> {
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

/// Document Library: the `k` swatches (at most) that the most pixels are nearest to (see
/// [`nearest_library`]).
fn library_colors(used: &[Used], swatches: &[[u8; 3]], k: usize) -> Vec<[f64; 3]> {
    let mut all: Vec<[f64; 3]> = vec![];
    for s in swatches {
        let c = s.map(f64::from);
        if !all.contains(&c) {
            all.push(c);
        }
    }
    let mut weight = vec![0.0; all.len()];
    let all_labs = labs(&all);
    for (_, c, n) in used {
        if let Some(w) = weight.get_mut(nearest_library(&all_labs, oklab(*c))) {
            *w += n;
        }
    }
    let mut order: Vec<usize> = (0..all.len()).collect();
    order.sort_by(|a, b| weight[*b].total_cmp(&weight[*a]).then(a.cmp(b)));
    order.into_iter().take(k.max(1)).filter_map(|i| all.get(i).copied()).collect()
}

/// A median-cut box: its weighted mean colour, pixel count and the pixel count of its heaviest bin.
pub(super) struct ColorBox {
    mean: [f64; 3],
    weight: f64,
    peak: f64,
}

pub(super) fn centers(boxes: &[ColorBox]) -> Vec<[f64; 3]> {
    boxes.iter().map(|b| b.mean).collect()
}

/// Median cut to `k` colours, then weighted k-means refinement over histogram bins.
fn kmeans_rgb(used: &[Used], k: usize) -> Vec<[f64; 3]> {
    refine(used, centers(&median_cut(used, k, 0.0)))
}

/// Divisive clustering in the manner of median cut, but each box is cut where that leaves its two
/// sides most alike within themselves (the cut, along one channel, with the largest between-side
/// variance: Otsu's criterion), and the box cut next is the one whose cut gains most. A cut at the
/// median, as in classic median cut, splits the largest colour of flat art in two along its noise
/// and leaves two smaller colours in one box. Boxes are cut while there are fewer than `k` and
/// some box spans more than `spread` levels of a channel.
pub(super) fn median_cut(used: &[Used], k: usize, spread: f64) -> Vec<ColorBox> {
    if used.is_empty() {
        return vec![ColorBox { mean: [255.0; 3], weight: 0.0, peak: 0.0 }];
    }
    /// A box's bins and its best cut: (gain, bins sorted along the cut's channel, bins on the first side).
    struct Cut {
        bins: Vec<usize>,
        best: Option<(f64, Vec<usize>, usize)>,
    }
    let plan = |bins: Vec<usize>| {
        let mut best: Option<(f64, Vec<usize>, usize)> = None;
        let widest = (0..3)
            .map(|ch| {
                let v = bins.iter().map(|&i| used[i].1[ch]);
                v.clone().fold(f64::NEG_INFINITY, f64::max) - v.fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max);
        if bins.len() >= 2 && widest > spread.max(0.0) {
            for ch in 0..3 {
                let mut sorted = bins.clone();
                sorted.sort_by(|x, y| used[*x].1[ch].total_cmp(&used[*y].1[ch]));
                let (total, sum): (f64, f64) = sorted.iter().fold((0.0, 0.0), |a, &i| (a.0 + used[i].2, a.1 + used[i].1[ch] * used[i].2));
                let (mut w, mut s) = (0.0, 0.0);
                let mut cut: Option<(f64, usize)> = None;
                for (j, &i) in sorted.iter().enumerate().take(sorted.len() - 1) {
                    w += used[i].2;
                    s += used[i].1[ch] * used[i].2;
                    let rest = total - w;
                    if w > 0.0 && rest > 0.0 {
                        let gain = w * rest / total * (s / w - (sum - s) / rest).powi(2);
                        if cut.is_none_or(|c| gain > c.0) {
                            cut = Some((gain, j + 1));
                        }
                    }
                }
                if let Some((gain, at)) = cut
                    && best.as_ref().is_none_or(|b| gain > b.0)
                {
                    best = Some((gain, sorted, at));
                }
            }
        }
        Cut { bins, best }
    };
    let mut boxes = vec![plan((0..used.len()).collect())];
    while boxes.len() < k {
        let gain = |c: &Cut| c.best.as_ref().map_or(f64::NEG_INFINITY, |b| b.0);
        let Some(bi) = (0..boxes.len()).filter(|&i| boxes[i].best.is_some()).max_by(|&a, &b| gain(&boxes[a]).total_cmp(&gain(&boxes[b]))) else {
            break;
        };
        let Some((_, mut sorted, at)) = boxes.swap_remove(bi).best else { break };
        let tail = sorted.split_off(at);
        boxes.push(plan(sorted));
        boxes.push(plan(tail));
    }
    boxes
        .iter()
        .map(|b| {
            let mut s = [0.0; 3];
            let (mut w, mut peak) = (0.0, 0.0f64);
            for &i in &b.bins {
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

/// Lloyd refinement of `centers` over the bins `used`: each bin goes to the centre that looks most
/// like it (nearest in Oklab), and each centre becomes the mean colour of its bins.
fn refine(used: &[Used], mut centers: Vec<[f64; 3]>) -> Vec<[f64; 3]> {
    let bins = labs(&used.iter().map(|u| u.1).collect::<Vec<_>>());
    for _ in 0..12 {
        let center_labs = labs(&centers);
        let mut sum = vec![[0.0; 3]; centers.len()];
        let mut cnt = vec![0.0; centers.len()];
        for ((_, c, n), lab) in used.iter().zip(&bins) {
            let j = nearest_lab(&center_labs, *lab);
            for ch in 0..3 {
                sum[j][ch] += c[ch] * n;
            }
            cnt[j] += n;
        }
        let mut moved = 0.0f64;
        for j in 0..centers.len() {
            if cnt[j] > 0.0 {
                let m = [sum[j][0] / cnt[j], sum[j][1] / cnt[j], sum[j][2] / cnt[j]];
                moved = moved.max((0..3).map(|ch| (m[ch] - centers[j][ch]).powi(2)).sum());
                centers[j] = m;
            }
        }
        if moved < 0.25 {
            break;
        }
    }
    centers
}
