//! Dash patterns. Dashes follow the path's arc length, restart on every subpath and join across
//! the start of a closed subpath. A dash of zero length is a [`Dot`]: with a round cap it paints
//! a disc, with a projecting cap a square turned along the path, with a butt cap nothing.
//!
//! Exact dashes keep the pattern's lengths and start `offset` into it. Dashes fitted to corners
//! ([`Dash::align_corners`]) ignore the offset: every run between corners and path ends holds a
//! whole number of periods, stretched or squeezed to fit, starting and ending in the middle of the
//! first dash, so a dash is centred on every corner and on both ends of an open path.

use kurbo::{BezPath, ParamCurve, ParamCurveArclen, PathSeg, Point, Shape, Vec2};
use vectorcraft_doc::{Dash, LineCap};

use super::{ARCLEN_ACCURACY, kink, push_seg, segments, subpaths, tangent};

/// More dashes than this (a tiny pattern on a long path) draws the line solid instead.
const MAX_DASHES: f64 = 200_000.0;
/// A pattern whose longest entry is shorter than this can't advance along the path (each entry
/// ends within the generator's tolerance of where it starts), so it draws the line solid.
const MIN_ENTRY: f64 = 1e-6;
/// A run between corners shorter than this keeps the pattern's own lengths (too short to fit).
const MIN_RUN: f64 = 1e-6;
/// Arc-length tolerance of the generator.
const EPS: f64 = 1e-9;

/// A zero-length dash: where it sits and the path's direction there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub at: Point,
    pub dir: Vec2,
    /// Fraction of its subpath's length at the dot (where a width profile is read).
    pub t: f64,
}

/// A dashed path: the dashes (open subpaths to stroke without a dash pattern) and the dots.
#[derive(Clone, Debug, Default)]
pub struct Dashed {
    pub path: BezPath,
    /// For each dash (subpath of `path`, in order), the fractions of its subpath's length where it
    /// starts and ends. The end passes 1 for a dash running on through the start of a closed path.
    pub spans: Vec<(f64, f64)>,
    pub dots: Vec<Dot>,
}

/// Apply dash pattern `d` to `bp`. `None` when the pattern is invalid (a negative or non-finite
/// entry, as in SVG and PDF) or has no positive length (the line is solid), or is too fine to draw
/// (then the line is drawn solid too).
pub fn dash(bp: &BezPath, d: &Dash) -> Option<Dashed> {
    if d.pattern.iter().any(|v| !v.is_finite() || *v < 0.0) || !d.offset.is_finite() {
        return None;
    }
    let mut pat = d.pattern.clone();
    if !pat.iter().any(|v| *v > MIN_ENTRY) {
        return None;
    }
    // An odd pattern repeats with dashes and gaps swapped (as in SVG and PDF).
    if pat.len() % 2 == 1 {
        pat.extend_from_within(..);
    }
    let period: f64 = pat.iter().sum();
    // Finite entries can still sum to infinity, which would never leave the phase loop below.
    if !period.is_finite() {
        return None;
    }
    let plans: Vec<Plan> = subpaths(bp)
        .into_iter()
        .map(|(r, closed)| {
            let segs = segments(&bp.elements()[r]);
            let lens = segs.iter().map(|s| s.arclen(ARCLEN_ACCURACY)).collect();
            if d.align_corners { Plan::fitted(segs, lens, closed, period) } else { Plan::exact(segs, lens, closed, period) }
        })
        .collect();
    if plans.iter().map(|p| p.periods).sum::<f64>() * pat.len() as f64 > MAX_DASHES {
        return None;
    }
    // Where an exact pattern starts: `offset` into it.
    let mut off = d.offset.rem_euclid(period);
    let mut ix0 = 0;
    while off > pat[ix0] {
        off -= pat[ix0];
        ix0 = (ix0 + 1) % pat.len();
    }
    let phase = Phase { ix: ix0, rem: pat[ix0] - off, on: ix0 % 2 == 0 };
    let mut out = Dashed::default();
    for plan in &plans {
        dash_subpath(&mut out, plan, &pat, phase);
    }
    Some(out)
}

/// How the pattern runs along one subpath.
struct Plan {
    segs: Vec<PathSeg>,
    lens: Vec<f64>,
    closed: bool,
    /// The pattern's scale on each segment (1 for exact dashes).
    scale: Vec<f64>,
    /// Segments that start a run between corners: the pattern restarts there in the middle of its
    /// first dash (fitted dashes only).
    restart: Vec<bool>,
    /// Arc length from the subpath's start to the start of `segs` (a fitted closed subpath starts
    /// at its first corner).
    shift: f64,
    /// Periods of the pattern drawn.
    periods: f64,
}

impl Plan {
    fn exact(segs: Vec<PathSeg>, lens: Vec<f64>, closed: bool, period: f64) -> Self {
        let n = segs.len();
        let periods = lens.iter().sum::<f64>() / period;
        Plan { segs, lens, closed, scale: vec![1.0; n], restart: vec![false; n], shift: 0.0, periods }
    }

    fn fitted(mut segs: Vec<PathSeg>, mut lens: Vec<f64>, closed: bool, period: f64) -> Self {
        let n = segs.len();
        // Corners: where segments with length meet at a kink (zero-length ones have no direction).
        let live: Vec<usize> = (0..n).filter(|&i| lens[i] > EPS).collect();
        let mut corners: Vec<usize> = live.windows(2).filter(|w| kink(&segs[w[0]], &segs[w[1]])).map(|w| w[1]).collect();
        let mut rot = 0;
        if closed && let (Some(&first), Some(&last)) = (live.first(), live.last()) {
            if kink(&segs[last], &segs[first]) {
                corners.insert(0, first);
            }
            // Start at a corner so that no run crosses the start point.
            rot = corners.first().copied().unwrap_or(0);
        }
        segs.rotate_left(rot);
        lens.rotate_left(rot);
        let shift = lens[n - rot..].iter().sum();
        let mut restart = vec![false; n];
        restart[0] = true;
        for c in corners {
            restart[(c + n - rot) % n] = true;
        }
        let (mut scale, mut periods) = (vec![1.0; n], 0.0);
        let mut i = 0;
        while i < n {
            let j = (i + 1..n).find(|&j| restart[j]).unwrap_or(n);
            let run: f64 = lens[i..j].iter().sum();
            if run > MIN_RUN {
                let k = (run / period).round().max(1.0);
                scale[i..j].fill(run / (k * period));
                periods += k;
            } else {
                periods += run / period;
            }
            i = j;
        }
        Plan { segs, lens, closed, scale, restart, shift, periods }
    }
}

/// Position in the pattern: entry index, length left in it, and whether it is a dash.
#[derive(Clone, Copy)]
struct Phase {
    ix: usize,
    rem: f64,
    on: bool,
}

/// A dash being collected: its segments and where it starts and ends along its plan.
#[derive(Default)]
struct Piece {
    segs: Vec<PathSeg>,
    start: f64,
    end: f64,
}

impl Piece {
    fn push(&mut self, seg: PathSeg, at: f64) {
        if self.segs.is_empty() {
            self.start = at;
        }
        self.segs.push(seg);
    }
}

fn dash_subpath(out: &mut Dashed, plan: &Plan, pat: &[f64], mut ph: Phase) {
    let first_dot = out.dots.len();
    let total: f64 = plan.lens.iter().sum();
    // Fraction of the subpath's length at `x` along the plan.
    let frac = |x: f64| {
        if total <= 0.0 {
            return 0.0;
        }
        let f = (x + plan.shift) / total;
        if f > 1.0 { f - 1.0 } else { f }
    };
    let mut dashes: Vec<Piece> = vec![];
    let mut cur = Piece::default();
    let (mut cuts, mut acc, mut starts_on) = (0, 0.0, false);
    for (i, (seg, &len)) in plan.segs.iter().zip(&plan.lens).enumerate() {
        let k = plan.scale[i];
        if plan.restart[i] {
            // The dash running into a corner goes on past it; a dot there ends it.
            if pat[0] <= 0.0 && !cur.segs.is_empty() {
                cur.end = acc;
                dashes.push(std::mem::take(&mut cur));
            }
            ph = Phase { ix: 0, rem: pat[0] * k / 2.0, on: true };
        }
        if i == 0 {
            starts_on = ph.on && ph.rem > 0.0;
        }
        let mut s = 0.0;
        loop {
            let left = len - s;
            if ph.rem > left + EPS {
                if ph.on && left > EPS {
                    cur.push(sub(seg, len, s, len), acc + s);
                }
                ph.rem -= left;
                break;
            }
            let end = (s + ph.rem).min(len);
            if ph.on {
                if pat[ph.ix] > 0.0 {
                    if end > s {
                        cur.push(sub(seg, len, s, end), acc + s);
                    }
                    if !cur.segs.is_empty() {
                        cur.end = acc + end;
                        dashes.push(std::mem::take(&mut cur));
                    }
                } else {
                    let t = param(seg, len, end);
                    push_dot(out, first_dot, Dot { at: seg.eval(t), dir: tangent(seg, t), t: frac(acc + end) });
                }
            }
            cuts += 1;
            s = end;
            ph.ix = (ph.ix + 1) % pat.len();
            ph.rem = pat[ph.ix] * k;
            ph.on = !ph.on;
        }
        acc += len;
    }
    // A fitted open path whose pattern starts with a dot ends on one, whatever the rounding.
    if !plan.closed
        && plan.restart[0]
        && pat[0] <= 0.0
        && let Some(seg) = plan.segs.last()
    {
        push_dot(out, first_dot, Dot { at: seg.end(), dir: tangent(seg, 1.0), t: 1.0 });
    }
    let ends_on = !cur.segs.is_empty();
    if ends_on {
        cur.end = total;
        dashes.push(cur);
    }
    if plan.closed {
        if cuts == 0 && ends_on {
            // One dash all the way round: keep it closed so the start gets a join, not caps.
            let d = dashes.pop().unwrap_or_default();
            write(&mut out.path, &d.segs);
            out.path.close_path();
            out.spans.push((frac(0.0), frac(0.0) + 1.0));
            return;
        }
        if starts_on && ends_on && dashes.len() > 1 {
            // The dash running through the start point is one dash.
            let mut last = dashes.pop().unwrap_or_default();
            last.segs.append(&mut dashes[0].segs);
            dashes[0] = Piece { segs: last.segs, start: last.start, end: dashes[0].end + total };
        }
        // A dot on the start point is reached again at the end.
        if out.dots.len() > first_dot + 1 && out.dots[first_dot].at.distance(out.dots[out.dots.len() - 1].at) < 1e-6 {
            out.dots.pop();
        }
    }
    for d in &dashes {
        write(&mut out.path, &d.segs);
        let t0 = frac(d.start);
        out.spans.push((t0, if total > 0.0 { t0 + (d.end - d.start) / total } else { t0 }));
    }
}

/// Add `dot` unless the subpath's last dot (from index `first`) is already there: a run ending on
/// a dot meets the next one starting on it.
fn push_dot(out: &mut Dashed, first: usize, dot: Dot) {
    if out.dots[first..].last().is_none_or(|d| d.at.distance(dot.at) > 1e-6) {
        out.dots.push(dot);
    }
}

fn write(out: &mut BezPath, segs: &[PathSeg]) {
    let Some(first) = segs.first() else { return };
    out.move_to(first.start());
    for s in segs {
        push_seg(out, s);
    }
}

/// Curve parameter at arc length `s` along `seg` (of total length `len`).
fn param(seg: &PathSeg, len: f64, s: f64) -> f64 {
    if s <= 0.0 {
        0.0
    } else if s >= len {
        1.0
    } else if let PathSeg::Line(_) = seg {
        s / len
    } else {
        seg.inv_arclen(s, ARCLEN_ACCURACY)
    }
}

/// The part of `seg` between arc lengths `s0` and `s1`.
fn sub(seg: &PathSeg, len: f64, s0: f64, s1: f64) -> PathSeg {
    seg.subsegment(param(seg, len, s0)..param(seg, len, s1))
}

/// Outlines of `dots` for a stroke of `width` with `cap`: discs (round), squares turned along the
/// path (projecting) or nothing (butt). Each outline winds like kurbo's stroke outlines, so a dot
/// touching a dash doesn't cancel it under the non-zero rule.
pub fn dot_outline(dots: &[Dot], width: f64, cap: LineCap, tol: f64) -> BezPath {
    let mut out = BezPath::new();
    let r = width / 2.0;
    if r <= 0.0 {
        return out;
    }
    for d in dots {
        match cap {
            LineCap::Butt => {}
            LineCap::Round => out.extend(kurbo::Circle::new(d.at, r).path_elements(tol)),
            LineCap::Square => {
                let (u, n) = (d.dir * r, Vec2::new(-d.dir.y, d.dir.x) * r);
                out.move_to(d.at - u - n);
                out.line_to(d.at + u - n);
                out.line_to(d.at + u + n);
                out.line_to(d.at - u + n);
                out.close_path();
            }
        }
    }
    out
}
