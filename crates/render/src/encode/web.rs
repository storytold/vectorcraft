//! Web image optimisation on top of [`super::quantize`] (Save for Web): web snap, the colour
//! table's edits (map to transparent, sort), lossy GIF compression, and comments in GIF and JPEG
//! files.

use super::quantize::{Indexed, web_safe};

/// How the colour table is ordered.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorSort {
    /// As the reduction made it.
    #[default]
    None,
    Hue,
    Luminance,
    /// Most used first.
    Popularity,
}

impl ColorSort {
    pub const ALL: [ColorSort; 4] = [ColorSort::None, ColorSort::Hue, ColorSort::Luminance, ColorSort::Popularity];

    pub fn id(self) -> &'static str {
        match self {
            ColorSort::None => "none",
            ColorSort::Hue => "hue",
            ColorSort::Luminance => "luminance",
            ColorSort::Popularity => "popularity",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ColorSort::None => "Unsorted",
            ColorSort::Hue => "Sort by Hue",
            ColorSort::Luminance => "Sort by Luminance",
            ColorSort::Popularity => "Sort by Popularity",
        }
    }

    pub fn from_id(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.id().eq_ignore_ascii_case(s) || v.label().eq_ignore_ascii_case(s))
    }
}

/// `c` as its nearest web-safe colour when every channel is within `tolerance` percent (0–100) of
/// it: 100 snaps every colour, 0 none.
pub fn snap(c: [u8; 3], tolerance: u8) -> Option<[u8; 3]> {
    // The farthest a channel can be from a web-safe value is 25 (half the step of 51).
    let max = u32::from(tolerance.min(100)) * 26 / 100;
    let s = web_safe(c);
    (tolerance > 0 && (0..3).all(|i| u32::from(c[i].abs_diff(s[i])) <= max)).then_some(s)
}

/// Make the palette entries `mapped` names (by index) transparent: their pixels use the
/// transparent entry (added at index 0 when there is none) and the entries go. Returns the old
/// index → new index map (`None`: the entry went).
pub fn to_transparent(ix: &mut Indexed, mapped: impl Fn(usize) -> bool) -> Vec<Option<u8>> {
    let n = ix.palette.len();
    let gone: Vec<bool> = (0..n).map(|i| Some(i) != ix.transparent.map(usize::from) && mapped(i)).collect();
    if !gone.contains(&true) {
        return (0..n).map(|i| u8::try_from(i).ok()).collect();
    }
    let clear = ix.transparent.map(usize::from);
    let mut palette = Vec::with_capacity(n);
    // The transparent entry first: the old one, or a new one (the first entry mapped's colour).
    let first_gone = gone.iter().position(|g| *g).unwrap_or(0);
    palette.push(ix.palette.get(clear.unwrap_or(first_gone)).copied().unwrap_or_default());
    let mut map = vec![Some(0u8); n];
    for (i, c) in ix.palette.iter().enumerate() {
        if gone[i] || Some(i) == clear {
            continue;
        }
        map[i] = u8::try_from(palette.len()).ok();
        palette.push(*c);
    }
    for v in &mut ix.indices {
        *v = map.get(*v as usize).copied().flatten().unwrap_or(0);
    }
    ix.palette = palette;
    ix.transparent = Some(0);
    map.into_iter().enumerate().map(|(i, m)| if gone[i] { None } else { m }).collect()
}

/// Lossy GIF compression: a pixel takes the entry of the pixel left of it when that entry is
/// within `amount` (0–100) of its own colour in `rgba` (the straight pixels it was reduced from).
/// Longer runs of one index compress better in a GIF, at the cost of some noise. No pixel moves
/// into or out of the transparent entry or an entry in `fixed`.
pub fn lossy(ix: &mut Indexed, rgba: &[u8], amount: u8, fixed: &[u8]) {
    let w = ix.width as usize;
    if amount == 0 || w == 0 || rgba.len() != ix.indices.len() * 4 {
        return;
    }
    // Up to 64 levels apart in each channel at 100.
    let limit = (f32::from(amount.min(100)) / 100.0 * 64.0).powi(2) * 3.0;
    let mut held = [false; 256];
    for i in fixed.iter().chain(&ix.transparent) {
        held[usize::from(*i)] = true;
    }
    let px = rgba.as_chunks::<4>().0;
    for (row, src) in ix.indices.chunks_mut(w).zip(px.chunks(w)) {
        for x in 1..row.len() {
            let (prev, cur) = (row[x - 1], row[x]);
            if prev == cur || held[usize::from(prev)] || held[usize::from(cur)] {
                continue;
            }
            let (Some(c), Some(p)) = (src.get(x), ix.palette.get(prev as usize)) else { continue };
            let d: f32 = (0..3).map(|i| (f32::from(c[i]) - f32::from(p[i])).powi(2)).sum();
            if d <= limit {
                row[x] = prev;
            }
        }
    }
}

/// Reorder the palette by `order`, the transparent entry staying first. Returns the new order (the
/// old index of each entry).
pub fn sort(ix: &mut Indexed, order: ColorSort) -> Vec<usize> {
    let n = ix.palette.len();
    let identity: Vec<usize> = (0..n).collect();
    if order == ColorSort::None || n < 2 {
        return identity;
    }
    let mut count = vec![0u64; n];
    for &v in &ix.indices {
        if let Some(c) = count.get_mut(v as usize) {
            *c += 1;
        }
    }
    let fixed = ix.transparent.map(usize::from);
    let mut rest: Vec<usize> = identity.into_iter().filter(|i| Some(*i) != fixed).collect();
    let key = |i: usize| -> (f32, f32) {
        let [r, g, b] = ix.palette.get(i).copied().unwrap_or_default().map(|v| f32::from(v) / 255.0);
        let luma = 0.299 * r + 0.587 * g + 0.114 * b;
        match order {
            ColorSort::Hue => (hue(r, g, b), luma),
            ColorSort::Luminance => (luma, 0.0),
            _ => (-(count.get(i).copied().unwrap_or(0) as f32), luma),
        }
    };
    rest.sort_by(|a, b| {
        let (x, y) = (key(*a), key(*b));
        x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)).then(a.cmp(b))
    });
    let order: Vec<usize> = fixed.into_iter().chain(rest).collect();
    let mut remap = vec![0u8; n];
    for (new, &old) in order.iter().enumerate() {
        if let (Some(r), Ok(v)) = (remap.get_mut(old), u8::try_from(new)) {
            *r = v;
        }
    }
    ix.palette = order.iter().filter_map(|&i| ix.palette.get(i).copied()).collect();
    for v in &mut ix.indices {
        *v = remap.get(*v as usize).copied().unwrap_or(0);
    }
    if fixed.is_some() {
        ix.transparent = Some(0);
    }
    order
}

/// Hue in 0..1 (greys first, at -1).
fn hue(r: f32, g: f32, b: f32) -> f32 {
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d <= f32::EPSILON {
        return -1.0;
    }
    let h = if max == r {
        ((g - b) / d).rem_euclid(6.0)
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    h / 6.0
}

/// `gif` with a comment extension holding `text` before its trailer (unchanged when it isn't a
/// GIF or `text` is empty).
pub fn gif_comment(mut gif: Vec<u8>, text: &str) -> Vec<u8> {
    if text.is_empty() || !gif.starts_with(b"GIF8") || gif.last() != Some(&0x3B) {
        return gif;
    }
    gif.pop();
    gif.extend_from_slice(&[0x21, 0xFE]);
    for block in text.as_bytes().chunks(255) {
        gif.push(block.len() as u8);
        gif.extend_from_slice(block);
    }
    gif.extend_from_slice(&[0x00, 0x3B]);
    gif
}

/// `jpeg` with a comment segment (`COM`) holding `text` after its JFIF header (unchanged when it
/// isn't a JPEG or `text` is empty; text past a segment's 65 533 bytes is cut at a character).
pub fn jpeg_comment(jpeg: Vec<u8>, text: &str) -> Vec<u8> {
    if text.is_empty() || !jpeg.starts_with(&[0xFF, 0xD8]) {
        return jpeg;
    }
    // After SOI, and after the APP0 (JFIF) segment when it comes first.
    let at = match jpeg.get(2..6) {
        Some([0xFF, 0xE0, hi, lo]) => 4 + usize::from(u16::from_be_bytes([*hi, *lo])),
        _ => 2,
    };
    if at > jpeg.len() {
        return jpeg;
    }
    let mut end = text.len().min(65_533);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let data = &text.as_bytes()[..end];
    let mut out = Vec::with_capacity(jpeg.len() + data.len() + 4);
    out.extend_from_slice(&jpeg[..at]);
    out.extend_from_slice(&[0xFF, 0xFE]);
    out.extend_from_slice(&((data.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(data);
    out.extend_from_slice(&jpeg[at..]);
    out
}
