use super::params::{Method, Mode, TraceParams};
#[cfg(test)]
use super::pipeline::bw_tones;
use super::pipeline::{TracedPath, is_ignored};
use super::raster::Raster;
use super::*;

/// What tracing one colour layer needs.
pub(crate) struct Layers<'a> {
    pub(crate) q: &'a Quantized,
    /// Each palette entry's layer, bottom (0) up.
    pub(crate) rank: &'a [usize],
    pub(crate) params: &'a TraceParams,
    pub(crate) img: &'a Raster,
    /// The colour Ignore Color leaves out, if not white.
    pub(crate) ignore: Option<[u8; 3]>,
    /// The box each palette entry's pixels lie in: `(x0, y0, x1, y1)`, the far sides excluded.
    pub(crate) boxes: Vec<Option<(usize, usize, usize, usize)>>,
    /// Gradients: the gradient each palette entry is, if it is one.
    pub(crate) gradients: &'a [Option<TracedGradient>],
    /// Black and White: the luminance of white and of black (see `bw_tones`).
    pub(crate) bw_tones: [f64; 2],
    /// How outlines and centre lines are fitted.
    pub(crate) style: logo::Style,
    pub(crate) w: usize,
    pub(crate) h: usize,
    /// Anchor points made so far, by every layer.
    pub(crate) anchors: AtomicUsize,
    pub(crate) max_anchors: usize,
}

impl Layers<'_> {
    /// Ignore White: white is no part of any layer (stacked layers reach under the layers above
    /// them, and must not reach under white that is left out).
    fn ignored(&self, label: u16) -> bool {
        self.params.ignore_white && self.q.palette.get(label as usize).is_some_and(|c| is_ignored(*c, self.ignore))
    }
    /// The region across a crack, for cutting outlines at their junctions: the outside pixel's
    /// colour. Stacked layers end on many colours at once and share no edge: one neighbour.
    fn neighbour(&self, c: Crack) -> u32 {
        if self.params.method == Method::Overlapping {
            return 0;
        }
        c.at(1).and_then(|i| self.q.labels.get(i)).map_or(u32::from(TRANSPARENT), |&l| u32::from(l))
    }
    /// The junction at pixel corner `(x, y)`, if there is one: where three or four regions
    /// meet, or two parts of a region touch by the corner only, or an edge reaches the image's
    /// border. It lies at the corner, moved across by what the edges that run up and down from it
    /// say, and up or down by what those to its left and right say (the mean, where there are
    /// two). It depends on the corner alone, so every region around it gets the same point; on the
    /// image's border it stays on the border.
    fn junction(&self, x: i32, y: i32) -> Option<vectorcraft_geom::Point> {
        let label = |px: i32, py: i32| Crack::new((px, py), (1, 0), self.w, self.h).at(0).and_then(|i| self.q.labels.get(i).copied());
        // The four pixels around the corner (`None` off the image), and the edges between them.
        let around = [label(x - 1, y - 1), label(x, y - 1), label(x, y), label(x - 1, y)];
        let edges = (0..4).filter(|&i| around[i] != around[(i + 1) % 4]).count();
        let apart = |a: usize, b: usize, c: usize, d: usize| around[a] == around[c] && around[b] != around[a] && around[d] != around[a];
        let touching = apart(0, 1, 2, 3) || apart(1, 2, 3, 0);
        // (Stacked layers share no edges: they are cut at the image's border only.)
        if edges < 3 && !touching || self.params.method == Method::Overlapping {
            return None;
        }
        // The shift of the crack between `a` and the pixel `step` from it, if both are in the
        // image and of different regions.
        let shift = |a: (i32, i32), step: (i32, i32)| {
            let (la, lb) = (label(a.0, a.1)?, label(a.0 + step.0, a.1 + step.1)?);
            (la != lb).then(|| self.shift(Crack::new(a, step, self.w, self.h)).clamp(-0.5, 0.5))
        };
        let mean = |a: Option<f64>, b: Option<f64>| match (a, b) {
            (Some(a), Some(b)) => (a + b) / 2.0,
            (Some(v), None) | (None, Some(v)) => v,
            (None, None) => 0.0,
        };
        let dx = mean(shift((x - 1, y - 1), (1, 0)), shift((x - 1, y), (1, 0)));
        let dy = mean(shift((x - 1, y - 1), (0, 1)), shift((x, y - 1), (0, 1)));
        Some(vectorcraft_geom::Point::new(f64::from(x) + dx, f64::from(y) + dy))
    }
    /// How far the edge lies from a crack between two pixels, towards the outside one (−0.5 to 0.5).
    ///
    /// Each pixel of the column across the crack is read as a blend of the tones on its two sides
    /// (the two pixels' palette colours; in Black and White the luminance of the image's black and of
    /// the white pixels). The edge is where the blends, summed, put it: as far out as there is
    /// inside tone in the outside pixels, less the outside tone in the inside ones (two pixels each
    /// way, while they are on the same side and no more blended than the pixel before: more would be
    /// another edge's doing).
    ///
    /// Where the inside is at most three pixels deep with the outside's colour behind it (a thin
    /// line), its two edges share those pixels and cannot be read apart; the inside tone of the
    /// whole column gives the line's middle (its centroid) and width (its sum) instead. A thin gap
    /// between two parts of the inside is read the same way, with the tones swapped.
    ///
    /// Black and White then moves the edge to where the luminance crosses the threshold: by the
    /// threshold's distance from the middle tone, over how steep the edge is (nothing for a
    /// threshold in the middle, more the softer the edge).
    ///
    /// The estimate is read from each side of the crack and the two readings averaged, so that
    /// the shift is exactly antisymmetric (`a - b == -(b - a)` in floating point) and the two
    /// regions of a crack get the same points. A single reading is not: it looks for a thin run on
    /// the inside before one on the outside, so where both sides are thin (a one- or two-pixel
    /// sliver beside another, as in soft gradients and noisy scans) each region got its own shift,
    /// their shared piece different points, and the abutting curves left a sliver uncovered.
    fn shift(&self, c: Crack) -> f64 {
        (self.shift_one_side(c) - self.shift_one_side(c.flipped())) / 2.0
    }

    /// [`Self::shift`] read from the crack's inside pixel only.
    fn shift_one_side(&self, c: Crack) -> f64 {
        let label = |k: i32| c.at(k).and_then(|i| self.q.labels.get(i).copied()).filter(|&l| l != TRANSPARENT);
        let (Some(la), Some(lb)) = (label(0), label(1)) else { return 0.0 };
        if la == lb {
            return 0.0;
        }
        let bw = self.params.mode == Mode::BlackAndWhite;
        // Black and White: black is the image's black; white is the lightest pixel within a few
        // pixels of the crack, the paper as it is here (a scan's paper is darker in one corner than
        // in another, and read against the image's lightest white it would count as part ink).
        let black = self.bw_tones[1];
        let white = if bw {
            let light = (-3..=4)
                .filter_map(|k| c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 3)))
                .map(|p| (f64::from(p[0]) * 299.0 + f64::from(p[1]) * 587.0 + f64::from(p[2]) * 114.0) / 1000.0);
            light.fold(f64::NEG_INFINITY, f64::max).clamp((black + 32.0).min(self.bw_tones[0]), self.bw_tones[0])
        } else {
            self.bw_tones[0]
        };
        let (ca, cb) = if bw {
            let tone = |l: u16| [if l == 1 { black } else { white }; 3];
            (tone(la), tone(lb))
        } else {
            // A side's own colour: its palette entry's, or its gradient's colour here.
            let own = |l: u16, k: i32| {
                let flat = self.q.palette.get(l as usize)?.map(f64::from);
                let Some(g) = self.gradients.get(l as usize).and_then(Option::as_ref) else { return Some(flat) };
                let i = c.at(k)?;
                Some(g.color_at(vectorcraft_geom::Point::new((i % self.w) as f64 + 0.5, (i / self.w) as f64 + 0.5)))
            };
            let (Some(ca), Some(cb)) = (own(la, 0), own(lb, 1)) else { return 0.0 };
            (ca, cb)
        };
        let d = [0, 1, 2].map(|k| cb[k] - ca[k]);
        let len2: f64 = d.iter().map(|v| v * v).sum();
        if len2 <= 0.0 {
            return 0.0;
        }
        // How much of the inside tone (0 to 1) the pixel `k` steps out has; none off the image.
        let inside = |k: i32| {
            let Some(p) = c.at(k).and_then(|i| self.img.rgba.get(i * 4..i * 4 + 3)) else { return 0.0 };
            let p = [f64::from(p[0]), f64::from(p[1]), f64::from(p[2])];
            let p = if bw { [(p[0] * 299.0 + p[1] * 587.0 + p[2] * 114.0) / 1000.0; 3] } else { p };
            1.0 - ((0..3).map(|k| (p[k] - ca[k]) * d[k]).sum::<f64>() / len2).clamp(0.0, 1.0)
        };
        // The edge from a thin run of `n` pixels starting at pixel `from` and going `dir`, by the
        // amounts `tone` of the run's own tone in it and in the two pixels at either end of it.
        // Returns the run's middle and width, measured from the crack towards the outside.
        let thin = |from: i32, dir: i32, n: i32, tone: &dyn Fn(i32) -> f64| {
            let (near, far) = (from - dir, from + n * dir);
            let mut cells: Vec<(i32, f64)> = (0..n).map(|j| from + j * dir).chain([near, far]).map(|k| (k, tone(k))).collect();
            for (edge, out) in [(near, -dir), (far, dir)] {
                if label(edge + out) == label(edge) {
                    cells.push((edge + out, tone(edge + out).min(tone(edge))));
                }
            }
            let width: f64 = cells.iter().map(|c| c.1).sum();
            // Pixel `k`'s centre is `k - 0.5` from the crack.
            (width > 1e-6).then(|| (cells.iter().map(|c| c.1 * (f64::from(c.0) - 0.5)).sum::<f64>() / width, width))
        };
        // How deep each side is here: up to three pixels of its own label, then the other's.
        let depth = |dir: i32, own: u16, other: u16| {
            let start = if dir < 0 { 0 } else { 1 };
            let n = (1..=3).take_while(|j| label(start + j * dir) == Some(own)).count() as i32 + 1;
            (n <= 3 && label(start + n * dir) == Some(other)).then_some(n)
        };
        let mut steep = 1.0;
        let mut shift = if let Some((middle, width)) = depth(-1, la, lb).and_then(|n| thin(0, -1, n, &inside)) {
            middle + width / 2.0
        } else if let Some((middle, width)) = depth(1, lb, la).and_then(|n| thin(1, 1, n, &|k| 1.0 - inside(k))) {
            middle - width / 2.0
        } else {
            let (a1, b1) = (1.0 - inside(0), inside(1));
            let a2 = (label(-1) == Some(la)).then(|| (1.0 - inside(-1)).min(a1));
            let b2 = (label(2) == Some(lb)).then(|| inside(2).min(b1));
            // The edge's steepness per pixel: the square of the change over two pixels (1 for an
            // anti-aliased edge, less for a blurred one).
            let change = [a2.map(|v| 1.0 - v - b1), b2.map(|v| 1.0 - a1 - v)].into_iter().flatten().fold(f64::NAN, f64::max);
            if !change.is_nan() {
                steep = (change * change).clamp(0.35, 1.0);
            }
            b1 + b2.unwrap_or(0.0) - a1 - a2.unwrap_or(0.0)
        };
        if bw {
            // The share of black at which a pixel turns black.
            let turn = ((white - f64::from(self.params.threshold)) / (white - black).max(1.0)).clamp(0.02, 0.98);
            shift += if la == 1 { 1.0 } else { -1.0 } * (0.5 - turn) / steep;
        }
        shift
    }

    /// Whether the pixels inside `l` (a loop along pixel edges) all have one label.
    fn one_colour_within(&self, l: &Loop) -> bool {
        let mut first = None;
        let (y0, y1) = l.pts.iter().fold((i32::MAX, i32::MIN), |m, p| (m.0.min(p.1), m.1.max(p.1)));
        for y in y0..y1 {
            // The loop's upright edges that pass this row of pixels, left to right: the pixels
            // between the first and the second are inside, and so on.
            let mut xs: Vec<i32> = l
                .pts
                .iter()
                .zip(l.pts.iter().cycle().skip(1))
                .filter(|(a, b)| a.0 == b.0 && a.1.min(b.1) <= y && y < a.1.max(b.1))
                .map(|(a, _)| a.0)
                .collect();
            xs.sort_unstable();
            for pair in xs.as_chunks::<2>().0 {
                for x in pair[0]..pair[1] {
                    let label = usize::try_from(y)
                        .ok()
                        .zip(usize::try_from(x).ok())
                        .and_then(|(y, x)| self.q.labels.get(y * self.w + x).copied().filter(|_| x < self.w));
                    if label.is_none() || *first.get_or_insert(label) != label {
                        return false;
                    }
                }
            }
        }
        first.is_some()
    }

    /// Shapes, Abutting: the shape the outline `sp` (fitted from the loop `l`, `hard` as that
    /// loop is) is, where it may be one. The edge is shared by the regions on its two sides, so
    /// both must become that same shape or neither: only a closed edge between two regions, a
    /// region whose whole outline is the hole of one other (no junction on it, not on the image's
    /// border), and only a shape inside the image. The shape is read from the outline run one way
    /// from one start, so the region and the hole it fills get the very same one.
    fn shared_shape(&self, l: &Loop, sp: &vectorcraft_geom::SubPath, hard: bool) -> Option<TracedShape> {
        let (w, h) = (i32::try_from(self.w).ok()?, i32::try_from(self.h).ok()?);
        if l.pts.iter().any(|&(x, y)| x <= 0 || y <= 0 || x >= w || y >= h || self.junction(x, y).is_some()) {
            return None;
        }
        let mut one_way = sp.clone();
        if one_way.signed_area() < 0.0 {
            one_way.reverse();
        }
        let first = one_way.anchors.iter().enumerate().min_by(|a, b| a.1.p.x.total_cmp(&b.1.p.x).then(a.1.p.y.total_cmp(&b.1.p.y)))?.0;
        one_way.anchors.rotate_left(first);
        let shape = shapes::recognise(&one_way, hard)?;
        let (wf, hf) = (self.w as f64, self.h as f64);
        let inside = shape
            .path()
            .subpaths
            .iter()
            .flat_map(|s| &s.anchors)
            .flat_map(|a| [a.p, a.h_in, a.h_out])
            .all(|p| (0.0..=wf).contains(&p.x) && (0.0..=hf).contains(&p.y));
        inside.then_some(shape)
    }

    /// The paths of layer `r` (palette entry `ci`), or `None` once the anchors made pass the
    /// maximum.
    pub(crate) fn layer(&self, r: usize, ci: usize) -> Option<Vec<TracedPath>> {
        let params = self.params;
        let color = *self.q.palette.get(ci)?;
        let overlapping = params.method == Method::Overlapping;
        let (w, h) = (self.w, self.h);
        // Create Strokes: this colour's lines, left out of its areas.
        let (thin, lines) = if params.strokes {
            // The ink the lines are read against: the layer's colour; Black and White's own black.
            let ink = if params.mode == Mode::BlackAndWhite { [self.bw_tones[usize::from(ci == 1)]; 3] } else { color.map(f64::from) };
            centerline::lines(&self.q.labels, u16::try_from(ci).unwrap_or(TRANSPARENT), w, h, params.stroke_width, self.img, ink, &self.style)
        } else {
            (vec![], vec![])
        };
        // The layer is traced within the box its pixels lie in, not the whole image.
        let part = |l: u16| if overlapping { self.rank.get(l as usize).is_some_and(|&lr| lr >= r) && !self.ignored(l) } else { l as usize == ci };
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for (l, b) in self.boxes.iter().enumerate() {
            if let Some(b) = b.filter(|_| part(l as u16)) {
                (x0, y0, x1, y1) = (x0.min(b.0), y0.min(b.1), x1.max(b.2), y1.max(b.3));
            }
        }
        if x0 >= x1 || y0 >= y1 {
            return Some(vec![]);
        }
        let (bw, bh) = (x1 - x0, y1 - y0);
        let mut mask = Vec::with_capacity(bw * bh);
        for y in y0..y1 {
            let row = self.q.labels.get(y * w + x0..y * w + x1)?;
            mask.extend(row.iter().enumerate().map(|(k, &l)| l != TRANSPARENT && !thin.get(y * w + x0 + k).copied().unwrap_or(false) && part(l)));
        }
        // Overlapping: a part of the mask with no pixel of this color holds only colors above it,
        // whose own layers fill it, and is left out. The layer reaches under the colors above it
        // only within the areas that hold its own color.
        if overlapping {
            let (comp, sizes) = contour::label(&mask, bw, bh);
            let mut own = vec![false; sizes.len()];
            for (k, &c) in comp.iter().enumerate() {
                if self.q.labels.get((y0 + k / bw) * w + x0 + k % bw).is_some_and(|&l| l as usize == ci)
                    && let Some(o) = own.get_mut(c as usize)
                {
                    *o = true;
                }
            }
            for (m, &c) in mask.iter_mut().zip(&comp) {
                *m = *m && own.get(c as usize).copied().unwrap_or(false);
            }
        }
        // Without Fills, areas are outlined with a 1 px stroke.
        let area_stroke = (params.strokes && !params.fills).then_some(1.0);
        let min_hole = params.noise.max(1) as i64;
        let mut paths = vec![];
        let mut add = |path: PathData, pixels: usize, stroke: Option<f64>, shape: Option<TracedShape>, square_caps: bool| {
            let n = path.anchor_count();
            if self.anchors.fetch_add(n, Ordering::Relaxed).saturating_add(n) > self.max_anchors {
                return false;
            }
            // (A gradient fills; a stroked line of it is its middle colour.)
            let gradient = self.gradients.get(ci).cloned().flatten().filter(|_| stroke.is_none());
            paths.push(TracedPath { path, color, pixels, stroke, shape, gradient, square_caps });
            true
        };
        let mut comps = trace_mask(&mask, bw, bh);
        drop(mask);
        for comp in &mut comps {
            for l in std::iter::once(&mut comp.outer).chain(&mut comp.holes) {
                for p in &mut l.pts {
                    *p = (p.0 + x0 as i32, p.1 + y0 as i32);
                }
            }
        }
        for comp in comps {
            // (`hard`: some of the outline has no anti-aliasing, and is known less exactly.)
            let hard = std::cell::Cell::new(false);
            // (Each loop fitted, and whether its own outline is hard.)
            let fit = |l: &Loop| {
                let (sp, is_hard) = fit::fit_loop(l, w, h, &|c| (self.shift(c), self.neighbour(c)), &|x, y| self.junction(x, y), &self.style)?;
                // Every trace straightens the pieces of an outline that lie close to a line (Snap
                // Curves To Lines straightens more of them), so the outline of a single pixel or
                // of a region one pixel thick can have all its pieces on one chord, and the loop
                // then has no area. Filled, such a loop paints nothing, and it is left out.
                // Without Fills its 1 px stroke draws it, so it is kept. (`fit::fit_closed` puts
                // back the positioned outline of a loop that is one closed piece when its fit has
                // no area.) The area is summed about the loop's first anchor.
                if area_stroke.is_none() {
                    use kurbo::ParamCurveArea;
                    let origin = sp.anchors.first()?.p.to_vec2();
                    let area: f64 = (0..sp.segment_count()).map(|i| (kurbo::Affine::translate(-origin) * sp.segment(i)).signed_area()).sum();
                    if area.abs() < 1e-6 {
                        return None;
                    }
                }
                hard.set(hard.get() || is_hard);
                Some((sp, is_hard))
            };
            let Some((outer, _)) = fit(&comp.outer) else { continue };
            let mut subs = vec![outer];
            for hole in &comp.holes {
                if hole.area2.abs() / 2 < min_hole && params.noise > 0 {
                    continue;
                }
                let Some((mut sp, own_hard)) = fit(hole) else { continue };
                // Shapes: a hole that is a shape is that shape, as what fills it is (the two are
                // one outline: they stay one). Only where one colour fills it: the parts of
                // several are not that shape, and would no longer meet it. Abutting, only where the
                // region that fills it is that shape too (see `shared_shape`).
                let exact = if !params.shapes || area_stroke.is_some() || !self.one_colour_within(hole) {
                    None
                } else if overlapping {
                    shapes::recognise(&sp, hard.get())
                } else {
                    // (As hard as this outline is, as the region that fills it reads it.)
                    self.shared_shape(hole, &sp, own_hard)
                };
                if let Some(exact) = exact.and_then(|shape| shape.path().subpaths.into_iter().next()) {
                    // (Which way round each goes: by the area of the polygon through points along it.)
                    let turn = |sp: &vectorcraft_geom::SubPath| {
                        use vectorcraft_geom::ParamCurve;
                        let pts: Vec<_> =
                            (0..sp.segment_count()).flat_map(|i| (0..6).map(move |k| sp.segment(i).eval(f64::from(k) / 6.0).to_vec2())).collect();
                        pts.iter().zip(pts.iter().cycle().skip(1)).map(|(a, b)| a.cross(*b)).sum::<f64>()
                    };
                    let mut exact = exact;
                    if (turn(&exact) < 0.0) != (turn(&sp) < 0.0) {
                        exact.reverse();
                    }
                    sp = exact;
                }
                subs.push(sp);
            }
            // Shapes: a filled outline without holes that is a shape becomes that shape.
            let shape = match subs.as_slice() {
                [only] if params.shapes && area_stroke.is_none() && overlapping => shapes::recognise(only, hard.get()),
                // Abutting: only an island in one other region, whose hole it is (see `shared_shape`).
                [only] if params.shapes && area_stroke.is_none() && comp.holes.is_empty() => self.shared_shape(&comp.outer, only, hard.get()),
                _ => None,
            };
            let path = shape.map_or_else(|| PathData::new(subs), |s| s.path());
            if !add(path, comp.pixels, area_stroke, shape, false) {
                return None;
            }
        }
        for line in lines {
            if !add(line.path, line.pixels, Some(line.width), None, line.square) {
                return None;
            }
        }
        Some(paths)
    }
}

/// Test probe: every crack between two labels of `img` quantised as `trace` does (no gradients),
/// as `[shift, shift from the other side, one-sided reading, one-sided reading from the other
/// side]`.
#[cfg(test)]
pub(crate) fn shift_probe(img: &Raster, params: &TraceParams) -> Vec<[f64; 4]> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut q = quantize(img, params);
    denoise(&mut q.labels, w, h, params.noise as usize);
    let n = q.palette.len();
    let rank: Vec<usize> = (0..n).collect();
    let gradients = vec![None; n];
    let job = Layers {
        q: &q,
        rank: &rank,
        params,
        img,
        ignore: None,
        boxes: vec![None; n],
        gradients: &gradients,
        bw_tones: bw_tones(img, &q),
        style: params.outline_style(),
        w,
        h,
        anchors: AtomicUsize::new(0),
        max_anchors: usize::MAX,
    };
    let mut out = vec![];
    for y in 0..h as i32 {
        for x in 0..w as i32 {
            for step in [(1, 0), (0, 1)] {
                let c = Crack::new((x, y), step, w, h);
                let (Some(a), Some(b)) = (c.at(0), c.at(1)) else { continue };
                if q.labels[a] != q.labels[b] {
                    out.push([job.shift(c), job.shift(c.flipped()), job.shift_one_side(c), job.shift_one_side(c.flipped())]);
                }
            }
        }
    }
    out
}

/// `f` of each of `items`, in order, spread over the available cores.
pub(crate) fn parallel_map<T: Sync, R: Send>(items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    parallel_map_at_most(usize::MAX, items, f)
}

/// `f` of each of `items`, in order, spread over the available cores, at most `at_a_time` at once.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn parallel_map_at_most<T: Sync, R: Send>(at_a_time: usize, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(items.len()).min(at_a_time);
    if threads <= 1 {
        return items.iter().map(f).collect();
    }
    let next = AtomicUsize::new(0);
    let mut done: Vec<(usize, R)> = std::thread::scope(|s| {
        let workers: Vec<_> = (0..threads)
            .map(|_| {
                s.spawn(|| {
                    let mut mine = vec![];
                    loop {
                        let i = next.fetch_add(1, Ordering::Relaxed);
                        let Some(item) = items.get(i) else { break };
                        mine.push((i, f(item)));
                    }
                    mine
                })
            })
            .collect();
        // A worker's panic goes on in this thread, as if it had run here (the engine's guard
        // reports it).
        workers.into_iter().flat_map(|w| w.join().unwrap_or_else(|e| std::panic::resume_unwind(e))).collect()
    });
    done.sort_by_key(|(i, _)| *i);
    done.into_iter().map(|(_, r)| r).collect()
}

/// `f` of each of `items`, in order (no threads on the web).
#[cfg(target_arch = "wasm32")]
pub(crate) fn parallel_map_at_most<T: Sync, R: Send>(_: usize, items: &[T], f: impl Fn(&T) -> R + Sync) -> Vec<R> {
    items.iter().map(f).collect()
}
