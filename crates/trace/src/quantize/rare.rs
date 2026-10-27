use super::palette::{Bin, HistKey, TracedAs, Used, labs, nearest_lab};
use super::*;

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

/// Beside the cut edge, a color is the anti-aliasing between two others only when at least one in
/// this many of its pixels has pixels of both around it (see [`rare_colours`]).
const CUT_EDGE_MIX_TOUCH: usize = 32;

/// Beside the cut edge, a color is the anti-aliasing of a rare color only when it lies at least
/// this far from the other color toward the rare one, 0 to 1 (see [`rare_colours`]).
const CUT_EDGE_MIX_FROM: f64 = 0.45;

/// Automatic on flat art: the colours too rare to count among its colours (the count keeps colours
/// on at least about 1% of the picture), or too close to a large one to get a box of their own,
/// that are still colours of the picture: a highlight, a stem, a thin outline. A rare colour is
/// kept when it is distinct, at least [`RARE_APART`] in Oklab (Ottosson 2020) from every colour of
/// `centers` and from the main colour each stands for; flat, its pixels mostly of one histogram
/// bin; and drawn, some 4-connected region of its own (not blended) pixels of at least
/// [`RARE_REGION`] pixels, or [`RARE_ISLANDS`] regions of at least [`RARE_ISLAND`]. At most [`RARE_BUDGET`] are kept, those with the largest regions first.
///
/// `own` marks the pixels that count, and `choose` the pixels that choose the palette (see
/// [`choosing_pixels`]). A color with pixels beside a pixel that `choose` leaves out, and without an
/// inside, is not kept when it lies between two colors, one a palette color or another kept rare
/// color and the other a kept rare color, at least [`CUT_EDGE_MIX_FROM`] of the way to the rare
/// one, and at least one in [`CUT_EDGE_MIX_TOUCH`] of its pixels has pixels of both around it. The
/// pixels around it that `choose` leaves out count when their color is within [`RARE_APART`] of
/// one of the two. It is returned in the second list instead, with the indexes in `centers`
/// followed by the first list of the rare color it traces as and of the color on the other side
/// of it (the rare color again when it traces as the rare color alone).
///
/// The palette is otherwise median cut (Heckbert 1982) refined by k-means (Lloyd 1982); keeping
/// salient colours that are rare by area follows the idea of salient-colour preservation in
/// palette reduction (Fukuda et al. 2019, doi:10.1117/12.2521607). The rule and its numbers are
/// VectorCraft's own.
pub(super) fn rare_colours(img: &Raster, own: &[bool], choose: &[bool], hk: &HistKey, centers: &[[f64; 3]]) -> (Vec<[f64; 3]>, Vec<TracedAs>) {
    let px = img.rgba.as_chunks::<4>().0;
    if centers.is_empty() || own.len() != px.len() {
        return (vec![], vec![]);
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
        return (vec![], vec![]);
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
    // Groups with a pixel beside a pixel that `choose` leaves out (see `choosing_pixels`): the blend
    // test of `own` saw no neighbor on that side.
    let mut beside_left_out = vec![false; groups.len()];
    let left_out = |j: usize| px.get(j).is_some_and(|p| p[3] >= 128) && !choose.get(j).copied().unwrap_or(true);
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
            if let Some(b) = beside_left_out.get_mut(g as usize).filter(|b| !**b) {
                let (x0, y0, x1, y1) = (x.saturating_sub(1), y.saturating_sub(1), (x + 1).min(w - 1), (y + 1).min(h - 1));
                *b = (y0..=y1).any(|ny| (x0..=x1).any(|nx| left_out(ny * w + nx)));
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
    let mut chosen: Vec<(usize, f64, [f64; 3], usize)> = vec![];
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
        chosen.push((region, weight, mean, g));
    }
    chosen.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.total_cmp(&a.1)).then(a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal)));
    // Beside the pixels that choose no color, the blend test of `own` does not see the
    // anti-aliasing between a palette color and a rare color drawn along the cut edge. There, a
    // color without an inside that lies between two colors, one of them another kept rare color,
    // and at least CUT_EDGE_MIX_FROM of the way from the other to it, is that anti-aliasing when at
    // least one in CUT_EDGE_MIX_TOUCH of its pixels has pixels of both colors around it: it is not
    // kept, and it traces as that rare color (the nearer one when there are several). Art that
    // reaches a rim drawn along the edge touches it in a few places only, and the rim keeps its
    // color. The smallest regions are tested first, so that of two such colors, one stays, and a
    // color found to trace as another counts as that color.
    let tested = |g: usize| beside_left_out.get(g).copied().unwrap_or(false) && inner.get(g).is_none_or(|&n| n < RARE_ISLAND);
    // The pixels of each color tested, gathered in one pass over the image.
    let mut pixels_of: Vec<Vec<usize>> = vec![vec![]; groups.len()];
    let wanted: Vec<bool> = (0..groups.len()).map(|g| tested(g) && chosen.iter().any(|c| c.3 == g)).collect();
    if wanted.contains(&true) {
        for i in 0..w * h {
            if let Some(g) = group_at(i).map(usize::from).filter(|&g| wanted.get(g).copied().unwrap_or(false))
                && let Some(p) = pixels_of.get_mut(g)
            {
                p.push(i);
            }
        }
    }
    let mut onto: Vec<Option<[f64; 3]>> = vec![None; chosen.len()];
    // For each color in `onto`, where the color on the other side of it comes from (see `from`
    // below), or `None` when the color traces as the rare color alone.
    let mut other_of: Vec<Option<usize>> = vec![None; chosen.len()];
    for k in (0..chosen.len()).rev() {
        let Some(&(_, _, c, g)) = chosen.get(k) else { continue };
        if !tested(g) {
            continue;
        }
        // The palette colors, then the other rare colors still kept, and where each comes from: a
        // palette color's index, or the number of palette colors plus the index in `chosen`.
        let from: Vec<usize> = (0..centers.len())
            .chain(chosen.iter().zip(&onto).enumerate().filter(|(o, (_, t))| *o != k && t.is_none()).map(|(o, _)| centers.len() + o))
            .collect();
        let with: Vec<[f64; 3]> = centers
            .iter()
            .copied()
            .chain(from.iter().skip(centers.len()).filter_map(|&f| chosen.get(f.checked_sub(centers.len())?)).map(|x| x.2))
            .collect();
        let pairs: Vec<(usize, usize)> = (0..with.len())
            .flat_map(|a| (a + 1..with.len()).map(move |b| (a, b)))
            .filter(|&(a, b)| matches!((with.get(a), with.get(b)), (Some(&x), Some(&y)) if is_mix(c, &[x, y])))
            .collect();
        if pairs.is_empty() {
            continue;
        }
        // Which of `with` the pixels in the 3 × 3 around the color's own pixels are: the nearest
        // among `with`, the color itself and the colors that trace as one of `with`. A pixel that
        // chooses no color counts only within RARE_APART of the one it is nearest to (beyond the
        // cut edge, an outline over the edge shows its own color).
        let mut near = with.clone();
        near.push(c);
        let mut counts_as: Vec<Option<usize>> = (0..with.len()).map(Some).chain([None]).collect();
        for (x, t) in chosen.iter().zip(&onto) {
            if let Some(t) = t {
                near.push(x.2);
                counts_as.push(with.iter().position(|w| w == t));
            }
        }
        let near_labs = labs(&near);
        // For each pair, how many of the color's own pixels have pixels of both around them.
        let mut both = vec![0usize; pairs.len()];
        let mut pixels = 0usize;
        let mut around = vec![false; with.len()];
        for &i in pixels_of.get(g).map_or(&[][..], Vec::as_slice) {
            let (x, y) = (i % w, i / w);
            pixels += 1;
            around.fill(false);
            for ny in y.saturating_sub(1)..=(y + 1).min(h - 1) {
                for nx in x.saturating_sub(1)..=(x + 1).min(w - 1) {
                    let j = ny * w + nx;
                    let Some(p) = px.get(j).filter(|p| p[3] >= 128) else { continue };
                    let o = oklab([p[0], p[1], p[2]].map(f64::from));
                    let n = nearest_lab(&near_labs, o);
                    if !choose.get(j).copied().unwrap_or(false) && near_labs.get(n).is_none_or(|l| dist(*l, o) >= RARE_APART) {
                        continue;
                    }
                    if let Some(&Some(a)) = counts_as.get(n)
                        && let Some(t) = around.get_mut(a)
                    {
                        *t = true;
                    }
                }
            }
            for (n, &(a, b)) in pairs.iter().enumerate() {
                if around.get(a).copied().unwrap_or(false)
                    && around.get(b).copied().unwrap_or(false)
                    && let Some(v) = both.get_mut(n)
                {
                    *v += 1;
                }
            }
        }
        let lab = oklab(c);
        // The nearest rare color it traces as, with where the other color of its pair comes from
        // when its pixels split between the two (see below).
        let mut best: Option<(f64, [f64; 3], Option<usize>)> = None;
        for (n, &(a, b)) in pairs.iter().enumerate() {
            if both.get(n).is_none_or(|&v| v == 0 || v * CUT_EDGE_MIX_TOUCH < pixels) {
                continue;
            }
            for (e, f) in [(a, b), (b, a)].into_iter().filter(|&(e, _)| e >= centers.len()) {
                let (Some(&x), Some(&y)) = (with.get(e), with.get(f)) else { continue };
                // How far `c` lies from the other color toward this one.
                let d2: f64 = (0..3).map(|q| (x[q] - y[q]).powi(2)).sum();
                let t = if d2 > 0.0 { (0..3).map(|q| (c[q] - y[q]) * (x[q] - y[q])).sum::<f64>() / d2 } else { 0.0 };
                if t < CUT_EDGE_MIX_FROM {
                    continue;
                }
                let d = dist(oklab(x), lab);
                if best.is_none_or(|b| d < b.0) {
                    // A rare color along the cut edge with a region of RARE_REGION pixels is a line
                    // of its own there: the anti-aliasing beside it splits between it and the other
                    // color. Otherwise the anti-aliasing is most of a thin line, or the part of a
                    // line over the edge that lies inside it, and traces as the rare color alone.
                    let body = from
                        .get(e)
                        .and_then(|&m| chosen.get(m.checked_sub(centers.len())?))
                        .is_some_and(|r| r.0 >= RARE_REGION && beside_left_out.get(r.3).copied().unwrap_or(false));
                    best = Some((d, x, from.get(f).copied().filter(|_| body)));
                }
            }
        }
        if let Some(o) = onto.get_mut(k) {
            *o = best.map(|b| b.1);
        }
        if let Some(o) = other_of.get_mut(k) {
            *o = best.and_then(|b| b.2);
        }
    }
    let (out, traced_as) = kept_rare(chosen.iter().zip(&onto).zip(&other_of).map(|(((_, _, c, _), o), &from)| (*c, *o, from)));
    // The index in `centers` followed by `out` of a palette color or a rare color, or of the kept
    // rare color one that is not kept traces as.
    let index = |from: usize| -> Option<usize> {
        let Some(m) = from.checked_sub(centers.len()) else { return Some(from) };
        let (_, _, x, _) = chosen.get(m)?;
        let x = onto.get(m).copied().flatten().unwrap_or(*x);
        Some(centers.len() + out.iter().position(|o| *o == x)?)
    };
    // A color whose rare color is not kept (past the budget) is left out.
    let traced_as = traced_as
        .into_iter()
        .filter_map(|(c, o, from)| {
            let rare = centers.len() + out.iter().position(|x| *x == o)?;
            let other = match from {
                Some(f) => index(f)?,
                None => rare,
            };
            Some((c, rare, other))
        })
        .collect();
    (out, traced_as)
}

/// Of `colors` (a rare color, the rare color it traces as, and where the other color of its pair
/// comes from), largest region first: the rare colors kept, at most [`RARE_BUDGET`] and each at
/// least [`RARE_APART`] from the ones before it, and every color that traces as another, past the
/// budget too.
pub(crate) fn kept_rare(colors: impl IntoIterator<Item = ([f64; 3], Option<[f64; 3]>, Option<usize>)>) -> (Vec<[f64; 3]>, Vec<TracedAsColor>) {
    let dist = |a: [f64; 3], b: [f64; 3]| (0..3).map(|k| (a[k] - b[k]).powi(2)).sum::<f64>().sqrt();
    let mut out: Vec<[f64; 3]> = vec![];
    let mut traced_as = vec![];
    for (c, o, from) in colors {
        if let Some(o) = o {
            traced_as.push((c, o, from));
            continue;
        }
        if out.len() >= RARE_BUDGET {
            continue;
        }
        let lab = oklab(c);
        if out.iter().all(|o| dist(oklab(*o), lab) >= RARE_APART) {
            out.push(c);
        }
    }
    (out, traced_as)
}

/// A color, the rare color it traces as, and where the other color of its pair comes from.
pub(crate) type TracedAsColor = ([f64; 3], [f64; 3], Option<usize>);

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
pub(super) fn refine_around(used: &[Used], mut centers: Vec<[f64; 3]>, fixed: usize) -> Vec<[f64; 3]> {
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
