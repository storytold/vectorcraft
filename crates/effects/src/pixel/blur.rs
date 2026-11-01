//! Blur › Radial Blur and Smart Blur (Normal, Edge Only and Overlay Edge).

use vectorcraft_geom::{Affine, Point, Vec2};

use super::{PixelSpace, Px16, box_blur, pick, to8, to16};

/// Zoom's total spread, in log scale: `amount` 100 scales the content between 1/√2 and √2.
pub(super) fn zoom_extent(amount: f64) -> f64 {
    (1.0 + amount / 100.0).ln()
}

/// `buf` (`w` × `h`) sampled bilinearly at pixel index position (`x`, `y`) (pixel centres at
/// whole coordinates); transparent outside.
fn bilinear(buf: &[Px16], w: usize, h: usize, x: f64, y: f64) -> [f32; 4] {
    if !(x > -1.0 && y > -1.0 && x < w as f64 && y < h as f64) {
        return [0.0; 4];
    }
    let (x0, y0) = (x.floor(), y.floor());
    let (fx, fy) = ((x - x0) as f32, (y - y0) as f32);
    let (xi, yi) = (x0 as isize, y0 as isize);
    let at = |cx: isize, cy: isize| -> [f32; 4] {
        if cx < 0 || cy < 0 || cx as usize >= w || cy as usize >= h {
            return [0.0; 4];
        }
        buf.get(cy as usize * w + cx as usize).map_or([0.0; 4], |p| p.map(f32::from))
    };
    let (a, b, c, d) = (at(xi, yi), at(xi + 1, yi), at(xi, yi + 1), at(xi + 1, yi + 1));
    let (wa, wb, wc, wd) = ((1.0 - fx) * (1.0 - fy), fx * (1.0 - fy), (1.0 - fx) * fy, fx * fy);
    std::array::from_fn(|i| a[i] * wa + b[i] * wb + c[i] * wc + d[i] * wd)
}

/// Radial Blur: the average of the content turned (spin) or scaled (zoom) about the centre over
/// evenly spaced steps spanning `amount`. Each pass averages the previous result with itself
/// turned both ways by half the last step, so `passes` passes average 2^`passes` samples at two
/// lookups a pixel each.
pub(super) fn radial(px: &mut [[u8; 4]], w: usize, h: usize, space: &PixelSpace, amount: f64, zoom: bool, passes: u32) {
    // The centre in pixel index coordinates (pixel centres at whole numbers).
    let c = space.to_doc.inverse() * space.center - Vec2::new(0.5, 0.5);
    if !(c.x.is_finite() && c.y.is_finite()) {
        return;
    }
    let total = if zoom { zoom_extent(amount) } else { amount.to_radians() };
    let about = |m: Affine| Affine::translate(c.to_vec2()) * m * Affine::translate(-c.to_vec2());
    // A pixel's reflection in the view (one axis flipped) only turns the spin the other way,
    // which a symmetric blur doesn't notice.
    let step = |d: f64| if zoom { about(Affine::scale(d.exp())) } else { about(Affine::rotate(d)) };
    let mut cur = to16(px);
    let mut next = vec![[0u16; 4]; cur.len()];
    for j in 1..=passes.min(12) {
        let d = total / f64::powi(2.0, j as i32 + 1);
        let (fwd, back) = (step(d), step(-d));
        // Where a pixel's two samples lie moves by a fixed step along a row.
        let (df, db) = (fwd * Point::new(1.0, 0.0) - fwd * Point::ZERO, back * Point::new(1.0, 0.0) - back * Point::ZERO);
        for (y, row) in next.chunks_exact_mut(w).enumerate().take(h) {
            let (mut f, mut b) = (fwd * Point::new(0.0, y as f64), back * Point::new(0.0, y as f64));
            for out in row.iter_mut() {
                let (s, t) = (bilinear(&cur, w, h, f.x, f.y), bilinear(&cur, w, h, b.x, b.y));
                *out = std::array::from_fn(|i| ((s[i] + t[i]) * 0.5).round().clamp(0.0, 65535.0) as u16);
                (f, b) = (f + df, b + db);
            }
        }
        std::mem::swap(&mut cur, &mut next);
    }
    for (p, q) in px.iter_mut().zip(&cur) {
        *p = q.map(to8);
    }
}

/// The largest radius (pixels) Smart Blur looks over: farther neighbours are as good as gone.
const MAX_SMART: f64 = 4096.0;

/// Smart Blur's modes, in its Mode menu's order: (label, parameter value).
pub const SMART_BLUR_MODES: [(&str, &str); 3] = [("Normal", "normal"), ("Edge Only", "edgeOnly"), ("Overlay Edge", "overlayEdge")];

/// What Smart Blur draws ([`SMART_BLUR_MODES`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SmartBlurMode {
    /// The object blurred.
    Normal,
    /// The edges in white on black.
    EdgeOnly,
    /// The edges in white over the object blurred.
    OverlayEdge,
}

impl SmartBlurMode {
    const ALL: [SmartBlurMode; 3] = [SmartBlurMode::Normal, SmartBlurMode::EdgeOnly, SmartBlurMode::OverlayEdge];

    /// The mode of `value` (a [`SMART_BLUR_MODES`] value, any case); Normal otherwise.
    pub fn parse(value: &str) -> Self {
        pick(&SMART_BLUR_MODES, &Self::ALL, value, SmartBlurMode::Normal)
    }
}

/// Smart Blur's `threshold` (8-bit levels) in the levels of [`Px16`].
fn levels(threshold: f64) -> i32 {
    (threshold.clamp(0.0, 255.0) * 257.0) as i32
}

/// Do all of `a`'s channels lie within `t` levels of `b`'s?
fn close(a: &Px16, b: &Px16, t: i32) -> bool {
    a.iter().zip(b).all(|(a, b)| (*a as i32 - *b as i32).abs() <= t)
}

/// The offsets Smart Blur looks at around a pixel out to about `r` pixels (up to round(`r`)),
/// with the radius of the box blur of the copy they are read from (0: the pixels themselves):
/// every pixel of the square of side 2 · round(`r`) + 1, or `samples` × `samples` points spread
/// over the square once it is wider than `samples` pixels, read from a copy box-blurred over the
/// gaps between them. The callers keep the offsets inside their disc.
fn smart_grid(r: f64, samples: u32) -> (Vec<Vec2>, usize) {
    let n = samples.clamp(3, 15) as f64;
    let k = r.round();
    if 2.0 * k < n {
        let k = k as i32;
        ((-k..=k).flat_map(|dy| (-k..=k).map(move |dx| Vec2::new(dx as f64, dy as f64))).collect(), 0)
    } else {
        let gap = 2.0 * r / (n - 1.0);
        let grid = (0..n as i32).map(|i| -r + gap * i as f64);
        (grid.clone().flat_map(|dy| grid.clone().map(move |dx| Vec2::new(dx.round(), dy.round()))).collect(), (gap / 2.0).floor() as usize)
    }
}

/// How far from a pixel Smart Blur's edge modes ([`smart_edges`]) read, in the units of `radius`:
/// `radius`, and the box blur of the copy that a wide radius's offsets are read from.
pub(super) fn edge_reach(radius: f64, samples: u32) -> f64 {
    radius + radius / (samples.clamp(3, 15) as f64 - 1.0)
}

/// `src` (rows of `w` pixels) box-blurred with radius `prefilter`, the copy that the offsets of
/// [`smart_grid`] are read from; `None` when they are read from `src` itself.
fn smart_near(src: &[Px16], w: usize, prefilter: usize) -> Option<Vec<Px16>> {
    (prefilter > 0).then(|| {
        let mut b = src.to_vec();
        box_blur(&mut b, w, prefilter);
        b
    })
}

/// Smart Blur (Normal mode): each pixel averaged with itself and the neighbours within `radius`
/// pixels whose premultiplied channels all lie within `threshold` levels of its own. Wide radii
/// look at `samples` × `samples` neighbours spread over the disc, read from a copy box-blurred
/// over the gaps between them so the blur stays smooth (a neighbour counts when both it and its
/// blurred surroundings are within the threshold, so at most a fringe under the threshold crosses
/// an edge).
pub(super) fn smart(px: &mut [[u8; 4]], w: usize, h: usize, radius: f64, threshold: f64, samples: u32) {
    if radius.is_nan() || radius < 0.5 {
        return;
    }
    let r = radius.min(MAX_SMART);
    let (offsets, prefilter) = smart_grid(r, samples);
    let offsets: Vec<(isize, isize)> =
        offsets.into_iter().filter(|o| o.hypot() <= r + 0.5 && *o != Vec2::ZERO).map(|o| (o.x as isize, o.y as isize)).collect();
    let src = to16(px);
    let blurred = smart_near(&src, w, prefilter);
    let near = blurred.as_deref().unwrap_or(&src);
    let t = levels(threshold);
    for (y, (row, out)) in src.chunks_exact(w).zip(px.chunks_exact_mut(w)).enumerate().take(h) {
        for (x, (c, o)) in row.iter().zip(out.iter_mut()).enumerate() {
            let mut acc = c.map(u64::from);
            let mut count = 1u64;
            for (dx, dy) in &offsets {
                let (sx, sy) = (x as isize + dx, y as isize + dy);
                if sx < 0 || sy < 0 || sx as usize >= w || sy as usize >= h {
                    continue;
                }
                let i = sy as usize * w + sx as usize;
                let (Some(s), Some(own)) = (near.get(i), src.get(i)) else { continue };
                // The neighbour itself and its surroundings both close enough: edges stay sharp.
                if close(own, c, t) && close(s, c, t) {
                    for (a, v) in acc.iter_mut().zip(s) {
                        *a += *v as u64;
                    }
                    count += 1;
                }
            }
            *o = acc.map(|a| to8(((a + count / 2) / count).min(65535) as u16));
        }
    }
}

/// Smart Blur's Edge Only (`overlay` false) and Overlay Edge (`overlay` true) modes: the pixels on
/// an edge drawn white, over black or over the Normal mode's result ([`smart`]). A pixel is on an
/// edge when one of the pixels the search reads within `radius` pixels of it (transparent beyond
/// the raster) has a premultiplied channel more than `threshold` levels from its own and its
/// channels add up to at least that pixel's. The search reads the Normal mode's offsets ([`smart_grid`]) for the whole
/// pixels of `radius`, kept within `radius`, from the pixels and, for a wide radius, from the
/// box-blurred copy, and always the four nearest pixels. A straight step between two colors gives
/// a line on the side whose channels add up to more, about `radius` wide (the whole pixels of
/// `radius` while every pixel within it is read, and at least one pixel), and lines on both sides
/// when the sums are equal. The white and the black keep the pixel's coverage.
pub(super) fn smart_edges(px: &mut [[u8; 4]], w: usize, h: usize, radius: f64, threshold: f64, samples: u32, overlay: bool) {
    let src = to16(px);
    let t = levels(threshold);
    let sum = |p: &Px16| p.iter().map(|v| u32::from(*v)).sum::<u32>();
    let r = if radius.is_nan() { 0.0 } else { radius.min(MAX_SMART) };
    // Spread over whole pixels, so that the outermost offsets lie within `r` and are kept.
    let (offsets, prefilter) = smart_grid(r.floor(), samples);
    let wide: Vec<(isize, isize)> = offsets.into_iter().filter(|o| o.hypot() > 1.0 && o.hypot() <= r).map(|o| (o.x as isize, o.y as isize)).collect();
    let nearest = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    let blurred = smart_near(&src, w, prefilter);
    let mut edge = vec![false; src.len()];
    for (y, (row, out)) in src.chunks_exact(w).zip(edge.chunks_exact_mut(w)).enumerate().take(h) {
        for (x, (c, e)) in row.iter().zip(out.iter_mut()).enumerate() {
            let own = sum(c);
            let differs = |n: &Px16| !close(n, c, t) && own >= sum(n);
            // The pixel at the offset and, with `copy`, its blurred copy; transparent beyond the raster.
            let found = |(dx, dy): &(isize, isize), copy: bool| {
                let (sx, sy) = (x as isize + dx, y as isize + dy);
                if sx < 0 || sy < 0 || sx as usize >= w || sy as usize >= h {
                    return differs(&[0; 4]);
                }
                let i = sy as usize * w + sx as usize;
                src.get(i).is_some_and(differs) || (copy && blurred.as_deref().and_then(|b| b.get(i)).is_some_and(differs))
            };
            *e = nearest.iter().any(|o| found(o, false)) || wide.iter().any(|o| found(o, true));
        }
    }
    if overlay {
        smart(px, w, h, radius, threshold, samples);
    }
    for ((p, c), on_edge) in px.iter_mut().zip(&src).zip(&edge) {
        let a = to8(c[3]);
        if *on_edge {
            *p = [a; 4];
        } else if !overlay {
            *p = [0, 0, 0, a];
        }
    }
}
