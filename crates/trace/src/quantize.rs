//! Colour reduction and noise removal. Colours are compared in Oklab, where equal distances look
//! equally different.

use crate::{Mode, Palette, Raster, TraceParams};

mod cut_edge;
mod denoise;
mod palette;
mod rare;

use cut_edge::{choosing_pixels, compact, settled};
pub(crate) use cut_edge::{continuous_tone, settle_cut_edges};
pub use denoise::denoise;
pub(crate) use denoise::denoise_keeping;
pub use palette::FULL_TONE_MAX;
use palette::{Bin, HistKey, TracedAs, Used, color_palette, flat_colors, kmeans_1d, labs, nearest_1d, nearest_lab, nearest_library};
#[cfg(test)]
pub(crate) use rare::kept_rare;
use rare::{rare_colours, refine_around};

/// Label of a transparent (untraced) pixel.
pub const TRANSPARENT: u16 = u16::MAX;

/// A palette-indexed image.
#[derive(Clone, Debug, PartialEq)]
pub struct Quantized {
    /// One palette index per pixel (row-major), or [`TRANSPARENT`].
    pub labels: Vec<u16>,
    pub palette: Vec<[u8; 3]>,
}

/// A pixel's gray (Rec. 601 luma): what Grayscale traces it as.
pub(crate) fn luma(p: [u8; 4]) -> u8 {
    ((p[0] as u32 * 299 + p[1] as u32 * 587 + p[2] as u32 * 114 + 500) / 1000) as u8
}

/// Reduce `img` to a palette according to `params.mode`.
pub fn quantize(img: &Raster, params: &TraceParams) -> Quantized {
    quantize_keeping(img, params).0
}

/// [`quantize`], and the palette colours that are rare but kept on purpose (see [`rare_colours`]):
/// noise removal must not erase their regions.
pub(crate) fn quantize_keeping(img: &Raster, params: &TraceParams) -> (Quantized, Vec<[u8; 3]>) {
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
            (Quantized { labels, palette: vec![[255, 255, 255], [0, 0, 0]] }, vec![])
        }
        Mode::Grayscale => {
            // The pixels that choose the grays: the opaque ones, except the semi-transparent ones
            // along the cut edge (see `choosing_pixels`).
            let choose = choosing_pixels(img, true);
            let own = own_colours(img);
            let own_choosing = own_colours_among(img, &choose);
            let histogram = |only_own: bool| {
                let mut hist = [0u64; 256];
                for ((p, &f), &c) in img.rgba.as_chunks::<4>().0.iter().zip(&own_choosing).zip(&choose) {
                    if p[3] >= 128 && c && (f || !only_own) {
                        hist[luma([p[0], p[1], p[2], p[3]]) as usize] += 1;
                    }
                }
                hist
            };
            let (every, calm) = (histogram(false), histogram(true));
            let hist = if palette_from_own(calm.iter().sum::<u64>() as f64, every.iter().sum::<u64>() as f64) { calm } else { every };
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
            (settled(merge_mixed(compact(labels, palette.collect()), img.width as usize, img.height as usize, &[]), img, &own), vec![])
        }
        // Logo mode traces without quantising; asked directly, it quantises like Color.
        Mode::Color | Mode::Logo => {
            // 15-bit colour histogram with per-bin sums (exact bin means), fitted to the image's
            // range per channel so that close tones (a dark, low-contrast photo) stay apart.
            let hk = HistKey::new(img);
            let pixels = img.rgba.as_chunks::<4>().0;
            // The pixels that choose the colors: the opaque ones, except the semi-transparent ones
            // along the cut edge (see `choosing_pixels`).
            let choose = choosing_pixels(img, false);
            let own = own_colours(img);
            let own_choosing = own_colours_among(img, &choose);
            let (mut all, mut every, mut calm) = (vec![Bin::default(); 1 << 15], vec![Bin::default(); 1 << 15], vec![Bin::default(); 1 << 15]);
            for ((p, &f), &c) in pixels.iter().zip(&own_choosing).zip(&choose) {
                if p[3] >= 128 {
                    let k = hk.key(p[0], p[1], p[2]);
                    all[k].add(p);
                    if c {
                        every[k].add(p);
                        if f {
                            calm[k].add(p);
                        }
                    }
                }
            }
            let used =
                |bins: &[Bin]| -> Vec<Used> { bins.iter().enumerate().filter(|(_, b)| b.n > 0).map(|(i, b)| (i, b.mean(), b.n as f64)).collect() };
            let (all, every, calm) = (used(&all), used(&every), used(&calm));
            let count = |u: &[Used]| u.iter().map(|b| b.2).sum::<f64>();
            let from = if palette_from_own(count(&calm), count(&every)) { &calm } else { &every };
            // Automatic on flat art: colours too rare for its count, but distinct and drawn as
            // regions of their own, are added (see `rare_colours`).
            let detail = (params.color_detail / 100.0).clamp(0.0, 1.0);
            let flat_auto = params.palette == Palette::Automatic && flat_colors(from, detail).is_some();
            let mut rare: Vec<[u8; 3]> = vec![];
            // The picture with `colors` colours asked for; fewer come back when it has fewer, or
            // when noise made several of one (see `merge_mixed`).
            let mut reduce = |colors: u32| {
                let mut centers = color_palette(from, &TraceParams { colors, ..params.clone() });
                // The anti-aliasing beside a rare color along the cut edge, with the indexes of
                // that color and of the color on the other side of it (see `rare_colours`).
                let mut traced_as: Vec<TracedAs> = vec![];
                if flat_auto {
                    let (found, blends) = rare_colours(img, &own_choosing, &choose, &hk, &centers);
                    if !found.is_empty() {
                        let common = centers.len();
                        traced_as = blends;
                        centers.extend(found);
                        centers = refine_around(from, centers, common);
                        rare = centers.iter().skip(common).map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
                    }
                }
                let mut lut = vec![0u16; 1 << 15];
                let center_labs = labs(&centers.iter().copied().chain(traced_as.iter().map(|t| t.0)).collect::<Vec<_>>());
                let library = params.palette == Palette::DocumentLibrary && !params.swatches.is_empty();
                let nearest = if library { nearest_library } else { nearest_lab };
                // Every opaque pixel takes a color, the cut edge's too. A bin takes the color
                // nearest to the mean of the pixels in it that choose the colors, and a bin with
                // none of them the color nearest to the mean of all its pixels. A bin nearest to
                // the anti-aliasing beside a rare color takes that rare color when it is at least
                // halfway from the color on the other side to it, and that color otherwise.
                for (i, c, _) in all.iter().chain(&every) {
                    let n = nearest(&center_labs, oklab(*c));
                    lut[*i] = n.checked_sub(centers.len()).and_then(|t| traced_as.get(t)).map_or(n, |&(_, rare, other)| {
                        let (Some(r), Some(o)) = (centers.get(rare), centers.get(other)) else { return rare };
                        let d2: f64 = (0..3).map(|q| (r[q] - o[q]).powi(2)).sum();
                        let t = if d2 > 0.0 { (0..3).map(|q| (c[q] - o[q]) * (r[q] - o[q])).sum::<f64>() / d2 } else { 1.0 };
                        if t >= 0.5 { rare } else { other }
                    }) as u16;
                }
                let labels =
                    img.rgba.as_chunks::<4>().0.iter().map(|p| if p[3] < 128 { TRANSPARENT } else { lut[hk.key(p[0], p[1], p[2])] }).collect();
                let palette = centers.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]).collect();
                let q = compact(labels, palette);
                // A swatch library's colours are used as they are.
                if library { q } else { merge_mixed(q, img.width as usize, img.height as usize, &rare) }
            };
            let q = reduce(params.colors);
            let q = settled(q, img, &own);
            rare.retain(|c| q.palette.contains(c));
            (q, rare)
        }
    }
}

/// The opaque pixels whose colour is the picture's own, not a blend: a pixel is a blend when it
/// lies between the two pixels on opposite sides of it (left and right, above and below, or
/// across a diagonal) that differ by more than 20 levels, which is what anti-aliasing and blur
/// leave along an edge. A line one pixel wide is no blend (the pixels on its two sides are alike),
/// nor is noise.
fn own_colours(img: &Raster) -> Vec<bool> {
    let opaque: Vec<bool> = img.rgba.as_chunks::<4>().0.iter().map(|p| p[3] >= 128).collect();
    own_colours_among(img, &opaque)
}

/// [`own_colours`] among the pixels that `among` marks: the others are neither own colours nor
/// the neighbours a pixel is a blend of.
fn own_colours_among(img: &Raster, among: &[bool]) -> Vec<bool> {
    let (w, h) = (img.width as isize, img.height as isize);
    let px = img.rgba.as_chunks::<4>().0;
    let at = |x: isize, y: isize| {
        let i = (x >= 0 && y >= 0 && x < w && y < h).then_some((y * w + x) as usize)?;
        let p = px.get(i).filter(|_| among.get(i).copied().unwrap_or(false))?;
        Some([f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
    };
    let mut out = Vec::with_capacity(px.len());
    for y in 0..h {
        for x in 0..w {
            let Some(p) = at(x, y) else {
                out.push(false);
                continue;
            };
            let blend = [(1, 0), (0, 1), (1, 1), (1, -1)].into_iter().any(|(dx, dy)| {
                let (Some(a), Some(b)) = (at(x - dx, y - dy), at(x + dx, y + dy)) else { return false };
                let d = [0, 1, 2].map(|k| b[k] - a[k]);
                let len2: f64 = d.iter().map(|v| v * v).sum();
                if len2 <= 400.0 {
                    return false;
                }
                let t = (0..3).map(|k| (p[k] - a[k]) * d[k]).sum::<f64>() / len2;
                let off2: f64 = (0..3).map(|k| (p[k] - a[k] - t * d[k]).powi(2)).sum();
                t > 0.06 && t < 0.94 && off2 < len2 / 16.0
            });
            out.push(!blend);
        }
    }
    out
}

/// Whether the palette is made from the picture's own colours alone (`own` of `all` opaque
/// pixels), so that the blends along edges take none of its colours. A picture with next to none
/// is all soft edges: there every pixel counts.
fn palette_from_own(own: f64, all: f64) -> bool {
    own >= 0.05 * all && own >= 16.0
}

/// sRGB (0–255) to Oklab × 100 (Björn Ottosson, "A perceptual color space for image processing",
/// 2020): equal distances look equally different, and 2 or 3 is about the least one sees.
fn oklab(c: [f64; 3]) -> [f64; 3] {
    let lin = c.map(|v| {
        let v = (v / 255.0).clamp(0.0, 1.0);
        if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    });
    let l = (0.412_221_470_8 * lin[0] + 0.536_332_536_3 * lin[1] + 0.051_445_992_9 * lin[2]).cbrt();
    let m = (0.211_903_498_2 * lin[0] + 0.680_699_545_1 * lin[1] + 0.107_396_956_6 * lin[2]).cbrt();
    let s = (0.088_302_461_9 * lin[0] + 0.281_718_837_6 * lin[1] + 0.629_978_700_5 * lin[2]).cbrt();
    [
        100.0 * (0.210_454_255_3 * l + 0.793_617_785 * m - 0.004_072_046_8 * s),
        100.0 * (1.977_998_495_1 * l - 2.428_592_205 * m + 0.450_593_709_9 * s),
        100.0 * (0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766 * s),
    ]
}

/// [`oklab`] of an sRGB colour, for the tests.
#[cfg(test)]
pub(crate) fn tests_oklab(c: [u8; 3]) -> [f64; 3] {
    oklab(c.map(f64::from))
}

/// `q` with the colours that noise made of one colour merged, largest mix first.
///
/// Asked for more colours than a picture has, k-means splits a flat colour along its noise, and
/// the two halves would trace as specks of one all over the other. Such halves are close (under 30
/// levels apart: noise is about as strong in a dark colour as in a light one, so the distance is
/// taken in RGB) and thoroughly mixed: the pixels of the lesser one have, on average, more than
/// half a neighbour of the other each; or they are closer still (under 12 levels) and mixed in
/// patches (compression blocks). The bands of a gradient are close too, but each keeps to itself
/// and they stay apart. The colours in `keep` merge with none.
fn merge_mixed(q: Quantized, w: usize, h: usize, keep: &[[u8; 3]]) -> Quantized {
    let k = q.palette.len();
    // A rare colour kept on purpose (see `rare_colours`) is distinct by choice: it merges with none.
    let kept: Vec<bool> = q.palette.iter().map(|c| keep.contains(c)).collect();
    if k < 2 || q.labels.len() != w * h {
        return q;
    }
    // How many pixels each colour has, and how many neighbouring pairs each pair of colours.
    let mut count = vec![0.0f64; k];
    let mut touch = vec![0.0f64; k * k];
    let label = |i: usize| q.labels.get(i).map(|&l| l as usize).filter(|&l| l < k);
    for i in 0..w * h {
        let Some(a) = label(i) else { continue };
        count[a] += 1.0;
        for j in [(i % w + 1 < w).then_some(i + 1), (i + w < w * h).then_some(i + w)].into_iter().flatten() {
            if let Some(b) = label(j).filter(|&b| b != a) {
                touch[a * k + b] += 1.0;
                touch[b * k + a] += 1.0;
            }
        }
    }
    let mut color: Vec<[f64; 3]> = q.palette.iter().map(|c| c.map(f64::from)).collect();
    // Which colour each has merged into (itself when none).
    let mut into: Vec<usize> = (0..k).collect();
    loop {
        let mut best: Option<(f64, usize, usize)> = None;
        for a in 0..k {
            for b in a + 1..k {
                if into[a] != a || into[b] != b || count[a] <= 0.0 || count[b] <= 0.0 || kept[a] || kept[b] {
                    continue;
                }
                let mixed = touch[a * k + b] / count[a].min(count[b]);
                let apart = (0..3).map(|c| (color[a][c] - color[b][c]).powi(2)).sum::<f64>().sqrt();
                if (mixed > 0.5 && apart < 30.0 || mixed > 0.15 && apart < 12.0) && best.is_none_or(|x| mixed > x.0) {
                    best = Some((mixed, a, b));
                }
            }
        }
        let Some((_, a, b)) = best else { break };
        color[a] = [0, 1, 2].map(|c| (color[a][c] * count[a] + color[b][c] * count[b]) / (count[a] + count[b]));
        count[a] += count[b];
        count[b] = 0.0;
        into[b] = a;
        for c in 0..k {
            if c != a && c != b {
                touch[a * k + c] += touch[b * k + c];
                touch[c * k + a] = touch[a * k + c];
            }
        }
    }
    if into.iter().enumerate().all(|(i, &t)| i == t) {
        return q;
    }
    // A colour may have merged into one that merged again: follow each to where it ended up.
    for i in 0..k {
        let mut to = into[i];
        while into[to] != to {
            to = into[to];
        }
        into[i] = to;
    }
    let labels = q.labels.iter().map(|&l| into.get(l as usize).map_or(l, |&t| t as u16)).collect();
    compact(labels, color.iter().map(|c| [c[0].round() as u8, c[1].round() as u8, c[2].round() as u8]).collect())
}
