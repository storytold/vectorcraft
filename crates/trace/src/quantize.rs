//! Colour reduction and noise removal. Colours are compared in Oklab, where equal distances look
//! equally different.

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
            let own = own_colours(img);
            let histogram = |only_own: bool| {
                let mut hist = [0u64; 256];
                for (p, &f) in img.rgba.as_chunks::<4>().0.iter().zip(&own) {
                    if p[3] >= 128 && (f || !only_own) {
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
            let own = own_colours(img);
            let (mut every, mut calm) = (vec![Bin::default(); 1 << 15], vec![Bin::default(); 1 << 15]);
            for (p, &f) in img.rgba.as_chunks::<4>().0.iter().zip(&own) {
                if p[3] >= 128 {
                    let k = hk.key(p[0], p[1], p[2]);
                    every[k].add(p);
                    if f {
                        calm[k].add(p);
                    }
                }
            }
            let used =
                |bins: &[Bin]| -> Vec<Used> { bins.iter().enumerate().filter(|(_, b)| b.n > 0).map(|(i, b)| (i, b.mean(), b.n as f64)).collect() };
            let (every, calm) = (used(&every), used(&calm));
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
                if flat_auto {
                    let found = rare_colours(img, &own, &hk, &centers);
                    if !found.is_empty() {
                        let common = centers.len();
                        centers.extend(found);
                        centers = refine_around(from, centers, common);
                        rare = centers.iter().skip(common).map(|c| c.map(|v| v.round().clamp(0.0, 255.0) as u8)).collect();
                    }
                }
                let mut lut = vec![0u16; 1 << 15];
                let center_labs = labs(&centers);
                let library = params.palette == Palette::DocumentLibrary && !params.swatches.is_empty();
                let nearest = if library { nearest_library } else { nearest_lab };
                for (i, c, _) in &every {
                    lut[*i] = nearest(&center_labs, oklab(*c)) as u16;
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
    let (w, h) = (img.width as isize, img.height as isize);
    let px = img.rgba.as_chunks::<4>().0;
    let at = |x: isize, y: isize| {
        let p = (x >= 0 && y >= 0 && x < w && y < h).then(|| px.get((y * w + x) as usize)).flatten()?;
        (p[3] >= 128).then(|| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])])
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

/// `q` with its blend pixels settled, and the colours that leaves unused dropped.
///
/// A pixel on an edge is a blend of the colours on its two sides, and the palette colour nearest
/// to a blend can be a third colour altogether (halfway from navy to yellow lies an olive that is
/// nearer to a green elsewhere in the picture), which would trace as a sliver of that colour along
/// the edge. A blend pixel (see [`own_colours`]) whose colour is none of those of the picture's
/// own pixels within two pixels of it takes the side it is nearer to of the pair of those colours
/// it is best a blend of, when that explains it twice as well as its own colour does. With one
/// colour around it, it keeps its own.
fn settled(mut q: Quantized, img: &Raster, own: &[bool]) -> Quantized {
    let (w, h) = (img.width as usize, img.height as usize);
    if q.labels.len() != w * h || own.len() != w * h {
        return q;
    }
    let before = q.labels.clone();
    let color = |l: u16| q.palette.get(l as usize).map(|c| c.map(f64::from));
    let dist = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt();
    let mut changed = false;
    for i in 0..w * h {
        let mine_l = before[i];
        if own[i] || mine_l == TRANSPARENT {
            continue;
        }
        let (x, y) = (i % w, i / w);
        // The colours of the picture's own pixels within two pixels (seldom more than three).
        let mut near: Vec<u16> = Vec::new();
        for yy in y.saturating_sub(2)..=(y + 2).min(h - 1) {
            for xx in x.saturating_sub(2)..=(x + 2).min(w - 1) {
                let j = yy * w + xx;
                if own[j] && before[j] != TRANSPARENT && !near.contains(&before[j]) {
                    near.push(before[j]);
                }
            }
        }
        let (Some(p), Some(mine)) = (img.rgba.get(i * 4..i * 4 + 3).map(|p| [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])]), color(mine_l))
        else {
            continue;
        };
        if near.len() < 2 || near.contains(&mine_l) {
            continue;
        }
        let mut best: Option<(f64, u16)> = None;
        for (n, &a) in near.iter().enumerate() {
            for &b in &near[n + 1..] {
                let (Some(ca), Some(cb)) = (color(a), color(b)) else { continue };
                let d = [0, 1, 2].map(|k| cb[k] - ca[k]);
                let len2: f64 = d.iter().map(|v| v * v).sum();
                let t = if len2 > 0.0 { ((0..3).map(|k| (p[k] - ca[k]) * d[k]).sum::<f64>() / len2).clamp(0.0, 1.0) } else { 0.0 };
                let off = dist(p, [0, 1, 2].map(|k| ca[k] + t * d[k]));
                if best.is_none_or(|x| off < x.0) {
                    best = Some((off, if t < 0.5 { a } else { b }));
                }
            }
        }
        if let Some((off, to)) = best
            && off < 0.5 * dist(p, mine)
        {
            q.labels[i] = to;
            changed = true;
        }
    }
    if changed { compact(q.labels, q.palette) } else { q }
}

/// Whether `img` is continuous tone (a photo, a gradient) rather than flat art, however noisy
/// or unevenly lit (a scan): whether eight colours leave much of it unsaid. Flat art is a few
/// colours and what noise and lighting make of them, and eight colours come within a few levels
/// of every pixel; a photo's tones spread all through its range, and eight colours stay far from
/// most of them (more than 6 levels, and more than a tenth of how far a single colour stays).
pub(crate) fn continuous_tone(img: &Raster) -> bool {
    let hk = HistKey::new(img);
    let mut bins = vec![Bin::default(); 1 << 15];
    for p in img.rgba.as_chunks::<4>().0 {
        if p[3] >= 128 {
            bins[hk.key(p[0], p[1], p[2])].add(p);
        }
    }
    let used: Vec<Used> = bins.iter().enumerate().filter(|(_, b)| b.n > 0).map(|(i, b)| (i, b.mean(), b.n as f64)).collect();
    // The root-mean-square distance (levels) from the pixels to the nearest of `k` colours.
    let unsaid = |k: usize| {
        let colors = centers(&median_cut(&used, k, 0.0));
        let (mut sum, mut n) = (0.0, 0.0);
        for (_, c, weight) in &used {
            let d2 = colors.iter().map(|m| (0..3).map(|ch| (m[ch] - c[ch]).powi(2)).sum::<f64>()).fold(f64::INFINITY, f64::min);
            sum += d2 * weight;
            n += weight;
        }
        if n > 0.0 { (sum / n).sqrt() } else { 0.0 }
    };
    let eight = unsaid(8);
    eight > 6.0 && eight > 0.1 * unsaid(1)
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

/// `q` with the semi-transparent pixels along the cut edge of `img` given the color beside them,
/// and the colors that leaves unused dropped.
///
/// A traced pixel with an alpha of 128 to 254 and a pixel above, below, left or right of it with an
/// alpha under 128 is on the cut edge. When its color is the color of a fully opaque pixel within
/// two pixels of it, or a darker shade of that color (see [`edge_blend_of`]), it is that color
/// blended with the transparent background, and its own color can lie nearest to an unrelated
/// palette color, which would trace as a sliver of that color along the edge. Such a pixel takes
/// the color that most of those opaque pixels have, and of colors that tie, the one nearest to its
/// own color. Only the pixels it reaches through pixels of an alpha of 128 or more, going up, down,
/// left or right, count, so that art across a transparent gap is left out. It keeps its own color
/// when one of the fully opaque pixels it reaches has it, or when its color is a blend of none of
/// them, as with a thin semi-transparent outline of another color. Pixels outside the image do not
/// count as transparent. With `gray` (Grayscale), a pixel's color is its gray (see [`luma`]).
pub(crate) fn settle_cut_edges(q: &mut Quantized, img: &Raster, gray: bool) {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    if q.labels.len() != w * h || px.len() != w * h {
        return;
    }
    let alpha = |j: usize| px.get(j).map_or(0, |p| p[3]);
    let color = |p: &[u8; 4]| if gray { [luma(*p); 3] } else { [p[0], p[1], p[2]] };
    let mut changed = false;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for i in 0..w * h {
        let mine = q.labels[i];
        if mine == TRANSPARENT || !(128..=254).contains(&alpha(i)) {
            continue;
        }
        let (x, y) = (i % w, i / w);
        let cut =
            (x > 0 && alpha(i - 1) < 128) || (x + 1 < w && alpha(i + 1) < 128) || (y > 0 && alpha(i - w) < 128) || (y + 1 < h && alpha(i + w) < 128);
        if !cut {
            continue;
        }
        let Some(p) = px.get(i) else { continue };
        let own = oklab(color(p).map(f64::from));
        // The colors of the fully opaque pixels within two pixels that it reaches and is a blend
        // of, and how many have each.
        let (x0, y0, x1, y1) = (x.saturating_sub(2), y.saturating_sub(2), (x + 2).min(w - 1), (y + 2).min(h - 1));
        let mut reached = [[false; 5]; 5];
        let mut near: Vec<(u16, usize)> = Vec::new();
        let mut keep = false;
        stack.clear();
        stack.push((x, y));
        if let Some(r) = reached.get_mut(y - y0).and_then(|row| row.get_mut(x - x0)) {
            *r = true;
        }
        while let Some((cx, cy)) = stack.pop() {
            let j = cy * w + cx;
            let l = q.labels[j];
            if alpha(j) == 255 && l != TRANSPARENT {
                keep |= l == mine;
                if px.get(j).is_some_and(|b| edge_blend_of(own, color(b), p[3])) {
                    match near.iter_mut().find(|n| n.0 == l) {
                        Some(n) => n.1 += 1,
                        None => near.push((l, 1)),
                    }
                }
            }
            for (nx, ny) in [(cx.wrapping_sub(1), cy), (cx + 1, cy), (cx, cy.wrapping_sub(1)), (cx, cy + 1)] {
                if nx < x0 || nx > x1 || ny < y0 || ny > y1 || alpha(ny * w + nx) < 128 {
                    continue;
                }
                let Some(r) = reached.get_mut(ny - y0).and_then(|row| row.get_mut(nx - x0)) else { continue };
                if !*r {
                    *r = true;
                    stack.push((nx, ny));
                }
            }
        }
        if keep || near.is_empty() {
            continue;
        }
        let most = near.iter().map(|n| n.1).max().unwrap_or(0);
        let tied: Vec<(u16, [f64; 3])> =
            near.iter().filter(|n| n.1 == most).filter_map(|n| Some((n.0, oklab(q.palette.get(n.0 as usize)?.map(f64::from))))).collect();
        let tied_labs: Vec<[f64; 3]> = tied.iter().map(|t| t.1).collect();
        let Some(&(to, _)) = tied.get(nearest_lab(&tied_labs, own)) else { continue };
        q.labels[i] = to;
        changed = true;
    }
    if changed {
        *q = compact(std::mem::take(&mut q.labels), std::mem::take(&mut q.palette));
    }
}

/// How far (Oklab × 100) the color of a pixel on a cut edge may lie from the color beside it, or
/// from a darker shade of that color, and still count as a blend of it with the transparent
/// background.
const EDGE_BLEND: f64 = 8.0;

/// Whether `own` (Oklab), the color of a pixel with an alpha of `alpha`, is `beside` (sRGB) blended
/// with the transparent background: whether it lies within [`EDGE_BLEND`] of the line in Oklab from
/// `beside` to `beside` with each channel scaled by `alpha` / 510.
fn edge_blend_of(own: [f64; 3], beside: [u8; 3], alpha: u8) -> bool {
    let to = oklab(beside.map(f64::from));
    let k = f64::from(alpha) / 510.0;
    let from = oklab(beside.map(|v| f64::from(v) * k));
    let d = [to[0] - from[0], to[1] - from[1], to[2] - from[2]];
    let len2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
    let t = if len2 > 0.0 { ((0..3).map(|c| (own[c] - from[c]) * d[c]).sum::<f64>() / len2).clamp(0.0, 1.0) } else { 0.0 };
    let off2: f64 = (0..3).map(|c| (own[c] - from[c] - t * d[c]).powi(2)).sum();
    off2 <= EDGE_BLEND * EDGE_BLEND
}

#[derive(Clone, Copy, Default)]
struct Bin {
    n: u64,
    sum: [u64; 3],
}

impl Bin {
    fn add(&mut self, p: &[u8; 4]) {
        self.n += 1;
        for (sum, v) in self.sum.iter_mut().zip(p) {
            *sum += u64::from(*v);
        }
    }
    fn mean(&self) -> [f64; 3] {
        let n = self.n.max(1) as f64;
        [self.sum[0] as f64 / n, self.sum[1] as f64 / n, self.sum[2] as f64 / n]
    }
}

/// The colour histogram's bin index: 5 bits per channel over that channel's range in the image.
/// A channel spanning at most 32 values keeps every value apart; a full-range one keeps the
/// plain top 5 bits.
struct HistKey {
    lo: [u8; 3],
    shift: [u8; 3],
}

impl HistKey {
    /// Fit the bins to the opaque pixels (alpha >= 128) of `img`.
    fn new(img: &Raster) -> Self {
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

    fn key(&self, r: u8, g: u8, b: u8) -> usize {
        let c = |v: u8, ch: usize| (v.saturating_sub(self.lo[ch]) >> self.shift[ch]).min(31) as usize;
        (c(r, 0) << 10) | (c(g, 1) << 5) | c(b, 2)
    }
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

/// The index of the colour of `labs` (Oklab) nearest to `c` (Oklab): the one that looks most like it.
fn nearest_lab(labs: &[[f64; 3]], c: [f64; 3]) -> usize {
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
fn nearest_library(labs: &[[f64; 3]], c: [f64; 3]) -> usize {
    let mut best = (f64::INFINITY, 0);
    for (i, m) in labs.iter().enumerate() {
        let d = (m[0] - c[0]).powi(2) + (LIBRARY_CHROMA_WEIGHT * (m[1] - c[1])).powi(2) + (LIBRARY_CHROMA_WEIGHT * (m[2] - c[2])).powi(2);
        if d < best.0 {
            best = (d, i);
        }
    }
    best.1
}

fn labs(colors: &[[f64; 3]]) -> Vec<[f64; 3]> {
    colors.iter().map(|c| oklab(*c)).collect()
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

/// Divisive clustering in the manner of median cut, but each box is cut where that leaves its two
/// sides most alike within themselves (the cut, along one channel, with the largest between-side
/// variance: Otsu's criterion), and the box cut next is the one whose cut gains most. A cut at the
/// median, as in classic median cut, splits the largest colour of flat art in two along its noise
/// and leaves two smaller colours in one box. Boxes are cut while there are fewer than `k` and
/// some box spans more than `spread` levels of a channel.
fn median_cut(used: &[Used], k: usize, spread: f64) -> Vec<ColorBox> {
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

/// Automatic: the most rare colours added to a flat picture's count.
const RARE_BUDGET: usize = 6;

/// How far apart (Oklab × 100) a rare colour must be from every palette colour and from the main
/// colour each stands for: a little more than the least difference one sees (about 2).
const RARE_APART: f64 = 2.5;

/// The fewest pixels of a 4-connected region that make a rare colour a colour of the picture (a
/// stem one pixel wide and eight long): scattered noise makes none so large.
const RARE_REGION: usize = 8;

/// A rare colour drawn as several small regions (highlights) is a colour of the picture too when
/// at least [`RARE_ISLANDS`] of its regions have this many pixels; noise kept by Noise removal is
/// no smaller than this either.
pub(crate) const RARE_ISLAND: usize = 4;

/// How many regions of [`RARE_ISLAND`] pixels make a rare colour coherent.
const RARE_ISLANDS: usize = 3;

/// Automatic on flat art: the colours too rare to count among its colours (the count keeps colours
/// on at least about 1% of the picture), or too close to a large one to get a box of their own,
/// that are still colours of the picture: a highlight, a stem, a thin outline. A rare colour is
/// kept when it is distinct, at least [`RARE_APART`] in Oklab (Ottosson 2020) from every colour of
/// `centers` and from the main colour each stands for; flat, its pixels mostly of one histogram
/// bin; and drawn, some 4-connected region of its own (not blended) pixels of at least
/// [`RARE_REGION`] pixels, or [`RARE_ISLANDS`] regions of at least [`RARE_ISLAND`]. At most [`RARE_BUDGET`] are kept, those with the largest regions first.
///
/// The palette is otherwise median cut (Heckbert 1982) refined by k-means (Lloyd 1982); keeping
/// salient colours that are rare by area follows the idea of salient-colour preservation in
/// palette reduction (Fukuda et al. 2019, doi:10.1117/12.2521607). The rule and its numbers are
/// VectorCraft's own.
fn rare_colours(img: &Raster, own: &[bool], hk: &HistKey, centers: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let px = img.rgba.as_chunks::<4>().0;
    if centers.is_empty() || own.len() != px.len() {
        return vec![];
    }
    // The picture's own (not blended) pixels by histogram bin.
    let mut bins = vec![Bin::default(); 1 << 15];
    for (p, _) in px.iter().zip(own).filter(|(p, f)| **f && p[3] >= 128) {
        bins[hk.key(p[0], p[1], p[2])].add(p);
    }
    let used: Vec<(usize, [f64; 3], f64)> = bins.iter().enumerate().filter(|(_, b)| b.n > 0).map(|(i, b)| (i, b.mean(), b.n as f64)).collect();
    let used_labs = labs(&used.iter().map(|u| u.1).collect::<Vec<_>>());
    let center_labs = labs(centers);
    // Each palette colour's main colour: its heaviest bin.
    let mut peak: Vec<Option<(f64, [f64; 3])>> = vec![None; centers.len()];
    for ((_, _, n), lab) in used.iter().zip(&used_labs) {
        let j = nearest_lab(&center_labs, *lab);
        if let Some(slot) = peak.get_mut(j)
            && slot.is_none_or(|(m, _)| *n > m)
        {
            *slot = Some((*n, *lab));
        }
    }
    let dist = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt();
    let apart = |lab: [f64; 3]| center_labs.iter().chain(peak.iter().flatten().map(|(_, l)| l)).all(|c| dist(lab, *c) >= RARE_APART);
    // The bins far from every colour, heaviest first, grouped: a bin joins the first group whose
    // heaviest bin is nearer than RARE_APART.
    let mut far: Vec<usize> = (0..used.len()).filter(|&u| used_labs.get(u).is_some_and(|l| apart(*l))).collect();
    far.sort_by(|&a, &b| used[b].2.total_cmp(&used[a].2).then(a.cmp(&b)));
    let mut group_of = vec![u16::MAX; 1 << 15];
    let mut groups: Vec<Vec<usize>> = vec![];
    for u in far {
        let (Some(lab), Some(&(bin, ..))) = (used_labs.get(u), used.get(u)) else { continue };
        let at = groups.iter().position(|g| g.first().and_then(|&f| used_labs.get(f)).is_some_and(|l| dist(*l, *lab) < RARE_APART));
        let g = match at {
            Some(g) => g,
            None if groups.len() < 64 => {
                groups.push(vec![]);
                groups.len() - 1
            }
            None => continue,
        };
        if let Some(members) = groups.get_mut(g) {
            members.push(u);
        }
        group_of[bin] = g as u16;
    }
    if groups.is_empty() {
        return vec![];
    }
    // The largest 4-connected region of each group's own pixels.
    let (w, h) = (img.width as usize, img.height as usize);
    let group_at = |i: usize| -> Option<u16> {
        let p = px.get(i)?;
        (own.get(i).copied().unwrap_or(false) && p[3] >= 128).then(|| group_of[hk.key(p[0], p[1], p[2])]).filter(|&g| g != u16::MAX)
    };
    let mut largest = vec![0usize; groups.len()];
    let mut islands = vec![0usize; groups.len()];
    // Pixels of a group whose four neighbours are of it too: an edge's blend is a line one pixel
    // wide and has next to none.
    let mut inner = vec![0usize; groups.len()];
    let mut seen = vec![false; w * h];
    let mut stack = vec![];
    for start in 0..w * h {
        let Some(g) = group_at(start).filter(|_| !seen[start]) else { continue };
        seen[start] = true;
        stack.push(start);
        let mut size = 0;
        while let Some(i) = stack.pop() {
            size += 1;
            let (x, y) = (i % w, i / w);
            let mut alike = 0;
            for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)].into_iter().flatten() {
                if group_at(j) == Some(g) {
                    alike += 1;
                    if !seen[j] {
                        seen[j] = true;
                        stack.push(j);
                    }
                }
            }
            if alike == 4
                && let Some(n) = inner.get_mut(g as usize)
            {
                *n += 1;
            }
        }
        if let Some(l) = largest.get_mut(g as usize) {
            *l = (*l).max(size);
        }
        if let Some(n) = islands.get_mut(g as usize).filter(|_| size >= RARE_ISLAND) {
            *n += 1;
        }
    }
    // Coherent and flat groups, largest region first.
    let mut chosen: Vec<(usize, f64, [f64; 3])> = vec![];
    for (g, members) in groups.iter().enumerate() {
        let weight: f64 = members.iter().filter_map(|&u| used.get(u)).map(|u| u.2).sum();
        let heaviest = members.first().and_then(|&u| used.get(u)).map_or(0.0, |u| u.2);
        let region = largest.get(g).copied().unwrap_or(0);
        let coherent = region >= RARE_REGION || islands.get(g).is_some_and(|&n| n >= RARE_ISLANDS);
        if !coherent || heaviest < 0.5 * weight || weight <= 0.0 {
            continue;
        }
        let mut mean = [0.0; 3];
        for u in members.iter().filter_map(|&u| used.get(u)) {
            for (m, c) in mean.iter_mut().zip(u.1) {
                *m += c * u.2 / weight;
            }
        }
        // Not an edge's blend of two colours that the blend test let through (a thin ring of
        // anti-aliasing between two bands is coherent too): a colour that lies between two others
        // is kept only with an inside, pixels whose neighbours are all of it.
        if is_mix(mean, centers) && inner.get(g).is_none_or(|&n| n < RARE_ISLAND) {
            continue;
        }
        chosen.push((region, weight, mean));
    }
    chosen.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal)));
    let mut out: Vec<[f64; 3]> = vec![];
    for (_, _, c) in chosen {
        if out.len() >= RARE_BUDGET {
            break;
        }
        let lab = oklab(c);
        if out.iter().all(|o| dist(oklab(*o), lab) >= RARE_APART) {
            out.push(c);
        }
    }
    out
}

/// Whether `c` (sRGB) lies between two of `centers`, as anti-aliasing mixes them: within 12 levels
/// of the line between them, and at least an eighth of the way from each.
fn is_mix(c: [f64; 3], centers: &[[f64; 3]]) -> bool {
    centers.iter().enumerate().any(|(i, a)| {
        centers.iter().skip(i + 1).any(|b| {
            let d = [0, 1, 2].map(|k| b[k] - a[k]);
            let len2: f64 = d.iter().map(|v| v * v).sum();
            if len2 <= 0.0 {
                return false;
            }
            let t = (0..3).map(|k| (c[k] - a[k]) * d[k]).sum::<f64>() / len2;
            let off2: f64 = (0..3).map(|k| (c[k] - a[k] - t * d[k]).powi(2)).sum();
            (0.12..=0.88).contains(&t) && off2 < 144.0
        })
    })
}

/// `centers` refined over the bins `used` as k-means does, but the colours from `fixed` on (rare
/// colours, see [`rare_colours`]) stay where they are: they take their own bins from the colours
/// they were averaged into, and those move back to their own.
fn refine_around(used: &[Used], mut centers: Vec<[f64; 3]>, fixed: usize) -> Vec<[f64; 3]> {
    let bins = labs(&used.iter().map(|u| u.1).collect::<Vec<_>>());
    for _ in 0..4 {
        let center_labs = labs(&centers);
        let mut sum = vec![[0.0; 3]; centers.len()];
        let mut cnt = vec![0.0; centers.len()];
        for ((_, c, n), lab) in used.iter().zip(&bins) {
            let j = nearest_lab(&center_labs, *lab);
            if let (Some(s), Some(k)) = (sum.get_mut(j), cnt.get_mut(j)) {
                for ch in 0..3 {
                    s[ch] += c[ch] * n;
                }
                *k += n;
            }
        }
        for (j, center) in centers.iter_mut().enumerate().take(fixed) {
            if let (Some(s), Some(&k)) = (sum.get(j), cnt.get(j))
                && k > 0.0
            {
                *center = [s[0] / k, s[1] / k, s[2] / k];
            }
        }
    }
    centers
}

/// Merge whole 4-connected regions, smallest first, into their most common neighbouring label.
/// The merge order does not depend on `min_area`: a higher threshold continues the same sequence
/// instead of splitting a region already merged at a lower threshold. Transparency is a barrier.
pub fn denoise(labels: &mut [u16], w: usize, h: usize, min_area: usize) {
    denoise_keeping(labels, w, h, min_area, &[]);
}

/// [`denoise`], except that a region of a label `keep` marks (a rare colour kept on purpose, see
/// [`rare_colours`]) that is at least [`RARE_ISLAND`] pixels stays whatever `min_area` is: Noise
/// clears specks, not a colour's only regions. Smaller regions of it go as any other.
pub(crate) fn denoise_keeping(labels: &mut [u16], w: usize, h: usize, min_area: usize, keep: &[bool]) {
    use std::cmp::Reverse;
    use std::collections::{BTreeMap, BinaryHeap};

    if min_area <= 1 || w == 0 || h == 0 || w.checked_mul(h) != Some(labels.len()) {
        return;
    }
    struct Region {
        label: u16,
        pixels: Vec<usize>,
    }
    // Region membership is kept current after every merge. In particular, no stale `seen`
    // flags can exclude previously recoloured pixels from a subsequent connected component.
    let mut at = vec![usize::MAX; labels.len()];
    let mut regions: Vec<Region> = Vec::new();
    let neighbours = |i: usize| {
        let (x, y) = (i % w, i / w);
        [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
    };
    for start in 0..labels.len() {
        if at[start] != usize::MAX || labels[start] == TRANSPARENT {
            continue;
        }
        let id = regions.len();
        let label = labels[start];
        let mut pixels = vec![start];
        at[start] = id;
        let mut cursor = 0;
        while let Some(&i) = pixels.get(cursor) {
            for j in neighbours(i).into_iter().flatten() {
                if at[j] == usize::MAX && labels[j] == label {
                    at[j] = id;
                    pixels.push(j);
                }
            }
            cursor += 1;
        }
        regions.push(Region { label, pixels });
    }
    let mut pending: BinaryHeap<_> = regions.iter().enumerate().map(|(id, r)| Reverse((r.pixels.len(), id))).collect();
    while let Some(Reverse((area, id))) = pending.pop() {
        if area >= min_area {
            break;
        }
        if regions[id].pixels.len() != area {
            continue; // absorbed region or an old heap entry for a region that grew
        }
        if area >= RARE_ISLAND && keep.get(regions[id].label as usize).copied().unwrap_or(false) {
            continue;
        }
        let mut borders: BTreeMap<usize, usize> = BTreeMap::new();
        for &i in &regions[id].pixels {
            for j in neighbours(i).into_iter().flatten() {
                let other = at[j];
                if other != id && other != usize::MAX {
                    *borders.entry(other).or_default() += 1;
                }
            }
        }
        let mut colours: BTreeMap<u16, usize> = BTreeMap::new();
        for (&other, &border) in &borders {
            *colours.entry(regions[other].label).or_default() += border;
        }
        let Some((&colour, _)) = colours.iter().max_by_key(|(label, border)| (**border, Reverse(**label))) else {
            continue; // an opaque island surrounded only by transparency stays intact
        };
        // All adjacent regions of the chosen colour become connected by this merge. Absorb
        // them together, keeping the largest one's storage to avoid repeatedly moving it.
        let Some(target) = borders
            .keys()
            .copied()
            .filter(|&other| regions[other].label == colour)
            .max_by_key(|&other| (regions[other].pixels.len(), Reverse(other)))
        else {
            continue;
        };
        let sources: Vec<_> =
            std::iter::once(id).chain(borders.keys().copied().filter(|&other| other != target && regions[other].label == colour)).collect();
        for source in sources {
            let pixels = std::mem::take(&mut regions[source].pixels);
            for &i in &pixels {
                at[i] = target;
            }
            regions[target].pixels.extend(pixels);
        }
        pending.push(Reverse((regions[target].pixels.len(), target)));
    }
    for region in regions {
        for i in region.pixels {
            labels[i] = region.label;
        }
    }
}
