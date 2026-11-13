use super::{Anchor, BezPath, CubicBez, PI, ParamCurve, PathData, Point, SubPath};

// =====================================================================================
// Warp maps (shared with vectorcraft-effects' Warp effects)
// =====================================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WarpStyle {
    Arc,
    ArcLower,
    ArcUpper,
    Arch,
    Bulge,
    ShellLower,
    ShellUpper,
    Flag,
    Wave,
    Fish,
    Rise,
    Fisheye,
    Inflate,
    Squeeze,
    Twist,
}

impl WarpStyle {
    /// From the id suffix (`arc`, `arcLower`, …).
    pub fn from_id(s: &str) -> Option<Self> {
        use WarpStyle::*;
        Some(match s {
            "arc" => Arc,
            "arcLower" => ArcLower,
            "arcUpper" => ArcUpper,
            "arch" => Arch,
            "bulge" => Bulge,
            "shellLower" => ShellLower,
            "shellUpper" => ShellUpper,
            "flag" => Flag,
            "wave" => Wave,
            "fish" => Fish,
            "rise" => Rise,
            "fisheye" => Fisheye,
            "inflate" => Inflate,
            "squeeze" => Squeeze,
            "twist" => Twist,
            _ => return None,
        })
    }
}

/// The warp map on normalised coordinates: `b` = bend (-1..1), `dh`/`dv` = distortion (-1..1);
/// `aspect` is the box's half-height over its half-width along the warp (Arc bends in real
/// proportions).
pub fn warp_point(style: WarpStyle, b: f64, dh: f64, dv: f64, aspect: f64, x: f64, y: f64) -> (f64, f64) {
    use WarpStyle::*;
    let t = (y + 1.0) / 2.0; // 0 at top, 1 at bottom
    let par = 1.0 - x * x; // parabola: 1 at centre, 0 at sides
    let (mut x2, mut y2) = match style {
        Arc => {
            if b.abs() < 1e-6 {
                (x, y)
            } else {
                // Bend the centre line into a circular arc of equal length (sweep = |b|·180°), in
                // the box's real proportions (lengths in half-widths, so y counts `aspect` each):
                // the edges are concentric arcs, and one that would pass the centre stops there
                // instead of folding over (#1578).
                let k = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
                let sweep = b.abs() * PI;
                let r0 = 2.0 / sweep;
                let yy = if b > 0.0 { y } else { -y } * k;
                let a = x / r0;
                let r = (r0 - yy).max(0.0);
                let (s, c) = a.sin_cos();
                let (nx, ny) = (r * s, (r0 - r * c) / k);
                (nx, if b > 0.0 { ny } else { -ny })
            }
        }
        ArcLower => (x, y + b * par * t),
        ArcUpper => (x, y - b * par * (1.0 - t)),
        Arch => (x, y - b * par),
        Bulge => (x, y + b * par * y),
        ShellLower => (x * (1.0 - 0.5 * b * (1.0 - t)), y + b * par * t),
        ShellUpper => (x * (1.0 - 0.5 * b * t), y - b * par * (1.0 - t)),
        Flag => (x, y + 0.5 * b * (PI * x).sin()),
        Wave => (x + 0.1 * b * (PI * y).sin(), y + 0.3 * b * (2.0 * PI * x).sin() * (0.5 + 0.5 * t)),
        Fish => (x, y * (1.0 + 0.5 * b * (0.75 * PI * (x + 1.0)).sin())),
        Rise => (x, y - b * ((x + 1.0) / 2.0).powi(2) * 2.0 + b),
        Fisheye => {
            let r2 = x * x + y * y;
            let s = if r2 < 1.0 { 1.0 + 0.5 * b * (1.0 - r2) } else { 1.0 };
            (x * s, y * s)
        }
        Inflate => (x * (1.0 + 0.5 * b * (1.0 - y * y)), y * (1.0 + 0.5 * b * par)),
        Squeeze => (x * (1.0 - 0.5 * b * (1.0 - y * y)), y * (1.0 + 0.3 * b * par)),
        Twist => {
            let r = (x * x + y * y).sqrt();
            let a = -b * PI * 0.5 * (1.0 - r / std::f64::consts::SQRT_2).max(0.0);
            let (s, c) = a.sin_cos();
            (x * c - y * s, x * s + y * c)
        }
    };
    // Perspective-like distortion: horizontal narrows one side, vertical one end.
    if dh != 0.0 {
        y2 *= (1.0 + dh * x2 * 0.5).max(0.0);
    }
    if dv != 0.0 {
        x2 *= (1.0 + dv * y2 * 0.5).max(0.0);
    }
    (x2, y2)
}

// =====================================================================================
// Warp nets: each style as one Bézier patch over its box
// =====================================================================================

/// The `version` a Warp effect's params or a warp envelope carries for the [`WarpNet`] map. With
/// none (or 0) a warp draws with [`warp_point`], as every warp did before, so saved documents keep
/// their look; new and imported warps are given this one.
pub const WARP_NET: u32 = 1;

/// Does a warp of map `version` draw with [`WarpNet`]? A version newer than this build knows draws
/// with the newest map it has.
pub fn uses_net(version: u32) -> bool {
    version >= WARP_NET
}

/// The map version `v` names when it is one this build draws: a whole number from 0 to
/// [`WARP_NET`], written as an integer or as a float (`1` or `1.0`).
pub fn warp_version(v: &serde_json::Value) -> Option<u32> {
    let n = v.as_f64()?;
    (n.fract() == 0.0 && (0.0..=f64::from(WARP_NET)).contains(&n)).then_some(n as u32)
}

/// A style's own control net: `rows` × `cols` points (2 to 4 each) of a Bézier patch, row by row
/// from the top, in the box's coordinates (origin at its top left, y down).
#[derive(Clone, Copy, Debug)]
struct Net {
    rows: usize,
    cols: usize,
    p: [[Point; 4]; 4],
}

impl Net {
    /// The undistorted box `w` × `h` as a `rows` × `cols` net.
    fn flat(w: f64, h: f64, rows: usize, cols: usize) -> Self {
        let mut p = [[Point::ZERO; 4]; 4];
        for (r, row) in p.iter_mut().enumerate().take(rows) {
            for (c, q) in row.iter_mut().enumerate().take(cols) {
                *q = Point::new(w * (c as f64 / (cols - 1) as f64), h * (r as f64 / (rows - 1) as f64));
            }
        }
        Self { rows, cols, p }
    }

    /// Upside down within a box `h` high: the rows in reverse order, each point mirrored.
    fn mirrored(self, h: f64) -> Self {
        let mut out = self;
        for (to, from) in out.p[..self.rows].iter_mut().zip(self.p[..self.rows].iter().rev()) {
            for (q, f) in to.iter_mut().zip(from).take(self.cols) {
                *q = Point::new(f.x, h - f.y);
            }
        }
        out
    }

    /// Rows and columns exchanged, and x with y: a net made on the box turned on its side.
    fn transposed(self) -> Self {
        let mut out = Self { rows: self.cols, cols: self.rows, p: [[Point::ZERO; 4]; 4] };
        for (r, row) in self.p[..self.rows].iter().enumerate() {
            for (c, q) in row[..self.cols].iter().enumerate() {
                out.p[c][r] = Point::new(q.y, q.x);
            }
        }
        out
    }

    /// Each column scaled about the midpoint of its end points by `1 + a (2t - 1)`, `t` running from
    /// 0 at the left column to 1 at the right: a horizontal distortion `a` (-1..1).
    fn distort_columns(&mut self, a: f64) {
        let (rows, cols) = (self.rows, self.cols);
        for c in 0..cols {
            let s = 1.0 + a * (2.0 * c as f64 / (cols - 1) as f64 - 1.0);
            let mid = half_way(self.p[0][c], self.p[rows - 1][c]);
            for row in &mut self.p[..rows] {
                row[c] = mid + (row[c] - mid) * s;
            }
        }
    }

    /// Each row scaled about the midpoint of its end points by `1 + a (2t - 1)`, `t` running from 0
    /// at the top row to 1 at the bottom: a vertical distortion `a` (-1..1).
    fn distort_rows(&mut self, a: f64) {
        let (rows, cols) = (self.rows, self.cols);
        for (r, row) in self.p[..rows].iter_mut().enumerate() {
            let s = 1.0 + a * (2.0 * r as f64 / (rows - 1) as f64 - 1.0);
            let mid = half_way(row[0], row[cols - 1]);
            for q in &mut row[..cols] {
                *q = mid + (*q - mid) * s;
            }
        }
    }

    /// The same patch as a bicubic net (degree elevation, which leaves the surface as it is).
    fn bicubic(self) -> [[Point; 4]; 4] {
        let rows: Vec<[Point; 4]> = self.p[..self.rows].iter().map(|row| cubic_line(&row[..self.cols])).collect();
        let mut out = [[Point::ZERO; 4]; 4];
        for c in 0..4 {
            let col: Vec<Point> = rows.iter().map(|row| row[c]).collect();
            for (to, q) in out.iter_mut().zip(cubic_line(&col)) {
                to[c] = q;
            }
        }
        out
    }
}

/// The point half way from `a` to `b`, as halves added: two finite points always give a finite one.
fn half_way(a: Point, b: Point) -> Point {
    Point::new(0.5 * a.x + 0.5 * b.x, 0.5 * a.y + 0.5 * b.y)
}

/// A Bézier curve of 2 to 4 control points as the 4 of the same curve's cubic form.
fn cubic_line(p: &[Point]) -> [Point; 4] {
    match *p {
        [a, b] => [a, a.lerp(b, 1.0 / 3.0), a.lerp(b, 2.0 / 3.0), b],
        [a, b, c] => [a, a.lerp(b, 2.0 / 3.0), c.lerp(b, 2.0 / 3.0), c],
        [a, b, c, d] => [a, b, c, d],
        _ => [Point::ZERO; 4],
    }
}

/// The handle offsets (along the chord, across it) of a cubic circular arc over a chord `chord`
/// long that turns through θ = |`b`| × 180°: each handle is 4/3 tan(θ/4) of the radius long, along
/// the arc's end tangent. With the radius `chord / (2 sin(θ/2))` that length is
/// `chord / (3 cos²(θ/4))`, which a bend near 0 leaves at a third of the chord.
fn arc_handles(chord: f64, b: f64) -> (f64, f64) {
    let th = b.abs().min(1.0) * PI;
    let len = chord / (3.0 * (th / 4.0).cos().powi(2));
    (len * (th / 2.0).cos(), len * (th / 2.0).sin())
}

/// A row from (0, `y`) to (`w`, `y`) bent into a circular arc of |`b`| × 180°: up for a positive
/// `b` when `up`, down when not, the other way for a negative one.
fn arc_row(w: f64, y: f64, b: f64, up: bool) -> [Point; 4] {
    let (ax, ay) = arc_handles(w, b);
    let dy = if (b > 0.0) == up { -ay } else { ay };
    [Point::new(0.0, y), Point::new(ax, y + dy), Point::new(w - ax, y + dy), Point::new(w, y)]
}

/// Arc at bend |`b`| as four rows: the bottom edge bends up through the bottom corners, the top
/// edge is the arc around the same center `h` further out, and the rows between are spaced evenly
/// along the radii. The top corners lie `h` out from the bottom ones along the radii, which lean
/// out by θ/2.
fn arc_rows(w: f64, h: f64, b: f64) -> [[Point; 4]; 4] {
    let th = b.abs().min(1.0) * PI;
    let (s, c) = (th / 2.0).sin_cos();
    let (tl, tr) = (Point::new(-h * s, h * (1.0 - c)), Point::new(w + h * s, h * (1.0 - c)));
    let (ax, ay) = arc_handles(tr.x - tl.x, b);
    let top = [tl, Point::new(tl.x + ax, tl.y - ay), Point::new(tr.x - ax, tr.y - ay), tr];
    let bottom = arc_row(w, h, b.abs(), true);
    let mut out = [[Point::ZERO; 4]; 4];
    for (r, row) in out.iter_mut().enumerate() {
        for (q, (t, u)) in row.iter_mut().zip(top.iter().zip(bottom)) {
            *q = t.lerp(u, r as f64 / 3.0);
        }
    }
    out
}

/// Warp style `style`'s own net at bend `b` (-1..1) over the box `w` × `h`, before distortion.
/// `vertical`: the net of the box turned on its side, where Rise moves the other half.
fn style_net(style: WarpStyle, b: f64, w: f64, h: f64, vertical: bool) -> Net {
    use WarpStyle::*;
    let two = |top: [Point; 4], bottom: [Point; 4]| {
        let mut p = [[Point::ZERO; 4]; 4];
        (p[0], p[1]) = (top, bottom);
        Net { rows: 2, cols: 4, p }
    };
    let flat_row = |y: f64| [0.0, 1.0, 2.0, 3.0].map(|c| Point::new(w * c / 3.0, y));
    match style {
        Arc => {
            let rows = arc_rows(w, h, b);
            let net = two(rows[0], rows[3]);
            if b < 0.0 { net.mirrored(h) } else { net }
        }
        ArcUpper => two(arc_row(w, 0.0, b, true), flat_row(h)),
        ArcLower => two(flat_row(0.0), arc_row(w, h, b, false)),
        Arch => {
            let top = arc_row(w, 0.0, b, true);
            two(top, top.map(|q| Point::new(q.x, q.y + h)))
        }
        Bulge => two(arc_row(w, 0.0, b, true), arc_row(w, h, b, false)),
        ShellLower | ShellUpper => {
            let a = arc_rows(w, h, b);
            let mut net = Net::flat(w, h, 4, 4);
            if b >= 0.0 {
                // The outer arc and the ends of the row inside it, upside down for Shell Lower.
                net.p[0] = a[0];
                (net.p[1][0], net.p[1][3]) = (a[1][0], a[1][3]);
                if style == ShellLower { net.mirrored(h) } else { net }
            } else {
                // The inner arc and the ends of the row outside it, upside down for Shell Upper.
                net.p[3] = a[3];
                (net.p[2][0], net.p[2][3]) = (a[2][0], a[2][3]);
                if style == ShellUpper { net.mirrored(h) } else { net }
            }
        }
        Flag | Fish | Rise => {
            let mut net = Net::flat(w, h, 2, 4);
            let d = 2.0 * b * h;
            match style {
                Flag => {
                    for row in &mut net.p[..2] {
                        (row[1].y, row[2].y) = (row[1].y + d, row[2].y - d);
                    }
                }
                Fish => {
                    (net.p[0][1].y, net.p[0][2].y) = (net.p[0][1].y - d, net.p[0][2].y + d);
                    (net.p[1][1].y, net.p[1][2].y) = (net.p[1][1].y + d, net.p[1][2].y - d);
                }
                _ => {
                    for row in &mut net.p[..2] {
                        if vertical {
                            (row[2].y, row[3].y) = (row[2].y - d, row[3].y - d);
                        } else {
                            (row[0].y, row[1].y) = (row[0].y + d, row[1].y + d);
                        }
                    }
                }
            }
            net
        }
        Wave => {
            let mut net = Net::flat(w, h, 3, 4);
            let d = 2.0 * b * h;
            (net.p[1][1].y, net.p[1][2].y) = (net.p[1][1].y + d, net.p[1][2].y - d);
            net
        }
        Fisheye => {
            let mut net = Net::flat(w, h, 4, 4);
            let c = Point::new(w / 2.0, h / 2.0);
            for row in &mut net.p[1..3] {
                for q in &mut row[1..3] {
                    *q = c + (*q - c) * (1.0 + 4.0 * b);
                }
            }
            net
        }
        Inflate | Squeeze => {
            let mut net = Net::flat(w, h, 3, 3);
            let side = if style == Inflate { b * w / 2.0 } else { -b * w / 2.0 };
            net.p[0][1].y -= b * h / 2.0;
            net.p[2][1].y += b * h / 2.0;
            net.p[1][0].x -= side;
            net.p[1][2].x += side;
            net
        }
        Twist => {
            // The four inner points move round: clockwise for a positive bend.
            let mut net = Net::flat(w, h, 4, 4);
            let (dx, dy) = (b.abs() * w, b.abs() * h);
            if b >= 0.0 {
                net.p[1][1].x += dx;
                net.p[1][2].y += dy;
                net.p[2][2].x -= dx;
                net.p[2][1].y -= dy;
            } else {
                net.p[1][1].y += dy;
                net.p[1][2].x -= dx;
                net.p[2][2].y -= dy;
                net.p[2][1].x += dx;
            }
            net
        }
    }
}

/// A warp as one bicubic Bézier patch over its box: the style's own net at the bend, its columns
/// scaled by the horizontal distortion and then its rows by the vertical one. Points are in the
/// box's coordinates (origin at its top left, y down).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WarpNet {
    /// The control points over the box divided by `scale`.
    net: [[Point; 4]; 4],
    /// 1, or a power of two when the net over the box itself does not fit in finite numbers.
    scale: f64,
    w: f64,
    h: f64,
}

impl WarpNet {
    /// The net of `style` over a box `w` × `h` at bend `b` and distortions `dh` and `dv` (each
    /// -1..1, clamped); `vertical` bends along the box's height.
    pub fn new(style: WarpStyle, b: f64, dh: f64, dv: f64, w: f64, h: f64, vertical: bool) -> Self {
        let unit = |x: f64| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 };
        let size = |x: f64| if x.is_finite() { x.max(0.0) } else { 0.0 };
        let (b, dh, dv, w, h) = (unit(b), unit(dh), unit(dv), size(w), size(h));
        let build = |w: f64, h: f64| {
            let mut net = if vertical { style_net(style, b, h, w, true).transposed() } else { style_net(style, b, w, h, false) };
            net.distort_columns(dh);
            net.distort_rows(dv);
            net.bicubic()
        };
        // A box near the largest finite size can overflow the net. The net is then built on the box
        // divided by the smallest power of two that fits (2 to 4096), an exact division for every
        // side above the subnormal range, and points are multiplied back as they are read.
        let mut out = Self { net: build(w, h), scale: 1.0, w, h };
        while out.scale < 4096.0 && !out.net.iter().flatten().all(|q| q.is_finite()) {
            let scale = out.scale * 2.0;
            out = Self { net: build(w / scale, h / scale), scale, w, h };
        }
        out
    }

    /// The point at `(u, v)` of the box (0..1 across and down; outside, the patch goes on). A
    /// coordinate the patch would send past the largest finite number is the box's own, held to
    /// the finite range.
    pub fn at(&self, u: f64, v: f64) -> Point {
        let p = self.at_scaled(u, v);
        let held = |x: f64| if x.is_nan() { 0.0 } else { x.clamp(-f64::MAX, f64::MAX) };
        let coord = |x: f64, flat: f64| if (x * self.scale).is_finite() { x * self.scale } else { held(flat) };
        Point::new(coord(p.x, u * self.w), coord(p.y, v * self.h))
    }

    /// [`Self::at`] divided by the net's scale.
    fn at_scaled(&self, u: f64, v: f64) -> Point {
        let bern = |t: f64| {
            let s = 1.0 - t;
            [s * s * s, 3.0 * t * s * s, 3.0 * t * t * s, t * t * t]
        };
        let (bu, bv) = (bern(u), bern(v));
        let (mut x, mut y) = (0.0, 0.0);
        for (row, wv) in self.net.iter().zip(bv) {
            for (q, wu) in row.iter().zip(bu) {
                x += wv * wu * q.x;
                y += wv * wu * q.y;
            }
        }
        Point::new(x, y)
    }
}

/// Segment as a cubic; straight lines get handles at 1/3 and 2/3 so a non-linear map bends them.
fn seg_cubic(sp: &SubPath, i: usize) -> CubicBez {
    let c = sp.segment(i);
    if sp.segment_is_line(i) { CubicBez::new(c.p0, c.p0.lerp(c.p3, 1.0 / 3.0), c.p0.lerp(c.p3, 2.0 / 3.0), c.p3) } else { c }
}

fn poly_len(c: &CubicBez) -> f64 {
    c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3)
}

/// Running length along a polyline: `0, |p0p1|, |p0p1| + |p1p2|, …` (one entry per point).
fn cumulative_lengths(pts: &[Point]) -> Vec<f64> {
    let mut total = 0.0;
    let mut cum = Vec::with_capacity(pts.len());
    cum.push(0.0);
    for w in pts.windows(2) {
        total += w[0].distance(w[1]);
        cum.push(total);
    }
    cum
}

/// Most pieces [`map_nonlinear`] splits one segment into.
const MAX_PIECES: usize = 64;

/// How many pieces of about `piece` a segment of length `len` is split into: one to `most`.
pub fn pieces(len: f64, piece: f64, most: usize) -> usize {
    ((len / piece).ceil() as usize).clamp(1, most.max(1))
}

/// The piece length to split segments of lengths `lens` with ([`pieces`], at most `most` each) so
/// that the pieces past each segment's first number at most `room`: `piece` when they already do,
/// longer pieces otherwise. A segment of `n` pieces adds `n - 1 ≤ len / piece` points to a path,
/// so pieces of the total length over `room` fit; with no room every segment stays one piece.
pub fn fit_piece(lens: &[f64], piece: f64, most: usize, room: usize) -> f64 {
    let added = lens.iter().map(|&len| pieces(len, piece, most) - 1).fold(0, usize::saturating_add);
    if added <= room {
        return piece;
    }
    if room == 0 {
        return f64::INFINITY;
    }
    // A NaN length is always one piece.
    piece.max(lens.iter().filter(|len| !len.is_nan()).sum::<f64>() / room as f64)
}

/// Map every point of `path` through the non-linear function `f`. Segments are split into pieces
/// no longer than about `max_piece` (up to 64 per segment) and their control points mapped, which
/// approximates the image curve to O(h²). Closedness and subpath structure are preserved.
pub fn map_nonlinear(path: &PathData, max_piece: f64, f: impl Fn(Point) -> Point) -> PathData {
    map_nonlinear_within(path, max_piece, usize::MAX, f)
}

/// [`map_nonlinear`] whose result has at most `max` anchors, or `path`'s count when that is more:
/// where pieces of `max_piece` would make more, the pieces grow until they don't ([`fit_piece`]),
/// a coarser approximation of the image curve. Geometry effects map within their anchor budget.
pub fn map_nonlinear_within(path: &PathData, max_piece: f64, max: usize, f: impl Fn(Point) -> Point) -> PathData {
    map_pieces_within(path, max_piece, max, false, f)
}

/// [`map_nonlinear_within`] whose handles follow the image curve's tangents: each handle is its
/// direction mapped through the derivative of `f` where it starts, instead of its end mapped
/// through `f`. The points a segment is split at stay smooth (their handles in line) however
/// strongly `f` bends between them, and each piece is the image curve's cubic Hermite
/// approximation, O(h⁴) instead of O(h²). For maps that turn the path a lot over a short length,
/// such as Twist, where mapped handle ends make a corner at every split point (#7069).
pub fn map_nonlinear_tangents_within(path: &PathData, max_piece: f64, max: usize, f: impl Fn(Point) -> Point) -> PathData {
    map_pieces_within(path, max_piece, max, true, f)
}

/// Step, as a fraction of a handle, of the one-sided difference that maps a handle through the
/// derivative of a map ([`map_nonlinear_tangents_within`]).
const TANGENT_STEP: f64 = 1e-5;

/// The handle from `at` (mapped to `fa`) to `h`, mapped through the derivative of `f` at `at`
/// along the handle; `f(h)` where that is not finite or the handle has no length.
fn tangent_handle(f: &impl Fn(Point) -> Point, at: Point, fa: Point, h: Point) -> Point {
    let v = h - at;
    if v.hypot2() == 0.0 {
        return fa;
    }
    let q = fa + (f(at + v * TANGENT_STEP) - fa) / TANGENT_STEP;
    if q.is_finite() { q } else { f(h) }
}

/// [`map_nonlinear_within`], with handles mapped through the derivative of `f` when `tangents`.
fn map_pieces_within(path: &PathData, max_piece: f64, max: usize, tangents: bool, f: impl Fn(Point) -> Point) -> PathData {
    let mut max_piece = max_piece.max(1e-3);
    // Each segment adds one anchor per piece past its first.
    let room = max.saturating_sub(path.anchor_count());
    let segs = path.subpaths.iter().map(SubPath::segment_count).fold(0, usize::saturating_add);
    if segs.saturating_mul(MAX_PIECES - 1) > room {
        let lens: Vec<f64> = path.subpaths.iter().flat_map(|sp| (0..sp.segment_count()).map(|i| poly_len(&seg_cubic(sp, i)))).collect();
        max_piece = fit_piece(&lens, max_piece, MAX_PIECES, room);
    }
    // A handle of the anchor at `at` (mapped to `fa`) ending at `h`.
    let handle = |at: Point, fa: Point, h: Point| if tangents { tangent_handle(&f, at, fa, h) } else { f(h) };
    let mut subs = Vec::with_capacity(path.subpaths.len());
    for sp in &path.subpaths {
        let n = sp.anchors.len();
        if n == 0 {
            continue;
        }
        let segs = sp.segment_count();
        let mut res: Vec<Anchor> = Vec::new();
        let mut pending_in: Option<Point> = None;
        for i in 0..n {
            let a = &sp.anchors[i];
            let p = f(a.p);
            let h_in = pending_in.take().unwrap_or_else(|| handle(a.p, p, a.h_in));
            res.push(Anchor { p, h_in, h_out: handle(a.p, p, a.h_out), kind: a.kind });
            if i < segs {
                let c = seg_cubic(sp, i);
                let k = pieces(poly_len(&c), max_piece, MAX_PIECES);
                for j in 0..k {
                    let sub = c.subsegment((j as f64 / k as f64)..((j + 1) as f64 / k as f64));
                    // With tangents, a split point's handle out was set with its handle in.
                    if (j == 0 || !tangents)
                        && let Some(last) = res.last_mut()
                    {
                        last.h_out = handle(sub.p0, last.p, sub.p1);
                    }
                    let end = f(sub.p3);
                    if j + 1 < k {
                        // A split point: its handles are in line before the map (equal pieces of
                        // the parameter make them equal and opposite), so with tangents both come
                        // from one central difference and stay in line after it.
                        let (h_in, h_out) = if tangents {
                            let w = sub.p3 - sub.p2;
                            let d = (f(sub.p3 + w * TANGENT_STEP) - f(sub.p3 - w * TANGENT_STEP)) / (2.0 * TANGENT_STEP);
                            if d.is_finite() { (end - d, end + d) } else { (f(sub.p2), f(sub.p3 + w)) }
                        } else {
                            (f(sub.p2), end)
                        };
                        res.push(Anchor { p: end, h_in, h_out, kind: Default::default() });
                    } else {
                        pending_in = Some(handle(sub.p3, end, sub.p2));
                    }
                }
            }
        }
        if let Some(h) = pending_in
            && sp.closed
            && let Some(first) = res.first_mut()
        {
            first.h_in = h;
        }
        let anchors = res.into_iter().map(|a| Anchor::with_handles(a.p, a.h_in, a.h_out)).collect();
        subs.push(SubPath::new(anchors, sp.closed));
    }
    PathData::new(subs)
}

// =====================================================================================
// Coons patches from outlines
// =====================================================================================

pub(super) fn flatten_subpath(sp: &SubPath, tol: f64) -> Vec<Point> {
    let mut bp = BezPath::new();
    sp.to_bezpath_into(&mut bp);
    let mut pts: Vec<Point> = Vec::new();
    vectorcraft_geom::kurbo::flatten(bp.iter(), tol, |el| match el {
        vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) if pts.last().is_none_or(|q| q.distance(p) > 1e-9) => pts.push(p),
        _ => {}
    });
    pts
}

pub(super) fn resample(poly: &[Point], m: usize) -> Vec<Point> {
    if poly.is_empty() {
        return vec![Point::ZERO; m];
    }
    if poly.len() == 1 {
        return vec![poly[0]; m];
    }
    let cum = cumulative_lengths(poly);
    let total = cum.last().copied().unwrap_or(0.0);
    let mut out = Vec::with_capacity(m);
    let mut i = 0;
    for k in 0..m {
        let s = if m > 1 { total * k as f64 / (m - 1) as f64 } else { 0.0 };
        while i + 2 < cum.len() && cum[i + 1] < s {
            i += 1;
        }
        let seg = (cum[i + 1] - cum[i]).max(1e-12);
        out.push(poly[i].lerp(poly[i + 1], ((s - cum[i]) / seg).clamp(0.0, 1.0)));
    }
    out
}
