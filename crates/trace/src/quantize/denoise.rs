use super::TRANSPARENT;
use super::rare::RARE_ISLAND;

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
    use std::collections::BinaryHeap;

    // Every index below is a `u32`: 4 bytes per pixel and per region instead of a `Vec<usize>` of pixels per region.
    const NONE: u32 = u32::MAX;
    if min_area <= 1 || w == 0 || h == 0 || w.checked_mul(h) != Some(labels.len()) || labels.len() >= NONE as usize {
        return;
    }
    // Region of each pixel as first found (`NONE` for transparent). A merge does not touch it: the region a pixel
    // belongs to now is `find(at[i])`.
    let mut at = vec![NONE; labels.len()];
    // Pixels of a region form a singly linked list through `next`, so absorbing a region is one pointer write.
    let mut next = vec![NONE; labels.len()];
    let mut head: Vec<u32> = Vec::new();
    let mut tail: Vec<u32> = Vec::new();
    let mut size: Vec<u32> = Vec::new();
    let mut colour: Vec<u16> = Vec::new();
    // Union-find over region ids: a region absorbed into another points at it.
    let mut parent: Vec<u32> = Vec::new();
    let neighbours = |i: usize| {
        let (x, y) = (i % w, i / w);
        [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
    };
    for start in 0..labels.len() {
        if at[start] != NONE || labels[start] == TRANSPARENT {
            continue;
        }
        let id = head.len() as u32;
        let label = labels[start];
        at[start] = id;
        let mut last = start;
        let mut count = 1u32;
        let mut cursor = start;
        loop {
            for j in neighbours(cursor).into_iter().flatten() {
                if at[j] == NONE && labels[j] == label {
                    at[j] = id;
                    next[last] = j as u32;
                    last = j;
                    count += 1;
                }
            }
            match next[cursor] {
                NONE => break,
                n => cursor = n as usize,
            }
        }
        head.push(start as u32);
        tail.push(last as u32);
        size.push(count);
        colour.push(label);
        parent.push(id);
    }
    fn find(parent: &mut [u32], mut a: u32) -> u32 {
        while parent[a as usize] != a {
            let up = parent[parent[a as usize] as usize];
            parent[a as usize] = up;
            a = up;
        }
        a
    }
    // Only regions below `min_area` are ever taken from the queue; a larger one ends the loop.
    let mut pending: BinaryHeap<_> =
        (0..size.len() as u32).filter(|&id| (size[id as usize] as usize) < min_area).map(|id| Reverse((size[id as usize], id))).collect();
    // Scratch for one merge: `(other region, shared pixel edges)` and `(colour, shared pixel edges)`.
    let mut borders: Vec<(u32, usize)> = Vec::new();
    let mut by_colour: Vec<(u16, usize)> = Vec::new();
    while let Some(Reverse((area, id))) = pending.pop() {
        if size[id as usize] != area {
            continue; // absorbed region or an old heap entry for a region that grew
        }
        if area as usize >= RARE_ISLAND && keep.get(colour[id as usize] as usize).copied().unwrap_or(false) {
            continue;
        }
        borders.clear();
        // Pairs are folded together whenever the list doubles, so a big region keeps it short.
        let mut limit = 4096;
        let mut pixel = head[id as usize];
        while pixel != NONE {
            for j in neighbours(pixel as usize).into_iter().flatten() {
                if at[j] != NONE {
                    let other = find(&mut parent, at[j]);
                    if other != id {
                        borders.push((other, 1));
                    }
                }
            }
            pixel = next[pixel as usize];
            if borders.len() >= limit {
                merge_counts(&mut borders);
                limit = limit.max(2 * borders.len());
            }
        }
        merge_counts(&mut borders);
        by_colour.clear();
        by_colour.extend(borders.iter().map(|&(other, border)| (colour[other as usize], border)));
        merge_counts(&mut by_colour);
        let Some(&(chosen, _)) = by_colour.iter().max_by_key(|(label, border)| (*border, Reverse(*label))) else {
            continue; // an opaque island surrounded only by transparency stays intact
        };
        // All adjacent regions of the chosen colour become connected by this merge. Absorb
        // them together, keeping the largest one as the survivor.
        let Some(target) = borders
            .iter()
            .map(|&(other, _)| other)
            .filter(|&other| colour[other as usize] == chosen)
            .max_by_key(|&other| (size[other as usize], Reverse(other)))
        else {
            continue;
        };
        for source in
            std::iter::once(id).chain(borders.iter().map(|&(other, _)| other).filter(|&other| other != target && colour[other as usize] == chosen))
        {
            next[tail[target as usize] as usize] = head[source as usize];
            tail[target as usize] = tail[source as usize];
            size[target as usize] += size[source as usize];
            size[source as usize] = 0;
            parent[source as usize] = target;
        }
        if (size[target as usize] as usize) < min_area {
            pending.push(Reverse((size[target as usize], target)));
        }
    }
    for (i, &region) in at.iter().enumerate() {
        if region != NONE {
            labels[i] = colour[find(&mut parent, region) as usize];
        }
    }
}

/// Sort `(key, count)` pairs and add up the counts of equal keys.
fn merge_counts<K: Ord + Copy>(pairs: &mut Vec<(K, usize)>) {
    pairs.sort_unstable_by_key(|p| p.0);
    pairs.dedup_by(|b, a| {
        let same = a.0 == b.0;
        if same {
            a.1 += b.1;
        }
        same
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first implementation (a pixel `Vec` per region, `BTreeMap` borders), kept to check that the compact one merges
    /// in exactly the same order.
    fn denoise_reference(labels: &mut [u16], w: usize, h: usize, min_area: usize, keep: &[bool]) {
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

    fn noisy(w: usize, h: usize, colours: u64, transparent_one_in: u64, seed: u64) -> Vec<u16> {
        let mut state = seed;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        (0..w * h)
            .map(|_| {
                // Runs of equal labels as well as single pixels, so regions of every size touch.
                if transparent_one_in > 0 && next() % transparent_one_in == 0 { TRANSPARENT } else { (next() % colours) as u16 }
            })
            .collect()
    }

    #[test]
    fn compact_denoise_matches_the_reference_merge_order() {
        for (seed, (w, h, colours, transparent)) in
            [(31, 17, 3, 0), (64, 64, 2, 0), (50, 70, 5, 9), (97, 41, 8, 0), (120, 90, 4, 5), (7, 300, 3, 0), (1, 40, 3, 0), (40, 1, 3, 0)]
                .into_iter()
                .enumerate()
        {
            let base = noisy(w, h, colours, transparent, seed as u64 + 1);
            for min_area in [2, 3, 5, 9, 25, 80, 400, 100_000] {
                for keep in [&[][..], &[false, true, false, true][..]] {
                    let (mut a, mut b) = (base.clone(), base.clone());
                    denoise_reference(&mut a, w, h, min_area, keep);
                    denoise_keeping(&mut b, w, h, min_area, keep);
                    assert_eq!(a, b, "{w}x{h}, {colours} colours, min_area {min_area}, keep {keep:?}");
                }
            }
        }
    }

    /// A minimum so large that regions keep merging into big ones.
    #[test]
    fn compact_denoise_matches_the_reference_with_a_large_min_area() {
        let (w, h) = (320, 320);
        let base = noisy(w, h, 6, 0, 99);
        let (mut a, mut b) = (base.clone(), base);
        denoise_reference(&mut a, w, h, 50_000, &[]);
        denoise_keeping(&mut b, w, h, 50_000, &[]);
        assert_eq!(a, b);
    }
}
