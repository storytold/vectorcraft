use super::layers::{Layers, parallel_map_at_most};
use super::params::{Method, Mode, TraceParams};
use super::raster::{Raster, TraceError};
use super::*;

/// One traced shape: an outer contour plus its holes, filled with `color`, or (with `stroke`)
/// lines stroked with it.
#[derive(Clone, Debug, PartialEq)]
pub struct TracedPath {
    /// In pixel coordinates (x right, y down; the image spans `0..width × 0..height`).
    pub path: PathData,
    pub color: [u8; 3],
    /// Filled pixel count of the component the path came from.
    pub pixels: usize,
    /// Create Strokes: the path is stroked this wide (px), not filled (centre lines, or the
    /// outlines of wider areas without Fills).
    pub stroke: Option<f64>,
    /// Shapes: the shape the path is, exactly (`path` is that shape's outline).
    pub shape: Option<TracedShape>,
    /// Gradients: the linear gradient the path is filled with (`color` is its middle colour).
    pub gradient: Option<TracedGradient>,
    /// Create Strokes: the stroke's ends are square (a cap that goes half the width past the
    /// path's end, cut straight), as the line's ink is; otherwise round.
    pub square_caps: bool,
}

/// The traced result, bottom-most path first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TraceResult {
    pub paths: Vec<TracedPath>,
    /// Colours actually used.
    pub palette: Vec<[u8; 3]>,
    /// Auto Grouping: for each of `paths`, the path it lies inside, if any (empty without the
    /// option). Each path drawn together with those inside it, in the order they have here, paints
    /// what `paths` paints in its order.
    pub parents: Vec<Option<usize>>,
    /// Flat Logo: the size in pixels the image was traced at when it was averaged down first;
    /// `None` when it was traced at its own size.
    pub worked_at: Option<(u32, u32)>,
}

impl TraceResult {
    pub fn anchor_count(&self) -> usize {
        self.paths.iter().map(|p| p.path.anchor_count()).sum()
    }
}

fn is_white(c: [u8; 3]) -> bool {
    c.iter().all(|&v| v >= 245)
}

/// Whether Ignore Color leaves palette colour `c` out: `target` is the colour picked (`None`: white).
pub(crate) fn is_ignored(c: [u8; 3], target: Option<[u8; 3]>) -> bool {
    match target {
        None => is_white(c),
        Some(t) => (0..3).map(|k| (f64::from(c[k]) - f64::from(t[k])).powi(2)).sum::<f64>() <= 30.0 * 30.0 || (is_white(t) && is_white(c)),
    }
}

/// Trace `img` with `params`. Failures (an image Logo mode refuses, say) give an empty result: use
/// [`trace_within`] to see why.
pub fn trace(img: &Raster, params: &TraceParams) -> TraceResult {
    // No trace has more than `usize::MAX` anchors.
    trace_within(img, params, usize::MAX).unwrap_or_default()
}

/// Trace `img` with `params`, giving up with [`TraceError::TooComplex`] once the paths have more
/// than `max_anchors` anchor points (a photo traced at high fidelity can make millions). The
/// colour layers are traced in parallel where threads are available.
pub fn trace_within(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w == 0 || h == 0 || w.checked_mul(h).and_then(|n| n.checked_mul(4)) != Some(img.rgba.len()) {
        return Ok(TraceResult::default());
    }
    let mut out = trace_paths(img, params, max_anchors)?;
    if params.auto_grouping {
        // Abutting areas cannot overlap; stacked layers, Logo's shapes and stroked lines can.
        let stacked = params.method == Method::Overlapping || params.mode == Mode::Logo || params.strokes;
        out.parents = group::parents(&out.paths, img.width as usize, img.height as usize, stacked);
    }
    Ok(out)
}

fn trace_paths(img: &Raster, params: &TraceParams, max_anchors: usize) -> Result<TraceResult, TraceError> {
    let (w, h) = (img.width as usize, img.height as usize);
    if params.mode == Mode::Logo {
        return logo::trace_logo(img, params, max_anchors);
    }
    // The colour Ignore Color leaves out, if not white.
    let ignore = match &params.ignore_color {
        Some(name) if params.ignore_white => Some(logo::parse_hex(name).ok_or_else(|| TraceError::BadPalette(name.clone()))?),
        _ => None,
    };
    // A photo is traced from a copy with its noise smoothed away (see `smooth`).
    let smoothed = (params.mode != Mode::BlackAndWhite && quantize::continuous_tone(img)).then(|| smooth::smoothed(img));
    let img = smoothed.as_ref().unwrap_or(img);
    // Gradients are found first and kept out of the palette: no colour is spent on bands of them.
    // Grayscale finds them in the image's grays (as its flat colours are), so their stops are gray.
    let grays = (params.gradients && params.mode == Mode::Grayscale).then(|| {
        let mut grays = img.clone();
        for p in grays.rgba.as_chunks_mut::<4>().0 {
            let g = quantize::luma(*p);
            p[..3].fill(g);
        }
        grays
    });
    let traced_as = grays.as_ref().unwrap_or(img);
    let found = (params.gradients && params.mode != Mode::BlackAndWhite).then(|| gradient::detect(traced_as)).filter(|f| !f.1.is_empty());
    let without = found.as_ref().map(|(of, _)| {
        let mut rest = img.clone();
        for (p, &g) in rest.rgba.as_chunks_mut::<4>().0.iter_mut().zip(of) {
            if g != u32::MAX {
                p[3] = 0;
            }
        }
        rest
    });
    let (mut q, rare) = quantize::quantize_keeping(without.as_ref().unwrap_or(img), params);
    // The semi-transparent edge of a cut-out image traces as the color beside it.
    if matches!(params.mode, Mode::Color | Mode::Grayscale) {
        quantize::settle_cut_edges(&mut q, img, params.mode == Mode::Grayscale);
    }
    drop(without);
    // Each gradient is a palette entry of its own (its colour: the gradient's middle).
    let mut gradients: Vec<Option<TracedGradient>> = vec![None; q.palette.len()];
    if let Some((mut of, found)) = found {
        gradient::settle_edges(traced_as, &mut of, &found, &mut q.labels);
        let first = q.palette.len();
        for (l, &g) in q.labels.iter_mut().zip(&of) {
            if g != u32::MAX {
                *l = u16::try_from(first + g as usize).unwrap_or(TRANSPARENT);
            }
        }
        q.palette.extend(found.iter().map(|g| g.color_at(g.start.midpoint(g.end)).map(|v| v.round().clamp(0.0, 255.0) as u8)));
        gradients.extend(found.into_iter().map(Some));
    }
    drop(grays);
    // Noise clears specks, not the regions of a rare colour Automatic kept on purpose.
    let keep: Vec<bool> = q.palette.iter().map(|c| rare.contains(c)).collect();
    quantize::denoise_keeping(&mut q.labels, w, h, params.noise as usize, &keep);
    // Pixel count per palette entry, and the box its pixels lie in.
    let mut counts = vec![0usize; q.palette.len()];
    let mut boxes: Vec<Option<(usize, usize, usize, usize)>> = vec![None; q.palette.len()];
    for (i, &l) in q.labels.iter().enumerate() {
        if let (Some(c), Some(b)) = (counts.get_mut(l as usize), boxes.get_mut(l as usize)) {
            *c += 1;
            let (x, y) = (i % w, i / w);
            *b = Some(b.map_or((x, y, x + 1, y + 1), |b| (b.0.min(x), b.1.min(y), b.2.max(x + 1), b.3.max(y + 1))));
        }
    }
    // Layers bottom → top: largest area first.
    let mut order: Vec<usize> = (0..q.palette.len()).filter(|&i| counts[i] > 0).collect();
    order.sort_by(|a, b| counts[*b].cmp(&counts[*a]).then(a.cmp(b)));
    if params.method == Method::Overlapping {
        // Stacked layers go by lightness instead, from the largest colour's end of the scale: each
        // layer reaches under those above it in the areas that hold its own color, so where two
        // colours meet, every layer between them in the stack that has a pixel in that area ends
        // on that same edge and shows through its anti-aliasing. Neighbours in a picture are
        // mostly neighbours in tone, and in this order they are neighbours in the stack too (by
        // area, an unrelated colour's edge would shine through as a hairline).
        let light = |i: usize| q.palette.get(i).map_or(0, |c| u32::from(c[0]) * 299 + u32::from(c[1]) * 587 + u32::from(c[2]) * 114);
        let first = order.first().copied();
        order.sort_by(|a, b| light(*a).cmp(&light(*b)).then(a.cmp(b)));
        if first.is_some_and(|f| order.iter().position(|&i| i == f).is_some_and(|at| at * 2 >= order.len())) {
            order.reverse();
        }
    }
    let mut rank = vec![usize::MAX; q.palette.len()];
    for (r, &i) in order.iter().enumerate() {
        rank[i] = r;
    }
    let layers: Vec<(usize, usize)> =
        order.iter().copied().enumerate().filter(|&(_, ci)| !(params.ignore_white && is_ignored(q.palette[ci], ignore))).collect();
    let job = Layers {
        q: &q,
        rank: &rank,
        params,
        img,
        ignore,
        boxes,
        gradients: &gradients,
        bw_tones: bw_tones(img, &q),
        style: params.outline_style(),
        w,
        h,
        anchors: AtomicUsize::new(0),
        max_anchors,
    };
    let mut out = TraceResult::default();
    // A layer takes about ten bytes a pixel of its box while it is traced (its mask, the labels
    // of its components, the stack that fills them, its edges): as many layers at a time as fit
    // in half a gigabyte, so that a large image is not traced at the cost of gigabytes.
    // (Overlapping labels a layer's mask once more before it is traced, with its labels and stack
    // freed before the trace starts.)
    let largest = job.boxes.iter().flatten().map(|b| (b.2 - b.0) * (b.3 - b.1)).max().unwrap_or(1).max(1);
    let at_a_time = ((512 << 20) / (largest * 10)).max(1);
    for (paths, &(_, ci)) in parallel_map_at_most(at_a_time, &layers, |&(r, ci)| job.layer(r, ci)).into_iter().zip(&layers) {
        let paths = paths.ok_or(TraceError::TooComplex { max: max_anchors })?;
        // A layer with no paths adds no color to the palette.
        if paths.is_empty() {
            continue;
        }
        out.paths.extend(paths);
        let color = q.palette[ci];
        if !out.palette.contains(&color) {
            out.palette.push(color);
        }
    }
    Ok(out)
}

/// The luminance of white and of black in a Black and White quantisation: what its lightest
/// white pixels and its darkest black ones have (the 98th and 2nd percentiles; pure white and
/// black where a side has no pixel). Not the means: where the black is thin lines, blurred or
/// scanned, most black pixels are part paper, their mean is a gray, and read against it the middle
/// of every line would count as more than all ink, now more and now less as the line crosses the
/// pixel rows, which put a ripple on its edges.
pub(crate) fn bw_tones(img: &Raster, q: &Quantized) -> [f64; 2] {
    let mut hist = [[0u64; 256]; 2];
    for (p, &l) in img.rgba.as_chunks::<4>().0.iter().zip(&q.labels) {
        if let Some(h) = hist.get_mut(l as usize) {
            h[((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114 + 500) / 1000).min(255) as usize] += 1;
        }
    }
    // The luminance a share `at` of the side's pixels are darker than.
    let percentile = |h: &[u64; 256], at: f64, none: f64| {
        let total: u64 = h.iter().sum();
        let mut seen = 0;
        for (v, n) in h.iter().enumerate() {
            seen += n;
            if total > 0 && seen as f64 >= at * total as f64 {
                return v as f64;
            }
        }
        none
    };
    [percentile(&hist[0], 0.98, 255.0), percentile(&hist[1], 0.02, 0.0)]
}
