//! Create Strokes: the parts of a colour layer no wider than the Stroke width, traced as stroked
//! centre lines instead of filled outlines.
//!
//! 1. **Distance**: an exact Euclidean distance transform in each blob crop gives each pixel
//!    its distance to the nearest pixel outside the blob.
//! 2. **Lines**: an 8-connected component whose widest point (twice its largest distance, less
//!    one) is within the stroke width is a line; wider ones stay filled areas.
//! 3. **Skeleton**: Guo–Hall thins the crop to one pixel, preserving diagonal ends, then
//!    staircase corners that don't connect anything are dropped.
//! 4. **Graph**: runs of skeleton pixels between ends and junctions (touching junction pixels
//!    placed at their deepest pixel) become polylines; rings become closed ones. Spurs are
//!    measured beyond each junction's maximal disc. Overlapping or split-crossing junctions
//!    merge, and runs join through nodes left with two ends.
//! 5. **Ink**: the skeleton is right to a pixel only, so each of its points is moved to the middle
//!    of the line's ink, read from the image: across the line (along the pixel column or row most
//!    nearly square to it) each pixel is as much ink as its colour is the line's rather than the
//!    paper's, and the ink's centroid is the line's middle there and its sum the line's width (an
//!    anti-aliased or blurred line says both to a fraction of a pixel). The stroke width is the
//!    median of those widths, and each end is put where the ink ends, less the half width a round
//!    cap adds.
//! 6. **Curves**: each polyline is fitted as outlines are ([`fit::fit_piece`]): a straight line
//!    is two anchors, corners stay corners, the rest is as few curves as Paths allows.

mod distance;
mod graph;
mod thin;

use vectorcraft_geom::{PathData, Point};

use crate::Raster;
use crate::fit;
use crate::logo::Style;

/// One line of a colour layer: its centre lines and the width to stroke them with.
pub(crate) struct Line {
    pub path: PathData,
    /// Stroke width in pixels.
    pub width: f64,
    pub pixels: usize,
    /// Its ends are cut square (the ink is as long beside the line's middle as along it), not round.
    pub square: bool,
}

/// The lines of colour `label` in `labels` (`w` × `h`, row-major) no wider than `max_width`
/// pixels, and which pixels they cover (to leave out of the filled areas).
/// `ink` is the colour of the layer's ink in `img` (its palette colour; in Black and White the
/// image's own black), which the lines' middles and widths are read against.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lines(
    labels: &[u16],
    label: u16,
    w: usize,
    h: usize,
    max_width: f64,
    img: &Raster,
    ink: [f64; 3],
    style: &Style,
) -> (Vec<bool>, Vec<Line>) {
    let mut out = vec![];
    let Some(n) = w.checked_mul(h).filter(|&n| n > 0 && n as u64 <= crate::MAX_PIXELS && n == labels.len()) else { return (vec![], out) };
    let mut thin = vec![false; n];
    if w == 0 || h == 0 || !max_width.is_finite() || max_width <= 0.0 || !ink.iter().all(|v| v.is_finite()) {
        return (thin, out);
    }
    let mask = |i: usize| labels.get(i) == Some(&label);
    let mut seen = vec![false; w * h];
    let mut stack = vec![];
    // Thin shapes nested in one another have overlapping boxes: the cells of all of them together
    // are bounded, so the work is too (a component past it stays as it is, not a line).
    let mut cells_left = n.saturating_mul(8).max(1 << 22);
    for start in 0..w * h {
        if !mask(start) || seen.get(start).copied().unwrap_or(false) {
            continue;
        }
        let mut comp = vec![];
        if let Some(v) = seen.get_mut(start) {
            *v = true;
        }
        stack.push(start);
        while let Some(i) = stack.pop() {
            comp.push(i);
            for j in neighbours(i, w, h) {
                if mask(j) && !seen.get(j).copied().unwrap_or(false) {
                    if let Some(v) = seen.get_mut(j) {
                        *v = true;
                    }
                    stack.push(j);
                }
            }
        }
        let Some(g) = Grid::from_component(&comp, w) else { continue };
        let Some(left) = cells_left.checked_sub(g.w.saturating_mul(g.h)) else { break };
        cells_left = left;
        let Some(dt) = distance::squared(&g.on, g.w, g.h) else { continue };
        let widest = 2.0 * f64::from(dt.iter().copied().fold(0.0f32, f32::max)).sqrt() - 1.0;
        if widest > max_width {
            continue;
        }
        let Some(line) = trace_line(g, &dt, comp.len(), max_width, img, ink, style) else { continue };
        for &i in &comp {
            if let Some(v) = thin.get_mut(i) {
                *v = true;
            }
        }
        out.push(line);
    }
    (thin, out)
}

/// The 8 neighbours of pixel `i` inside the `w` × `h` image.
fn neighbours(i: usize, w: usize, h: usize) -> impl Iterator<Item = usize> {
    let (x, y) = ((i % w) as i64, (i / w) as i64);
    const AROUND: [(i64, i64); 8] = [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)];
    AROUND.into_iter().filter_map(move |(dx, dy)| {
        let (nx, ny) = (x + dx, y + dy);
        (nx >= 0 && ny >= 0 && nx < w as i64 && ny < h as i64).then(|| ny as usize * w + nx as usize)
    })
}

/// A valid crop is at most the raster limit plus a one-pixel paper border.
fn crop_size(w: usize, h: usize) -> Option<usize> {
    if w < 3 || h < 3 {
        return None;
    }
    let inner = (w - 2).checked_mul(h - 2)?;
    if inner as u64 > crate::MAX_PIXELS {
        return None;
    }
    w.checked_mul(h).filter(|&n| n <= isize::MAX as usize)
}

/// A grid around one line: its pixels with a one-pixel empty border.
struct Grid {
    on: Vec<bool>,
    w: usize,
    h: usize,
    /// Image position of grid pixel (0, 0).
    x0: i64,
    y0: i64,
}

impl Grid {
    fn from_component(comp: &[usize], image_width: usize) -> Option<Self> {
        if image_width == 0 {
            return None;
        }
        let xs = comp.iter().map(|&i| i % image_width);
        let ys = comp.iter().map(|&i| i / image_width);
        let (x0, x1, y0, y1) = (xs.clone().min()?, xs.max()?, ys.clone().min()?, ys.max()?);
        let (w, h) = (x1.checked_sub(x0)?.checked_add(3)?, y1.checked_sub(y0)?.checked_add(3)?);
        let n = crop_size(w, h)?;
        let mut g = Self { on: vec![false; n], w, h, x0: x0 as i64 - 1, y0: y0 as i64 - 1 };
        for &i in comp {
            *g.on.get_mut((i / image_width - y0 + 1) * w + i % image_width - x0 + 1)? = true;
        }
        Some(g)
    }
    #[cfg(test)]
    fn from_mask(mask: &[bool], w: usize, h: usize) -> Option<Self> {
        if w.checked_mul(h)? != mask.len() {
            return None;
        }
        let comp: Vec<_> = mask.iter().enumerate().filter_map(|(i, &on)| on.then_some(i)).collect();
        Self::from_component(&comp, w)
    }
    fn offsets(&self) -> [isize; 8] {
        let w = self.w as isize;
        [-w, -w + 1, 1, w + 1, w, w - 1, -1, -w - 1]
    }
    fn is_interior(&self, i: usize) -> bool {
        self.w > 0 && i < self.on.len() && !i.is_multiple_of(self.w) && i % self.w + 1 < self.w && i / self.w > 0 && i / self.w + 1 < self.h
    }
    /// N, NE, E, SE, S, SW, W, NW, with paper outside the interior.
    fn ring(&self, i: usize) -> [bool; 8] {
        if !self.is_interior(i) {
            return [false; 8];
        }
        self.offsets().map(|o| self.on.get(i.wrapping_add_signed(o)).copied().unwrap_or(false))
    }
    fn interior(&self) -> impl Iterator<Item = usize> + use<> {
        let (w, h) = (self.w, self.h);
        (1..h.saturating_sub(1)).flat_map(move |y| (1..w.saturating_sub(1)).map(move |x| y * w + x))
    }
    fn neighbours(&self, i: usize) -> impl Iterator<Item = usize> + '_ {
        self.offsets().into_iter().map(move |o| i.wrapping_add_signed(o)).filter(move |&j| self.is_interior(i) && self.on.get(j) == Some(&true))
    }
    fn degree(&self, i: usize) -> usize {
        self.neighbours(i).count()
    }
    /// The centre of pixel `i` in image coordinates.
    fn centre(&self, i: usize) -> Point {
        Point::new((self.x0 + (i % self.w) as i64) as f64 + 0.5, (self.y0 + (i / self.w) as i64) as f64 + 0.5)
    }
}

/// The centre lines of one line component (pixel indices of the `w`-wide image).
fn trace_line(mut g: Grid, dist: &[f32], pixels: usize, max_width: f64, img: &Raster, ink: [f64; 3], style: &Style) -> Option<Line> {
    thin::thin(&mut g);
    let mut graph = graph::build(&g, dist)?;
    graph.simplify(max_width)?;
    let meet = graph.degrees()?;
    let mut runs = graph.branches;
    if runs.is_empty() {
        return None;
    }
    // The line's mean width: its pixels over its length (the skeleton stops about half a width
    // short of each end).
    let width = |runs: &[graph::Branch]| {
        let length: f64 = runs.iter().map(graph::Branch::length).sum();
        let px = pixels as f64;
        ((-length + (length * length + 4.0 * px).sqrt()) / 2.0).clamp(1.0, max_width.max(1.0))
    };
    let coarse = width(&runs);
    let ink = Ink { img, color: ink, reach: (max_width / 2.0).ceil().clamp(1.0, 64.0) as i64 + 2 };
    let (mut widths, mut soft, mut read) = (vec![], 0usize, 0usize);
    for r in &mut runs {
        let n = r.pts.len();
        let old = r.pts.clone();
        for i in 0..n {
            // A junction stays where its pixels put it.
            let end = if i == 0 {
                r.ends[0]
            } else if i + 1 == n {
                r.ends[1]
            } else {
                None
            };
            if end.is_some_and(|e| meet.get(e).is_some_and(|&n| n > 1)) {
                continue;
            }
            let (a, b) = if r.closed {
                (*old.get((i + n - 2 % n) % n)?, *old.get((i + 2) % n)?)
            } else {
                (*old.get(i.saturating_sub(2))?, *old.get((i + 2).min(n - 1))?)
            };
            let d = b - a;
            if d.hypot() < 1e-9 {
                continue;
            }
            let upright = d.x.abs() >= d.y.abs();
            let Some(across) = ink.across(*old.get(i)?, upright) else { continue };
            *r.pts.get_mut(i)? = *old.get(i)? + if upright { kurbo::Vec2::new(0.0, across.middle) } else { kurbo::Vec2::new(across.middle, 0.0) };
            widths.push(across.width * if upright { d.x.abs() } else { d.y.abs() } / d.hypot());
            read += 1;
            soft += usize::from(across.soft);
        }
    }
    // The stroke is as wide as the ink at the line's middle point (where no ink could be read, as
    // wide as its pixels make it).
    widths.sort_by(f64::total_cmp);
    let width = widths.get(widths.len() / 2).copied().unwrap_or(coarse).clamp(0.25, max_width.max(1.0));
    // Each end goes where the ink ends, less the half width its cap adds. The cap is square when
    // the ends wide enough to tell are, taken together: the ink reaches as far a third of the width to either side
    // of the line's middle as along it, where a round end stops short by a seventh of the width.
    let (mut short_by, mut told) = (0.0, 0);
    for r in runs.iter_mut().filter(|r| !r.closed && r.pts.len() >= 2) {
        for k in 0..2 {
            if r.ends.get(k).copied().flatten().is_some_and(|e| meet.get(e).is_some_and(|&n| n > 1)) {
                continue;
            }
            let n = r.pts.len();
            let at = |i: usize| if k == 0 { i.min(n - 1) } else { n - 1 - i.min(n - 1) };
            // The last points of a wide line's skeleton turn towards a corner of its end: the end
            // lies where the line was going a width back, and they are put on that line.
            let back = (width.round().clamp(0.0, n as f64) as usize + 2).max(3);
            let (tip, inner) = (*r.pts.get(at(0))?, *r.pts.get(at(back))?);
            // (Only an end that has left that line: over so few pixels the line's direction is known
            // to a few degrees at best, less without anti-aliasing, and an end already on it is
            // better left where it is.)
            let before = inner - *r.pts.get(at(2 * back))?;
            let wide = width >= 3.0 && n > 2 * back && before.cross(tip - inner).abs() > (0.25 * width).max(1.0) * before.hypot();
            let d = if wide { before } else { tip - inner };
            if d.hypot() < 1e-9 {
                continue;
            }
            let along = d / d.hypot();
            let on_line = |p: Point| inner + along * (p - inner).dot(along).max(0.0);
            if wide {
                for i in 0..back {
                    let point = r.pts.get_mut(at(i))?;
                    *point = on_line(*point);
                }
            }
            let tip = on_line(tip);
            let reach = ink.beyond(tip, along);
            if width >= 3.0 {
                let aside = kurbo::Vec2::new(-along.y, along.x) * (0.35 * width);
                let short = reach - (ink.beyond(tip + aside, along) + ink.beyond(tip - aside, along)) / 2.0;
                short_by += short / width;
                told += 1;
            }
            let more = reach - width / 2.0;
            if more.abs() > 0.05 && more > 1.0 - (tip - inner).hypot() {
                *r.pts.get_mut(at(0))? = tip + along * more;
            }
        }
    }
    let mut style = *style;
    if read > 0 && soft * 4 < read {
        // Hard pixels: the middle is only known to about half a pixel (as for outlines).
        style.tol *= 2.5;
        style.line_tol *= 2.5;
        style.sigma = 1.5;
    }
    let subs: Vec<_> = runs
        .iter()
        .filter_map(|r| {
            if r.closed { fit::subpath(&fit::fit_closed(&r.pts, &style)?, true) } else { fit::subpath(&fit::fit_piece(&r.pts, &style, false), false) }
        })
        .collect();
    if subs.is_empty() {
        return None;
    }
    Some(Line { path: PathData::new(subs), width, pixels, square: told > 0 && short_by / f64::from(told) < 0.07 })
}

/// What the image says of a line's ink.
struct Ink<'a> {
    img: &'a Raster,
    /// The line's colour.
    color: [f64; 3],
    /// How far (pixels) from the skeleton the ink is looked for.
    reach: i64,
}

/// The ink across a line at one point: where its middle lies from the point, how wide it is, and
/// whether its edges are soft (some pixel is part ink, part paper).
struct Across {
    middle: f64,
    width: f64,
    soft: bool,
}

impl Ink<'_> {
    /// How far the colour of pixel `(x, y)` is from the line's (`None` off the image).
    fn far(&self, x: i64, y: i64) -> Option<f64> {
        let (w, h) = (i64::from(self.img.width), i64::from(self.img.height));
        if x < 0 || y < 0 || x >= w || y >= h {
            return None;
        }
        let at = (y as usize).checked_mul(w as usize)?.checked_add(x as usize)?.checked_mul(4)?;
        let p = self.img.rgba.get(at..at.checked_add(3)?)?;
        Some(p.iter().zip(self.color).map(|(&v, c)| (f64::from(v) - c).powi(2)).sum::<f64>().sqrt())
    }
    /// How far the paper's colour is from the line's around pixel `(x, y)`: the farthest colour
    /// within reach, along its row and its column.
    fn paper(&self, x: i64, y: i64) -> f64 {
        (-self.reach..=self.reach).flat_map(|k| [self.far(x + k, y), self.far(x, y + k)]).flatten().fold(0.0, f64::max)
    }
    /// The ink across the line through the pixel `p` lies in: along its column (`upright`) or row.
    fn across(&self, p: Point, upright: bool) -> Option<Across> {
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        let (x, y) = (p.x.floor() as i64, p.y.floor() as i64);
        let paper = self.paper(x, y);
        // Paper too like the ink says nothing.
        if paper < 24.0 {
            return None;
        }
        let tone = |k: i64| (if upright { self.far(x, y + k) } else { self.far(x + k, y) }).map_or(0.0, |d| (1.0 - d / paper).clamp(0.0, 1.0));
        // From the pixel with most ink near the skeleton's (thinning leaves it up to a pixel or two
        // off the line's darkest row) out to where the ink ends, or rises again (another line).
        let peak = [0, -1, 1, -2, 2].into_iter().fold(0, |best, k| if tone(k) > tone(best) + 1e-9 { k } else { best });
        let here = tone(peak);
        let (mut sum, mut moment, mut soft) = (here, here * peak as f64, here > 0.1 && here < 0.9);
        for side in [-1, 1] {
            let mut last = here;
            for k in 1..=self.reach {
                let t = tone(peak + side * k);
                if t < 0.04 || t > last + 0.1 {
                    break;
                }
                sum += t;
                moment += t * (peak + side * k) as f64;
                soft |= t > 0.1 && t < 0.9;
                last = t;
            }
        }
        // The pixel's centre is at `p`'s own half-pixel; the middle is measured from `p`.
        let centre = if upright { y as f64 + 0.5 - p.y } else { x as f64 + 0.5 - p.x };
        (sum > 0.0).then(|| Across { middle: centre + moment / sum, width: sum, soft })
    }
    /// How much ink (pixels) lies beyond `tip`, going `dir` (a unit vector).
    fn beyond(&self, tip: Point, dir: kurbo::Vec2) -> f64 {
        if !tip.x.is_finite() || !tip.y.is_finite() || !dir.x.is_finite() || !dir.y.is_finite() {
            return 0.0;
        }
        let paper = self.paper(tip.x.floor() as i64, tip.y.floor() as i64);
        if paper < 24.0 {
            return 0.0;
        }
        let mut sum = 0.0;
        for step in 1..=4 * (self.reach + 2) {
            let q = tip + dir * (step as f64 * 0.25);
            let t = self.far(q.x.floor() as i64, q.y.floor() as i64).map_or(0.0, |d| (1.0 - d / paper).clamp(0.0, 1.0));
            if t < 0.04 {
                break;
            }
            sum += 0.25 * t;
        }
        sum
    }
}
