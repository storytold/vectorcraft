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
    // A Limited palette of 9 to 16 colors keeps a nearly flat dark ground in one color (see [`kmeans_rgb`]).
    let limited = || kmeans_rgb(used, k, if FEW_COLORS.contains(&k) { LEAST_APART } else { 0.0 });
    match params.palette {
        Palette::Limited => limited(),
        Palette::FullTone => full_tone(),
        Palette::Automatic => match flat_colors(used, detail) {
            Some(n) => kmeans_rgb(used, n, 0.0),
            None => full_tone(),
        },
        Palette::DocumentLibrary if !params.swatches.is_empty() => library_colors(used, &params.swatches, k),
        // No library colours to use: as Limited.
        Palette::DocumentLibrary => limited(),
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
    // Each colour once, in their order: looked up in a set, not in the colours kept so far.
    let mut seen = std::collections::HashSet::new();
    let all: Vec<[f64; 3]> = swatches.iter().filter(|s| seen.insert(**s)).map(|s| s.map(f64::from)).collect();
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

/// How far apart (Oklab × 100) the two sides of a cut must be to look different: below about 3
/// they look alike (see [`oklab`]).
const LEAST_APART: f64 = 3.0;

/// The color counts of a Limited palette whose cuts are weighed by [`LEAST_APART`].
const FEW_COLORS: std::ops::RangeInclusive<usize> = 9..=16;

/// The darkest colors (Oklab lightness × 100 below this) are the ones whose cuts are weighed.
const DARK: f64 = 30.0;

/// The grayest colors (Oklab chroma × 100 below this) are the ones whose cuts are weighed.
const GRAY: f64 = 3.0;

/// Median cut to `k` colors, then weighted k-means refinement over histogram bins. With `apart`
/// above 0, a cut whose two sides are dark grays less than `apart` apart has its gain scaled down
/// (see [`median_cut_apart`]). A large, nearly flat dark ground then keeps one color, and the
/// color it would have taken goes to another cut.
fn kmeans_rgb(used: &[Used], k: usize, apart: f64) -> Vec<[f64; 3]> {
    refine(used, centers(&median_cut_apart(used, k, 0.0, apart)))
}

/// Divisive clustering in the manner of median cut, but each box is cut where that leaves its two
/// sides most alike within themselves (the cut, along one channel, with the largest between-side
/// variance: Otsu's criterion), and the box cut next is the one whose cut gains most. A cut at the
/// median, as in classic median cut, splits the largest colour of flat art in two along its noise
/// and leaves two smaller colours in one box. Boxes are cut while there are fewer than `k` and
/// some box spans more than `spread` levels of a channel.
pub(super) fn median_cut(used: &[Used], k: usize, spread: f64) -> Vec<ColorBox> {
    median_cut_apart(used, k, spread, 0.0)
}

/// [`median_cut`], except when both sides of a cut have a dark gray mean color (see [`DARK`] and
/// [`GRAY`]) and the two means lie `d` (Oklab × 100) apart with `d` < `apart`: then the cut's gain
/// is scaled by (`d` / `apart`)². With `apart` 0 it is [`median_cut`].
fn median_cut_apart(used: &[Used], k: usize, spread: f64, apart: f64) -> Vec<ColorBox> {
    if used.is_empty() {
        return vec![ColorBox { mean: [255.0; 3], weight: 0.0, peak: 0.0 }];
    }
    /// A box's bins and its best cut: (gain, bins sorted along the cut's channel, bins on the first side).
    struct Cut {
        bins: Vec<usize>,
        best: Option<(f64, Vec<usize>, usize)>,
    }
    // How much of a cut's gain counts when its sides' mean colors are `a` and `b`: all of it when
    // they look different or either is not a dark gray, less as two dark grays look more alike.
    let dark_gray = |c: [f64; 3]| c[0] < DARK && c[1].hypot(c[2]) < GRAY;
    let seen = |a: [f64; 3], b: [f64; 3]| {
        let (a, b) = (oklab(a), oklab(b));
        if !dark_gray(a) || !dark_gray(b) {
            return 1.0;
        }
        ((0..3).map(|q| (a[q] - b[q]).powi(2)).sum::<f64>() / (apart * apart)).min(1.0)
    };
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
                // The sums of each side's colors are kept only when cuts are weighed by `apart`.
                let weigh = apart > 0.0;
                let all = if weigh { sorted.iter().fold([0.0; 3], |a, &i| [0, 1, 2].map(|q| a[q] + used[i].1[q] * used[i].2)) } else { [0.0; 3] };
                let (mut w, mut s, mut side) = (0.0, 0.0, [0.0; 3]);
                let mut cut: Option<(f64, usize)> = None;
                for (j, &i) in sorted.iter().enumerate().take(sorted.len() - 1) {
                    w += used[i].2;
                    s += used[i].1[ch] * used[i].2;
                    if weigh {
                        for (acc, v) in side.iter_mut().zip(used[i].1) {
                            *acc += v * used[i].2;
                        }
                    }
                    let rest = total - w;
                    if w > 0.0 && rest > 0.0 {
                        let mut gain = w * rest / total * (s / w - (sum - s) / rest).powi(2);
                        // A scaled gain is never more than the full one, so only a cut that gains more
                        // than the best so far, on this channel and the ones before it, is scaled.
                        if weigh && gain > cut.map_or(f64::NEG_INFINITY, |c| c.0).max(best.as_ref().map_or(f64::NEG_INFINITY, |b| b.0)) {
                            gain *= seen(side.map(|v| v / w), [0, 1, 2].map(|q| (all[q] - side[q]) / rest));
                        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A large ground of three near-black grays (too close to tell apart) and a ramp of 120 colors.
    fn ground_and_ramp() -> Vec<Used> {
        let ground = [5.0, 7.0, 9.0].map(|g| [g; 3]).into_iter().map(|c| (c, 14_400.0));
        let ramp = (0..120).map(|i| f64::from(i) * 255.0 / 119.0).map(|t| ([t, 255.0 - t, t / 2.0], 120.0));
        ground.chain(ramp).enumerate().map(|(i, (c, n))| (i, c, n)).collect()
    }

    /// The same ground, six far-apart colors and a small pair of colors 60 levels apart: eight
    /// boxes, so the ninth color goes to the ground or to the pair.
    fn ground_and_pair() -> Vec<Used> {
        let ground = [5.0, 7.0, 9.0].map(|g| ([g; 3], 14_400.0));
        let far = [[255.0, 0.0, 0.0], [0.0, 255.0, 0.0], [0.0, 0.0, 255.0], [255.0, 255.0, 0.0], [0.0, 255.0, 255.0], [255.0; 3]].map(|c| (c, 120.0));
        let pair = [([128.0, 64.0, 0.0], 30.0), ([128.0, 64.0, 60.0], 30.0)];
        ground.into_iter().chain(far).chain(pair).enumerate().map(|(i, (c, n))| (i, c, n)).collect()
    }

    #[test]
    fn only_a_limited_palette_of_nine_to_sixteen_colors_weighs_alike_sides() {
        let used = ground_and_ramp();
        let plain = |k: usize| refine(&used, centers(&median_cut(&used, k, 0.0)));
        let palette = |palette: Palette, colors: u32| color_palette(&used, &TraceParams { palette, colors, ..TraceParams::default() });
        // At 9 to 16 colors the ground keeps one color, so the palette differs from a plain median cut.
        assert_ne!(palette(Palette::Limited, 16), plain(16));
        assert_ne!(palette(Palette::DocumentLibrary, 16), plain(16));
        // At 9 colors, the pair takes the ninth color instead of the ground.
        let pair = ground_and_pair();
        let plain_pair = refine(&pair, centers(&median_cut(&pair, 9, 0.0)));
        for limited in [Palette::Limited, Palette::DocumentLibrary] {
            let ninth = color_palette(&pair, &TraceParams { palette: limited, colors: 9, ..TraceParams::default() });
            assert_ne!(ninth, plain_pair, "{limited:?}");
        }
        // Below 9 and above 16 colors, and for Full Tone and Automatic, the palette is a plain median cut's.
        for colors in [2, 3, 6, 8, 17, 24, 32, 64, 256] {
            assert_eq!(palette(Palette::Limited, colors), plain(colors as usize), "{colors} colors");
            assert_eq!(palette(Palette::DocumentLibrary, colors), plain(colors as usize), "{colors} colors");
        }
        let detail = (TraceParams::default().color_detail / 100.0).clamp(0.0, 1.0);
        assert_eq!(palette(Palette::FullTone, 16), refine(&used, centers(&median_cut(&used, FULL_TONE_MAX, 96.0 - 88.0 * detail))));
        let automatic = match flat_colors(&used, detail) {
            Some(n) => plain(n),
            None => refine(&used, centers(&median_cut(&used, FULL_TONE_MAX, 96.0 - 88.0 * detail))),
        };
        assert_eq!(palette(Palette::Automatic, 16), automatic);
    }

    #[test]
    fn a_dark_colored_figure_on_a_nearly_flat_dark_ground_keeps_its_color() {
        // Dark navy text (a few pixels) on the near-black ground: only cuts between two dark grays
        // are weighed, so the navy still gets a color of its own.
        let navy = [10.0, 20.0, 70.0];
        let mut used = ground_and_ramp();
        used.push((used.len(), navy, 400.0));
        let palette = color_palette(&used, &TraceParams { palette: Palette::Limited, colors: 16, ..TraceParams::default() });
        let near = |c: [f64; 3]| {
            let (a, b) = (oklab(c), oklab(navy));
            (0..3).map(|q| (a[q] - b[q]).powi(2)).sum::<f64>().sqrt()
        };
        let nearest = palette.iter().map(|&c| near(c)).fold(f64::INFINITY, f64::min);
        assert!(nearest < LEAST_APART, "nearest {nearest} in {palette:?}");
    }
}
