//! Gradient meshes: `/Mesh X!`, then on `%_` lines `value /key X#` entries — the colour space
//! (`/CS`), and per patch its place (`/P [row col]`) and four corners (`/N`, from the corner nearest
//! the origin round to the one beside it), ended by `/E` — and `/End X!`.
//!
//! A corner is `[colour… opacity x y` then the handle towards the next corner, a weight, the handle
//! towards the previous one and a weight `]`, in art space. A patch whose edges bend through more
//! anchors has them as `[x y` with a handle towards each neighbor and a weight `] /A` or `/S`,
//! after the corner their edge starts from (before the first corner for the edge back to it), in
//! the mesh's own direction whichever the key: along the first two edges, against the last two. A
//! patch with single cubic edges becomes a 1×1 mesh; one with more anchors, the Coons patch of its
//! edges, cut into cells where any edge has an anchor.
//!
//! `X!`/`X#` aren't publicly documented (the AI7 specification predates meshes).

use kurbo::{Affine, Point, Vec2};
use vectorcraft_color::Color;
use vectorcraft_doc::live::{GradientMesh, H_DOWN, H_LEFT, H_RIGHT, H_UP, MeshPoint};

use super::obj::V;

/// The most patches read from one mesh.
const MAX_PATCHES: usize = 4096;
/// The most points kept from one mesh: a patch with anchors on its edges is cut into cells, up to
/// 34 × 34 points, so the patch cap alone doesn't bound them.
const MAX_POINTS: usize = MAX_PATCHES * 16;

#[derive(Debug, Default)]
pub(super) struct MeshReader {
    space: String,
    corners: Vec<Vec<f64>>,
    /// The patch's anchors between its corners (`/A`, `/S`): the edge, and a point and its two handles.
    anchors: Vec<(usize, [f64; 6])>,
    pub patches: Vec<GradientMesh>,
    /// The points of [`Self::patches`].
    points: usize,
    /// Something in it this doesn't read.
    pub unread: bool,
}

impl MeshReader {
    /// `value /key X#`; `to_doc`: art space → the document.
    pub fn entry(&mut self, vals: &[V], to_doc: Affine) {
        let Some(at) = vals.iter().rposition(|v| matches!(v, V::Name(_))) else { return };
        let key = vals.get(at).and_then(V::text).unwrap_or_default();
        let value = at.checked_sub(1).and_then(|i| vals.get(i));
        match key.as_str() {
            "CS" => self.space = value.and_then(V::text).unwrap_or_default(),
            "N" => {
                if let Some(V::Arr(a)) = value {
                    if self.corners.len() < 4 {
                        self.corners.push(a.iter().filter_map(V::num).take(64).collect());
                    } else {
                        self.unread = true;
                    }
                }
            }
            "A" | "S" => {
                if let Some(V::Arr(a)) = value {
                    let n: Vec<f64> = a.iter().filter_map(V::num).take(7).collect();
                    match n.as_slice() {
                        &[x, y, ax, ay, bx, by, ..] if self.anchors.len() < 64 && [x, y, ax, ay, bx, by].iter().all(|v| v.is_finite()) => {
                            let p = |x, y| to_doc * Point::new(x, y);
                            let (at, a, b) = (p(x, y), p(ax, ay), p(bx, by));
                            self.anchors.push(((self.corners.len() + 3) % 4, [at.x, at.y, a.x, a.y, b.x, b.y]));
                        }
                        _ => self.unread = true,
                    }
                }
            }
            "E" => {
                let corners = std::mem::take(&mut self.corners);
                let anchors = std::mem::take(&mut self.anchors);
                match self.patch(&corners, &anchors, to_doc) {
                    Some(m) if self.patches.len() < MAX_PATCHES && self.points + m.points.len() <= MAX_POINTS => {
                        self.points += m.points.len();
                        self.patches.push(m);
                    }
                    _ => self.unread = true,
                }
            }
            _ => {}
        }
    }

    fn color(&self, c: &[f64]) -> Option<Color> {
        let u = |i: usize| c.get(i).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32);
        Some(match (self.space.as_str(), c.len()) {
            ("DeviceRGB", 3) => Color::rgb(u(0)?, u(1)?, u(2)?),
            ("DeviceCMYK", 4) => Color::cmyk(u(0)?, u(1)?, u(2)?, u(3)?),
            ("DeviceGray", 1) => Color::gray(1.0 - u(0)?),
            _ => return None,
        })
    }

    fn patch(&self, corners: &[Vec<f64>], anchors: &[(usize, [f64; 6])], to_doc: Affine) -> Option<GradientMesh> {
        let [n0, n1, n2, n3] = corners else { return None };
        // (anchor, towards the next corner, towards the previous one, colour, opacity)
        let read = |n: &Vec<f64>| -> Option<(Point, Vec2, Vec2, Color, f32)> {
            let comps = n.len().checked_sub(9)?;
            let &[opacity, x, y, nx, ny, _, px, py, _] = n.get(comps..)? else { return None };
            if ![opacity, x, y, nx, ny, px, py].iter().all(|v| v.is_finite()) {
                return None;
            }
            let at = to_doc * Point::new(x, y);
            let color = self.color(n.get(..comps)?)?;
            Some((at, to_doc * Point::new(nx, ny) - at, to_doc * Point::new(px, py) - at, color, opacity.clamp(0.0, 1.0) as f32))
        };
        let (a, b, c, d) = (read(n0)?, read(n1)?, read(n2)?, read(n3)?);
        if !anchors.is_empty() {
            return Some(coons([a, b, c, d], anchors));
        }
        // Rows run from the first corner to the second, columns from the first to the fourth.
        let point = |(p, _, _, color, opacity): (Point, Vec2, Vec2, Color, f32), handles: [(usize, Vec2); 2]| {
            let mut h = [Vec2::ZERO; 4];
            for (i, v) in handles {
                if let Some(slot) = h.get_mut(i) {
                    *slot = v;
                }
            }
            MeshPoint { p, color, opacity, handles: h }
        };
        let points = vec![
            point(a, [(H_DOWN, a.1), (H_RIGHT, a.2)]),
            point(d, [(H_LEFT, d.1), (H_DOWN, d.2)]),
            point(b, [(H_RIGHT, b.1), (H_UP, b.2)]),
            point(c, [(H_UP, c.1), (H_LEFT, c.2)]),
        ];
        Some(GradientMesh { rows: 1, cols: 1, points })
    }
}

type Corner = (Point, Vec2, Vec2, Color, f32);
type Cubic = [Point; 4];

/// A patch whose edges bend through more anchors: its edges (first corner → second → third → fourth
/// → first) as runs of cubics, each anchor on the edge it lies along, and the Coons patch they bound
/// as a mesh cut wherever an edge has an anchor, so each cell's edges are single cubics again.
fn coons(corners: [Corner; 4], anchors: &[(usize, [f64; 6])]) -> GradientMesh {
    let runs: Vec<Vec<Cubic>> = (0..4)
        .map(|e| {
            let (p, q) = (&corners[e], &corners[(e + 1) % 4]);
            // Each anchor's first handle points to the one listed before it, back along the first two edges.
            let against = e >= 2;
            let mut stops: Vec<(Point, Point, Point)> = anchors
                .iter()
                .filter(|(edge, ..)| *edge == e)
                .map(|&(_, [x, y, ax, ay, bx, by])| {
                    let (at, h1, h2) = (Point::new(x, y), Point::new(ax, ay), Point::new(bx, by));
                    if against { (h2, at, h1) } else { (h1, at, h2) }
                })
                .collect();
            if against {
                stops.reverse();
            }
            let (mut from, mut out) = (p.0, p.0 + p.1);
            let mut run = vec![];
            for (back, at, ahead) in stops {
                run.push([from, out, back, at]);
                (from, out) = (at, ahead);
            }
            run.push([from, out, q.0 + q.2, q.0]);
            run
        })
        .collect();
    // The patch is over u (first corner → fourth) and v (first → second).
    let [left, bottom, right, top] = [
        Path { run: runs[0].clone(), reversed: false },
        Path { run: runs[1].clone(), reversed: false },
        Path { run: runs[2].clone(), reversed: true },
        Path { run: runs[3].clone(), reversed: true },
    ];
    let [ca, cb, cc, cd] = corners.map(|c| c.0.to_vec2());
    let surface = |u: f64, v: f64| -> Vec2 {
        top.at(u) * (1.0 - v) + bottom.at(u) * v + left.at(v) * (1.0 - u) + right.at(v) * u
            - (ca * ((1.0 - u) * (1.0 - v)) + cd * (u * (1.0 - v)) + cb * ((1.0 - u) * v) + cc * (u * v))
    };
    let du = |u: f64, v: f64, after: bool| -> Vec2 {
        top.slope(u, after) * (1.0 - v) + bottom.slope(u, after) * v - left.at(v) + right.at(v) - (cd - ca) * (1.0 - v) - (cc - cb) * v
    };
    let dv = |u: f64, v: f64, after: bool| -> Vec2 {
        left.slope(v, after) * (1.0 - u) + right.slope(v, after) * u - top.at(u) + bottom.at(u) - (cb - ca) * (1.0 - u) - (cc - cd) * u
    };
    let cuts = |a: &Path, b: &Path| {
        let mut c: Vec<f64> = a.cuts().into_iter().chain(b.cuts()).collect();
        c.sort_by(f64::total_cmp);
        c.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
        c
    };
    let (us, vs) = (cuts(&top, &bottom), cuts(&left, &right));
    let (ra, rb, rc, rd) = (&corners[0], &corners[1], &corners[2], &corners[3]);
    let mut points = Vec::with_capacity(us.len() * vs.len());
    for (r, &v) in vs.iter().enumerate() {
        for (c, &u) in us.iter().enumerate() {
            let mut h = [Vec2::ZERO; 4];
            if let Some(&next) = us.get(c + 1) {
                h[H_RIGHT] = du(u, v, true) * ((next - u) / 3.0);
            }
            if let Some(&prev) = c.checked_sub(1).and_then(|i| us.get(i)) {
                h[H_LEFT] = du(u, v, false) * (-(u - prev) / 3.0);
            }
            if let Some(&next) = vs.get(r + 1) {
                h[H_DOWN] = dv(u, v, true) * ((next - v) / 3.0);
            }
            if let Some(&prev) = r.checked_sub(1).and_then(|i| vs.get(i)) {
                h[H_UP] = dv(u, v, false) * (-(v - prev) / 3.0);
            }
            let (u32_, v32) = (u as f32, v as f32);
            let color = mix(&mix(&ra.3, &rd.3, u32_), &mix(&rb.3, &rc.3, u32_), v32);
            let opacity = (ra.4 * (1.0 - u32_) + rd.4 * u32_) * (1.0 - v32) + (rb.4 * (1.0 - u32_) + rc.4 * u32_) * v32;
            points.push(MeshPoint { p: surface(u, v).to_point(), color, opacity, handles: h });
        }
    }
    GradientMesh { rows: (vs.len() - 1) as u32, cols: (us.len() - 1) as u32, points }
}

/// An edge as a run of cubics, each taking an equal share of its parameter, read backwards if
/// `reversed`.
struct Path {
    run: Vec<Cubic>,
    reversed: bool,
}

impl Path {
    fn piece(&self, t: f64, after: bool) -> (usize, f64) {
        let n = self.run.len();
        let t = if self.reversed { 1.0 - t } else { t };
        // At a joint, the piece the moving direction enters.
        let towards_end = after != self.reversed;
        let scaled = t * n as f64;
        let mut i = if towards_end { scaled.floor() } else { scaled.ceil() - 1.0 }.clamp(0.0, (n - 1) as f64) as usize;
        if i >= n {
            i = n - 1;
        }
        (i, scaled - i as f64)
    }
    fn at(&self, t: f64) -> Vec2 {
        let (i, s) = self.piece(t, true);
        let [p0, p1, p2, p3] = self.run[i].map(|p| p.to_vec2());
        let r = 1.0 - s;
        p0 * (r * r * r) + p1 * (3.0 * r * r * s) + p2 * (3.0 * r * s * s) + p3 * (s * s * s)
    }
    /// The derivative along the patch's parameter, from the piece `after` (towards larger `t`) or before.
    fn slope(&self, t: f64, after: bool) -> Vec2 {
        let (i, s) = self.piece(t, after);
        let [p0, p1, p2, p3] = self.run[i].map(|p| p.to_vec2());
        let r = 1.0 - s;
        let d = (p1 - p0) * (3.0 * r * r) + (p2 - p1) * (6.0 * r * s) + (p3 - p2) * (3.0 * s * s);
        let scale = self.run.len() as f64;
        if self.reversed { d * -scale } else { d * scale }
    }
    /// Where its pieces meet, with its ends, in the patch's parameter.
    fn cuts(&self) -> Vec<f64> {
        let n = self.run.len();
        (0..=n).map(|i| i as f64 / n as f64).collect()
    }
}

/// `a` towards `b` by `t`, in their own colour model where they share one.
fn mix(a: &Color, b: &Color, t: f32) -> Color {
    let l = |x: f32, y: f32| x + (y - x) * t;
    match (a, b) {
        (Color::Rgb { r, g, b: bl }, Color::Rgb { r: r2, g: g2, b: b2 }) => Color::rgb(l(*r, *r2), l(*g, *g2), l(*bl, *b2)),
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => Color::cmyk(l(*c, *c2), l(*m, *m2), l(*y, *y2), l(*k, *k2)),
        (Color::Gray { k }, Color::Gray { k: k2 }) => Color::gray(l(*k, *k2)),
        _ => a.lerp(b, t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn corner(values: &[f64]) -> Vec<V> {
        vec![V::Arr(values.iter().map(|v| V::Num(*v)).collect()), V::Name("N".into())]
    }

    fn patch(reader: &mut MeshReader, first: &[f64]) {
        reader.entry(&corner(first), Affine::IDENTITY);
        for (x, y) in [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0)] {
            reader.entry(&corner(&[0.5, 1.0, x, y, x, y, 1.0, x, y, 1.0]), Affine::IDENTITY);
        }
        reader.entry(&[V::Num(0.0), V::Name("E".into())], Affine::IDENTITY);
    }

    /// An anchor on a patch's edge (`/A` after the first corner: on the edge from it to the second)
    /// bends the edge through it: the patch is cut there into two cells whose shared corner is the
    /// anchor, with the anchor's handles along the edge.
    #[test]
    fn an_edge_anchor_cuts_its_patch_there() {
        let mut reader = MeshReader { space: "DeviceGray".into(), ..MeshReader::default() };
        let corner = |x: f64, y: f64, next: (f64, f64), prev: (f64, f64)| {
            vec![V::Arr([0.0, 1.0, x, y, next.0, next.1, 1.0, prev.0, prev.1, 1.0].map(V::Num).to_vec()), V::Name("N".into())]
        };
        reader.entry(&corner(0.0, 0.0, (0.0, 2.0), (2.0, 0.0)), Affine::IDENTITY);
        reader.entry(&[V::Arr([-1.0, 5.0, -1.0, 4.0, -1.0, 6.0, 1.0].map(V::Num).to_vec()), V::Name("A".into())], Affine::IDENTITY);
        reader.entry(&corner(0.0, 10.0, (2.0, 10.0), (0.0, 8.0)), Affine::IDENTITY);
        reader.entry(&corner(10.0, 10.0, (10.0, 8.0), (8.0, 10.0)), Affine::IDENTITY);
        reader.entry(&corner(10.0, 0.0, (8.0, 0.0), (10.0, 2.0)), Affine::IDENTITY);
        reader.entry(&[V::Num(0.0), V::Name("E".into())], Affine::IDENTITY);
        assert!(!reader.unread);
        let mesh = &reader.patches[0];
        assert_eq!((mesh.rows, mesh.cols), (2, 1));
        let left = &mesh.points[2];
        assert!((left.p - Point::new(-1.0, 5.0)).hypot() < 1e-9, "{:?}", left.p);
        assert!((left.handles[H_UP] - Vec2::new(0.0, -1.0)).hypot() < 1e-9 && (left.handles[H_DOWN] - Vec2::new(0.0, 1.0)).hypot() < 1e-9);
        // The edge still starts and ends at its corners, with their handles.
        assert!((mesh.points[0].handles[H_DOWN] - Vec2::new(0.0, 2.0)).hypot() < 1e-9);
        assert!((mesh.points[4].p - Point::new(0.0, 10.0)).hypot() < 1e-9);
    }

    /// Anchors are listed in the mesh's direction whatever their key: two `/S` anchors after the
    /// first corner run along its edge, as the patch above a shared edge lists them.
    #[test]
    fn anchors_keyed_s_on_a_first_edge_run_along_it() {
        let mut reader = MeshReader { space: "DeviceGray".into(), ..MeshReader::default() };
        let corner = |x: f64, y: f64, next: (f64, f64), prev: (f64, f64)| {
            vec![V::Arr([0.0, 1.0, x, y, next.0, next.1, 1.0, prev.0, prev.1, 1.0].map(V::Num).to_vec()), V::Name("N".into())]
        };
        let anchor = |y: f64| vec![V::Arr([-1.0, y, -1.0, y - 1.0, -1.0, y + 1.0, 1.0].map(V::Num).to_vec()), V::Name("S".into())];
        reader.entry(&corner(0.0, 0.0, (0.0, 1.0), (1.0, 0.0)), Affine::IDENTITY);
        reader.entry(&anchor(3.0), Affine::IDENTITY);
        reader.entry(&anchor(7.0), Affine::IDENTITY);
        reader.entry(&corner(0.0, 10.0, (1.0, 10.0), (0.0, 9.0)), Affine::IDENTITY);
        reader.entry(&corner(10.0, 10.0, (10.0, 9.0), (9.0, 10.0)), Affine::IDENTITY);
        reader.entry(&corner(10.0, 0.0, (9.0, 0.0), (10.0, 1.0)), Affine::IDENTITY);
        reader.entry(&[V::Num(0.0), V::Name("E".into())], Affine::IDENTITY);
        let mesh = &reader.patches[0];
        assert_eq!((mesh.rows, mesh.cols), (3, 1));
        let (first, second) = (&mesh.points[2], &mesh.points[4]);
        assert!(
            (first.p - Point::new(-1.0, 3.0)).hypot() < 1e-9 && (second.p - Point::new(-1.0, 7.0)).hypot() < 1e-9,
            "{:?} {:?}",
            first.p,
            second.p
        );
        assert!((first.handles[H_DOWN] - Vec2::new(0.0, 1.0)).hypot() < 1e-9, "{:?}", first.handles);
    }

    #[test]
    fn hostile_meshes_are_capped_and_checked() {
        let mut reader = MeshReader { space: "DeviceGray".into(), ..MeshReader::default() };
        for _ in 0..=MAX_PATCHES {
            patch(&mut reader, &[0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        }
        assert!(reader.unread && reader.patches.len() == MAX_PATCHES);
        // Patches cut into cells count by their points.
        let mut reader = MeshReader { space: "DeviceGray".into(), points: MAX_POINTS - 3, ..MeshReader::default() };
        patch(&mut reader, &[0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert!(reader.unread && reader.patches.is_empty());
        // NaN passes `clamp`: a non-finite opacity or colour leaves the patch unread.
        for bad in [[0.5, f64::NAN], [f64::INFINITY, 1.0]] {
            let mut reader = MeshReader { space: "DeviceGray".into(), ..MeshReader::default() };
            patch(&mut reader, &[bad[0], bad[1], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
            assert!(reader.unread && reader.patches.is_empty(), "{bad:?}");
        }
        // More corners than a patch has.
        let mut reader = MeshReader { space: "DeviceGray".into(), ..MeshReader::default() };
        for _ in 0..1000 {
            reader.entry(&corner(&[0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]), Affine::IDENTITY);
        }
        assert!(reader.unread && reader.corners.len() == 4);
    }
}
