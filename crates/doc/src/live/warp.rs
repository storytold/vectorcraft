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
    let mut max_piece = max_piece.max(1e-3);
    // Each segment adds one anchor per piece past its first.
    let room = max.saturating_sub(path.anchor_count());
    let segs = path.subpaths.iter().map(SubPath::segment_count).fold(0, usize::saturating_add);
    if segs.saturating_mul(MAX_PIECES - 1) > room {
        let lens: Vec<f64> = path.subpaths.iter().flat_map(|sp| (0..sp.segment_count()).map(|i| poly_len(&seg_cubic(sp, i)))).collect();
        max_piece = fit_piece(&lens, max_piece, MAX_PIECES, room);
    }
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
            let h_in = pending_in.take().unwrap_or_else(|| f(a.h_in));
            res.push(Anchor { p: f(a.p), h_in, h_out: f(a.h_out), kind: a.kind });
            if i < segs {
                let c = seg_cubic(sp, i);
                let k = pieces(poly_len(&c), max_piece, MAX_PIECES);
                for j in 0..k {
                    let sub = c.subsegment((j as f64 / k as f64)..((j + 1) as f64 / k as f64));
                    if let Some(last) = res.last_mut() {
                        last.h_out = f(sub.p1);
                    }
                    if j + 1 < k {
                        res.push(Anchor { p: f(sub.p3), h_in: f(sub.p2), h_out: f(sub.p3), kind: Default::default() });
                    } else {
                        pending_in = Some(f(sub.p2));
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
