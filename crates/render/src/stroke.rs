//! Raster-only stroke fitting: preserve thin curved bands through fill flattening, and draw the
//! rims of wide curved strokes as lines centered on their curves.

use crate::effects;
use crate::kurbo::{Affine, CubicBez, Line, ParamCurve, ParamCurveDeriv, PathEl, PathSeg, Point, QuadBez, Vec2};
use vectorcraft_doc::StrokeLayer;
use vectorcraft_geom::BezPath;

/// Vello flattens the two sides of a filled stroke independently at 0.25 device pixels.
/// For strokes narrower than four times the raster tolerance, fit and preflatten the
/// band more tightly. Wider strokes and the document/export geometry are unchanged.
pub(crate) fn outline(line: &BezPath, st: &StrokeLayer, width: f64, tol: f64) -> BezPath {
    if width >= tol * 4.0 {
        return effects::stroke::line_outline(line, st, width, tol);
    }
    // Closed endpoint rounding must not consume a substantial part of a subpixel band.
    let fine = (tol / 8.0).min(width / 32.0).max(1e-4);
    let outline = effects::stroke::line_outline(line, st, width, fine);
    let mut flat = BezPath::new();
    // Keep upstream's per-curve work cap for untrusted or extreme coordinates.
    vectorcraft_geom::flatten(outline.iter(), fine, |el| flat.push(el));
    flat
}

/// Strokes at least this many tolerances wide (8 device pixels at a tolerance of 0.25 px) have
/// their long curves drawn as lines by [`wide`].
const WIDE: f64 = 32.0;
/// A curve of a wide stroke's outline is drawn as lines when its control points span at least this
/// many tolerances (32 device pixels at 0.25 px) across or down.
const SPAN: f64 = 128.0;
/// The chords [`wide`] draws a curve with lie within about this many tolerances of it, before
/// their ends are moved out.
const CHORD: f64 = 0.8;
/// The most lines [`wide`] draws one outline's curves with. An outline that would take more is
/// drawn as it is.
const MAX_LINES: usize = 1 << 20;
/// Two curves meet smoothly when the sine of the angle between their tangents where they meet is
/// at most this.
const SMOOTH: f64 = 0.01;

/// Whether an outline made in the space `m` maps to device pixels, with a tolerance of at most a
/// quarter of `px` (the size of a device pixel in that space, [`crate::frame::Frame::px`]), stays
/// within that tolerance on the device: whether `m` stretches no length by more than `1 / px`.
/// `false` for a transform that collapses or isn't finite.
pub(crate) fn fine(m: Affine, px: f64) -> bool {
    let [a, b, c, d, _, _] = m.as_coeffs();
    let (sum, det) = (a * a + b * b + c * c + d * d, a * d - b * c);
    // The largest singular value of the linear part.
    let most = ((sum + (sum * sum - 4.0 * det * det).max(0.0).sqrt()) / 2.0).sqrt();
    det != 0.0 && most.is_finite() && px.is_finite() && px > 0.0 && most * px <= 1.0 + 1e-9
}

/// `outline`, the outline of a stroke `width` wide made at tolerance `tol`, with its long curves
/// ([`SPAN`]) drawn as lines that lie within about half of `tol` on either side of the curve, when
/// the stroke is at least [`WIDE`] tolerances wide. Vello draws a curve with chords that all lie
/// on its inner side, up to 0.25 device pixels in, which leaves a wide stroke's rims too light
/// outside and too dark inside. Here each line end is moved out by half the gap between the curve
/// and its chords, and the lines cross the curve. Where two such curves meet smoothly the shared
/// end moves too; elsewhere (a corner, a line, a short curve) the end stays on the curve and the
/// first or last line is half as long. Narrower strokes, outlines with no long curve and outlines
/// that would take more than [`MAX_LINES`] lines are returned as they are.
pub(crate) fn wide(outline: BezPath, width: f64, tol: f64) -> BezPath {
    if !(tol > 0.0 && tol.is_finite() && width >= tol * WIDE) {
        return outline;
    }
    lines(&outline, tol).unwrap_or(outline)
}

/// One subpath: where it starts, its segments, each with the most steps it is drawn with as lines
/// ([`steps`]; 0: drawn as it is), and whether it is closed.
struct Sub {
    start: Point,
    segs: Vec<(PathSeg, usize)>,
    closed: bool,
}

/// `path` with its long curves drawn as lines ([`wide`]); `None` when it has no long curve, would
/// take more than [`MAX_LINES`] lines, or does not start each subpath with a move.
fn lines(path: &BezPath, tol: f64) -> Option<BezPath> {
    let chord = tol * CHORD;
    let subs = subpaths(path.elements(), tol, chord)?;
    let mut total = 0usize;
    for (_, steps) in subs.iter().flat_map(|s| &s.segs) {
        if *steps > 0 {
            total = total.saturating_add(steps.saturating_add(2));
        }
    }
    if total == 0 || total > MAX_LINES {
        return None;
    }
    let mut out = BezPath::new();
    for sub in &subs {
        draw_sub(&mut out, sub, chord);
    }
    Some(out)
}

/// The subpaths of `els`, each segment with its steps ([`steps`]). A close adds a line back to the
/// start when the subpath ends elsewhere. `None` when a segment comes before any move or after a
/// close without a move.
fn subpaths(els: &[PathEl], tol: f64, chord: f64) -> Option<Vec<Sub>> {
    let mut subs: Vec<Sub> = Vec::new();
    for el in els {
        if let PathEl::MoveTo(p) = *el {
            subs.push(Sub { start: p, segs: Vec::new(), closed: false });
            continue;
        }
        let sub = subs.last_mut()?;
        if sub.closed {
            return None;
        }
        let last = sub.segs.last().map_or(sub.start, |(s, _)| s.end());
        let seg = match *el {
            PathEl::LineTo(p) => PathSeg::Line(Line::new(last, p)),
            PathEl::QuadTo(a, b) => PathSeg::Quad(QuadBez::new(last, a, b)),
            PathEl::CurveTo(a, b, c) => PathSeg::Cubic(CubicBez::new(last, a, b, c)),
            PathEl::ClosePath | PathEl::MoveTo(_) => {
                if last != sub.start {
                    sub.segs.push((PathSeg::Line(Line::new(last, sub.start)), 0));
                }
                sub.closed = true;
                continue;
            }
        };
        sub.segs.push((seg, steps(seg, tol, chord)));
    }
    Some(subs)
}

/// The cubic that draws curve `seg`; `None` for a line.
fn cubic(seg: PathSeg) -> Option<CubicBez> {
    match seg {
        PathSeg::Cubic(c) => Some(c),
        PathSeg::Quad(q) => Some(q.raise()),
        PathSeg::Line(_) => None,
    }
}

/// How many equal steps of its parameter keep the chords of curve `seg` within `chord` of it (by
/// the second differences of its control points), when its control points span at least [`SPAN`]
/// times `tol`; 0 for a line, a shorter curve, a curve with a point that isn't finite, and a curve
/// that would take more than [`MAX_LINES`] steps.
fn steps(seg: PathSeg, tol: f64, chord: f64) -> usize {
    let Some(c) = cubic(seg) else { return 0 };
    let pts = [c.p0, c.p1, c.p2, c.p3];
    if !pts.iter().all(|p| p.is_finite()) {
        return 0;
    }
    let (lo, hi) = pts.iter().fold((Point::new(f64::INFINITY, f64::INFINITY), Point::new(f64::NEG_INFINITY, f64::NEG_INFINITY)), |(lo, hi), p| {
        (Point::new(lo.x.min(p.x), lo.y.min(p.y)), Point::new(hi.x.max(p.x), hi.y.max(p.y)))
    });
    let span = (hi.x - lo.x).max(hi.y - lo.y);
    if !span.is_finite() || span < tol * SPAN {
        return 0;
    }
    let second = |a: Point, b: Point, c: Point| (a.to_vec2() - b.to_vec2() * 2.0 + c.to_vec2()).hypot();
    let bend = second(c.p0, c.p1, c.p2).max(second(c.p1, c.p2, c.p3));
    let n = (0.75 * bend / chord).sqrt().ceil().max(1.0);
    if !n.is_finite() || n > MAX_LINES as f64 {
        return 0;
    }
    n as usize
}

/// The direction curve `c` leaves its start in, or `None` when all its points are at its start.
fn start_tangent(c: &CubicBez) -> Option<Vec2> {
    [c.p1, c.p2, c.p3].into_iter().map(|p| p - c.p0).find(|v| v.hypot2() > 0.0)
}

/// The direction curve `c` reaches its end in, or `None` when all its points are at its end.
fn end_tangent(c: &CubicBez) -> Option<Vec2> {
    [c.p2, c.p1, c.p0].into_iter().map(|p| c.p3 - p).find(|v| v.hypot2() > 0.0)
}

/// How many pieces of a curve [`params`] measures its bending on.
const SAMPLES: usize = 32;

/// The parameters curve `c` has its line ends at, and whether its first and its last step are
/// halves. Each step holds an equal share of the curve's bending (the integral of the square root
/// of its curvature along it), and there are as many as keep each chord within about `chord` of the
/// curve, at most `most` (its equal steps, [`steps`]). The steps are equal when the bending can't
/// be measured (a cusp, or a point that isn't finite) or is 0. The first or last step is halved
/// when that end stays on the curve (`pin_start`, `pin_end`); a curve of one step is then drawn
/// in two halves.
fn params(c: &CubicBez, most: usize, chord: f64, pin_start: bool, pin_end: bool) -> (Vec<f64>, (bool, bool)) {
    let (d1, d2) = (c.deriv(), c.deriv().deriv());
    let bend = |u: f64| {
        let (v, a) = (d1.eval(u).to_vec2(), d2.eval(u).to_vec2());
        (v.cross(a).abs() / v.hypot()).sqrt()
    };
    let mut sums = [0.0; SAMPLES + 1];
    let mut before = bend(0.0);
    for j in 1..=SAMPLES {
        let here = bend(j as f64 / SAMPLES as f64);
        sums[j] = sums[j - 1] + (before + here) / 2.0;
        before = here;
    }
    let total = sums[SAMPLES];
    let most = most.max(1);
    let n = if total.is_finite() && total > 0.0 { ((total / SAMPLES as f64 / (8.0 * chord).sqrt()).ceil() as usize).clamp(1, most) } else { most };
    // The parameter at which a share `s` (0 to 1) of the bending lies before it.
    let at = |s: f64| -> f64 {
        if !(total.is_finite() && total > 0.0) {
            return s;
        }
        let want = s * total;
        let j = sums.partition_point(|&x| x <= want).clamp(1, SAMPLES);
        let (lo, hi) = (sums[j - 1], sums[j]);
        let part = if hi > lo { ((want - lo) / (hi - lo)).clamp(0.0, 1.0) } else { 0.0 };
        (j as f64 - 1.0 + part) / SAMPLES as f64
    };
    if n == 1 && (pin_start || pin_end) {
        return (vec![0.0, at(0.5), 1.0], (true, true));
    }
    let h = 1.0 / n as f64;
    let mut t = Vec::with_capacity(n + 3);
    t.push(0.0);
    if pin_start {
        t.push(at(h / 2.0));
    }
    t.extend((1..n).map(|k| at(k as f64 * h)));
    if pin_end {
        t.push(at(1.0 - h / 2.0));
    }
    t.push(1.0);
    (t, (pin_start, pin_end))
}

/// How far the curve `c` lies from the chord between parameters `a` and `b`, at its parameter
/// midway: the curve's point there minus the chord's middle.
fn bulge(c: &CubicBez, a: f64, b: f64) -> Vec2 {
    c.eval((a + b) / 2.0) - c.eval(a).midpoint(c.eval(b))
}

/// A curve of a subpath drawn as lines: the curve, its line ends' parameters, whether its first and
/// its last step are halves ([`params`]), and how far the curve lies from its first and last
/// chords ([`bulge`]).
struct Drawn {
    curve: CubicBez,
    t: Vec<f64>,
    halves: (bool, bool),
    first: Vec2,
    last: Vec2,
}

/// Draws `sub` into `out`, its curves with steps as lines ([`wide`]) with chords within about
/// `chord` of them, and its other segments as they are.
fn draw_sub(out: &mut BezPath, sub: &Sub, chord: f64) {
    let n = sub.segs.len();
    let curve = |i: usize| sub.segs.get(i).filter(|(_, steps)| *steps > 0).and_then(|(s, _)| cubic(*s));
    // Whether segment `i` starts where the one before it ends, both drawn as lines and meeting
    // smoothly: there the shared end moves off the curve with both.
    let smooth: Vec<bool> = (0..n)
        .map(|i| {
            let before = if i == 0 { sub.closed.then(|| n - 1) } else { Some(i - 1) };
            let (Some(a), Some(b)) = (before.and_then(curve), curve(i)) else { return false };
            match (end_tangent(&a), start_tangent(&b)) {
                (Some(u), Some(v)) => u.dot(v) > 0.0 && u.cross(v).abs() <= SMOOTH * u.hypot() * v.hypot(),
                _ => false,
            }
        })
        .collect();
    // Whether segment `i` ends where the next one starts, as above; the last one's next is the
    // first, which starts smoothly only on a closed subpath.
    let meets = |i: usize| smooth.get((i + 1) % n).copied().unwrap_or(false);
    let drawn: Vec<Option<Drawn>> = (0..n)
        .map(|i| {
            let c = curve(i)?;
            let steps = sub.segs.get(i).map_or(1, |(_, s)| *s);
            let (t, halves) = params(&c, steps, chord, !smooth.get(i).copied().unwrap_or(false), !meets(i));
            let (first, last) = match (t.first().zip(t.get(1)), t.len().checked_sub(2).and_then(|k| t.get(k).zip(t.last()))) {
                (Some((a, b)), Some((c2, d))) => (bulge(&c, *a, *b), bulge(&c, *c2, *d)),
                _ => (Vec2::ZERO, Vec2::ZERO),
            };
            Some(Drawn { curve: c, t, halves, first, last })
        })
        .collect();
    // How far the start of segment `i` moves: half the curve's gap from the chords on either side.
    let moved = |i: usize| -> Vec2 {
        if !smooth.get(i).copied().unwrap_or(false) {
            return Vec2::ZERO;
        }
        let before = if i == 0 { n - 1 } else { i - 1 };
        match (drawn.get(before).and_then(Option::as_ref), drawn.get(i).and_then(Option::as_ref)) {
            (Some(a), Some(b)) => (a.last + b.first) / 4.0,
            _ => Vec2::ZERO,
        }
    };
    out.move_to(sub.start + moved(0));
    for (i, (seg, _)) in sub.segs.iter().enumerate() {
        let end = seg.end() + moved((i + 1) % n);
        match (drawn.get(i).and_then(Option::as_ref), seg) {
            (Some(d), _) => draw_lines(out, d, end),
            (None, PathSeg::Line(_)) => out.line_to(end),
            (None, PathSeg::Quad(q)) => out.quad_to(q.p1, end),
            (None, PathSeg::Cubic(c)) => out.curve_to(c.p1, c.p2, end),
        }
    }
    if sub.closed {
        out.close_path();
    }
}

/// Draws curve `d` into `out` as lines to its inner line ends, each moved out by half the curve's
/// gap from the chords beside it, and a line to `end`. Beside a halved step, which lies a quarter
/// as far from the curve, the whole step's gap is used: half the other chord's gap, or the two
/// halves' gaps together.
fn draw_lines(out: &mut BezPath, d: &Drawn, end: Point) {
    let t = &d.t;
    let last = t.len().saturating_sub(2);
    for k in 1..t.len().saturating_sub(1) {
        let (Some(&a), Some(&u), Some(&b)) = (t.get(k - 1), t.get(k), t.get(k + 1)) else { continue };
        let (before, after) = (bulge(&d.curve, a, u), bulge(&d.curve, u, b));
        let shift = match (d.halves.0 && k == 1, d.halves.1 && k == last) {
            (true, true) => before + after,
            (true, false) => after / 2.0,
            (false, true) => before / 2.0,
            (false, false) => (before + after) / 4.0,
        };
        out.line_to(d.curve.eval(u) + shift);
    }
    out.line_to(end);
}
