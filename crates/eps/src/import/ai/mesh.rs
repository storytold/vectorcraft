//! Gradient meshes: `/Mesh X!`, then on `%_` lines `value /key X#` entries — the colour space
//! (`/CS`, see [`Space`]), and per patch its place (`/P [row col]`) and four corners (`/N`, from the corner nearest
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

/// The most patches read from one mesh (files users sent have up to 97 × 72).
const MAX_PATCHES: usize = 16_384;
/// The most points kept from one mesh: a patch with anchors on its edges is cut into cells, up to
/// 34 × 34 points, so the patch cap alone doesn't bound them.
const MAX_POINTS: usize = MAX_PATCHES * 16;

/// The largest grid `/Size` read as one mesh, a side.
const MAX_SIDE: usize = 128;
/// The most inks of an `/NChannel` colour space read.
const MAX_INKS: usize = 32;
/// The most rows or columns a mesh stitched from its patches may have once the lines through the
/// anchors on its patches' edges are added.
const MAX_LINES: usize = 256;

/// A patch as read: its place in the grid (`/P`), its corners and the anchors on its edges.
#[derive(Debug, Clone)]
struct Cell {
    at: Option<(i64, i64)>,
    corners: [Corner; 4],
    anchors: Vec<(usize, [f64; 6])>,
}

/// Why the patches of a mesh don't make one grid.
#[derive(Debug, PartialEq)]
pub(super) enum NotOne {
    /// They don't say where they are in it (no `/Size` or `/P`, or ones that don't add up): read
    /// as separate patches, as files that don't say are.
    Unplaced,
    /// They fill the grid but don't join up (corners or edge anchors that don't meet, or more
    /// lines than are kept).
    Unjoined,
}

/// A mesh's colour space (`/CS`). The files write one only where it changes, before the corner
/// it applies to, even within a patch; a mesh starts grey. Besides the device spaces it may be a
/// named colour, `[c m y k /DeviceCMYK (name) /Process]` (or `/Spot`), or several process or spot
/// inks, `[[ink] [ink] … n /NChannel]` (each ink `[c m y k /DeviceCMYK (name) /Separation]`). A
/// corner gives a named colour's tint (1 is the full colour), or each ink's in order.
#[derive(Debug, Clone, Default, PartialEq)]
enum Space {
    #[default]
    Gray,
    Rgb,
    Cmyk,
    /// The inks' full colours.
    Inks(Vec<Color>),
    /// One this doesn't read: its corners are unread.
    Other,
}

impl Space {
    fn read(value: Option<&V>) -> Space {
        match value {
            Some(v @ (V::Name(_) | V::Str(_))) => match v.text().as_deref() {
                Some("DeviceGray") => Space::Gray,
                Some("DeviceRGB") => Space::Rgb,
                Some("DeviceCMYK") => Space::Cmyk,
                _ => Space::Other,
            },
            Some(V::Arr(a)) => match a.last().and_then(V::text).as_deref() {
                Some("Process" | "Spot") => ink(a).map_or(Space::Other, |c| Space::Inks(vec![c])),
                Some("NChannel") => {
                    let inks: Vec<&Vec<V>> = a.iter().filter_map(|v| if let V::Arr(i) = v { Some(i) } else { None }).collect();
                    let count = a.len().checked_sub(2).and_then(|i| a.get(i)).and_then(V::num);
                    let read: Option<Vec<Color>> = inks.iter().map(|i| ink(i)).collect();
                    match read {
                        Some(read) if !read.is_empty() && read.len() <= MAX_INKS && count == Some(read.len() as f64) => Space::Inks(read),
                        _ => Space::Other,
                    }
                }
                _ => Space::Other,
            },
            _ => Space::Other,
        }
    }

    /// The colour components a corner gives.
    fn components(&self) -> Option<usize> {
        match self {
            Space::Gray => Some(1),
            Space::Rgb => Some(3),
            Space::Cmyk => Some(4),
            Space::Inks(inks) => Some(inks.len()),
            Space::Other => None,
        }
    }

    fn color(&self, c: &[f64]) -> Option<Color> {
        let u = |i: usize| c.get(i).filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32);
        Some(match self {
            Space::Gray => Color::gray(1.0 - u(0)?),
            Space::Rgb => Color::rgb(u(0)?, u(1)?, u(2)?),
            Space::Cmyk => Color::cmyk(u(0)?, u(1)?, u(2)?, u(3)?),
            Space::Inks(inks) => match inks.as_slice() {
                [one] => one.tinted(u(0)?),
                // Several inks overprint: their CMYK at their tints, added up.
                many => {
                    let mut sum = [0.0f32; 4];
                    for (i, ink) in many.iter().enumerate() {
                        let Color::Cmyk { c, m, y, k } = *ink else { return None };
                        let t = u(i)?;
                        for (s, v) in sum.iter_mut().zip([c, m, y, k]) {
                            *s += v * t;
                        }
                    }
                    let [c, m, y, k] = sum.map(|v| v.min(1.0));
                    Color::cmyk(c, m, y, k)
                }
            },
            Space::Other => return None,
        })
    }
}

/// A named colour or ink's full colour: `[c m y k /DeviceCMYK (name) /kind]` or `[r g b /DeviceRGB …]`.
fn ink(a: &[V]) -> Option<Color> {
    let at = a.iter().position(|v| matches!(v, V::Name(_)))?;
    let nums: Vec<f32> = a.get(..at)?.iter().map(|v| v.num().filter(|v| v.is_finite()).map(|v| v.clamp(0.0, 1.0) as f32)).collect::<Option<_>>()?;
    match (a.get(at).and_then(V::text).as_deref(), nums.as_slice()) {
        (Some("DeviceCMYK"), &[c, m, y, k]) => Some(Color::cmyk(c, m, y, k)),
        (Some("DeviceRGB"), &[r, g, b]) => Some(Color::rgb(r, g, b)),
        _ => None,
    }
}

/// A corner as read: its colour in the colour space it was given in, and the numbers after it
/// (opacity, unless the file is older, then the point and its handles).
#[derive(Debug, Clone)]
struct RawCorner {
    color: Option<Color>,
    rest: Vec<f64>,
}

#[derive(Debug, Default)]
pub(super) struct MeshReader {
    space: Space,
    /// `/Type` (`Cartesian` in the files read).
    kind: Option<String>,
    /// `/Size [a b]`: the grid's patches along the first and the second index of `/P`.
    size: Option<(i64, i64)>,
    /// The current patch's `/P [i j]`.
    at: Option<(i64, i64)>,
    /// The patches read, in order, for [`Self::one_mesh`].
    cells: Vec<Cell>,
    corners: Vec<RawCorner>,
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
            "CS" => self.space = Space::read(value),
            "Type" => self.kind = value.and_then(V::text),
            "Size" => self.size = pair(value),
            "P" => self.at = pair(value),
            "N" => {
                if let Some(V::Arr(a)) = value {
                    if self.corners.len() < 4 {
                        // A corner holding anything but numbers is unread: skipping it would shift
                        // the numbers after it into the colour or the opacity.
                        let n: Vec<f64> = a.iter().take(64).map(V::num).collect::<Option<_>>().unwrap_or_default();
                        let comps = self.space.components();
                        let color = comps.and_then(|k| self.space.color(n.get(..k)?));
                        let rest = comps.and_then(|k| n.get(k..)).map(<[f64]>::to_vec).unwrap_or_default();
                        self.corners.push(RawCorner { color, rest });
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
                let at = self.at.take();
                if let Some(c) = self.read_corners(&corners, to_doc)
                    && self.cells.len() < MAX_PATCHES
                {
                    self.cells.push(Cell { at, corners: c, anchors: anchors.clone() });
                }
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

    /// The four corners of a patch, in the document.
    fn read_corners(&self, corners: &[RawCorner], to_doc: Affine) -> Option<[Corner; 4]> {
        let [n0, n1, n2, n3] = corners else { return None };
        // (anchor, towards the next corner, towards the previous one, colour, opacity)
        let read = |n: &RawCorner| -> Option<(Point, Vec2, Vec2, Color, f32)> {
            // Older editing streams omit opacity; the colour space determines the number
            // of colour components, so a coordinate must never be mistaken for opacity.
            let (opacity, geometry) = match n.rest.as_slice() {
                geometry if geometry.len() == 8 => (1.0, geometry),
                [opacity, geometry @ ..] if geometry.len() == 8 => (*opacity, geometry),
                _ => return None,
            };
            let &[x, y, nx, ny, nw, px, py, pw] = geometry else { return None };
            // Weights are 1, or 0 on some corners, which draw as the same cubic edges (checked
            // against the files' pages). Others would need a rational surface.
            if ![nw, pw].iter().all(|w| *w == 1.0 || *w == 0.0) {
                return None;
            }
            if ![opacity, x, y, nx, ny, px, py].iter().all(|v| v.is_finite()) {
                return None;
            }
            let at = to_doc * Point::new(x, y);
            let color = n.color?;
            Some((at, to_doc * Point::new(nx, ny) - at, to_doc * Point::new(px, py) - at, color, opacity.clamp(0.0, 1.0) as f32))
        };
        Some([read(n0)?, read(n1)?, read(n2)?, read(n3)?])
    }

    fn patch(&self, corners: &[RawCorner], anchors: &[(usize, [f64; 6])], to_doc: Affine) -> Option<GradientMesh> {
        let [a, b, c, d] = self.read_corners(corners, to_doc)?;
        if !anchors.is_empty() {
            return Some(coons([a, b, c, d], anchors, None));
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

impl MeshReader {
    /// The mesh as one grid of shared points: each patch placed by its `/P` in a grid of `/Size`,
    /// the corners neighbours share joined, and where a patch's edge bends through anchors, a mesh
    /// line through the whole row or column there (every patch in it cut at the same place), so
    /// each cell's edges stay single cubics. In each patch, rows run from its first corner to its
    /// second and columns from its first to its fourth; which index of `/P` is which is taken from
    /// the one under which the shared corners meet.
    pub fn one_mesh(&self) -> Result<GradientMesh, NotOne> {
        if self.unread || self.kind.as_deref().is_some_and(|k| k != "Cartesian") {
            return Err(NotOne::Unplaced);
        }
        let Some((a, b)) = self.size else { return Err(NotOne::Unplaced) };
        let side = |n: i64| usize::try_from(n).ok().filter(|n| (1..=MAX_SIDE).contains(n));
        let (Some(a), Some(b)) = (side(a), side(b)) else { return Err(NotOne::Unplaced) };
        if self.cells.len() != a * b {
            return Err(NotOne::Unplaced);
        }
        // Each patch's place, once each.
        let mut seen = vec![false; a * b];
        let mut places = Vec::with_capacity(self.cells.len());
        for cell in &self.cells {
            let Some((i, j)) = cell.at else { return Err(NotOne::Unplaced) };
            let (Ok(i), Ok(j)) = (usize::try_from(i), usize::try_from(j)) else { return Err(NotOne::Unplaced) };
            if i >= a || j >= b {
                return Err(NotOne::Unplaced);
            }
            let Some(slot) = seen.get_mut(i * b + j) else { return Err(NotOne::Unplaced) };
            if std::mem::replace(slot, true) {
                return Err(NotOne::Unplaced);
            }
            places.push((i, j));
        }
        // The first index along the columns (`/P [i j]` at row j, column i), or along the rows.
        for across in [true, false] {
            let (rows, cols) = if across { (b, a) } else { (a, b) };
            let at: Vec<(usize, usize)> = places.iter().map(|&(i, j)| if across { (j, i) } else { (i, j) }).collect();
            if let Some(mesh) = stitch(&self.cells, &at, rows, cols) {
                return Ok(mesh);
            }
        }
        Err(NotOne::Unjoined)
    }
}

/// The cuts (0 to 1) of a patch's columns and rows: where the anchors on its edges along each
/// direction cut it, the pieces of an edge taking equal shares (as [`coons`] lays them out).
fn own_cuts(cell: &Cell) -> (Vec<f64>, Vec<f64>) {
    let pieces = |e: usize| cell.anchors.iter().filter(|(edge, ..)| *edge == e).count() + 1;
    let cuts = |n: usize| (0..=n).map(|i| i as f64 / n as f64).collect::<Vec<_>>();
    // Columns along the second and fourth edges (u), rows along the first and third (v).
    let merge = |x: Vec<f64>, y: Vec<f64>| union(&x, &y);
    (merge(cuts(pieces(1)), cuts(pieces(3))), merge(cuts(pieces(0)), cuts(pieces(2))))
}

/// The sorted union of two lists of cuts.
fn union(a: &[f64], b: &[f64]) -> Vec<f64> {
    let mut c: Vec<f64> = a.iter().chain(b).copied().filter(|v| v.is_finite()).collect();
    c.sort_by(f64::total_cmp);
    c.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    c
}

/// The patches `cells`, each at `at` (row, column) of a `rows` × `cols` grid, as one mesh; `None`
/// when their shared points don't meet, a value isn't finite, or the mesh would be too large.
fn stitch(cells: &[Cell], at: &[(usize, usize)], rows: usize, cols: usize) -> Option<GradientMesh> {
    // The cuts each row and column of patches takes: those of every patch in it.
    let (mut col_cuts, mut row_cuts) = (vec![vec![0.0, 1.0]; cols], vec![vec![0.0, 1.0]; rows]);
    for (cell, &(r, c)) in cells.iter().zip(at) {
        let (us, vs) = own_cuts(cell);
        let cc = col_cuts.get_mut(c)?;
        *cc = union(cc, &us);
        let rc = row_cuts.get_mut(r)?;
        *rc = union(rc, &vs);
    }
    // Where each row and column of patches starts among the mesh's lines.
    let starts = |cuts: &[Vec<f64>]| -> Vec<usize> {
        cuts.iter()
            .scan(0usize, |n, c| {
                let s = *n;
                *n += c.len().saturating_sub(1);
                Some(s)
            })
            .collect()
    };
    let (col_at, row_at) = (starts(&col_cuts), starts(&row_cuts));
    let total = |cuts: &[Vec<f64>]| cuts.iter().map(|c| c.len().saturating_sub(1)).sum::<usize>();
    let (mesh_cols, mesh_rows) = (total(&col_cuts), total(&row_cuts));
    if mesh_cols == 0 || mesh_rows == 0 || mesh_cols > MAX_LINES || mesh_rows > MAX_LINES || (mesh_rows + 1) * (mesh_cols + 1) > MAX_POINTS {
        return None;
    }
    let width = mesh_cols + 1;
    let mut points: Vec<Option<MeshPoint>> = vec![None; (mesh_rows + 1) * width];
    for (cell, &(r, c)) in cells.iter().zip(at) {
        let (us, vs) = (col_cuts.get(c)?, row_cuts.get(r)?);
        let part = coons(cell.corners, &cell.anchors, Some((us, vs)));
        let part_width = part.cols as usize + 1;
        for (k, q) in part.points.into_iter().enumerate() {
            let (pr, pc) = (k / part_width, k % part_width);
            let slot = points.get_mut((row_at.get(r)? + pr) * width + col_at.get(c)? + pc)?;
            match slot {
                None => *slot = Some(q),
                Some(had) => {
                    // A point two patches share: where both have it.
                    if (had.p - q.p).hypot() > 1e-6 * (1.0 + had.p.to_vec2().hypot()) {
                        return None;
                    }
                    // Its handles towards each side come from the patch on that side.
                    for (h, n) in had.handles.iter_mut().zip(q.handles) {
                        if h.hypot() == 0.0 {
                            *h = n;
                        }
                    }
                }
            }
        }
    }
    let points: Vec<MeshPoint> = points.into_iter().collect::<Option<_>>()?;
    let finite = |v: f64| v.is_finite();
    if !points.iter().all(|q| finite(q.p.x) && finite(q.p.y) && q.opacity.is_finite() && q.handles.iter().all(|h| finite(h.x) && finite(h.y))) {
        return None;
    }
    Some(GradientMesh { rows: mesh_rows as u32, cols: mesh_cols as u32, points })
}

/// A two-number value (`[a b]`) as whole numbers.
fn pair(v: Option<&V>) -> Option<(i64, i64)> {
    let V::Arr(a) = v? else { return None };
    let n: Vec<f64> = a.iter().filter_map(V::num).collect();
    match n.as_slice() {
        &[x, y] if x.fract() == 0.0 && y.fract() == 0.0 && x.abs() < 1e6 && y.abs() < 1e6 => Some((x as i64, y as i64)),
        _ => None,
    }
}

type Corner = (Point, Vec2, Vec2, Color, f32);
type Cubic = [Point; 4];

/// A patch whose edges bend through more anchors: its edges (first corner → second → third → fourth
/// → first) as runs of cubics, each anchor on the edge it lies along, and the Coons patch they bound
/// as a mesh cut wherever an edge has an anchor, so each cell's edges are single cubics again.
///
/// `cuts`: the columns and rows (0 to 1) to cut it at instead of its own, which they include: those
/// of the mesh's whole column and row of patches ([`stitch`]).
fn coons(corners: [Corner; 4], anchors: &[(usize, [f64; 6])], cuts_at: Option<(&[f64], &[f64])>) -> GradientMesh {
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
    let (us, vs) = match cuts_at {
        Some((us, vs)) => (us.to_vec(), vs.to_vec()),
        None => (cuts(&top, &bottom), cuts(&left, &right)),
    };
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
        let mut reader = MeshReader::default();
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
        let mut reader = MeshReader::default();
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

    /// A corner's handle weights are 1, or 0 on some corners, which draw as the same patch; any
    /// other weight leaves the patch unread.
    #[test]
    fn corners_with_handle_weights_of_zero_are_read() {
        let read = |nw: f64, pw: f64| {
            let mut reader = MeshReader::default();
            patch(&mut reader, &[0.5, 1.0, 0.0, 0.0, 0.0, 0.3, nw, 0.3, 0.0, pw]);
            reader
        };
        for (nw, pw) in [(0.0, 0.0), (0.0, 1.0), (1.0, 0.0)] {
            let reader = read(nw, pw);
            assert!(!reader.unread, "{nw} {pw}");
            let p = &reader.patches[0].points[0];
            assert_eq!((p.handles[H_DOWN], p.handles[H_RIGHT]), (Vec2::new(0.0, 0.3), Vec2::new(0.3, 0.0)));
        }
        let reader = read(0.5, 1.0);
        assert!(reader.unread && reader.patches.is_empty());
    }

    #[test]
    fn hostile_meshes_are_capped_and_checked() {
        let mut reader = MeshReader::default();
        for _ in 0..=MAX_PATCHES {
            patch(&mut reader, &[0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        }
        assert!(reader.unread && reader.patches.len() == MAX_PATCHES);
        // Patches cut into cells count by their points.
        let mut reader = MeshReader { points: MAX_POINTS - 3, ..MeshReader::default() };
        patch(&mut reader, &[0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
        assert!(reader.unread && reader.patches.is_empty());
        // NaN passes `clamp`: a non-finite opacity or colour leaves the patch unread.
        for bad in [[0.5, f64::NAN], [f64::INFINITY, 1.0]] {
            let mut reader = MeshReader::default();
            patch(&mut reader, &[bad[0], bad[1], 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]);
            assert!(reader.unread && reader.patches.is_empty(), "{bad:?}");
        }
        // More corners than a patch has.
        let mut reader = MeshReader::default();
        for _ in 0..1000 {
            reader.entry(&corner(&[0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0]), Affine::IDENTITY);
        }
        assert!(reader.unread && reader.corners.len() == 4);
    }

    /// Hand-written grid of grey patches: `/Type`, `/Size [cols rows]` and each patch at `/P [x y]`
    /// (the first index along x, as the files read write it), its corners from the one nearest the
    /// origin up, across and down (`a b c d`), straight edges with handles a third of the way, the
    /// grey of a point `at(x, y)`. `moved` puts the points listed elsewhere; `anchor` adds an anchor
    /// to the edges at x = 10 (between the first two columns), at height 5 and 1 to the right.
    struct Grid {
        cols: usize,
        rows: usize,
        moved: Vec<((f64, f64), (f64, f64))>,
        anchor: bool,
        place: bool,
    }

    impl Grid {
        fn new(cols: usize, rows: usize) -> Self {
            Grid { cols, rows, moved: vec![], anchor: false, place: true }
        }
        fn at(&self, x: f64, y: f64) -> (f64, f64) {
            self.moved.iter().find(|(from, _)| *from == (x, y)).map_or((x, y), |(_, to)| *to)
        }
        fn read(&self, size: [f64; 2]) -> MeshReader {
            let mut r = MeshReader::default();
            let name = |n: &str| V::Name(n.into());
            let arr = |v: &[f64]| V::Arr(v.iter().map(|x| V::Num(*x)).collect());
            r.entry(&[name("DeviceGray"), name("CS")], Affine::IDENTITY);
            r.entry(&[name("Cartesian"), name("Type")], Affine::IDENTITY);
            r.entry(&[arr(&size), name("Size")], Affine::IDENTITY);
            for i in 0..self.cols {
                for j in 0..self.rows {
                    let (x0, y0, x1, y1) = (i as f64 * 10.0, j as f64 * 10.0, (i + 1) as f64 * 10.0, (j + 1) as f64 * 10.0);
                    if self.place {
                        r.entry(&[arr(&[i as f64, j as f64]), name("P")], Affine::IDENTITY);
                    }
                    let third = |p: (f64, f64), q: (f64, f64)| (p.0 + (q.0 - p.0) / 3.0, p.1 + (q.1 - p.1) / 3.0);
                    let (a, b, c, d) = (self.at(x0, y0), self.at(x0, y1), self.at(x1, y1), self.at(x1, y0));
                    let grey = |p: (f64, f64)| (p.0 + p.1) / 40.0;
                    for (k, (p, next, prev)) in [(a, b, d), (b, c, a), (c, d, b), (d, a, c)].into_iter().enumerate() {
                        let (n, q) = (third(p, next), third(p, prev));
                        r.entry(&[arr(&[grey(p), 1.0, p.0, p.1, n.0, n.1, 1.0, q.0, q.1, 1.0]), name("N")], Affine::IDENTITY);
                        // The edge at x = 10: the third edge (c → d) of the first column, the first
                        // (a → b) of the second; listed upwards either way.
                        let on_shared = (k == 2 && i == 0) || (k == 0 && i == 1);
                        if self.anchor && on_shared {
                            r.entry(&[arr(&[11.0, 5.0, 11.0, 3.0, 11.0, 7.0, 1.0]), name("A")], Affine::IDENTITY);
                        }
                    }
                    r.entry(&[V::Num(0.0), name("E")], Affine::IDENTITY);
                }
            }
            r
        }
    }

    /// A 2 × 2 mesh comes in as one mesh of 9 shared points, each where the patches put it, with
    /// the colours the separate patches had at every place.
    #[test]
    fn a_grid_of_patches_is_one_mesh() {
        let g = Grid::new(2, 2);
        let r = g.read([2.0, 2.0]);
        let mesh = r.one_mesh().unwrap();
        assert_eq!((mesh.rows, mesh.cols, mesh.points.len()), (2, 2, 9));
        for row in 0..=2 {
            for col in 0..=2 {
                let p = mesh.points[mesh.idx(row, col)].p;
                assert!((p - Point::new(col as f64 * 10.0, row as f64 * 10.0)).hypot() < 1e-9, "({row}, {col}) at {p:?}");
            }
        }
        // The colour across each cell is that of the patch it was (patches are read x first).
        for (k, patch) in r.patches.iter().enumerate() {
            let (col, row) = (k / 2, k % 2);
            for (u, v) in [(0.25, 0.25), (0.5, 0.5), (0.9, 0.1)] {
                let (one, _) = mesh.color_at(row, col, u, v);
                let (own, _) = patch.color_at(0, 0, u, v);
                assert!(
                    one.to_rgb().iter().zip(own.to_rgb()).all(|(a, b)| (a - b).abs() < 1e-5),
                    "cell ({row}, {col}) at {u} {v}: {one:?} vs {own:?}"
                );
            }
        }
        // Inner handles point at the neighbours, a third of the way.
        let inner = &mesh.points[mesh.idx(1, 1)];
        assert!((inner.handles[H_RIGHT] - Vec2::new(10.0 / 3.0, 0.0)).hypot() < 1e-9);
        assert!((inner.handles[H_UP] - Vec2::new(0.0, -10.0 / 3.0)).hypot() < 1e-9);
    }

    /// A mesh point moved inside the grid is one point of the mesh, which the four cells round it
    /// share: moving it moves them all, leaving no hole.
    #[test]
    fn a_moved_inner_point_stays_shared() {
        let mut g = Grid::new(2, 2);
        g.moved = vec![((10.0, 10.0), (13.0, 8.0))];
        let mut mesh = g.read([2.0, 2.0]).one_mesh().unwrap();
        let k = mesh.idx(1, 1);
        assert!((mesh.points[k].p - Point::new(13.0, 8.0)).hypot() < 1e-9);
        mesh.points[k].p = Point::new(15.0, 15.0);
        for (r, c) in [(0, 0), (0, 1), (1, 0), (1, 1)] {
            let corner = mesh.eval(r, c, 1.0 - c as f64, 1.0 - r as f64);
            assert!((corner - Point::new(15.0, 15.0)).hypot() < 1e-9, "cell ({r}, {c}): {corner:?}");
        }
    }

    /// An anchor on the edge two patches share runs a mesh line through the whole mesh there: one
    /// mesh, three points across that edge, the anchor's the middle one, its handles along the edge.
    #[test]
    fn an_anchor_on_a_shared_edge_adds_a_line_through_the_mesh() {
        let mut g = Grid::new(2, 1);
        g.anchor = true;
        let r = g.read([2.0, 1.0]);
        let mesh = r.one_mesh().unwrap();
        assert_eq!((mesh.rows, mesh.cols), (2, 2), "split at the anchor's height");
        let mid = &mesh.points[mesh.idx(1, 1)];
        assert!((mid.p - Point::new(11.0, 5.0)).hypot() < 1e-9, "{:?}", mid.p);
        assert!(
            (mid.handles[H_UP] - Vec2::new(0.0, -2.0)).hypot() < 1e-9 && (mid.handles[H_DOWN] - Vec2::new(0.0, 2.0)).hypot() < 1e-9,
            "{:?}",
            mid.handles
        );
        // The patch without an anchor of its own is cut on the same line, on its straight edge.
        let left = mesh.points[mesh.idx(1, 0)].p;
        assert!((left - Point::new(0.0, 5.0)).hypot() < 1e-9, "{left:?}");
    }

    /// A patch whose first corner and the anchors on its edges sit on one point (no handles)
    /// comes in, every value a number.
    #[test]
    fn a_collapsed_patch_comes_in_with_finite_values() {
        let mut r = MeshReader::default();
        let name = |n: &str| V::Name(n.into());
        let arr = |v: &[f64]| V::Arr(v.iter().map(|x| V::Num(*x)).collect());
        r.entry(&[name("DeviceGray"), name("CS")], Affine::IDENTITY);
        r.entry(&[arr(&[1.0, 1.0]), name("Size")], Affine::IDENTITY);
        r.entry(&[arr(&[0.0, 0.0]), name("P")], Affine::IDENTITY);
        // An anchor on the edge back to the first corner, before it; then the corner on that point.
        r.entry(&[arr(&[5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 1.0]), name("A")], Affine::IDENTITY);
        r.entry(&[arr(&[0.5, 1.0, 5.0, 5.0, 5.0, 5.0, 1.0, 5.0, 5.0, 1.0]), name("N")], Affine::IDENTITY);
        r.entry(&[arr(&[5.0, 5.0, 5.0, 5.0, 5.0, 5.0, 1.0]), name("A")], Affine::IDENTITY);
        for (x, y) in [(5.0, 10.0), (10.0, 10.0), (10.0, 5.0)] {
            r.entry(&[arr(&[0.2, 1.0, x, y, x, y, 1.0, x, y, 1.0]), name("N")], Affine::IDENTITY);
        }
        r.entry(&[V::Num(0.0), name("E")], Affine::IDENTITY);
        let mesh = r.one_mesh().unwrap();
        assert!(mesh.points.iter().all(|q| q.p.x.is_finite() && q.p.y.is_finite() && q.handles.iter().all(|h| h.x.is_finite() && h.y.is_finite())));
    }

    /// Patches that don't say where they are, or not as many as the grid has, are read as before:
    /// each a mesh of its own. Ones that fill the grid but don't meet say so.
    #[test]
    fn unplaced_or_unjoined_patches_stay_separate() {
        let mut g = Grid::new(2, 2);
        g.place = false;
        assert_eq!(g.read([2.0, 2.0]).one_mesh().unwrap_err(), NotOne::Unplaced);
        let g = Grid::new(2, 2);
        assert_eq!(g.read([3.0, 2.0]).one_mesh().unwrap_err(), NotOne::Unplaced);
        assert_eq!(g.read([f64::NAN, 2.0]).one_mesh().unwrap_err(), NotOne::Unplaced);
        assert_eq!(g.read([2.0, 2.0]).patches.len(), 4, "the separate patches are still read");
        // A corner that only one of the patches sharing it moved.
        let mut r = Grid::new(2, 1).read([2.0, 1.0]);
        if let Some(c) = r.cells.get_mut(1) {
            c.corners[0].0 = Point::new(10.0, 1.0);
        }
        assert_eq!(r.one_mesh().unwrap_err(), NotOne::Unjoined);
    }

    fn nums(v: &[f64]) -> V {
        V::Arr(v.iter().map(|x| V::Num(*x)).collect())
    }

    /// One unit-square patch: each corner's colour components, then (as version 1 meshes write,
    /// without opacity) its point and handles at the point; `before[i]`: entries before corner `i`.
    fn square(reader: &mut MeshReader, colors: [&[f64]; 4], before: [Vec<(V, &str)>; 4]) {
        let at = [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)];
        for ((color, (x, y)), entries) in colors.into_iter().zip(at).zip(before) {
            for (value, key) in entries {
                reader.entry(&[value, V::Name(key.into())], Affine::IDENTITY);
            }
            let corner: Vec<f64> = color.iter().copied().chain([x, y, x, y, 1.0, x, y, 1.0]).collect();
            reader.entry(&[nums(&corner), V::Name("N".into())], Affine::IDENTITY);
        }
        reader.entry(&[V::Num(0.0), V::Name("E".into())], Affine::IDENTITY);
    }

    fn assert_color(got: Color, want: Color) {
        let (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) = (got, want) else {
            return assert_eq!(got, want);
        };
        assert!([c - c2, m - m2, y - y2, k - k2].iter().all(|d| d.abs() < 1e-6), "{got:?} != {want:?}");
    }

    /// The files write `/CS` only where the colour space changes, before the corner it applies to,
    /// even within a patch, and a mesh starts grey: a corner before any `/CS` is one grey
    /// component, and one after a `/CS` takes that space. (Patch points: first corner, fourth,
    /// second, third.)
    #[test]
    fn corners_take_the_colour_space_last_set_before_them() {
        let cs = |n: &str| vec![(V::Name(n.into()), "CS")];
        let mut reader = MeshReader::default();
        square(&mut reader, [&[0.25], &[1.0, 0.0, 0.0], &[0.75], &[0.0, 0.0, 1.0]], [vec![], cs("DeviceRGB"), cs("DeviceGray"), cs("DeviceRGB")]);
        assert!(!reader.unread);
        let colors: Vec<Color> = reader.patches[0].points.iter().map(|p| p.color).collect();
        assert_eq!(colors, [Color::gray(0.75), Color::rgb(0.0, 0.0, 1.0), Color::rgb(1.0, 0.0, 0.0), Color::gray(0.25)]);
        // A mesh with no `/CS` at all is grey.
        let mut reader = MeshReader::default();
        square(&mut reader, [&[0.0], &[1.0], &[1.0], &[0.0]], Default::default());
        assert!(!reader.unread && reader.patches[0].points[0].color == Color::gray(1.0));
    }

    /// A named colour (`[c m y k /DeviceCMYK (name) /Process]`) paints at each corner's tint, 1 its
    /// full colour; `/NChannel` inks overprint at their tints. Ones that don't add up stay unread.
    #[test]
    fn named_colours_and_inks_paint_at_their_tints() {
        let ink = |c: [f64; 4], name: &str, kind: &str| {
            let mut v: Vec<V> = c.iter().map(|x| V::Num(*x)).collect();
            v.extend([V::Name("DeviceCMYK".into()), V::Str(name.as_bytes().to_vec()), V::Name(kind.into())]);
            V::Arr(v)
        };
        let rose = ink([0.1, 0.6, 0.4, 0.2], "Rose", "Process");
        let mut reader = MeshReader::default();
        // Version 2 corners: tint, then opacity.
        square(&mut reader, [&[1.0, 1.0], &[0.5, 1.0], &[1.0, 1.0], &[1.0, 1.0]], [vec![(rose, "CS")], vec![], vec![], vec![]]);
        assert!(!reader.unread);
        let points = &reader.patches[0].points;
        assert_color(points[0].color, Color::cmyk(0.1, 0.6, 0.4, 0.2));
        assert_color(points[2].color, Color::cmyk(0.05, 0.3, 0.2, 0.1));

        let channels = |n: f64| {
            V::Arr(vec![
                ink([0.0, 0.0, 1.0, 0.0], "Yellow", "Separation"),
                ink([0.0, 1.0, 0.0, 0.0], "Magenta", "Separation"),
                V::Num(n),
                V::Name("NChannel".into()),
            ])
        };
        let mut reader = MeshReader::default();
        square(&mut reader, [&[0.25, 1.0], &[0.0, 0.0], &[1.0, 1.0], &[0.0, 0.0]], [vec![(channels(2.0), "CS")], vec![], vec![], vec![]]);
        assert!(!reader.unread);
        assert_color(reader.patches[0].points[0].color, Color::cmyk(0.0, 1.0, 0.25, 0.0));
        assert_color(reader.patches[0].points[3].color, Color::cmyk(0.0, 1.0, 1.0, 0.0));

        // An ink count that doesn't match its inks, or an ink in a space not read.
        let lab = V::Arr(vec![V::Num(50.0), V::Num(0.0), V::Num(0.0), V::Name("Lab".into()), V::Str(b"Grey".to_vec()), V::Name("Spot".into())]);
        for space in [channels(3.0), lab] {
            let mut reader = MeshReader::default();
            square(&mut reader, [&[1.0, 1.0]; 4], [vec![(space, "CS")], vec![], vec![], vec![]]);
            assert!(reader.unread && reader.patches.is_empty());
        }
    }

    /// A corner holding anything but numbers is unread: skipping it would shift the numbers after
    /// it into its colour and opacity (two inks' tints `0.25 /bad 0.5`, read as `0.25 0.5` and no
    /// opacity).
    #[test]
    fn a_corner_with_something_other_than_numbers_is_unread() {
        let ink = |c: [f64; 4], name: &str| {
            let mut v: Vec<V> = c.iter().map(|x| V::Num(*x)).collect();
            v.extend([V::Name("DeviceCMYK".into()), V::Str(name.as_bytes().to_vec()), V::Name("Separation".into())]);
            V::Arr(v)
        };
        let channels =
            V::Arr(vec![ink([0.0, 0.0, 1.0, 0.0], "Yellow"), ink([0.0, 1.0, 0.0, 0.0], "Magenta"), V::Num(2.0), V::Name("NChannel".into())]);
        let mut reader = MeshReader::default();
        reader.entry(&[channels, V::Name("CS".into())], Affine::IDENTITY);
        for (x, y) in [(0.0, 0.0), (0.0, 1.0), (1.0, 1.0), (1.0, 0.0)] {
            let mut corner = vec![V::Num(0.25), V::Name("bad".into()), V::Num(0.5)];
            corner.extend([x, y, x, y, 1.0, x, y, 1.0].map(V::Num));
            reader.entry(&[V::Arr(corner), V::Name("N".into())], Affine::IDENTITY);
        }
        reader.entry(&[V::Num(0.0), V::Name("E".into())], Affine::IDENTITY);
        assert!(reader.unread && reader.patches.is_empty(), "{:?}", reader.patches);
    }

    /// Meshes as large as files users sent (97 × 72 patches) are read, as one mesh.
    #[test]
    fn a_large_grid_is_read_as_one_mesh() {
        let r = Grid::new(97, 72).read([97.0, 72.0]);
        assert!(!r.unread);
        let mesh = r.one_mesh().unwrap();
        assert_eq!((mesh.rows.min(mesh.cols), mesh.rows.max(mesh.cols)), (72, 97));
    }
}
