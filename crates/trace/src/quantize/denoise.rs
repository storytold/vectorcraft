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
