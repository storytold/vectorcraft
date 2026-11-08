use super::palette::{Bin, HistKey, Used, centers, median_cut, nearest_lab};
use super::{Quantized, Raster, TRANSPARENT, luma, oklab};

/// `q` with its blend pixels settled, and the colours that leaves unused dropped.
///
/// A pixel on an edge is a blend of the colours on its two sides, and the palette colour nearest
/// to a blend can be a third colour altogether (halfway from navy to yellow lies an olive that is
/// nearer to a green elsewhere in the picture), which would trace as a sliver of that colour along
/// the edge. A blend pixel (see [`own_colours`]) whose colour is none of those of the picture's
/// own pixels within two pixels of it takes the side it is nearer to of the pair of those colours
/// it is best a blend of, when that explains it twice as well as its own colour does. With one
/// colour around it, it keeps its own.
pub(super) fn settled(mut q: Quantized, img: &Raster, own: &[bool]) -> Quantized {
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
pub(super) fn compact(mut labels: Vec<u16>, palette: Vec<[u8; 3]>) -> Quantized {
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
    let color = |p: &[u8; 4]| if gray { [luma(*p); 3] } else { [p[0], p[1], p[2]] };
    let edge = cut_edge(img);
    let mut changed = false;
    let mut stack: Vec<(usize, usize)> = Vec::new();
    for i in 0..w * h {
        let mine = q.labels[i];
        if mine == TRANSPARENT || !edge.get(i).copied().unwrap_or(false) {
            continue;
        }
        let Some(p) = px.get(i) else { continue };
        let own = oklab(color(p).map(f64::from));
        // The colors of the fully opaque pixels within two pixels that it reaches and is a blend
        // of, and how many have each.
        let mut near: Vec<(u16, usize)> = Vec::new();
        let mut keep = false;
        reached_opaque(img, (i % w, i / w), &mut stack, |j| {
            let l = q.labels[j];
            if l != TRANSPARENT {
                keep |= l == mine;
                if px.get(j).is_some_and(|b| edge_blend_of(own, color(b), p[3])) {
                    match near.iter_mut().find(|n| n.0 == l) {
                        Some(n) => n.1 += 1,
                        None => near.push((l, 1)),
                    }
                }
            }
        });
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

/// Calls `visit` with the index of each fully opaque pixel of `img` within two pixels of the pixel
/// at `at` that it reaches through pixels of an alpha of 128 or more, going up, down, left or
/// right. `stack` is scratch space.
fn reached_opaque(img: &Raster, at: (usize, usize), stack: &mut Vec<(usize, usize)>, mut visit: impl FnMut(usize)) {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    let alpha = |j: usize| px.get(j).map_or(0, |p| p[3]);
    let (x, y) = at;
    if x >= w || y >= h {
        return;
    }
    let (x0, y0, x1, y1) = (x.saturating_sub(2), y.saturating_sub(2), (x + 2).min(w - 1), (y + 2).min(h - 1));
    let mut reached = [[false; 5]; 5];
    stack.clear();
    stack.push((x, y));
    if let Some(r) = reached.get_mut(y - y0).and_then(|row| row.get_mut(x - x0)) {
        *r = true;
    }
    while let Some((cx, cy)) = stack.pop() {
        let j = cy * w + cx;
        if alpha(j) == 255 {
            visit(j);
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
}

/// Which pixels of `img` are on its cut edge: an alpha of 128 to 254, and a pixel above, below,
/// left or right of it with an alpha under 128. Pixels outside the image do not count as
/// transparent.
fn cut_edge(img: &Raster) -> Vec<bool> {
    let (w, h) = (img.width as usize, img.height as usize);
    let px = img.rgba.as_chunks::<4>().0;
    let alpha = |j: usize| px.get(j).map_or(0, |p| p[3]);
    (0..px.len())
        .map(|i| {
            let (x, y) = (i % w.max(1), i / w.max(1));
            (128..=254).contains(&alpha(i))
                && ((x > 0 && alpha(i - 1) < 128)
                    || (x + 1 < w && alpha(i + 1) < 128)
                    || (y > 0 && alpha(i - w) < 128)
                    || (y + 1 < h && alpha(i + w) < 128))
        })
        .collect()
}

/// How many steps in from the cut edge (see [`cut_edge`]) the semi-transparent pixels that choose
/// no color reach.
const CUT_EDGE_BAND: usize = 2;

/// Which pixels of `img` choose the palette's colors: the opaque ones (an alpha of 128 or more),
/// except the pixels that [`settle_cut_edges`] gives the color beside them, and the pixels with an
/// alpha of 128 to 254 reached from those in at most [`CUT_EDGE_BAND`] steps up, down, left or
/// right through such pixels whose color is a blend of the same kind. The pixels it gives the color
/// beside them are the ones on the cut edge (see [`cut_edge`]) whose color is the color of a fully
/// opaque pixel that they reach blended with the transparent background (see [`edge_blend_of`]).
/// Counted, these pixels could add a second shade of a color to the palette. A thin
/// semi-transparent outline of another color still chooses. When no pixel is left to choose, every
/// opaque pixel chooses. With `gray` (Grayscale), a pixel's color is its gray (see [`luma`]), as in
/// [`settle_cut_edges`].
pub(super) fn choosing_pixels(img: &Raster, gray: bool) -> Vec<bool> {
    let (w, n) = (img.width as usize, img.rgba.len() / 4);
    let px = img.rgba.as_chunks::<4>().0;
    let semi = |j: usize| px.get(j).is_some_and(|p| (128..=254).contains(&p[3]));
    let color = |p: &[u8; 4]| if gray { [luma(*p); 3] } else { [p[0], p[1], p[2]] };
    let edge = cut_edge(img);
    let mut stack: Vec<(usize, usize)> = Vec::new();
    // Whether the color of pixel `i` is the color of a fully opaque pixel it reaches blended with
    // the transparent background.
    let mut blend_of_art = |i: usize| {
        let Some(p) = px.get(i) else { return false };
        let own = oklab(color(p).map(f64::from));
        let mut blend = false;
        reached_opaque(img, (i % w.max(1), i / w.max(1)), &mut stack, |j| {
            blend |= px.get(j).is_some_and(|b| edge_blend_of(own, color(b), p[3]));
        });
        blend
    };
    let mut left_out: Vec<bool> = (0..n).map(|i| edge.get(i).copied().unwrap_or(false) && blend_of_art(i)).collect();
    let mut front: Vec<usize> = (0..left_out.len()).filter(|&i| left_out.get(i).copied().unwrap_or(false)).collect();
    for _ in 0..CUT_EDGE_BAND {
        let mut next = Vec::new();
        for i in front {
            let (x, y) = (i % w.max(1), i / w.max(1));
            for j in [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (i + w < n).then(|| i + w)].into_iter().flatten() {
                if left_out.get(j) == Some(&false) && semi(j) && blend_of_art(j) {
                    if let Some(out) = left_out.get_mut(j) {
                        *out = true;
                    }
                    next.push(j);
                }
            }
        }
        front = next;
    }
    let choose: Vec<bool> = px.iter().zip(&left_out).map(|(p, &out)| p[3] >= 128 && !out).collect();
    if choose.iter().any(|&c| c) { choose } else { px.iter().map(|p| p[3] >= 128).collect() }
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
