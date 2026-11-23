//! Draws a [`Folded`] blank as pixels: each panel's outline is projected through the view and filled
//! by scanline, with a depth buffer, and each pixel takes its color from a raster of the flat print
//! (the printed side) or from the inside color (the back), shaded by the panel's slope to a fixed
//! light. The inverse of the panel's projective map gives the flat point under every pixel, so the
//! print is drawn without seams under perspective. With perspective, each panel is cut at a near
//! plane in front of the eye before it is projected.

use vectorcraft_geom::{Point, Rect};

use super::pose::{Folded, polygon};
use super::{Blank, Panel};

/// Most pixels a side of the view or of the print raster may have.
pub const MAX_SIDE: u32 = 4096;
/// Most pixels the view may have in all (2,048 × 1,024; 32 MiB of buffers).
pub const MAX_PIXELS: u32 = 1 << 21;
/// The share of the eye's distance a panel is cut at in front of the eye.
const NEAREST: f64 = 1e-3;
/// How far apart (points of view depth) two panels may be and still tie.
const TIE: f64 = 1e-4;
/// The light's direction: above, to the left of and toward the viewer.
const LIGHT: [f64; 3] = [-0.37, 0.56, 0.74];

type V3 = [f64; 3];

fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The view: its size in pixels, its turn (degrees: `rotate_x` tips the top toward the viewer,
/// `rotate_y` turns the right side toward the viewer), its scale (pixels per point at the center's
/// depth), how far it is moved (pixels), its perspective (the field of view in degrees; 0 for none)
/// and the 3D point at its middle.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    pub width: u32,
    pub height: u32,
    pub rotate_x: f64,
    pub rotate_y: f64,
    pub scale: f64,
    pub pan: [f64; 2],
    pub perspective: f64,
    pub center: [f64; 3],
}

impl Camera {
    /// The turn from 3D to the view's own axes (x right, y up, z toward the viewer), as rows.
    fn rotation(&self) -> [[f64; 3]; 3] {
        let (sx, cx) = self.rotate_x.to_radians().sin_cos();
        let (sy, cy) = (-self.rotate_y).to_radians().sin_cos();
        // Rx · Ry
        [[cy, 0.0, sy], [sx * sy, cx, -sx * cy], [-cx * sy, sx, cx * cy]]
    }

    /// The point `p` in the view's own axes, its middle at the origin.
    pub fn to_view(&self, p: V3) -> V3 {
        let r = self.rotation();
        let d = [p[0] - self.center[0], p[1] - self.center[1], p[2] - self.center[2]];
        [dot(r[0], d), dot(r[1], d), dot(r[2], d)]
    }

    /// The distance (points) from the eye to the view's middle and the focal length (pixels) under
    /// perspective; `None` without it.
    fn eye(&self) -> Option<(f64, f64)> {
        if self.perspective <= 0.0 || self.perspective >= 180.0 {
            return None;
        }
        let half = f64::from(self.width.min(self.height)) / 2.0;
        let focal = half / (self.perspective.to_radians() / 2.0).tan();
        let eye = focal / self.scale;
        (focal.is_finite() && eye.is_finite() && eye > 0.0).then_some((eye, focal))
    }

    fn usable(&self) -> bool {
        let finite =
            [self.rotate_x, self.rotate_y, self.scale, self.pan[0], self.pan[1], self.perspective].iter().chain(&self.center).all(|v| v.is_finite());
        let pixels = u64::from(self.width) * u64::from(self.height);
        finite && self.scale > 0.0 && (1..=MAX_SIDE).contains(&self.width) && (1..=MAX_SIDE).contains(&self.height) && pixels <= u64::from(MAX_PIXELS)
    }
}

/// One level of the print raster: premultiplied RGBA, 4 bytes a pixel.
#[derive(Clone, Debug)]
pub struct Level {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Level {
    /// The color at pixel `(x, y)`, clamped to the edges.
    fn at(&self, x: i64, y: i64) -> [f32; 4] {
        let x = x.clamp(0, i64::from(self.width) - 1);
        let y = y.clamp(0, i64::from(self.height) - 1);
        let i = usize::try_from((y * i64::from(self.width) + x) * 4).unwrap_or(usize::MAX);
        match self.pixels.get(i..i.saturating_add(4)) {
            Some(&[r, g, b, a]) => [f32::from(r), f32::from(g), f32::from(b), f32::from(a)],
            _ => [0.0; 4],
        }
    }

    /// The color at `(x, y)` (pixels), between the four nearest pixels.
    fn sample(&self, x: f64, y: f64) -> [f32; 4] {
        let (fx, fy) = (x - 0.5, y - 0.5);
        let (x0, y0) = (fx.floor(), fy.floor());
        let (tx, ty) = ((fx - x0) as f32, (fy - y0) as f32);
        let (x0, y0) = (x0 as i64, y0 as i64);
        let (a, b, c, d) = (self.at(x0, y0), self.at(x0 + 1, y0), self.at(x0, y0 + 1), self.at(x0 + 1, y0 + 1));
        let mut out = [0.0; 4];
        for (k, o) in out.iter_mut().enumerate() {
            let top = a[k] + (b[k] - a[k]) * tx;
            let bottom = c[k] + (d[k] - c[k]) * tx;
            *o = top + (bottom - top) * ty;
        }
        out
    }

    /// This level at half its size, rounded up, each pixel the mean of the pixels it covers weighted
    /// by the area of the full print they cover. A pixel of this level is `step` pixels of the full
    /// print (`base` pixels across and down) on a side, and fewer in the last column and row when
    /// `base` is not a multiple of `step`.
    fn half(&self, base: (u32, u32), step: u32) -> Option<Level> {
        let (w, h) = (self.width.div_ceil(2), self.height.div_ceil(2));
        if (w, h) == (self.width, self.height) {
            return None;
        }
        // The pixels of the full print that column or row `i` of this level covers, or 0 past it.
        let covers = |i: u32, side: u32| u64::from(side).saturating_sub(u64::from(i) * u64::from(step)).min(u64::from(step)) as f32;
        let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0.0f32; 4];
                let mut area = 0.0;
                for (sx, sy) in [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)] {
                    if sx >= self.width || sy >= self.height {
                        continue;
                    }
                    let a = covers(sx, base.0) * covers(sy, base.1);
                    let p = self.at(i64::from(sx), i64::from(sy));
                    for (s, c) in sum.iter_mut().zip(p) {
                        *s += c * a;
                    }
                    area += a;
                }
                for s in sum {
                    pixels.push(if area > 0.0 { (s / area).round() as u8 } else { 0 });
                }
            }
        }
        Some(Level { width: w, height: h, pixels })
    }
}

/// The flat print: the document rect it covers and its raster at full size and halved again and
/// again down to one pixel.
#[derive(Clone, Debug)]
pub struct Print {
    pub region: Rect,
    pub levels: Vec<Level>,
}

impl Print {
    /// The print of `region` from `pixels` (premultiplied RGBA, `width` × `height`); `None` when
    /// the sizes do not agree, a side is 0 or more than [`MAX_SIDE`], or the region is empty.
    pub fn new(region: Rect, width: u32, height: u32, pixels: Vec<u8>) -> Option<Print> {
        let fits = (1..=MAX_SIDE).contains(&width) && (1..=MAX_SIDE).contains(&height);
        let empty = !(region.width() > 0.0 && region.height() > 0.0 && region.is_finite());
        if !fits || empty || pixels.len() != width as usize * height as usize * 4 {
            return None;
        }
        let mut levels = vec![Level { width, height, pixels }];
        let mut step = 1;
        while let Some(next) = levels.last().and_then(|l| l.half((width, height), step)) {
            levels.push(next);
            step *= 2;
        }
        Some(Print { region, levels })
    }

    /// The print's color (premultiplied, 0 to 255) at document point `p`, from the level whose
    /// pixels are closest to `footprint` points across; transparent outside the region.
    fn sample(&self, p: Point, footprint: f64) -> [f32; 4] {
        let Some(base) = self.levels.first() else { return [0.0; 4] };
        if !self.region.contains(p) {
            return [0.0; 4];
        }
        let per_point = f64::from(base.width) / self.region.width();
        let level = if footprint.is_finite() && footprint > 0.0 { (footprint * per_point).log2().floor().max(0.0) as usize } else { 0 };
        let level = level.min(self.levels.len() - 1);
        let Some(l) = self.levels.get(level) else { return [0.0; 4] };
        // Each level halves the one before it, so a pixel of level n covers 2ⁿ of the first's.
        let shrink = f64::from(1u32 << level);
        let sx = f64::from(base.width) / self.region.width() / shrink;
        let sy = f64::from(base.height) / self.region.height() / shrink;
        l.sample((p.x - self.region.x0) * sx, (p.y - self.region.y0) * sy)
    }
}

/// What [`render`] draws: a folded blank, its print (if any), the color of the board's back and of
/// its printed side where the print has no art (straight RGBA; the board is opaque, so their alpha
/// is not used), and the view.
pub struct Scene<'a> {
    pub blank: &'a Blank,
    pub folded: &'a Folded,
    pub print: Option<&'a Print>,
    pub inside: [u8; 4],
    pub outside: [u8; 4],
    pub camera: Camera,
}

/// A drawn view: premultiplied RGBA, 4 bytes a pixel, transparent where no panel is; and for each
/// pixel the panel drawn there plus 1 (0 for none).
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub panels: Vec<u16>,
}

impl Image {
    /// The panel drawn at pixel `(x, y)`.
    pub fn panel_at(&self, x: u32, y: u32) -> Option<usize> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = y as usize * self.width as usize + x as usize;
        self.panels.get(i).and_then(|&p| usize::from(p).checked_sub(1))
    }
}

/// The 3D bounds of `blank`'s panels folded as `folded` places them (low and high corners).
pub fn bounds(blank: &Blank, folded: &Folded) -> Option<(V3, V3)> {
    let mut lo = [f64::INFINITY; 3];
    let mut hi = [f64::NEG_INFINITY; 3];
    for (panel, f) in blank.panels.iter().zip(&folded.panels) {
        let Some(b) = panel.outline.bounds() else { continue };
        for c in [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x0, b.y1), Point::new(b.x1, b.y1)] {
            let p = f.pose.place(c);
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    lo.iter().chain(&hi).all(|v| v.is_finite()).then_some((lo, hi))
}

/// A panel ready to fill: its projective map from the flat blank to the view and back, its depth
/// plane in view terms, and its color factors.
struct Mapped {
    /// Flat (u, v, 1) → (x w, y w, w) in pixels, as rows.
    forward: [[f64; 3]; 3],
    /// The inverse of `forward`.
    inverse: [[f64; 3]; 3],
    /// View depth at flat (u, v): u a + v b + c.
    depth: [f64; 3],
    printed: bool,
    shade: f32,
}

fn mul(m: &[[f64; 3]; 3], v: V3) -> V3 {
    [dot(m[0], v), dot(m[1], v), dot(m[2], v)]
}

fn invert(m: &[[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let co = [e * i - f * h, f * g - d * i, d * h - e * g];
    let det = a * co[0] + b * co[1] + c * co[2];
    if !(det.is_finite() && det.abs() > 1e-300) {
        return None;
    }
    let inv = [
        [co[0] / det, (c * h - b * i) / det, (b * f - c * e) / det],
        [co[1] / det, (a * i - c * g) / det, (c * d - a * f) / det],
        [co[2] / det, (b * g - a * h) / det, (a * e - b * d) / det],
    ];
    inv.iter().flatten().all(|v| v.is_finite()).then_some(inv)
}

fn map_panel(camera: &Camera, f: &super::FoldedPanel) -> Option<Mapped> {
    let r = camera.rotation();
    let turn = |v: V3| mul(&r, f.pose.turn(v));
    let (a, b) = (turn([1.0, 0.0, 0.0]), turn([0.0, 1.0, 0.0]));
    let o = camera.to_view(f.pose.t);
    let (cx, cy) = (f64::from(camera.width) / 2.0 + camera.pan[0], f64::from(camera.height) / 2.0 + camera.pan[1]);
    let normal = turn([0.0, 0.0, 1.0]);
    let (w, k, printed) = match camera.eye() {
        Some((eye, focal)) => ([-a[2], -b[2], eye - o[2]], focal, dot(normal, [-o[0], -o[1], eye - o[2]]) > 0.0),
        None => ([0.0, 0.0, 1.0], camera.scale, normal[2] > 0.0),
    };
    let x = [cx * w[0] + k * a[0], cx * w[1] + k * b[0], cx * w[2] + k * o[0]];
    let y = [cy * w[0] - k * a[1], cy * w[1] - k * b[1], cy * w[2] - k * o[1]];
    let forward = [x, y, w];
    let inverse = invert(&forward)?;
    let facing = if printed { normal } else { [-normal[0], -normal[1], -normal[2]] };
    let shade = (0.6 + 0.4 * dot(facing, LIGHT).max(0.0)) as f32;
    Some(Mapped { forward, inverse, depth: [a[2], b[2], o[2]], printed, shade })
}

/// `ring` (flat points) cut to the side of the plane `w · (u, v, 1) ≥ near`.
fn clip(ring: &[[f64; 2]], w: V3, near: f64) -> Vec<[f64; 2]> {
    let at = |p: [f64; 2]| dot(w, [p[0], p[1], 1.0]) - near;
    let mut out = Vec::with_capacity(ring.len() + 4);
    for (k, &p) in ring.iter().enumerate() {
        let Some(&q) = ring.get((k + 1) % ring.len()) else { continue };
        let (dp, dq) = (at(p), at(q));
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dq >= 0.0) {
            let t = dp / (dp - dq);
            out.push([p[0] + (q[0] - p[0]) * t, p[1] + (q[1] - p[1]) * t]);
        }
    }
    out
}

/// The buffers [`render`] fills.
struct Target {
    width: usize,
    height: usize,
    color: Vec<u8>,
    depth: Vec<f64>,
    rank: Vec<u16>,
    panel: Vec<u16>,
}

/// One panel's fill: its map, rank (for ties), panel number and colors.
struct Fill<'a> {
    mapped: &'a Mapped,
    rank: u16,
    panel: u16,
    print: Option<&'a Print>,
    inside: [f32; 4],
    outside: [f32; 4],
}

impl Fill<'_> {
    /// The document point under pixel center `(x, y)`, or `None` behind the eye.
    fn flat_at(&self, x: f64, y: f64) -> Option<(f64, f64)> {
        let [u, v, w] = mul(&self.mapped.inverse, [x, y, 1.0]);
        (w > 0.0 && w.is_finite()).then(|| (u / w, v / w)).filter(|(u, v)| u.is_finite() && v.is_finite())
    }

    /// The color of the panel at flat point `(u, v)`, `footprint` points across a pixel.
    fn color(&self, u: f64, v: f64, footprint: f64) -> [f32; 4] {
        let base = if self.mapped.printed {
            let p = self.print.map_or([0.0; 4], |pr| pr.sample(Point::new(u, -v), footprint));
            let under = 1.0 - p[3] / 255.0;
            [p[0] + self.outside[0] * under, p[1] + self.outside[1] * under, p[2] + self.outside[2] * under, 255.0]
        } else {
            self.inside
        };
        let s = self.mapped.shade;
        [base[0] * s, base[1] * s, base[2] * s, base[3]]
    }

    /// Fill the span of row `row` from `x0` to `x1` (pixel coordinates) into `target`.
    fn span(&self, target: &mut Target, row: usize, x0: f64, x1: f64) {
        let first = (x0 - 0.5).ceil().max(0.0);
        let last = ((x1 - 0.5).ceil() - 1.0).min(target.width as f64 - 1.0);
        if first > last {
            return;
        }
        let y = row as f64 + 0.5;
        for col in first as usize..=last as usize {
            let x = col as f64 + 0.5;
            let Some((u, v)) = self.flat_at(x, y) else { continue };
            let [a, b, c] = self.mapped.depth;
            let z = u * a + v * b + c;
            let i = row * target.width + col;
            let (Some(&zb), Some(&rb)) = (target.depth.get(i), target.rank.get(i)) else { continue };
            let wins = z > zb + TIE || ((z - zb).abs() <= TIE && self.rank > rb);
            if !wins {
                continue;
            }
            // The print's footprint is the longer of a pixel's two sides in flat terms.
            let across = |dx: f64, dy: f64| self.flat_at(x + dx, y + dy).map_or(0.0, |(u2, v2)| (u2 - u).hypot(v2 - v));
            let footprint = across(1.0, 0.0).max(across(0.0, 1.0));
            let color = self.color(u, v, footprint);
            if let Some(px) = target.color.get_mut(i * 4..i * 4 + 4) {
                for (o, c) in px.iter_mut().zip(color) {
                    *o = c.round().clamp(0.0, 255.0) as u8;
                }
            }
            if let Some(d) = target.depth.get_mut(i) {
                *d = z;
            }
            if let Some(r) = target.rank.get_mut(i) {
                *r = self.rank;
            }
            if let Some(p) = target.panel.get_mut(i) {
                *p = self.panel;
            }
        }
    }

    /// Fill the projected `rings` (pixel coordinates), even-odd, into `target`.
    fn rings(&self, target: &mut Target, rings: &[Vec<[f64; 2]>]) {
        let ys = rings.iter().flatten().map(|p| p[1]);
        let (lo, hi) = ys.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), y| (lo.min(y), hi.max(y)));
        if !(lo.is_finite() && hi.is_finite()) {
            return;
        }
        let first = (lo - 0.5).ceil().max(0.0);
        let last = (hi - 0.5).floor().min(target.height as f64 - 1.0);
        if first > last {
            return;
        }
        let mut xs = vec![];
        for row in first as usize..=last as usize {
            let y = row as f64 + 0.5;
            xs.clear();
            for ring in rings {
                for (k, p) in ring.iter().enumerate() {
                    let Some(q) = ring.get((k + 1) % ring.len()) else { continue };
                    if (p[1] <= y) != (q[1] <= y) {
                        xs.push(p[0] + (y - p[1]) * (q[0] - p[0]) / (q[1] - p[1]));
                    }
                }
            }
            xs.retain(|x| x.is_finite());
            xs.sort_by(f64::total_cmp);
            for &[a, b] in xs.as_chunks::<2>().0 {
                self.span(target, row, a, b);
            }
        }
    }
}

/// The board color `c` as drawn: its RGB, opaque.
fn opaque(c: [u8; 4]) -> [f32; 4] {
    [f32::from(c[0]), f32::from(c[1]), f32::from(c[2]), 255.0]
}

/// The outline rings of `panel` in flat terms (u, v), flattened finely enough for `scale` pixels a
/// point.
fn flat_rings(panel: &Panel, scale: f64) -> Option<Vec<Vec<[f64; 2]>>> {
    let b = panel.outline.bounds()?;
    let tolerance = (0.25 / scale).max((b.width() + b.height()) * 1e-5).max(1e-6);
    Some(polygon(&panel.outline, tolerance)?.into_iter().map(|r| r.into_iter().map(|p| [p.x, -p.y]).collect()).collect())
}

/// `scene` drawn; `None` when the view is not usable (a size of 0, a side over [`MAX_SIDE`], more
/// than [`MAX_PIXELS`] pixels, or a value that is not finite). Panels whose map is not finite, or whose outline has more than
/// [`super::MAX_POLYGON_POINTS`] points, are left out.
pub fn render(scene: &Scene) -> Option<Image> {
    let camera = scene.camera;
    if !camera.usable() {
        return None;
    }
    let (width, height) = (camera.width as usize, camera.height as usize);
    let pixels = width * height;
    let mut target =
        Target { width, height, color: vec![0; pixels * 4], depth: vec![f64::NEG_INFINITY; pixels], rank: vec![0; pixels], panel: vec![0; pixels] };
    let (inside, outside) = (opaque(scene.inside), opaque(scene.outside));
    let near = camera.eye().map(|(eye, _)| eye * NEAREST);
    for (rank, &pi) in scene.folded.order.iter().enumerate() {
        let (Some(panel), Some(f)) = (scene.blank.panels.get(pi), scene.folded.panels.get(pi)) else { continue };
        let Some(mapped) = map_panel(&camera, f) else { continue };
        let Some(rings) = flat_rings(panel, camera.scale) else { continue };
        let projected: Vec<Vec<[f64; 2]>> = rings
            .iter()
            .map(|r| match near {
                Some(near) => clip(r, mapped.forward[2], near),
                None => r.clone(),
            })
            .filter(|r| r.len() >= 3)
            .map(|r| {
                r.into_iter()
                    .filter_map(|[u, v]| {
                        let [x, y, w] = mul(&mapped.forward, [u, v, 1.0]);
                        let p = [x / w, y / w];
                        (w > 0.0 && p[0].is_finite() && p[1].is_finite()).then_some(p)
                    })
                    .collect::<Vec<_>>()
            })
            .filter(|r| r.len() >= 3)
            .collect();
        let fill = Fill {
            mapped: &mapped,
            rank: u16::try_from(rank + 1).unwrap_or(u16::MAX),
            panel: u16::try_from(pi + 1).unwrap_or(u16::MAX),
            print: scene.print,
            inside,
            outside,
        };
        fill.rings(&mut target, &projected);
    }
    Some(Image { width: camera.width, height: camera.height, pixels: target.color, panels: target.panel })
}

#[cfg(test)]
mod tests;
