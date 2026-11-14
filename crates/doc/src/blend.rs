//! Live blends: interpolating key objects and laying the steps out along the spine.
//!
//! - **Paths** are matched anchor by anchor ([`PathPair`]): subpaths paired in order (a missing
//!   one grows out of a point), resampled to equal anchor counts, closed ones turned to the same
//!   winding and to the start points that keep them from twisting, or to the anchors the Blend
//!   tool clicked.
//! - **Paint**: colours, gradients (stops resampled to a common set when their counts differ),
//!   opacity, stroke weight, dashes, miter limit, width profiles and the pen pressure, tilt,
//!   bearing and rotation that brushes read interpolate; what can't
//!   (caps, joins, brushes, fonts, symbols, text content) switches halfway.
//! - **Groups** pair their children in stacking order; the children one group has more of grow
//!   out of the other group's centre. Compound paths stay compound paths.
//! - **Text, symbols and images** interpolate their transforms (rotation, scale, shear and
//!   position separately); text interpolates its colours and sizes run by run.
//! - **Spine** ([`blend_spine`]): the straight lines between the key centres until it is edited
//!   or replaced; then a path whose anchors the keys sit on (`key_anchors`). Steps follow it by
//!   arc length; Align to Path turns them to its direction.

use std::f64::consts::PI;
use std::sync::Arc;

use vectorcraft_color::{Color, Gradient, GradientMethod, GradientPaint, GradientSpread, GradientStop, Paint};
use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveArclen, ParamCurveDeriv};
use vectorcraft_geom::{Affine, Anchor, BezPath, FillRule, PathData, Point, Rect, SubPath, Vec2};

use crate::appearance::{AngleProfile, Appearance, AppearanceItem, Dash, PressureProfile, StrokeGradientMode, StrokeLayer, WidthProfile};
use crate::live::{BlendOrientation, BlendSpacing, BlendSpec};
use crate::node::{Node, NodeId, NodeKind};
use crate::text::TextObject;

// =====================================================================================
// Colour / paint interpolation
// =====================================================================================

/// Interpolate two colours in their shared model (display RGB when the models differ).
pub fn lerp_color(a: &Color, b: &Color, t: f32) -> Color {
    a.lerp_keep_model(b, t)
}

fn lerp_f32(x: f32, y: f32, t: f32) -> f32 {
    x + (y - x) * t
}

fn lerp_f64(x: f64, y: f64, t: f64) -> f64 {
    x + (y - x) * t
}

/// `like` with every stop painting `c`, linked as the solid is (`swatch` at `tint`).
fn solid_gradient(c: Color, swatch: Option<&str>, tint: f32, like: &Gradient) -> Gradient {
    let mut g = like.clone();
    for s in &mut g.stops {
        s.color = c;
        s.opacity = 1.0;
        s.swatch = swatch.map(str::to_string);
        s.tint = tint;
    }
    g
}

/// The swatch and tint `g` paints at `t`: a stop's own link, or the shared swatch of the two stops
/// around `t` at the tint between theirs (unlinked when they differ), as [`Gradient::sample_with`] walks.
fn link_at(g: &Gradient, t: f32) -> (Option<String>, f32) {
    link_in(&g.stops, t)
}

/// [`link_at`] of a gradient's `stops`.
fn link_in(stops: &[GradientStop], t: f32) -> (Option<String>, f32) {
    let link = |s: &GradientStop| (s.swatch.clone(), s.tint);
    let Some(first) = stops.first() else { return (None, 1.0) };
    if t <= first.offset {
        return link(first);
    }
    for w in stops.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if t <= b.offset {
            let span = (b.offset - a.offset).max(1e-6);
            let u = (t - a.offset) / span;
            let m = a.midpoint.clamp(0.01, 0.99);
            let v = if u < m { 0.5 * u / m } else { 0.5 + 0.5 * (u - m) / (1.0 - m) };
            return match (&a.swatch, &b.swatch) {
                (Some(x), Some(y)) if x == y => (Some(x.clone()), lerp_f32(a.tint, b.tint, v)),
                _ if v <= 0.0 => link(a),
                _ if v >= 1.0 => link(b),
                _ => (None, 1.0),
            };
        }
    }
    stops.last().map_or((None, 1.0), link)
}

/// How close two places a gradient changes are for [`common_stops`] to give them one stop (two
/// where either gradient steps there: two of its stops at the same offset).
const STEP: f32 = 1e-4;

/// `g` with its stops at `offsets`, each painting what `g` paints there (midpoints centred). An
/// offset given twice is a step: the second stop paints what `g` steps to there (its last stop
/// within [`STEP`] of it), or what it paints there when it does not step.
fn resample_stops(g: &Gradient, offsets: &[f32]) -> Gradient {
    let mut out = g.clone();
    out.stops = offsets
        .iter()
        .enumerate()
        .map(|(i, &o)| {
            let after = i.checked_sub(1).and_then(|j| offsets.get(j)) == Some(&o);
            let mut near = g.stops.iter().filter(|s| (s.offset - o).abs() < STEP);
            let stepped = if after && near.next().is_some() { near.next_back() } else { None };
            match stepped {
                Some(s) => GradientStop { opacity: s.opacity, swatch: s.swatch.clone(), tint: s.tint, ..GradientStop::new(o, s.color) },
                None => {
                    let (color, opacity) = g.sample_keep_model(o);
                    let (swatch, tint) = link_at(g, o);
                    GradientStop { opacity, swatch, tint, ..GradientStop::new(o, color) }
                }
            }
        })
        .collect();
    out
}

/// Where a gradient changes: its stop offsets and its off-centre midpoints.
fn gradient_breaks(g: &Gradient, out: &mut Vec<f32>) {
    for (i, s) in g.stops.iter().enumerate() {
        out.push(s.offset);
        if let Some(n) = g.stops.get(i + 1)
            && (s.midpoint - 0.5).abs() > 1e-3
        {
            out.push(s.offset + (n.offset - s.offset) * s.midpoint);
        }
    }
}

/// Two gradients of one kind with the same number of stops: as they are when the counts match,
/// else both resampled at every place either changes, with two stops where either steps.
fn common_stops(a: &Gradient, b: &Gradient) -> Option<(Gradient, Gradient)> {
    if a.kind != b.kind {
        return None;
    }
    if a.stops.len() == b.stops.len() {
        return Some((a.clone(), b.clone()));
    }
    if a.stops.is_empty() || b.stops.is_empty() {
        return None;
    }
    let mut offs = Vec::with_capacity(2 * (a.stops.len() + b.stops.len()));
    gradient_breaks(a, &mut offs);
    gradient_breaks(b, &mut offs);
    offs.retain(|o| o.is_finite());
    offs.sort_by(f32::total_cmp);
    offs.dedup_by(|x, y| (*x - *y).abs() < STEP);
    let mut steps: Vec<f32> =
        [a, b].into_iter().flat_map(|g| g.stops.windows(2).filter(|w| w[1].offset == w[0].offset).map(|w| w[0].offset)).collect();
    steps.sort_by(f32::total_cmp);
    let mut at = Vec::with_capacity(offs.len() + steps.len());
    let mut k = 0;
    for o in offs {
        at.push(o);
        while steps.get(k).is_some_and(|&s| s <= o - STEP) {
            k += 1;
        }
        if steps.get(k).is_some_and(|&s| (s - o).abs() < STEP) {
            at.push(o);
        }
    }
    Some((resample_stops(a, &at), resample_stops(b, &at)))
}

fn lerp_gradient(a: &GradientPaint, b: &GradientPaint, t: f32) -> Option<GradientPaint> {
    let (ga, gb) = common_stops(&a.gradient, &b.gradient)?;
    let mut out = if t < 0.5 { a.clone() } else { b.clone() };
    out.gradient.stops = ga
        .stops
        .iter()
        .zip(&gb.stops)
        .map(|(sa, sb)| {
            // Stops on two tints of one swatch stay linked to it, as solids do (#3234).
            let (swatch, tint) = match (&sa.swatch, &sb.swatch) {
                (Some(x), Some(y)) if x == y => (Some(x.clone()), lerp_f32(sa.tint, sb.tint, t)),
                _ => (None, 1.0),
            };
            GradientStop {
                opacity: lerp_f32(sa.opacity, sb.opacity, t),
                midpoint: lerp_f32(sa.midpoint, sb.midpoint, t),
                swatch,
                tint,
                ..GradientStop::new(lerp_f32(sa.offset, sb.offset, t), lerp_color(&sa.color, &sb.color, t))
            }
        })
        .collect();
    // Two gradients that repeat (or reflect) step their count from one to the other; a spread
    // that differs switches halfway, as the method does.
    if a.gradient.is_spread() && a.gradient.spread == b.gradient.spread {
        out.gradient.repeats = lerp_f32(a.gradient.repeat_count() as f32, b.gradient.repeat_count() as f32, t).round() as u32;
    }
    out.angle = a.angle + (b.angle - a.angle) * t as f64;
    out.swatch = None;
    if let (Some(ga), Some(gb)) = (a.geom, b.geom) {
        let mut g = ga;
        g.start = ga.start.lerp(gb.start, t as f64);
        g.end = ga.end.lerp(gb.end, t as f64);
        g.aspect = ga.aspect + (gb.aspect - ga.aspect) * t as f64;
        g.focal = (ga.focal.is_some() || gb.focal.is_some()).then(|| ga.focal_point().lerp(gb.focal_point(), t as f64));
        out.geom = Some(g);
    } else {
        out.geom = None;
    }
    Some(out)
}

/// Interpolate paints: solid↔solid, gradients of one kind (stops resampled to a common set),
/// solid↔gradient; otherwise switch halfway.
pub fn lerp_paint(a: &Paint, b: &Paint, t: f32) -> Paint {
    match (a, b) {
        // Two tints of one swatch blend into tints of that swatch (a spot stays a spot on export).
        (Paint::Solid { color: x, swatch: Some(sa), tint: ta }, Paint::Solid { color: y, swatch: Some(sb), tint: tb }) if sa == sb => {
            Paint::Solid { color: lerp_color(x, y, t), swatch: Some(sa.clone()), tint: lerp_f32(*ta, *tb, t) }
        }
        (Paint::Solid { color: x, .. }, Paint::Solid { color: y, .. }) => Paint::solid(lerp_color(x, y, t)),
        (Paint::Gradient(ga), Paint::Gradient(gb)) => match lerp_gradient(ga, gb, t) {
            Some(g) => Paint::Gradient(Box::new(g)),
            None => (if t < 0.5 { a } else { b }).clone(),
        },
        (Paint::Solid { color, swatch, tint }, Paint::Gradient(g)) => {
            let ga = GradientPaint { gradient: solid_gradient(*color, swatch.as_deref(), *tint, &g.gradient), ..(**g).clone() };
            Paint::Gradient(Box::new(lerp_gradient(&ga, g, t).unwrap_or_else(|| (**g).clone())))
        }
        (Paint::Gradient(g), Paint::Solid { color, swatch, tint }) => {
            let gb = GradientPaint { gradient: solid_gradient(*color, swatch.as_deref(), *tint, &g.gradient), ..(**g).clone() };
            Paint::Gradient(Box::new(lerp_gradient(g, &gb, t).unwrap_or_else(|| (**g).clone())))
        }
        _ => (if t < 0.5 { a } else { b }).clone(),
    }
}

/// Interpolate dash patterns. A solid stroke counts as the other's pattern with its gaps closed,
/// so dashes open out of a solid line; patterns of different lengths repeat to a common one.
fn lerp_dash(x: Option<&Dash>, y: Option<&Dash>, t: f64) -> Option<Dash> {
    let solid_like = |d: &Dash| Dash {
        // An odd pattern repeats twice per period, so pair up the doubled one to keep every gap closed.
        pattern: if d.pattern.len() % 2 == 1 { d.pattern.repeat(2) } else { d.pattern.clone() }
            .chunks(2)
            .flat_map(|c| [c.iter().sum::<f64>(), 0.0])
            .collect(),
        offset: d.offset,
        align_corners: d.align_corners,
    };
    let (x, y) = match (x.filter(|d| d.is_dashed()), y.filter(|d| d.is_dashed())) {
        (None, None) => return None,
        (Some(d), None) => (d.clone(), solid_like(d)),
        (None, Some(d)) => (solid_like(d), d.clone()),
        (Some(a), Some(b)) => (a.clone(), b.clone()),
    };
    // An odd pattern repeats twice per period (dash, gap, dash / gap, dash, gap).
    let even = |p: &[f64]| if p.len() % 2 == 1 { p.repeat(2) } else { p.to_vec() };
    let (pa, pb) = (even(&x.pattern), even(&y.pattern));
    let n = if pa.is_empty() || pb.is_empty() { 0 } else { lcm(pa.len(), pb.len()).min(12) };
    let at = |p: &[f64], i: usize| p.get(i % p.len().max(1)).copied().unwrap_or(0.0);
    Some(Dash {
        pattern: (0..n).map(|i| lerp_f64(at(&pa, i), at(&pb, i), t)).collect(),
        offset: lerp_f64(x.offset, y.offset, t),
        align_corners: if t < 0.5 { x.align_corners } else { y.align_corners },
    })
}

fn lcm(a: usize, b: usize) -> usize {
    let (mut x, mut y) = (a, b);
    while y != 0 {
        (x, y) = (y, x % y);
    }
    a.checked_div(x).map_or(0, |q| q * b)
}

/// Interpolate width profiles (none is uniform) at every position either has a width point,
/// keeping their discontinuous points (a position where either steps gives two points).
fn lerp_profile(x: Option<&WidthProfile>, y: Option<&WidthProfile>, t: f64) -> Option<WidthProfile> {
    if x.is_none() && y.is_none() {
        return None;
    }
    let uniform = WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] };
    let (x, y) = (x.unwrap_or(&uniform), y.unwrap_or(&uniform));
    let mut at: Vec<f64> = x.points.iter().chain(&y.points).map(|p| p.0).filter(|p| p.is_finite()).collect();
    at.sort_by(f64::total_cmp);
    // Within the distance `around` reads a discontinuous point at, so a point near one keeps it.
    at.dedup_by(|a, b| (*a - *b).abs() <= 1e-7);
    // Each profile is a smooth curve through its own points: samples between the positions keep
    // both curves' shapes where the other profile's points and steps change the tangents. Two
    // profiles with points at the same positions need none (so a preset on both keys stays it).
    let same_positions = x.points.len() == y.points.len() && x.points.iter().zip(&y.points).all(|(p, q)| (p.0 - q.0).abs() <= 1e-7);
    let per_span = if same_positions { 1 } else { (4096 / at.len().max(1)).clamp(1, 8) };
    let mut points = Vec::with_capacity(at.len() * per_span + 2);
    let mix = |p: f64, (l1, r1): (f64, f64), (l2, r2): (f64, f64)| (p, lerp_f64(l1, l2, t), lerp_f64(r1, r2, t));
    for (k, &p) in at.iter().enumerate() {
        let ((xb, xa), (yb, ya)) = (x.around(p), y.around(p));
        points.push(mix(p, xb, yb));
        // A discontinuous point ends the curve on each side even where both widths match.
        if steps_at(x, p) || steps_at(y, p) {
            points.push(mix(p, xa, ya));
        }
        if let Some(&q) = at.get(k + 1) {
            for j in 1..per_span {
                let f = p + (q - p) * j as f64 / per_span as f64;
                points.push(mix(f, x.at(f), y.at(f)));
            }
        }
    }
    Some(WidthProfile { points })
}

/// Whether width profile `w` has a discontinuous point at `p` (two points there, within `1e-7`,
/// as [`WidthProfile::around`] reads them).
fn steps_at(w: &WidthProfile, p: f64) -> bool {
    let i = w.points.partition_point(|q| q.0 < p - 1e-7);
    w.points.get(i..).unwrap_or_default().iter().take_while(|q| q.0 <= p + 1e-7).count() > 1
}

/// The positions along the stroke where either of two pen recordings has a sample, in order.
fn sample_positions(x: Option<&[(f64, f64)]>, y: Option<&[(f64, f64)]>) -> Vec<f64> {
    let mut at: Vec<f64> = x.unwrap_or_default().iter().chain(y.unwrap_or_default()).map(|p| p.0).filter(|p| p.is_finite()).collect();
    at.sort_by(f64::total_cmp);
    // Within the distance `step_after` reads a step at, so a sample near a step keeps the step.
    at.dedup_by(|a, b| (*a - *b).abs() <= 1e-9);
    if at.is_empty() {
        at = vec![0.0, 1.0];
    }
    at
}

/// The value pen recording `p` steps to at position `f`, where two or more of its samples share
/// that position (reading at `f` gives the first of them); None where it doesn't step.
fn step_after(p: Option<&[(f64, f64)]>, f: f64) -> Option<f64> {
    let p = p?;
    let i = p.partition_point(|q| q.0 < f - 1e-9);
    let j = p.partition_point(|q| q.0 <= f + 1e-9).checked_sub(1)?;
    (j > i && (p.get(i)?.0 - f).abs() <= 1e-9).then(|| p.get(j).map(|q| q.1)).flatten()
}

/// Two pen recordings read at every position either has a sample, as (position, x's value, y's
/// value): `read_x` and `read_y` read them, and `clean` makes a stepped sample's value one they
/// would read. A position where either steps gives two, before and after the step.
fn paired_samples(
    x: Option<&[(f64, f64)]>,
    y: Option<&[(f64, f64)]>,
    read_x: impl Fn(f64) -> f64,
    read_y: impl Fn(f64) -> f64,
    clean: impl Fn(f64) -> f64,
) -> Vec<(f64, f64, f64)> {
    let mut out = Vec::new();
    for f in sample_positions(x, y) {
        let (a, b) = (read_x(f), read_y(f));
        out.push((f, a, b));
        let (sa, sb) = (step_after(x, f), step_after(y, f));
        if sa.is_some() || sb.is_some() {
            out.push((f, sa.map_or(a, &clean), sb.map_or(b, &clean)));
        }
    }
    out
}

/// Interpolate pen pressure or tilt recordings at every position either has a sample, keeping
/// their steps. A stroke without one reads as [`PressureProfile::MID`], as brushes read it; None
/// when neither has one.
fn lerp_pressure(x: Option<&PressureProfile>, y: Option<&PressureProfile>, t: f64) -> Option<PressureProfile> {
    if x.is_none() && y.is_none() {
        return None;
    }
    let read = |p: Option<&PressureProfile>, f: f64| p.map_or(PressureProfile::MID, |p| p.at(f));
    let clean = |v: f64| if v.is_finite() { v.clamp(0.0, 1.0) } else { PressureProfile::MID };
    let samples = paired_samples(x.map(|p| p.points.as_slice()), y.map(|p| p.points.as_slice()), |f| read(x, f), |f| read(y, f), clean);
    PressureProfile::from_samples(samples.into_iter().map(|(f, a, b)| (f, lerp_f64(a, b, t))))
}

/// Interpolate pen angle recordings (bearing or barrel rotation) at every position either has a
/// sample: the steps turn from one key's angle to the other's the shorter way round at the
/// stroke's start, and keep the whole turns each recording makes along the stroke. A stroke
/// without one reads as 0°, as brushes read it; None when neither has one.
fn lerp_angle(x: Option<&AngleProfile>, y: Option<&AngleProfile>, t: f64) -> Option<AngleProfile> {
    if x.is_none() && y.is_none() {
        return None;
    }
    let read = |p: Option<&AngleProfile>, f: f64| p.map_or(0.0, |p| p.unwrapped_at(f));
    let clean = |v: f64| if v.is_finite() { v } else { 0.0 };
    let pairs = paired_samples(x.map(|p| p.points.as_slice()), y.map(|p| p.points.as_slice()), |f| read(x, f), |f| read(y, f), clean);
    // Whole turns that bring the two angles within half a turn of each other at the first position.
    let d0 = pairs.first().map_or(0.0, |&(_, a, b)| b - a);
    let turns = vectorcraft_geom::normalize_deg(d0) - d0;
    let value = |a: f64, b: f64| a + (b - a + turns) * t;
    // Both curves are straight between two positions; samples are added where the steps turn
    // through 90° or more between them, since `AngleProfile::from_samples` joins neighbours the
    // shorter way round. A document's recordings can be long, so the added samples are limited.
    let mut budget = 4 * PressureProfile::MAX_POINTS;
    let mut samples = Vec::with_capacity(pairs.len());
    let mut prev: Option<(f64, f64)> = None;
    for (f, a, b) in pairs {
        let v = value(a, b);
        if let Some((pf, pv)) = prev.filter(|p| f - p.0 > 1e-9) {
            let parts = ((v - pv).abs() / 90.0).ceil().min((budget + 1) as f64) as usize;
            for k in 1..parts {
                let g = pf + (f - pf) * k as f64 / parts as f64;
                samples.push((g, value(read(x, g), read(y, g))));
            }
            budget -= parts.saturating_sub(1).min(budget);
        }
        samples.push((f, v));
        prev = Some((f, v));
    }
    AngleProfile::from_samples(samples)
}

/// Interpolate a stroke's paint, weight, opacity, miter limit, dashes, arrowhead scale, width
/// profile and the pen pressure, tilt, bearing and rotation its brush reads into `o` (a copy of
/// the nearer stroke, so caps, joins, alignment, arrowheads and brushes switch halfway).
fn lerp_stroke(o: &mut StrokeLayer, x: &StrokeLayer, y: &StrokeLayer, t: f64) {
    let tf = t as f32;
    o.paint = lerp_paint(&x.paint, &y.paint, tf);
    o.width = lerp_f64(x.width, y.width, t);
    o.opacity = lerp_f32(x.opacity, y.opacity, tf);
    o.miter_limit = lerp_f64(x.miter_limit, y.miter_limit, t);
    o.dash = lerp_dash(x.dash.as_ref(), y.dash.as_ref(), t);
    o.arrow_scale = (lerp_f64(x.arrow_scale.0, y.arrow_scale.0, t), lerp_f64(x.arrow_scale.1, y.arrow_scale.1, t));
    o.profile = lerp_profile(x.profile.as_ref(), y.profile.as_ref(), t);
    o.pressure = lerp_pressure(x.pressure.as_ref(), y.pressure.as_ref(), t);
    o.tilt = lerp_pressure(x.tilt.as_ref(), y.tilt.as_ref(), t);
    o.bearing = lerp_angle(x.bearing.as_ref(), y.bearing.as_ref(), t);
    o.rotation = lerp_angle(x.rotation.as_ref(), y.rotation.as_ref(), t);
}

/// Interpolate appearance stacks: item by item when their structure matches, else the top fill
/// and stroke only (on the structure of the nearer key).
pub fn lerp_appearance(a: &Appearance, b: &Appearance, t: f64) -> Appearance {
    let tf = t as f32;
    let mut out = if t < 0.5 { a.clone() } else { b.clone() };
    let same = a.items.len() == b.items.len()
        && a.items.iter().zip(&b.items).all(|(x, y)| {
            matches!((x, y), (AppearanceItem::Fill(_), AppearanceItem::Fill(_)) | (AppearanceItem::Stroke(_), AppearanceItem::Stroke(_)))
        });
    if same {
        for ((it, x), y) in out.items.iter_mut().zip(&a.items).zip(&b.items) {
            match (it, x, y) {
                (AppearanceItem::Fill(o), AppearanceItem::Fill(x), AppearanceItem::Fill(y)) => {
                    o.paint = lerp_paint(&x.paint, &y.paint, tf);
                    o.opacity = lerp_f32(x.opacity, y.opacity, tf);
                }
                (AppearanceItem::Stroke(o), AppearanceItem::Stroke(x), AppearanceItem::Stroke(y)) => lerp_stroke(o, x, y, t),
                _ => {}
            }
        }
        return out;
    }
    let (fa, fb) = (a.fill_paint(), b.fill_paint());
    if !(fa.is_none() && fb.is_none()) {
        out.set_fill(lerp_paint(&fa, &fb, tf));
    }
    let (sa, sb) = (a.stroke_paint(), b.stroke_paint());
    if !(sa.is_none() && sb.is_none()) {
        out.set_stroke(lerp_paint(&sa, &sb, tf));
        let w = a.stroke_width() + (b.stroke_width() - a.stroke_width()) * t;
        let (xs, ys) = (a.stroke().cloned(), b.stroke().cloned());
        if let Some(s) = out.stroke_mut() {
            if let (Some(x), Some(y)) = (&xs, &ys) {
                lerp_stroke(s, x, y, t);
            }
            s.width = w;
        }
    }
    out
}

// =====================================================================================
// Transforms
// =====================================================================================

/// An affine split into translation, rotation, x scale, shear and y scale (`R · [sx sh; 0 sy]`).
fn decompose(m: Affine) -> [f64; 6] {
    let [a, b, c, d, e, f] = m.as_coeffs();
    let sx = a.hypot(b);
    let ang = if sx > 1e-12 { b.atan2(a) } else { 0.0 };
    let (s, co) = ang.sin_cos();
    [e, f, ang, sx, co * c + s * d, -s * c + co * d]
}

fn compose([e, f, ang, sx, sh, sy]: [f64; 6]) -> Affine {
    let (s, co) = ang.sin_cos();
    Affine::new([co * sx, s * sx, co * sh - s * sy, s * sh + co * sy, e, f])
}

/// Interpolate two transforms by their parts: the rotation takes the shorter way round.
pub fn lerp_affine(x: Affine, y: Affine, t: f64) -> Affine {
    let (p, mut q) = (decompose(x), decompose(y));
    let turn = q[2] - p[2];
    q[2] = p[2] + (turn + PI).rem_euclid(2.0 * PI) - PI;
    let mut out = [0.0; 6];
    for (o, (a, b)) in out.iter_mut().zip(p.iter().zip(&q)) {
        *o = lerp_f64(*a, *b, t);
    }
    compose(out)
}

// =====================================================================================
// Path interpolation
// =====================================================================================

/// Grow a subpath to `n` anchors by splitting its longest segments.
fn grow(sp: &mut SubPath, n: usize) {
    let mut guard = 0;
    while sp.anchors.len() < n && guard < 20_000 {
        guard += 1;
        if sp.segment_count() == 0 {
            match sp.anchors.last().copied() {
                Some(a) => sp.anchors.push(Anchor::corner(a.p)),
                None => return,
            }
            continue;
        }
        let seg = (0..sp.segment_count())
            .max_by(|i, j| {
                let (ci, cj) = (sp.segment(*i), sp.segment(*j));
                (ci.p3 - ci.p0).hypot().total_cmp(&(cj.p3 - cj.p0).hypot())
            })
            .unwrap_or(0);
        sp.insert_anchor(seg, 0.5);
    }
}

fn lerp_anchor(x: &Anchor, y: &Anchor, t: f64) -> Anchor {
    Anchor { p: x.p.lerp(y.p, t), h_in: x.h_in.lerp(y.h_in, t), h_out: x.h_out.lerp(y.h_out, t), kind: if t < 0.5 { x.kind } else { y.kind } }
}

/// Start `sp` at anchor `i`: a closed subpath turns round to it, an open one clicked at its last
/// anchor runs the other way (a click on any other anchor of an open path is ignored).
fn start_at(sp: &mut SubPath, i: usize) {
    let n = sp.anchors.len();
    if i >= n {
        return;
    }
    if sp.closed {
        sp.anchors.rotate_left(i);
    } else if i + 1 == n && n > 1 {
        sp.reverse();
    }
}

/// Reverse a closed subpath keeping its start anchor first.
fn reverse_closed(sp: &mut SubPath) {
    sp.reverse();
    sp.anchors.rotate_right(1);
}

/// How a key's subpaths were turned for its blend, so that its stroke's profiles, pen
/// recordings and arrowheads turn with it: position `u` along the turned subpath is position
/// `start + u` (`start - u` when `reversed`) along the key's own, wrapping round a closed one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Orient {
    start: f64,
    reversed: bool,
    closed: bool,
}

impl Orient {
    fn of(sp: &SubPath) -> Self {
        Self { closed: sp.closed, ..Self::default() }
    }

    /// The subpath now starts at the fraction `f` of its length.
    fn rotate(&mut self, f: f64) {
        self.start += if self.reversed { -f } else { f };
    }

    /// The subpath now runs the other way.
    fn reverse(&mut self) {
        self.start += if self.reversed { -1.0 } else { 1.0 };
        self.reversed = !self.reversed;
    }

    /// As [`start_at`] turns `sp` (call it before).
    fn start_at(&mut self, sp: &SubPath, i: usize) {
        let n = sp.anchors.len();
        if i >= n {
            return;
        }
        if sp.closed {
            self.rotate(arc_fraction(sp, i));
        } else if i + 1 == n && n > 1 {
            self.reverse();
        }
    }

    /// Where a closed subpath now starts, as a fraction of its length; 0 for an open one.
    fn seam(&self) -> f64 {
        let s = if self.closed { self.start.rem_euclid(1.0) } else { 0.0 };
        if s > 1e-9 && s < 1.0 - 1e-9 { s } else { 0.0 }
    }

    fn is_identity(&self) -> bool {
        !self.reversed && self.seam() == 0.0
    }

    /// Whether two subpaths were turned alike, so one stroke along both turns the same way.
    fn same_turn(&self, o: &Self) -> bool {
        self.reversed == o.reversed && (self.seam() - o.seam()).abs() < 1e-9
    }
}

/// How far along `sp` its anchor `i` sits, as a fraction of its length.
fn arc_fraction(sp: &SubPath, i: usize) -> f64 {
    let pos = sp.anchor_arc_positions();
    match (pos.get(i), pos.last()) {
        (Some(&at), Some(&total)) if total.is_finite() && total > 1e-12 => (at / total).clamp(0.0, 1.0),
        _ => 0.0,
    }
}

/// Samples (position, value) along a key's subpath, read along the subpath turned as `o` says.
/// A closed subpath starting elsewhere starts the samples there too: the ones before it move
/// past the seam (`wrap` applied), which keeps any step the seam makes, and a value between
/// samples is read with `at`. A reversed subpath then reads them backwards (`flip` applied).
fn reoriented<V: Copy>(points: &[(f64, V)], o: Orient, at: impl Fn(f64) -> V, wrap: impl Fn(V) -> V, flip: impl Fn(V) -> V) -> Vec<(f64, V)> {
    let s = o.seam();
    let mut out = points.to_vec();
    if s > 0.0
        && let (Some(&(_, first)), Some(&(_, last))) = (points.first(), points.last())
    {
        let i = points.partition_point(|q| q.0 < s - 1e-9);
        let j = points.partition_point(|q| q.0 <= s + 1e-9).max(i);
        let (before, after) = match points.get(i..j) {
            Some([p, .., q]) => (p.1, q.1),
            Some([p]) => (p.1, p.1),
            _ => (at(s), at(s)),
        };
        out = Vec::with_capacity(points.len() + 4);
        out.push((0.0, after));
        out.extend(points.get(j..).unwrap_or_default().iter().map(|&(t, v)| (t - s, v)));
        out.push((1.0 - s, last));
        out.push((1.0 - s, wrap(first)));
        out.extend(points.get(..i).unwrap_or_default().iter().map(|&(t, v)| (t + 1.0 - s, wrap(v))));
        out.push((1.0, wrap(before)));
    }
    if o.reversed {
        out = out.into_iter().rev().map(|(t, v)| (1.0 - t, flip(v))).collect();
    }
    out
}

/// Gradient `g` laid along a closed subpath that now starts at fraction `s` of its length: the
/// part from `s` on runs first, then the part before it, stepping where the old start now falls.
/// The stops follow its colors as [`Gradient::span`] reads them, mixed linearly, and run once.
/// One of `g`'s own stops keeps its swatch link (each stop of a step its own), and a stop between
/// two of them the link [`link_at`] reads there.
fn gradient_from(g: &Gradient, s: f32) -> Gradient {
    let g = g.unrolled();
    let mut own = g.stops.clone();
    own.sort_by(|a, b| a.offset.total_cmp(&b.offset));
    let mut stops = Vec::new();
    // The part from `s` to the end runs first, then the part up to `s`.
    for (from, to, at) in [(s, 1.0, 0.0), (0.0, s, 1.0 - s)] {
        // The first of `own` not yet matched: the parts' stops run in order, so the links are
        // found in one pass.
        let mut j = 0;
        for st in g.span(from, to).stops {
            let p = from + (to - from) * st.offset;
            while own.get(j).is_some_and(|o| o.offset <= p - STEP) {
                j += 1;
            }
            let hit = (j..own.len())
                .take_while(|&i| own.get(i).is_some_and(|o| o.offset < p + STEP))
                .find(|&i| own.get(i).is_some_and(|o| o.color == st.color && o.opacity == st.opacity));
            let (swatch, tint) = match hit.and_then(|i| own.get(i).map(|o| (i, o))) {
                Some((i, o)) => {
                    j = i + 1;
                    (o.swatch.clone(), o.tint)
                }
                None => link_in(own.get(j.saturating_sub(1)..(j + 1).min(own.len())).unwrap_or_default(), p),
            };
            stops.push(GradientStop { offset: at + (to - from) * st.offset, swatch, tint, ..st });
        }
    }
    Gradient { stops, method: GradientMethod::Classic, spread: GradientSpread::Pad, repeats: 1, ..g.into_owned() }
}

/// Dash pattern `d` read along subpaths of lengths `lens` turned as `o` says. Reversed, the
/// pattern runs backwards: from the start of a dash, that is the dash and then the other entries
/// in reverse order. Exact dashes also start where the turned subpaths now start. The pattern
/// restarts on every subpath at the one offset, so the offset moves only when every subpath needs
/// the same one (as subpaths whose lengths differ by whole periods do); dashes fitted to corners
/// ignore the offset, which stays as it is. A dash pattern starts once per subpath, so on a closed
/// subpath started elsewhere the dashes match the key's as far as its own start, and past it run
/// on without restarting there.
fn reorient_dash(d: &mut Dash, o: Orient, lens: &[f64]) {
    if !d.is_dashed() || !d.offset.is_finite() {
        return;
    }
    let mut pat = d.pattern.clone();
    // An odd pattern repeats twice per period (dash, gap, dash / gap, dash, gap).
    if pat.len() % 2 == 1 {
        pat.extend_from_within(..);
    }
    let period: f64 = pat.iter().sum();
    // Where along the key's own subpath the turned one starts (a closed one wraps round).
    let at = if o.closed { o.seam() } else { o.start };
    let first = pat.first().copied().unwrap_or(0.0);
    let offset = |len: f64| if o.reversed { first - at * len - d.offset } else { d.offset + at * len };
    let offsets: Vec<f64> = lens.iter().map(|&len| offset(len)).collect();
    let agree = offsets.first().is_some_and(|&a| {
        offsets.iter().all(|&b| {
            let r = (b - a).rem_euclid(period);
            r.is_finite() && r.min(period - r) <= 1e-6
        })
    });
    if o.reversed {
        if let Some(rest) = pat.get_mut(1..) {
            rest.reverse();
        }
        d.pattern = pat;
    }
    if let Some(&off) = offsets.first().filter(|_| agree && !d.align_corners) {
        d.offset = off;
    }
}

/// A key's stroke read along its subpaths turned as `o` says (`lens`: their lengths): its width
/// profile (whose sides swap when reversed), pen pressure, tilt, bearing and rotation, its dashes,
/// a gradient along the path or across it (whose sides swap when reversed), and on a reversed
/// open path its arrowheads.
fn reorient_stroke(s: &mut StrokeLayer, o: Orient, lens: &[f64]) {
    if o.is_identity() {
        return;
    }
    if let Some(d) = &mut s.dash {
        reorient_dash(d, o, lens);
    }
    let along = s.gradient_mode == StrokeGradientMode::Along;
    if s.path_gradient().is_some()
        && let Paint::Gradient(g) = &mut s.paint
    {
        let start = along && o.seam() > 0.0;
        if start {
            g.gradient = gradient_from(&g.gradient, o.seam() as f32);
        }
        // Stops that reflect an even number of times read the same either way.
        let (spread, runs) = g.gradient.painted_spread();
        let reverse = o.reversed && !(spread == GradientSpread::Reflect && runs % 2 == 0);
        if reverse {
            g.gradient.reverse();
        }
        if start || reverse {
            // The turned gradient is no longer the swatch's.
            g.swatch = None;
        }
    }
    let same = |v: f64| v;
    if let Some(p) = &mut s.profile {
        let mut pts: Vec<(f64, (f64, f64))> = p.points.iter().map(|&(t, l, r)| (t, (l, r))).collect();
        // A width profile is a smooth curve through its points: where the new start splits a span,
        // samples of that span keep its curve once its ends become ends of the profile.
        let seam = o.seam();
        let i = p.points.partition_point(|q| q.0 < seam);
        if seam > 0.0
            && let (Some(a), Some(b)) = (i.checked_sub(1).and_then(|j| p.points.get(j)), p.points.get(i))
            && seam - a.0 > 1e-9
            && b.0 - seam > 1e-9
        {
            const SAMPLES: usize = 32;
            let (a, b) = (a.0, b.0);
            let extra: Vec<(f64, (f64, f64))> =
                (1..SAMPLES).map(|k| a + (b - a) * k as f64 / SAMPLES as f64).filter(|f| (f - seam).abs() > 1e-9).map(|f| (f, p.at(f))).collect();
            pts.splice(i..i, extra);
        }
        let out = reoriented(&pts, o, |f| p.at(f), |v| v, |(l, r)| (r, l));
        p.points = out.into_iter().map(|(t, (l, r))| (t, l, r)).collect();
    }
    for p in [&mut s.pressure, &mut s.tilt].into_iter().flatten() {
        let out = reoriented(&p.points, o, |f| p.at(f), same, same);
        p.points = out;
    }
    for p in [&mut s.bearing, &mut s.rotation].into_iter().flatten() {
        // The samples moved past the seam carry the whole turns the recording makes along the
        // stroke, so the angle does not jump by them there.
        let turns = match (p.points.first(), p.points.last()) {
            (Some(a), Some(b)) if (b.1 - a.1).is_finite() => ((b.1 - a.1) / 360.0).round() * 360.0,
            _ => 0.0,
        };
        let out = reoriented(&p.points, o, |f| p.unwrapped_at(f), |v| v + turns, same);
        p.points = out;
    }
    if o.reversed && !o.closed {
        std::mem::swap(&mut s.start_arrow, &mut s.end_arrow);
        s.arrow_scale = (s.arrow_scale.1, s.arrow_scale.0);
    }
}

/// `n` with every stroke read along its subpaths turned as `o` says.
fn reorient_strokes(n: &mut Node, o: Orient) {
    if o.is_identity() {
        return;
    }
    let lens: Vec<f64> =
        node_path(n).map(|(p, ..)| p.subpaths.iter().map(|sp| PathData::new(vec![sp.clone()]).length()).collect()).unwrap_or_default();
    for it in n.appearance.items.iter_mut() {
        if let AppearanceItem::Stroke(s) = it {
            reorient_stroke(s, o, &lens);
        }
    }
}

/// Anchor positions of a subpath relative to its box, scaled to unit size.
fn normalised(sp: &SubPath) -> Vec<Vec2> {
    let pts = sp.anchors.iter().map(|a| a.p);
    let (mut lo, mut hi) = (Point::new(f64::MAX, f64::MAX), Point::new(f64::MIN, f64::MIN));
    for p in pts.clone() {
        lo = Point::new(lo.x.min(p.x), lo.y.min(p.y));
        hi = Point::new(hi.x.max(p.x), hi.y.max(p.y));
    }
    let c = lo.midpoint(hi);
    let s = (hi.x - lo.x).max(hi.y - lo.y).max(1e-9);
    pts.map(|p| (p - c) / s).collect()
}

/// The rotation of `b`'s anchors that best matches `a`'s (least squared distance between
/// corresponding anchors, each shape centred and scaled to unit size): the start point that keeps
/// a closed blend from twisting. Long subpaths try every few rotations first, then refine.
fn best_rotation(a: &SubPath, b: &SubPath) -> usize {
    let (qa, qb) = (normalised(a), normalised(b));
    let m = qa.len().min(qb.len());
    if m < 2 {
        return 0;
    }
    let cost = |s: usize| -> f64 { (0..m).map(|j| (qa[j] - qb[(j + s) % m]).hypot2()).sum() };
    let stride = m.div_ceil(512);
    let mut best = (0..m).step_by(stride).min_by(|x, y| cost(*x).total_cmp(&cost(*y))).unwrap_or(0);
    if stride > 1 {
        best = (best + m - stride..=best + m + stride).map(|s| s % m).min_by(|x, y| cost(*x).total_cmp(&cost(*y))).unwrap_or(best);
    }
    best
}

/// Two paths prepared for interpolation: subpaths paired in order, with equal anchor counts, the
/// same winding and matching start points.
#[derive(Clone, Debug)]
pub struct PathPair {
    a: Vec<SubPath>,
    b: Vec<SubPath>,
    /// How each path's subpaths were turned, where all of them turned alike (else unturned).
    orient: (Orient, Orient),
}

impl PathPair {
    /// `starts`: the anchor of each path's first subpath its blend starts from (the Blend tool's
    /// clicks); with neither, closed subpaths start where they twist least.
    pub fn new(a: &PathData, b: &PathData, starts: (Option<usize>, Option<usize>)) -> Self {
        let n = a.subpaths.len().max(b.subpaths.len());
        let ca = a.bounds().map(|r| r.center()).unwrap_or_default();
        let cb = b.bounds().map(|r| r.center()).unwrap_or_default();
        let degenerate = |like: &SubPath, c: Point| SubPath::new(vec![Anchor::corner(c); like.anchors.len().max(1)], like.closed);
        let (mut va, mut vb) = (Vec::with_capacity(n), Vec::with_capacity(n));
        let mut orient = (Orient::default(), Orient::default());
        for i in 0..n {
            let (mut sa, mut sb) = match (a.subpaths.get(i), b.subpaths.get(i)) {
                (Some(x), Some(y)) => (x.clone(), y.clone()),
                (Some(x), None) => (x.clone(), degenerate(x, cb)),
                (None, Some(y)) => (degenerate(y, ca), y.clone()),
                (None, None) => continue,
            };
            let (mut oa, mut ob) = (Orient::of(&sa), Orient::of(&sb));
            let explicit = i == 0 && (starts.0.is_some() || starts.1.is_some());
            if explicit {
                if let Some(s) = starts.0 {
                    oa.start_at(&sa, s);
                    start_at(&mut sa, s);
                }
                if let Some(s) = starts.1 {
                    ob.start_at(&sb, s);
                    start_at(&mut sb, s);
                }
            }
            let closed = sa.closed && sb.closed && sa.anchors.len() > 2 && sb.anchors.len() > 2;
            if closed && (sa.area() * sb.area()) < 0.0 {
                ob.reverse();
                reverse_closed(&mut sb);
            }
            let m = sa.anchors.len().max(sb.anchors.len());
            grow(&mut sa, m);
            grow(&mut sb, m);
            if closed && !explicit {
                let r = best_rotation(&sa, &sb);
                if r != 0 {
                    ob.rotate(arc_fraction(&sb, r));
                }
                sb.anchors.rotate_left(r);
            }
            // One stroke runs along every subpath of a key, each reading it by its own length, so
            // it turns only when all of the key's subpaths turned alike.
            if i == 0 {
                orient = (oa, ob);
            }
            if i < a.subpaths.len() && !orient.0.same_turn(&oa) {
                orient.0 = Orient::default();
            }
            if i < b.subpaths.len() && !orient.1.same_turn(&ob) {
                orient.1 = Orient::default();
            }
            va.push(sa);
            vb.push(sb);
        }
        Self { a: va, b: vb, orient }
    }

    /// The path at `t` (0 = `a`, 1 = `b`).
    pub fn at(&self, t: f64) -> PathData {
        let subs = self
            .a
            .iter()
            .zip(&self.b)
            .map(|(sa, sb)| {
                let anchors = sa.anchors.iter().zip(&sb.anchors).map(|(x, y)| lerp_anchor(x, y, t)).collect();
                SubPath::new(anchors, if t < 0.5 { sa.closed } else { sb.closed })
            })
            .collect();
        PathData::new(subs)
    }

    /// Subpaths at `t`, in order (see [`Self::at`]).
    fn len(&self) -> usize {
        self.a.len()
    }
}

/// Interpolate two paths anchor-by-anchor (see [`PathPair`]).
pub fn lerp_path(a: &PathData, b: &PathData, t: f64) -> PathData {
    PathPair::new(a, b, (None, None)).at(t)
}

/// Path data of a path or compound path node (compound children concatenated), its fill rule and
/// the number of subpaths each compound member holds.
pub(crate) fn node_path(n: &Node) -> Option<(PathData, FillRule, Option<Vec<usize>>)> {
    match &n.kind {
        NodeKind::Path { path, rule, .. } => Some((path.clone(), *rule, None)),
        NodeKind::Compound { children, rule } => {
            let members: Vec<&PathData> = children.iter().filter_map(|c| c.path_data()).collect();
            let counts = members.iter().map(|p| p.subpaths.len()).collect();
            let subs = members.iter().flat_map(|p| p.subpaths.iter().cloned()).collect();
            Some((PathData::new(subs), *rule, Some(counts)))
        }
        _ => None,
    }
}

// =====================================================================================
// Object interpolation
// =====================================================================================

/// How two key objects interpolate.
#[derive(Clone, Debug)]
enum Shape {
    /// Groups of one kind: children paired in stacking order.
    Group { children: Vec<Lerp>, clip: bool },
    /// Gradient meshes with the same grid.
    Mesh,
    /// Paths and compound paths; `members`: the subpath count of each member when both keys are
    /// compound paths.
    Path { pair: PathPair, rules: (FillRule, FillRule), members: Option<(Vec<usize>, Vec<usize>)> },
    /// Text, symbols and images: their transforms interpolate.
    Placed,
    /// Anything else: a copy of the nearer key moved and scaled into the interpolated box.
    Other,
}

/// Two key objects prepared for interpolation, so each step of a blend only interpolates.
#[derive(Clone, Debug)]
pub struct Lerp {
    a: Node,
    b: Node,
    shape: Shape,
}

/// `n` shrunk to a point at `c`: the counterpart of an object only one of two groups has.
fn collapsed(n: &Node, c: Point) -> Node {
    let mut x = n.clone();
    if let Some(b) = n.geometric_bounds() {
        x.transform(Affine::translate(c.to_vec2()) * Affine::scale(1e-6) * Affine::translate(-b.center().to_vec2()), false);
    }
    x
}

fn center_of(n: &Node) -> Point {
    n.geometric_bounds().map(|b| b.center()).unwrap_or_default()
}

impl Lerp {
    /// Prepare `a` → `b`; `starts` as in [`PathPair::new`].
    pub fn new(a: &Node, b: &Node, starts: (Option<usize>, Option<usize>)) -> Self {
        let shape = match (&a.kind, &b.kind) {
            (NodeKind::Group { children: ca, clip: k1 }, NodeKind::Group { children: cb, clip: k2 })
                if k1 == k2 && !(*k1 && ca.len() != cb.len()) =>
            {
                let (oa, ob) = (center_of(a), center_of(b));
                let n = ca.len().max(cb.len());
                let children = (0..n)
                    .filter_map(|i| match (ca.get(i), cb.get(i)) {
                        (Some(x), Some(y)) => Some(Lerp::new(x, y, (None, None))),
                        (Some(x), None) => Some(Lerp::new(x, &collapsed(x, ob), (None, None))),
                        (None, Some(y)) => Some(Lerp::new(&collapsed(y, oa), y, (None, None))),
                        (None, None) => None,
                    })
                    .collect();
                Shape::Group { children, clip: *k1 }
            }
            (NodeKind::Mesh(ma), NodeKind::Mesh(mb)) if ma.rows == mb.rows && ma.cols == mb.cols && ma.points.len() == mb.points.len() => Shape::Mesh,
            (NodeKind::Text(_), NodeKind::Text(_))
            | (NodeKind::SymbolInstance { .. }, NodeKind::SymbolInstance { .. })
            | (NodeKind::Image(_), NodeKind::Image(_))
            | (NodeKind::PlacedDocument(_), NodeKind::PlacedDocument(_)) => Shape::Placed,
            _ => match (node_path(a), node_path(b)) {
                (Some((pa, ra, ma)), Some((pb, rb, mb))) => {
                    Shape::Path { pair: PathPair::new(&pa, &pb, starts), rules: (ra, rb), members: ma.zip(mb) }
                }
                _ => Shape::Other,
            },
        };
        let (mut a, mut b) = (a.clone(), b.clone());
        if let Shape::Path { pair, .. } = &shape {
            reorient_strokes(&mut a, pair.orient.0);
            reorient_strokes(&mut b, pair.orient.1);
        }
        Self { a, b, shape }
    }

    /// The object at `t` (0 = `a`, 1 = `b`), with id 0.
    pub fn at(&self, t: f64) -> Node {
        self.at_eased(t, t)
    }

    /// The object whose shape is at `t` and whose colours (fills, strokes, opacity, mesh and text
    /// colours) are at `tc`: a blend with its colours eased apart from its spacing.
    pub fn at_eased(&self, t: f64, tc: f64) -> Node {
        let (a, b) = (&self.a, &self.b);
        let base = if t < 0.5 { a } else { b };
        let mut n = match &self.shape {
            Shape::Group { children, clip } => {
                let mut g =
                    Node::new(NodeId(0), NodeKind::Group { children: children.iter().map(|c| Arc::new(c.at_eased(t, tc))).collect(), clip: *clip });
                g.isolate = base.isolate;
                g.knockout = base.knockout;
                g
            }
            Shape::Mesh => {
                let mut n = base.clone();
                if let (NodeKind::Mesh(m), NodeKind::Mesh(ma), NodeKind::Mesh(mb)) = (&mut n.kind, &a.kind, &b.kind) {
                    for ((p, x), y) in m.points.iter_mut().zip(&ma.points).zip(&mb.points) {
                        p.p = x.p.lerp(y.p, t);
                        p.color = lerp_color(&x.color, &y.color, tc as f32);
                        p.opacity = lerp_f32(x.opacity, y.opacity, tc as f32);
                        for h in 0..4 {
                            p.handles[h] = x.handles[h].lerp(y.handles[h], t);
                        }
                    }
                }
                n.extras_mut().mask = None;
                n
            }
            Shape::Path { pair, rules, members } => {
                let path = pair.at(t);
                let rule = if t < 0.5 { rules.0 } else { rules.1 };
                match members {
                    Some((ma, mb)) => compound(path, rule, if t < 0.5 { ma } else { mb }, pair.len()),
                    None => {
                        let mut n = Node::path(NodeId(0), path, Appearance::default());
                        if let NodeKind::Path { rule: r, .. } = &mut n.kind {
                            *r = rule;
                        }
                        n
                    }
                }
            }
            Shape::Placed => {
                let mut n = base.clone();
                match (&mut n.kind, &a.kind, &b.kind) {
                    (NodeKind::Text(o), NodeKind::Text(x), NodeKind::Text(y)) => lerp_text(o, x, y, t, tc),
                    (NodeKind::SymbolInstance { xf: o, .. }, NodeKind::SymbolInstance { xf: x, .. }, NodeKind::SymbolInstance { xf: y, .. }) => {
                        *o = lerp_affine(*x, *y, t)
                    }
                    (NodeKind::Image(o), NodeKind::Image(x), NodeKind::Image(y)) => o.xf = lerp_affine(x.xf, y.xf, t),
                    (NodeKind::PlacedDocument(o), NodeKind::PlacedDocument(x), NodeKind::PlacedDocument(y)) => o.xf = lerp_affine(x.xf, y.xf, t),
                    _ => {}
                }
                n.extras_mut().mask = None;
                n
            }
            Shape::Other => {
                // Different structure: move/scale a copy of the nearer key into the interpolated box.
                let mut n = base.clone();
                if let (Some(ba), Some(bb), Some(bn)) = (a.geometric_bounds(), b.geometric_bounds(), base.geometric_bounds()) {
                    let w = ba.width() + (bb.width() - ba.width()) * t;
                    let h = ba.height() + (bb.height() - ba.height()) * t;
                    let c = ba.center().lerp(bb.center(), t);
                    let sx = if bn.width() > 1e-9 { w / bn.width() } else { 1.0 };
                    let sy = if bn.height() > 1e-9 { h / bn.height() } else { 1.0 };
                    n.transform(
                        Affine::translate(c.to_vec2()) * Affine::scale_non_uniform(sx, sy) * Affine::translate(-bn.center().to_vec2()),
                        false,
                    );
                }
                if let NodeKind::Path { live, .. } = &mut n.kind {
                    *live = None;
                }
                // A copy's members are new objects too.
                for c in n.children_mut().into_iter().flatten() {
                    zero_ids(Arc::make_mut(c));
                }
                n.extras_mut().mask = None;
                n
            }
        };
        n.id = NodeId(0);
        n.appearance = lerp_appearance(&a.appearance, &b.appearance, tc).into();
        n.opacity = lerp_f32(a.opacity, b.opacity, tc as f32);
        n.blend = base.blend;
        n.visible = true;
        n.locked = false;
        n.name = None;
        n
    }
}

/// `n` and its descendants with id 0 (generated objects).
fn zero_ids(n: &mut Node) {
    n.id = NodeId(0);
    for c in n.children_mut().into_iter().flatten() {
        zero_ids(Arc::make_mut(c));
    }
}

/// A compound path of `path`'s subpaths, split into members as `counts` says (one member per
/// subpath when the counts don't add up to `total`).
fn compound(path: PathData, rule: FillRule, counts: &[usize], total: usize) -> Node {
    let mut subs = path.subpaths.into_iter();
    let counts: Vec<usize> = if counts.iter().sum::<usize>() == total && !counts.contains(&0) { counts.to_vec() } else { vec![1; total] };
    let children =
        counts.iter().map(|&c| Arc::new(Node::path(NodeId(0), PathData::new(subs.by_ref().take(c).collect()), Appearance::default()))).collect();
    Node::new(NodeId(0), NodeKind::Compound { children, rule })
}

/// Interpolate text `x` → `y` into `o` (a copy of the nearer): the transform by its parts, an
/// area frame anchor by anchor, and each run's colours, stroke weight and size from the runs at
/// the same place (by character) of both.
/// The run of `of` at fraction `u` of its characters.
fn run_at(of: &TextObject, u: f64) -> Option<&crate::text::TextRun> {
    let total: usize = of.runs.iter().map(|r| r.text.chars().count()).sum();
    let target = (u * total as f64) as usize;
    let mut seen = 0;
    for r in &of.runs {
        seen += r.text.chars().count();
        if seen > target {
            return Some(r);
        }
    }
    of.runs.last()
}

/// The fraction of the characters of `runs` at the middle of each run.
fn run_middles(runs: &[crate::text::TextRun]) -> Vec<f64> {
    let total: usize = runs.iter().map(|r| r.text.chars().count()).sum::<usize>().max(1);
    let mut start = 0usize;
    runs.iter()
        .map(|r| {
            let len = r.text.chars().count();
            let u = (start as f64 + len as f64 / 2.0) / total as f64;
            start += len;
            u
        })
        .collect()
}

fn lerp_text(o: &mut TextObject, x: &TextObject, y: &TextObject, t: f64, tc: f64) {
    o.xf = lerp_affine(x.xf, y.xf, t);
    if let (crate::text::TextKind::Area { frame: f }, crate::text::TextKind::Area { frame: fx }, crate::text::TextKind::Area { frame: fy }) =
        (&mut o.kind, &x.kind, &y.kind)
    {
        *f = lerp_path(fx, fy, t);
    }
    let tf = tc as f32;
    let mut resized = false;
    let middles = run_middles(&o.runs);
    for (r, u) in o.runs.iter_mut().zip(middles) {
        let (Some(rx), Some(ry)) = (run_at(x, u), run_at(y, u)) else { continue };
        let (sx, sy) = (&rx.style, &ry.style);
        r.style.fill = lerp_paint(&sx.fill, &sy.fill, tf);
        r.style.stroke = lerp_paint(&sx.stroke, &sy.stroke, tf);
        r.style.stroke_width = lerp_f64(sx.stroke_width, sy.stroke_width, t);
        let size = lerp_f64(sx.size, sy.size, t);
        resized |= size != r.style.size;
        r.style.size = size;
    }
    if resized {
        o.cached_bounds = None;
        o.cached_baselines.clear();
        o.cached_ink = None;
    }
}

/// Interpolate two objects at `t` (0 = `a`, 1 = `b`). A blend prepares each pair once with
/// [`Lerp`] instead.
pub fn lerp_node(a: &Node, b: &Node, t: f64) -> Node {
    Lerp::new(a, b, (None, None)).at(t)
}

// =====================================================================================
// Step counts
// =====================================================================================

/// Largest channel difference between two solid colors, as a fraction of the channel's range: two
/// tints of the same swatch by their tints, RGB by its 0 to 255 values over 256, CMYK and gray by
/// their percentages, and colors in different spaces (or Lab) by the difference of their RGB
/// values over 255. A solid paint against no paint, or anything that isn't solid, differs by 0:
/// [`lerp_paint`] switches such paints halfway rather than mixing them.
fn paint_difference(x: &Paint, y: &Paint) -> f64 {
    let (Paint::Solid { color: p, swatch: sp, tint: tp }, Paint::Solid { color: q, swatch: sq, tint: tq }) = (x, y) else {
        return 0.0;
    };
    if sp.is_some() && sp == sq && tp != tq {
        return f64::from((tp - tq).abs());
    }
    let most = |a: &[f32], b: &[f32]| a.iter().zip(b).fold(0.0f64, |m, (u, v)| m.max(f64::from((u - v).abs())));
    match (p, q) {
        (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) => most(&[*r, *g, *b], &[*r2, *g2, *b2]) * 255.0 / 256.0,
        (Color::Cmyk { c, m, y, k }, Color::Cmyk { c: c2, m: m2, y: y2, k: k2 }) => most(&[*c, *m, *y, *k], &[*c2, *m2, *y2, *k2]),
        (Color::Gray { k }, Color::Gray { k: k2 }) => most(&[*k], &[*k2]),
        _ => most(&p.to_rgb(), &q.to_rgb()),
    }
}

/// Largest difference (see [`paint_difference`]) between pairs of fills and strokes, or `None`
/// when any of them is a gradient or a pattern.
fn paints_difference(pairs: &[(&Paint, &Paint)]) -> Option<f64> {
    let mut d = 0.0f64;
    for (x, y) in pairs {
        if [x, y].iter().any(|p| matches!(p, Paint::Gradient(_) | Paint::Pattern { .. })) {
            return None;
        }
        d = d.max(paint_difference(x, y));
    }
    Some(d)
}

/// Largest channel difference (0..1, see [`paint_difference`]) between the solid colors of two
/// keys: their top fill and stroke, the runs of two text objects paired as [`lerp_text`] pairs
/// them, and the members of two groups paired as [`Lerp::new`] pairs them. `None` when any of
/// these holds a gradient or a pattern, whose steps Smooth Color counts by distance.
fn color_distance(a: &Node, b: &Node) -> Option<f64> {
    let mut d =
        paints_difference(&[(&a.appearance.fill_paint(), &b.appearance.fill_paint()), (&a.appearance.stroke_paint(), &b.appearance.stroke_paint())])?;
    match (&a.kind, &b.kind) {
        (NodeKind::Group { children: ca, clip: k1 }, NodeKind::Group { children: cb, clip: k2 }) if k1 == k2 && !(*k1 && ca.len() != cb.len()) => {
            for (x, y) in ca.iter().zip(cb) {
                d = d.max(color_distance(x, y)?);
            }
        }
        (NodeKind::Text(x), NodeKind::Text(y)) => {
            // The runs `lerp_text` pairs: those of `x` and of `y` at the middle of each run of either.
            for u in run_middles(&x.runs).into_iter().chain(run_middles(&y.runs)) {
                let (Some(rx), Some(ry)) = (run_at(x, u), run_at(y, u)) else { continue };
                d = d.max(paints_difference(&[(&rx.style.fill, &ry.style.fill), (&rx.style.stroke, &ry.style.stroke)])?);
            }
        }
        _ => {}
    }
    d.is_finite().then_some(d)
}

/// Smooth Color's steps between two keys by distance: the largest distance between their
/// corresponding visible bounding-box edges, in mean key widths (left and right edges) or heights
/// (top and bottom), rounded, less one, from 1 to 1000. Keys with no width are measured in their
/// mean height across as well, keys with no height in their mean width, and keys with neither in
/// points.
fn spatial_steps(ba: Rect, bb: Rect) -> usize {
    let (w, h) = ((ba.width() + bb.width()) / 2.0, (ba.height() + bb.height()) / 2.0);
    let size = |s: f64, other: f64| {
        if s > 1e-9 {
            s
        } else if other > 1e-9 {
            other
        } else {
            1.0
        }
    };
    let (w, h) = (size(w, h), size(h, w));
    let q = ((ba.x0 - bb.x0).abs().max((ba.x1 - bb.x1).abs()) / w).max((ba.y0 - bb.y0).abs().max((ba.y1 - bb.y1).abs()) / h);
    if !q.is_finite() {
        return 1;
    }
    ((q + 0.5).floor() - 1.0).clamp(1.0, 1000.0) as usize
}

/// `255 × d` steps for a color difference `d` (see [`color_distance`]), a half step rounding down,
/// from 1 to 254.
fn level_steps(d: f64) -> f64 {
    // The tolerance keeps a difference that lands on a half step (128 of 256 RGB values gives
    // 127.5), stored as an `f32`, from rounding up.
    (255.0 * d - 0.5 - 1e-4).ceil().clamp(1.0, 254.0)
}

/// Smooth Color's steps between two keys whose colors differ by `d` (see [`color_distance`]):
/// [`level_steps`], and at most eight per point of the largest of the keys' widths and heights
/// and the distance between their centers along either axis.
fn color_steps(d: f64, ba: Rect, bb: Rect) -> usize {
    let by_color = level_steps(d);
    let (ca, cb) = (ba.center(), bb.center());
    let reach = [(ca.x - cb.x).abs(), (ca.y - cb.y).abs(), ba.width(), ba.height(), bb.width(), bb.height()].into_iter().fold(0.0f64, f64::max);
    let by_density = (8.0 * reach + 0.5).floor().clamp(1.0, 254.0);
    if !by_density.is_finite() {
        return by_color as usize;
    }
    by_color.min(by_density) as usize
}

/// Number of intermediate steps between two keys. Specified Distance counts along the spine, `len`
/// long between them. Smooth Color counts from the keys' colors and visible bounds, whatever the
/// spine: keys whose colors differ make [`color_steps`] ([`level_steps`] when either key has no
/// bounds), and others make [`spatial_steps`] (1 when either key has no bounds).
pub fn blend_step_count(a: &Node, b: &Node, spacing: BlendSpacing, len: f64) -> usize {
    match spacing {
        BlendSpacing::Steps(n) => n.clamp(1, 1000) as usize,
        BlendSpacing::Distance(d) => {
            let d = d.max(0.01);
            ((len / d).round() as i64 - 1).clamp(0, 1000) as usize
        }
        BlendSpacing::SmoothColor => {
            let bounds = a.visual_bounds().zip(b.visual_bounds());
            match (color_distance(a, b), bounds) {
                // Colors that differ by less than a thousandth of a step count as the same.
                (Some(d), Some((ba, bb))) if d * 255.0 > 1e-3 => color_steps(d, ba, bb),
                (Some(d), None) if d * 255.0 > 1e-3 => level_steps(d) as usize,
                // Same colors, gradients and patterns: the distance between the objects.
                (_, Some((ba, bb))) => spatial_steps(ba, bb),
                (_, None) => 1,
            }
        }
    }
}

// =====================================================================================
// Spine
// =====================================================================================

#[cfg(test)]
thread_local! {
    /// How many times [`Spine::new`] has run on this thread, for tests that count how often a path
    /// is flattened.
    pub(crate) static SPINES_BUILT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A spine (the first subpath of a path) flattened to a polyline, with cumulative lengths and the
/// arc length at each of its anchors.
pub struct Spine {
    pts: Vec<Point>,
    cum: Vec<f64>,
    anchors: Vec<f64>,
    closed: bool,
    /// The subpath the spine was flattened from, kept so [`Spine::at_length`] can read the
    /// original curves' tangents. Only the first subpath is stored: the others never affect a
    /// spine.
    subpath: SubPath,
}

impl Spine {
    /// Build a spine from the first subpath of `path`: flatten it to a polyline, record cumulative
    /// arc lengths, and the arc length at each anchor. `None` when `path` has no subpath, or when
    /// the first one flattens to fewer than two points (so it has no length to walk).
    pub fn new(path: &PathData) -> Option<Self> {
        #[cfg(test)]
        SPINES_BUILT.with(|n| n.set(n.get() + 1));
        let sp = path.subpaths.first()?;
        let mut pts: Vec<Point> = Vec::new();
        let mut cum: Vec<f64> = Vec::new();
        let mut anchors = Vec::with_capacity(sp.anchors.len());
        for i in 0..sp.segment_count() {
            anchors.push(cum.last().copied().unwrap_or(0.0));
            let c = sp.segment(i);
            let mut bp = BezPath::new();
            bp.move_to(c.p0);
            if sp.segment_is_line(i) {
                bp.line_to(c.p3);
            } else {
                bp.curve_to(c.p1, c.p2, c.p3);
            }
            vectorcraft_geom::kurbo::flatten(bp.iter(), 0.1, |el| {
                if let vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) = el
                    && p.x.is_finite()
                    && p.y.is_finite()
                    && pts.last().is_none_or(|q| q.distance(p) > 1e-9)
                {
                    let s = match (pts.last(), cum.last()) {
                        (Some(q), Some(l)) => l + q.distance(p),
                        _ => 0.0,
                    };
                    pts.push(p);
                    cum.push(s);
                }
            });
        }
        if !sp.closed {
            anchors.push(cum.last().copied().unwrap_or(0.0));
        }
        if pts.len() < 2 {
            return None;
        }
        Some(Self { pts, cum, anchors, closed: sp.closed, subpath: sp.clone() })
    }
    /// The spine's flattened length in arc length.
    pub fn length(&self) -> f64 {
        *self.cum.last().unwrap_or(&0.0)
    }
    /// Arc length at anchor `i`.
    pub fn anchor_length(&self, i: usize) -> Option<f64> {
        self.anchors.get(i).copied()
    }
    /// Arc length of the point of the spine nearest `p`. An open spine's end segments run on past
    /// its ends as lines, so the result is below 0 before the start and above [`Spine::length`]
    /// beyond the end. With `from`, the arc length of an earlier point a drag followed, each point
    /// counts as farther by half its distance along the spine from there, so a pointer passing
    /// near another part of the spine stays on the part it follows. Where two points count as
    /// near as each other, the one earlier along the spine counts.
    pub fn length_near(&self, p: Point, from: Option<f64>) -> f64 {
        self.nearest(p, from).map_or(0.0, |(_, s)| s)
    }
    /// [`Spine::length_near`] with how near the point counts: its distance from `p`, plus half its
    /// distance along the spine from `from`. `None` for a point that is not finite.
    pub fn nearest(&self, p: Point, from: Option<f64>) -> Option<(f64, f64)> {
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        let last = self.pts.len().saturating_sub(2);
        let mut best: Option<(f64, f64)> = None;
        for (i, (w, c)) in self.pts.windows(2).zip(&self.cum).enumerate() {
            let [a, b] = [w[0], w[1]];
            let d = b - a;
            let l2 = d.dot(d);
            if !l2.is_finite() || l2 <= 0.0 {
                continue;
            }
            let t = (p - a).dot(d) / l2;
            let t = match (self.closed, i == 0, i == last) {
                (false, true, true) => t,
                (false, true, false) => t.min(1.0),
                (false, false, true) => t.max(0.0),
                _ => t.clamp(0.0, 1.0),
            };
            let s = c + t * l2.sqrt();
            let dist = p.distance(a + d * t) + from.filter(|f| f.is_finite()).map_or(0.0, |f| self.apart(s, f) / 2.0);
            if dist.is_finite() && best.is_none_or(|(b, _)| dist < b) {
                best = Some((dist, s));
            }
        }
        best
    }
    /// How far apart arc lengths `s` and `f` are along the spine: the shorter way round a closed
    /// spine, across its first anchor.
    pub fn apart(&self, s: f64, f: f64) -> f64 {
        let len = self.length();
        let d = (s - f).abs();
        if self.closed && len > 0.0 { d.rem_euclid(len).min(len - d.rem_euclid(len)) } else { d }
    }
    /// Point and tangent angle (radians) at arc-length fraction `f` (0..1).
    pub fn at(&self, f: f64) -> (Point, f64) {
        self.at_length(f.clamp(0.0, 1.0) * self.length())
    }
    /// Point and tangent angle (radians) at arc length `s` (clamped to the spine). The point and
    /// tangent come from the original curve at that arc length, not from the flattened polyline: a
    /// flattened chord's direction is piecewise constant, so steps oriented to a curved spine with
    /// `Align to Path` jump between chords instead of following the curve smoothly.
    pub fn at_length(&self, s: f64) -> (Point, f64) {
        let s = if s.is_finite() { s.clamp(0.0, self.length()) } else { 0.0 };
        let sp = &self.subpath;
        let n = sp.segment_count();
        if n == 0 {
            return (self.pts.first().copied().unwrap_or_default(), 0.0);
        }
        let i = segment_at(&self.anchors, n, s, || {});
        let c = sp.segment(i);
        let start = if i == 0 { 0.0 } else { self.anchor_length(i).unwrap_or(0.0) };
        let end = self.anchor_length(i + 1).unwrap_or(self.length());
        let local_s = (s - start).max(0.0) / (end - start).max(1e-12) * c.arclen(1e-9);
        let t = c.inv_arclen(local_s, 1e-9).clamp(0.0, 1.0);
        let e = c.eval(t);
        let tv = c.deriv().eval(t).to_vec2();
        // Where a handle sits on its anchor the derivative vanishes there: take the
        // direction between points a little before and after instead.
        let tv = if tv.hypot2() < 1e-24 { c.eval((t + 1e-3).min(1.0)) - c.eval((t - 1e-3).max(0.0)) } else { tv };
        (Point::new(e.x, e.y), tv.y.atan2(tv.x))
    }
}

/// The segment of a spine with `n` segments (`n > 0`) that holds arc length `s`: the first one whose
/// end is at or past `s`, or the last one when none is. `anchors` holds the arc length at each
/// anchor, so segment `i` ends at `anchors[i + 1]`, and the last segment of a closed spine, which has
/// no entry for its end, ends at the spine's length. The lengths never decrease (each is a sum of
/// distances that are not negative, at most `+inf` and never NaN), so a binary search finds the
/// segment in about `log2(n)` comparisons. `probe` is called once for each comparison.
fn segment_at(anchors: &[f64], n: usize, s: f64, mut probe: impl FnMut()) -> usize {
    let ends = anchors.get(1..n.min(anchors.len())).unwrap_or_default();
    ends.partition_point(|&end| {
        probe();
        end < s
    })
    .min(n.saturating_sub(1))
}

/// The anchor of the first subpath each key sits on, when they are all valid for `n` anchors.
fn pinned(spec: &BlendSpec, keys: usize, n: usize) -> Option<Vec<usize>> {
    (spec.key_anchors.len() == keys && keys >= 2 && spec.key_anchors.iter().all(|a| (*a as usize) < n))
        .then(|| spec.key_anchors.iter().map(|a| *a as usize).collect())
}

/// The spine of a blend of `keys` and the anchor of it each key sits on:
/// - without a stored spine: the straight lines between the key centres;
/// - a pinned spine (`key_anchors`): the stored spine's first subpath with each key's anchor (and
///   its handles) moved onto the key's centre, so moving a key moves its end of the spine;
/// - a spine from Replace Spine: its first subpath, `None` for the anchors (the keys spread
///   evenly along it by arc length) while the keys sit there; once one is moved off, pinned as
///   above at the anchors where they were spread.
///
/// `None` for a blend made without a spine ([`BlendSpec::no_spine`]).
pub fn blend_spine(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<(PathData, Option<Vec<usize>>)> {
    if spec.no_spine {
        return None;
    }
    layout_spine(keys, spec)
}

/// The line the steps of a blend of `keys` are placed along: [`blend_spine`]'s path for a blend
/// with a spine, and the straight lines between the key centers, in key order, for a blend made
/// without one ([`BlendSpec::no_spine`]). For a blend without a spine the line places the steps
/// only; it is not a spine and is not drawn as one. `None` with fewer than two keys.
pub fn blend_layout_line(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<PathData> {
    layout_spine(keys, spec).map(|(path, _)| path)
}

/// The spine `spec` stores, unless the blend was made without one.
fn stored_spine(spec: &BlendSpec) -> Option<&PathData> {
    spec.spine.as_ref().filter(|_| !spec.no_spine)
}

/// [`blend_spine`] for placing the steps: a blend made without a spine still places them on the
/// straight lines between its key centers.
fn layout_spine(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<(PathData, Option<Vec<usize>>)> {
    if keys.len() < 2 {
        return None;
    }
    let Some(stored) = stored_spine(spec) else {
        let centers: Vec<Point> = keys.iter().map(|k| center_of(k)).collect();
        return Some((PathData::single(SubPath::polyline(&centers, false)), Some((0..keys.len()).collect())));
    };
    let mut first = stored.subpaths.first()?.clone();
    let Some(ka) = pinned(spec, keys.len(), first.anchors.len()) else {
        let mut path = PathData::single(first);
        // A key moved off where Replace Spine spread it takes its point of the spine along, as
        // on a pinned spine; otherwise the steps would put it back on the spine (#6809).
        if Spine::new(&path).is_none_or(|sp| keys_spread_along(keys, &sp)) {
            return Some((path, None));
        }
        let Some(mut ka) = spread_anchors(&mut path, keys.len()) else { return Some((path, None)) };
        let mut first = path.subpaths.into_iter().next()?;
        open_at_start(&mut first, &mut ka);
        for (k, &ai) in keys.iter().zip(&ka) {
            if let Some(a) = first.anchors.get_mut(ai) {
                let d = center_of(k) - a.p;
                a.translate(d);
            }
        }
        return Some((PathData::single(first), Some(ka)));
    };
    for (k, &ai) in keys.iter().zip(&ka) {
        let c = center_of(k);
        if let Some(a) = first.anchors.get_mut(ai) {
            let d = c - a.p;
            a.translate(d);
        }
    }
    Some((PathData::single(first), Some(ka)))
}

/// [`blend_spine`] with every key on an anchor: a spine from Replace Spine gets anchors where its
/// keys sit (by arc length), so editing it keeps them in place.
pub fn pin_spine(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<(PathData, Vec<usize>)> {
    // A spine from Replace Spine is pinned as stored, even with a key moved off it (where
    // [`blend_spine`] moves that key's point along): the layout is the same.
    if keys.len() >= 2
        && let Some(first) = stored_spine(spec).and_then(|s| s.subpaths.first())
        && pinned(spec, keys.len(), first.anchors.len()).is_none()
    {
        let mut path = PathData::single(first.clone());
        let mut out = spread_anchors(&mut path, keys.len())?;
        // A closed spine's end keys get points of their own, so that once pinned each still
        // moves only its own end.
        let mut sp = path.subpaths.into_iter().next()?;
        open_at_start(&mut sp, &mut out);
        return Some((PathData::single(sp), out));
    }
    let (path, anchors) = blend_spine(keys, spec)?;
    Some((path, anchors?))
}

/// Do `keys` sit where a spine from Replace Spine spreads them: evenly along `spine` by arc length
/// (within a hundredth of a point, as [`SpreadLengths::anchor_at_length`] matches anchors)?
fn keys_spread_along(keys: &[Arc<Node>], spine: &Spine) -> bool {
    let k = keys.len();
    keys.iter().enumerate().all(|(i, key)| {
        let f = if k > 1 { i as f64 / (k - 1) as f64 } else { 0.0 };
        center_of(key).distance(spine.at(f).0) < 0.01
    })
}

/// Open closed spine `sp` at its first anchor when the last key sits there with the first (as
/// [`spread_anchors`] puts them): a copy of that anchor at the end takes the closing segment and
/// the last key, so the two end keys move their ends of the spine apart.
fn open_at_start(sp: &mut SubPath, ka: &mut [usize]) {
    let Some(&start) = sp.anchors.first() else { return };
    if !sp.closed || ka.len() < 2 {
        return;
    }
    if let Some(last) = ka.last_mut().filter(|a| **a == 0) {
        *last = sp.anchors.len();
        sp.anchors.push(start);
        sp.closed = false;
    }
}

/// The anchors of `path`'s first subpath that `k` keys spread evenly along it by arc length sit
/// on, splitting segments where none is. `None`, leaving `path` as it was, when the subpath has no
/// length to spread along, when its length is not finite (a path so large its length overflows),
/// or when an arc length it would split at is not finite.
fn spread_anchors(path: &mut PathData, k: usize) -> Option<Vec<usize>> {
    spread_anchors_with(path, k, |_| {})
}

/// [`spread_anchors`], calling `work` for each step of work it does.
fn spread_anchors_with(path: &mut PathData, k: usize, mut work: impl FnMut(SpreadWork)) -> Option<Vec<usize>> {
    let sp = path.subpaths.first_mut()?;
    let lengths = SpreadLengths::new(sp, &mut work);
    let total = lengths.total().filter(|t| t.is_finite())?;
    let targets = (1..k.saturating_sub(1)).map(|i| total * i as f64 / (k - 1) as f64);
    let mut out = vec![0usize];
    out.extend(anchors_at_lengths(sp, lengths, targets, &mut work)?);
    out.push(if sp.closed { 0 } else { sp.anchors.len().saturating_sub(1) });
    Some(out)
}

/// The anchors of `sp` at each arc length in `targets`, in order, splitting segments where none is
/// (see [`SpreadLengths::anchor_at_length`]); `lengths` measures `sp`. The splits are made on a copy
/// of `sp`, which replaces it only when every target has its anchor: `None` leaves `sp` as it was.
fn anchors_at_lengths(
    sp: &mut SubPath,
    mut lengths: SpreadLengths,
    targets: impl IntoIterator<Item = f64>,
    work: &mut impl FnMut(SpreadWork),
) -> Option<Vec<usize>> {
    let mut split = sp.clone();
    let mut out = Vec::new();
    for s in targets {
        out.push(lengths.anchor_at_length(&mut split, s, work)?);
    }
    *sp = split;
    Some(out)
}

/// A step of the work [`spread_anchors_with`] does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SpreadWork {
    /// Flattening every segment of the subpath.
    WholePath,
    /// Flattening one segment after a split.
    Segment,
    /// Adding one segment's flattened points to the arc lengths.
    Measure,
    /// Comparing an anchor's arc length with the one looked for.
    Compare,
}

/// Where the measurement of a subpath's flattened points stands at the start of a segment: the last
/// point kept, the arc length there (0 before any point), and how many points were kept.
#[derive(Clone, Copy)]
struct Measured {
    last: Option<Point>,
    len: f64,
    kept: usize,
}

impl Measured {
    const START: Self = Self { last: None, len: 0.0, kept: 0 };

    /// The measurement after `pts`, keeping each point more than 1e-9 from the last one kept, as
    /// [`Spine::new`] does.
    fn then(mut self, pts: &[Point]) -> Self {
        for &p in pts {
            if self.last.is_none_or(|q| q.distance(p) > 1e-9) {
                self.len = match self.last {
                    Some(q) => self.len + q.distance(p),
                    None => 0.0,
                };
                self.last = Some(p);
                self.kept += 1;
            }
        }
        self
    }
}

/// The arc lengths of a subpath's anchors, as [`Spine::new`] measures them, kept up to date while
/// [`spread_anchors`] splits its segments. Each segment is flattened once, and again only when it
/// is split. A split leaves the arc lengths of the anchors before it as they are; those after it
/// are measured again, in order from the split, only as far as the next arc length looked for, so
/// the sums are the same as [`Spine::new`]'s on the split subpath.
struct SpreadLengths {
    /// The finite points the segments flatten to, each segment's in one run.
    pts: Vec<Point>,
    /// Where each segment's run is in `pts`. A split segment's halves get runs at the end, and its
    /// old run is not used again.
    flat: Vec<std::ops::Range<usize>>,
    /// The measurement at the start of each segment, up to the first one not measured since the
    /// last split; after the last segment too once every segment is measured.
    marks: Vec<Measured>,
}

impl SpreadLengths {
    /// Flatten and measure every segment of `sp`.
    fn new(sp: &SubPath, work: &mut impl FnMut(SpreadWork)) -> Self {
        work(SpreadWork::WholePath);
        let n = sp.segment_count();
        let mut lengths = Self { pts: Vec::new(), flat: Vec::with_capacity(n), marks: Vec::with_capacity(n + 1) };
        for i in 0..n {
            let run = lengths.flatten(sp, i);
            lengths.flat.push(run);
        }
        lengths.marks.push(Measured::START);
        lengths.measure_to(f64::INFINITY, work);
        lengths
    }

    /// Flatten segment `i` of `sp` as [`Spine::new`] does, adding its finite points to `pts`, and
    /// return where they are.
    fn flatten(&mut self, sp: &SubPath, i: usize) -> std::ops::Range<usize> {
        let start = self.pts.len();
        let c = sp.segment(i);
        let mut bp = BezPath::new();
        bp.move_to(c.p0);
        if sp.segment_is_line(i) {
            bp.line_to(c.p3);
        } else {
            bp.curve_to(c.p1, c.p2, c.p3);
        }
        vectorcraft_geom::kurbo::flatten(bp.iter(), 0.1, |el| {
            if let vectorcraft_geom::PathEl::MoveTo(p) | vectorcraft_geom::PathEl::LineTo(p) = el
                && p.x.is_finite()
                && p.y.is_finite()
            {
                self.pts.push(p);
            }
        });
        start..self.pts.len()
    }

    /// The subpath's flattened length, `None` when it flattens to fewer than two points.
    fn total(&self) -> Option<f64> {
        self.marks.last().filter(|m| self.marks.len() > self.flat.len() && m.kept >= 2).map(|m| m.len)
    }

    /// Measure the segments that are not measured, from the first, until one starts past `s` (with
    /// two points kept before it) or every one is measured.
    fn measure_to(&mut self, s: f64, work: &mut impl FnMut(SpreadWork)) {
        while let Some(run) = self.flat.get(self.marks.len() - 1) {
            let Some(&m) = self.marks.last() else { return };
            if self.marks.len() >= 2 && m.kept >= 2 && m.len > s {
                return;
            }
            work(SpreadWork::Measure);
            self.marks.push(m.then(self.pts.get(run.clone()).unwrap_or_default()));
        }
    }

    /// The anchor of `sp` at arc length `s`: an existing one within a hundredth of a point, else a
    /// new one splitting the segment there. The segment is the last one starting at or before `s`
    /// (the first one when none does), found by a binary search. `None`, leaving `sp` as it was,
    /// when `s` or the arc length at the end of the segment is not finite, or when `sp` flattens
    /// to fewer than two points.
    fn anchor_at_length(&mut self, sp: &mut SubPath, s: f64, work: &mut impl FnMut(SpreadWork)) -> Option<usize> {
        if !s.is_finite() {
            return None;
        }
        self.measure_to(s, work);
        let n = self.flat.len();
        if self.marks.last().is_none_or(|m| m.kept < 2) {
            return None;
        }
        let starts = &self.marks[..self.marks.len().min(n)];
        let seg = starts
            .partition_point(|m| {
                work(SpreadWork::Compare);
                m.len <= s
            })
            .saturating_sub(1);
        let from = self.marks.get(seg)?.len;
        let to = self.marks.get(seg + 1)?.len;
        if s - from < 0.01 {
            return Some(seg);
        }
        if to - s < 0.01 {
            return Some((seg + 1) % sp.anchors.len().max(1));
        }
        if !from.is_finite() || !to.is_finite() {
            return None;
        }
        let c = sp.segment(seg);
        // The flattened length and the curve's own differ slightly: scale into the curve's.
        let arc = c.arclen(1e-4).max(1e-12);
        let t = c.inv_arclen((s - from) / (to - from).max(1e-12) * arc, 1e-4).clamp(1e-4, 1.0 - 1e-4);
        if !t.is_finite() {
            return None;
        }
        let at = sp.insert_anchor(seg, t);
        work(SpreadWork::Segment);
        work(SpreadWork::Segment);
        let halves = [self.flatten(sp, seg), self.flatten(sp, seg + 1)];
        self.flat.splice(seg..=seg, halves);
        self.marks.truncate(seg + 1);
        Some(at)
    }
}

/// Where a blend's keys and steps go along its spine (see [`blend_spine`]).
struct Rail {
    spine: Spine,
    /// Arc length of each key.
    stops: Vec<f64>,
    centers: Vec<Point>,
    rotate: bool,
}

impl Rail {
    /// `None` when nothing moves: no stored spine and the steps keep the page's orientation.
    fn new(keys: &[Arc<Node>], spec: &BlendSpec) -> Option<Self> {
        let rotate = spec.orientation == BlendOrientation::AlignToPath;
        if stored_spine(spec).is_none() && !rotate {
            return None;
        }
        let (path, anchors) = layout_spine(keys, spec)?;
        let spine = Spine::new(&path)?;
        let total = spine.length();
        let k = keys.len();
        let stops = match anchors {
            Some(ka) => {
                let mut v: Vec<f64> = Vec::with_capacity(k);
                for a in ka {
                    let mut s = spine.anchor_length(a).unwrap_or(0.0);
                    // On a closed spine the key back at the start sits at its far end.
                    if spine.closed
                        && let Some(prev) = v.last()
                        && s <= *prev + 1e-9
                    {
                        s += total;
                    }
                    v.push(s);
                }
                v
            }
            None => (0..k).map(|i| total * i as f64 / (k - 1).max(1) as f64).collect(),
        };
        Some(Self { spine, stops, centers: keys.iter().map(|k| center_of(k)).collect(), rotate })
    }

    /// Arc length between keys `i` and `i + 1`.
    fn len(&self, i: usize) -> f64 {
        match (self.stops.get(i), self.stops.get(i + 1)) {
            (Some(a), Some(b)) => (b - a).abs(),
            _ => 0.0,
        }
    }

    /// Move `n`, the object at `t` between keys `i` and `i + 1` (a key at `t` = 0), onto the
    /// spine: by how far the spine there is from the straight line between the key centres (so
    /// steps keep their own interpolated position on a straight spine), turned to the spine's
    /// direction with Align to Path.
    fn place(&self, n: &mut Node, i: usize, t: f64) {
        let (Some(&s0), Some(&c0)) = (self.stops.get(i), self.centers.get(i)) else { return };
        let (s, base) = match (self.stops.get(i + 1), self.centers.get(i + 1)) {
            (Some(&s1), Some(&c1)) if t > 0.0 => (lerp_f64(s0, s1, t), c0.lerp(c1, t)),
            _ => (s0, c0),
        };
        let (p, ang) = self.spine.at_length(if self.spine.closed { s.rem_euclid(self.spine.length().max(1e-12)) } else { s });
        let off = p - base;
        let mut m = if off.hypot() > 1e-9 { Affine::translate(off) } else { Affine::IDENTITY };
        if self.rotate && ang.abs() > 1e-12 {
            let c = (center_of(n) + off).to_vec2();
            m = Affine::translate(c) * Affine::rotate(ang) * Affine::translate(-c) * m;
        }
        if m != Affine::IDENTITY {
            n.transform(m, false);
        }
    }
}

/// A box round everything a blend of `keys` can paint once [`Rail::place`] puts its keys and steps
/// on its spine, turned to it with Align to Path: the spine's box grown by three times the farthest
/// any key paints from its centre. A step's shape lies within that distance of the point on the
/// spine it is moved to, and so does its own centre, which it turns about. `None` when nothing
/// moves.
pub(crate) fn rail_reach(keys: &[Arc<Node>], spec: &BlendSpec, symbols: Option<&crate::SymbolBoxes>) -> Option<Rect> {
    if stored_spine(spec).is_none() && spec.orientation != BlendOrientation::AlignToPath {
        return None;
    }
    let spine = layout_spine(keys, spec)?.0.bounds()?;
    let radius = keys
        .iter()
        .filter_map(|k| {
            let (c, b) = (center_of(k), k.visual(symbols)?);
            let corners = [Point::new(b.x0, b.y0), Point::new(b.x1, b.y0), Point::new(b.x0, b.y1), Point::new(b.x1, b.y1)];
            Some(corners.into_iter().map(|q| (q - c).hypot()).fold(0.0, f64::max))
        })
        .fold(0.0, f64::max);
    Some(spine.inflate(3.0 * radius, 3.0 * radius))
}

// =====================================================================================
// Blend evaluation
// =====================================================================================

/// Whether `n`, every part of it, paints opaquely with Normal blending and no opacity mask or
/// effect: knocking such objects out of each other changes nothing.
fn paints_opaquely(n: &Node) -> bool {
    let paint = |p: &Paint| match p {
        Paint::Gradient(g) => g.gradient.stops.iter().all(|s| s.opacity >= 1.0),
        Paint::Pattern { .. } => false,
        _ => true,
    };
    n.opacity >= 1.0
        && n.blend == vectorcraft_color::BlendMode::Normal
        && n.extras().mask.is_none()
        && n.appearance.effects.is_empty()
        && n.appearance
            .items
            .iter()
            .all(|i| i.opacity() >= 1.0 && i.blend() == vectorcraft_color::BlendMode::Normal && i.effects().is_empty() && paint(i.paint()))
        && n.children().is_none_or(|c| c.iter().all(|c| paints_opaquely(c)))
}

/// Whether knocking out the children of `n` (a blend's evaluated steps) shows: some step is
/// translucent or blends.
pub fn knockout_shows(n: &Node) -> bool {
    n.children().is_some_and(|c| !c.iter().all(|c| paints_opaquely(c)))
}

/// [`knockout_shows`] for evaluated steps.
pub fn steps_knockout_shows(steps: &[Arc<Node>]) -> bool {
    !steps.iter().all(|c| paints_opaquely(c))
}

/// Most steps a blend makes in all, the steps of the blends inside its keys and steps included.
/// Each step of a blend whose keys are blends is a copy of one of them, which makes its own steps
/// when drawn, so nesting multiplies the step counts capped pair by pair: five levels of 15 steps
/// are 1.3 million steps from a 23 KB file, 3.4 GB to draw (#3579). Past it, a blend makes
/// proportionally fewer steps between each pair of keys ([`fit_steps`]).
pub const MAX_BLEND_STEPS: usize = 100_000;

/// The steps between each pair of keys, as the blend's spacing asks for them.
fn requested_steps(keys: &[Arc<Node>], spec: &BlendSpec, rail: Option<&Rail>) -> Vec<usize> {
    keys.windows(2)
        .enumerate()
        .map(|(i, w)| match w {
            [a, b] => blend_step_count(a, b, spec.spacing, rail.map_or_else(|| center_of(a).distance(center_of(b)), |r| r.len(i))),
            _ => 0,
        })
        .collect()
}

/// Most steps `spacing` makes between keys `a` and `b`, whatever their size and distance
/// ([`blend_step_count`]'s clamps). A copy of a blend is moved and scaled, never recolored
/// ([`Shape::Other`]), so Smooth Color between colors that differ makes no more than their color
/// change asks for, whatever the copy's scale or rotation. By distance, a copy turned by Align to
/// Path can measure its edges differently, so it counts as up to 1000.
fn most_steps(a: &Node, b: &Node, spacing: BlendSpacing) -> usize {
    match spacing {
        BlendSpacing::Steps(n) => n.clamp(1, 1000) as usize,
        BlendSpacing::Distance(_) => 1000,
        BlendSpacing::SmoothColor => match color_distance(a, b) {
            Some(d) if d * 255.0 > 1e-3 => level_steps(d) as usize,
            _ => 1000,
        },
    }
}

/// `want` steps between each pair of keys, fewer where they would make more than
/// [`MAX_BLEND_STEPS`] in all with `keys` steps already made by the keys and `each[i]` made by
/// one step of pair `i` (itself and what the blends in it make): every pair keeps the same share
/// of its steps, rounded down, and none when the keys alone fill the budget.
fn fit_steps(want: &[usize], each: &[usize], keys: usize) -> Vec<usize> {
    let room = MAX_BLEND_STEPS.saturating_sub(keys) as u128;
    let demand = want.iter().zip(each).map(|(&n, &c)| n as u128 * c as u128).fold(0u128, u128::saturating_add);
    if demand <= room {
        return want.to_vec();
    }
    // `demand` is more than `room`, so it isn't 0, and each share is below `n`.
    want.iter().map(|&n| usize::try_from(n as u128 * room / demand).unwrap_or(0)).collect()
}

/// Counts the steps blends make in all ([`MAX_BLEND_STEPS`]), each object counted once by its
/// address while the count runs.
#[derive(Default)]
struct StepCount {
    known: std::collections::HashMap<(usize, bool), usize>,
}

impl StepCount {
    /// Steps the blends in `n` make in all when drawn, each keeping to [`MAX_BLEND_STEPS`].
    /// `copy`: `n` is part of a blend's step, a copy moved and scaled into place. A blend spaced
    /// by distance, or by Smooth Color, can make more steps scaled up, so a copy counts the most
    /// its spacing can make between each pair ([`most_steps`]), an upper bound for any scale.
    fn node(&mut self, n: &Node, copy: bool) -> usize {
        let at = (std::ptr::from_ref(n) as usize, copy);
        if let Some(&known) = self.known.get(&at) {
            return known;
        }
        let count = match &n.kind {
            NodeKind::Blend { children, spec } if copy => self.most(children, spec),
            NodeKind::Blend { children, spec } => self.blend(children, spec, Rail::new(children, spec).as_ref()).1,
            // Each instance draws the source again.
            NodeKind::Repeat(r) => r.instances(None).saturating_mul(self.nodes(&r.source, copy)),
            _ => self.nodes(n.children().map_or(&[], Vec::as_slice), copy),
        };
        self.known.insert(at, count);
        count
    }

    fn nodes(&mut self, nodes: &[Arc<Node>], copy: bool) -> usize {
        nodes.iter().map(|c| self.node(c, copy)).fold(0, usize::saturating_add)
    }

    /// What one step between keys `a` and `b` makes: itself and the steps of the blends in it.
    /// Groups pair their children as [`Lerp::new`] does; anything else is a copy of the nearer key.
    fn step(&mut self, a: &Node, b: &Node) -> usize {
        let inner = match (&a.kind, &b.kind) {
            (NodeKind::Group { children: ca, clip: k1 }, NodeKind::Group { children: cb, clip: k2 })
                if k1 == k2 && !(*k1 && ca.len() != cb.len()) =>
            {
                (0..ca.len().max(cb.len()))
                    .map(|i| match (ca.get(i), cb.get(i)) {
                        (Some(x), Some(y)) => self.step(x, y).saturating_sub(1),
                        (Some(x), None) | (None, Some(x)) => self.node(x, true),
                        (None, None) => 0,
                    })
                    .fold(0, usize::saturating_add)
            }
            _ => self.node(a, true).max(self.node(b, true)),
        };
        inner.saturating_add(1)
    }

    /// What each step between each pair of `keys` makes ([`Self::step`]).
    fn steps(&mut self, keys: &[Arc<Node>]) -> Vec<usize> {
        keys.windows(2)
            .map(|w| match w {
                [a, b] => self.step(a, b),
                _ => 1,
            })
            .collect()
    }

    /// The steps between each pair of keys a blend makes ([`fit_steps`]), and the steps it makes
    /// in all, its keys' included.
    fn blend(&mut self, keys: &[Arc<Node>], spec: &BlendSpec, rail: Option<&Rail>) -> (Vec<usize>, usize) {
        let inside = self.nodes(keys, false);
        let each = self.steps(keys);
        let kept = fit_steps(&requested_steps(keys, spec, rail), &each, inside);
        let total = kept.iter().zip(&each).map(|(&n, &c)| n.saturating_mul(c)).fold(inside, usize::saturating_add);
        (kept, total)
    }

    /// The most steps a copy of a blend makes in all, at any scale: what [`Self::blend`] counts
    /// with [`most_steps`] between each pair, before rounding.
    fn most(&mut self, keys: &[Arc<Node>], spec: &BlendSpec) -> usize {
        let inside = self.nodes(keys, true);
        let each = self.steps(keys);
        let most = keys.windows(2).map(|w| match w {
            [a, b] => most_steps(a, b, spec.spacing),
            _ => 0,
        });
        let demand = each.iter().zip(most).map(|(&c, m)| c.saturating_mul(m)).fold(0, usize::saturating_add);
        inside.saturating_add(demand.min(MAX_BLEND_STEPS.saturating_sub(inside)))
    }
}

/// How many steps `keys` blended with `spec` make between each pair of keys: as many as the
/// spacing asks for, fewer when they and the steps of the blends nested in them would pass
/// [`MAX_BLEND_STEPS`] ([`blend_expand`] makes these).
pub fn blend_steps(keys: &[Arc<Node>], spec: &BlendSpec) -> Vec<usize> {
    StepCount::default().blend(keys, spec, Rail::new(keys, spec).as_ref()).0
}

/// Evaluate a blend: the keys and generated steps in paint order. The steps keep to
/// [`MAX_BLEND_STEPS`] in all ([`blend_steps`]).
pub fn blend_expand(keys: &[Arc<Node>], spec: &BlendSpec) -> Vec<Node> {
    let k = keys.len();
    if k < 2 {
        return keys.iter().map(|n| (**n).clone()).collect();
    }
    let rail = Rail::new(keys, spec);
    let (counts, _) = StepCount::default().blend(keys, spec, rail.as_ref());
    let mut out = Vec::new();
    for (i, a) in keys.iter().enumerate() {
        let mut key = (**a).clone();
        if let Some(r) = &rail {
            r.place(&mut key, i, 0.0);
        }
        out.push(key);
        let Some(b) = keys.get(i + 1) else { break };
        let n = counts.get(i).copied().unwrap_or(0);
        let lerp = Lerp::new(a, b, (spec.start(i), spec.start(i + 1)));
        for j in 1..=n {
            // Even position of the step, then where the easing puts its shape and its colours.
            let u = j as f64 / (n + 1) as f64;
            // Easing needs a spine: a blend made without one keeps its steps evenly spaced.
            let eased = rail.is_some() && spec.eases_along_spine();
            let (t, tc) = (if eased { spec.ease.apply(u) } else { u }, spec.color_position(u));
            let mut s = lerp.at_eased(t, tc);
            if let Some(r) = &rail {
                r.place(&mut s, i, t);
            }
            out.push(s);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::appearance::Arrowhead;
    use vectorcraft_geom::{Rect, shapes};

    #[test]
    fn clicked_start_points_pair_up() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut b = shapes::rectangle(Rect::new(100.0, 0.0, 110.0, 10.0));
        b.subpaths[0].anchors.rotate_left(2);
        // Starting both at their anchor 0 pairs each corner of `a` with the opposite one of `b`:
        // the middle step shrinks to a point.
        assert!(PathPair::new(&a, &b, (Some(0), Some(0))).at(0.5).bounds().unwrap().height() < 1e-6);
        // Starting `b` at its anchor 2 (its first corner again) pairs like corners.
        let r = PathPair::new(&a, &b, (Some(0), Some(2))).at(0.5).bounds().unwrap();
        assert!((r.width() - 10.0).abs() < 1e-6 && (r.height() - 10.0).abs() < 1e-6, "{r:?}");
        // An open path clicked at its end runs the other way.
        let line = |x: f64| PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0)], false));
        let m = PathPair::new(&line(0.0), &line(100.0), (None, Some(1))).at(0.5);
        assert_eq!(m.subpaths[0].anchors[0].p, Point::new(55.0, 0.0));
    }

    fn key(x: f64) -> Arc<Node> {
        Arc::new(Node::path(NodeId(1), vectorcraft_geom::shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default()))
    }

    #[test]
    fn a_replaced_spine_gets_anchors_where_its_keys_sit() {
        let keys = vec![key(0.0), key(100.0), key(200.0)];
        let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 50.0), Point::new(300.0, 50.0)], false));
        let spec = BlendSpec { spine: Some(line), ..Default::default() };
        let (path, anchors) = pin_spine(&keys, &spec).unwrap();
        assert_eq!(anchors, vec![0, 1, 2]);
        assert!((path.subpaths[0].anchors[1].p.x - 150.0).abs() < 1e-6, "the middle key sits halfway");
        // Pinned, each key's anchor follows it.
        let pinned = BlendSpec { spine: Some(path), key_anchors: vec![0, 1, 2], ..Default::default() };
        let (moved, _) = blend_spine(&[key(0.0), key(100.0), key(300.0)], &pinned).unwrap();
        assert_eq!(moved.subpaths[0].anchors[2].p, Point::new(305.0, 5.0));
    }

    #[test]
    fn a_blend_without_a_spine_has_a_layout_line_through_its_key_centers() {
        let at =
            |x: f64, y: f64| Arc::new(Node::path(NodeId(1), shapes::rectangle(Rect::new(x - 5.0, y - 5.0, x + 5.0, y + 5.0)), Appearance::default()));
        let line_of = |path: &PathData| path.subpaths.iter().flat_map(|s| s.anchors.iter().map(|a| a.p)).collect::<Vec<_>>();
        // A stored spine is ignored once the blend is made without one.
        let stored = PathData::single(SubPath::polyline(&[Point::new(0.0, 50.0), Point::new(300.0, 50.0)], false));
        let spec = BlendSpec { no_spine: true, spine: Some(stored), ..Default::default() };
        for keys in [vec![at(10.0, 20.0), at(110.0, 20.0)], vec![at(10.0, 20.0), at(110.0, 20.0), at(110.0, 220.0), at(-40.0, 0.0)]] {
            assert!(blend_spine(&keys, &spec).is_none(), "still no spine");
            let line = blend_layout_line(&keys, &spec).unwrap();
            assert_eq!(line.subpaths.len(), 1);
            assert!(!line.subpaths[0].closed);
            let centers: Vec<Point> = keys.iter().map(|k| center_of(k)).collect();
            assert_eq!(line_of(&line), centers, "in key order");
        }
        // With a spine, the layout line is the spine.
        let keys = vec![key(0.0), key(100.0), key(300.0)];
        assert_eq!(blend_layout_line(&keys, &BlendSpec::default()), blend_spine(&keys, &BlendSpec::default()).map(|(p, _)| p));
        assert!(blend_layout_line(&keys[..1], &spec).is_none(), "one key");
    }

    #[test]
    fn steps_follow_a_bent_spine() {
        let keys = vec![key(0.0), key(100.0)];
        // A spine through the key centres bent up through (55, -95).
        let spine = PathData::single(SubPath::polyline(&[Point::new(5.0, 5.0), Point::new(55.0, -95.0), Point::new(105.0, 5.0)], false));
        let spec = BlendSpec { spacing: BlendSpacing::Steps(1), spine: Some(spine), key_anchors: vec![0, 2], ..Default::default() };
        let out = blend_expand(&keys, &spec);
        let mid = out[1].geometric_bounds().unwrap().center();
        assert!(mid.distance(Point::new(55.0, -95.0)) < 1e-6, "{mid:?}");
        assert_eq!(out[0].geometric_bounds().unwrap().center(), Point::new(5.0, 5.0), "keys stay put");
    }

    #[test]
    fn orientations_follow_a_curved_spine_smoothly() {
        // A curved spine: from (0, 0) with handles (+50, +50) / (-50, -50) to (100, 0) with handles
        // (+50, -50) / (-50, +50) — the classic up-bow arc.
        let spine = PathData::single(SubPath::new(
            vec![Anchor::smooth(Point::new(0.0, 0.0), Point::new(50.0, 50.0)), Anchor::smooth(Point::new(100.0, 0.0), Point::new(50.0, -50.0))],
            false,
        ));
        let sp = Spine::new(&spine).unwrap();
        // Many steps along the bow: each step's tangent comes from the curve at its arc length,
        // not from a flattened chord — so the orientations must sweep the true angle, smoothly.
        let len = sp.length();
        let mut prev: Option<f64> = None;
        for i in 1..=200 {
            let (pt, ang) = sp.at_length(len * i as f64 / 201.0);
            if let Some(p) = prev {
                let step = (ang - p).abs().min(2.0 * std::f64::consts::PI - (ang - p).abs());
                assert!(step < 0.1, "tangent step {} is too big at {} ({}, {})", step, i, pt.x, pt.y);
            }
            prev = Some(ang);
        }
    }

    /// Where the spine starts at an anchor whose handle sits on it, the first step still faces along
    /// the curve (towards the next handle), not along a direction of zero length.
    #[test]
    fn a_handle_on_its_anchor_still_gives_the_curve_direction() {
        let spine = PathData::single(SubPath::new(
            vec![Anchor::corner(Point::new(0.0, 0.0)), Anchor::smooth(Point::new(100.0, 0.0), Point::new(150.0, -50.0))],
            false,
        ));
        let sp = Spine::new(&spine).unwrap();
        let (p, angle) = sp.at_length(0.0);
        assert!(p.x.abs() < 1e-6 && p.y.abs() < 1e-6, "{p:?}");
        assert!((angle - std::f64::consts::FRAC_PI_4).abs() < 0.05, "faces the in handle at (50, 50): {angle}");
        let (_, end) = sp.at_length(sp.length());
        assert!(end.is_finite(), "{end}");
    }

    /// [`Spine::at_length`] by walking the segments from the first, as a reference for the search.
    fn walked_at_length(sp: &Spine, s: f64) -> (Point, f64) {
        let s = if s.is_finite() { s.clamp(0.0, sp.length()) } else { 0.0 };
        let n = sp.subpath.segment_count();
        let mut start = 0.0;
        for i in 0..n {
            let c = sp.subpath.segment(i);
            let end = sp.anchor_length(i + 1).unwrap_or(sp.length());
            if s <= end || i + 1 == n {
                let local_s = (s - start).max(0.0) / (end - start).max(1e-12) * c.arclen(1e-9);
                let t = c.inv_arclen(local_s, 1e-9).clamp(0.0, 1.0);
                let e = c.eval(t);
                let tv = c.deriv().eval(t).to_vec2();
                let tv = if tv.hypot2() < 1e-24 { c.eval((t + 1e-3).min(1.0)) - c.eval((t - 1e-3).max(0.0)) } else { tv };
                return (Point::new(e.x, e.y), tv.y.atan2(tv.x));
            }
            start = end;
        }
        panic!("a spine without segments")
    }

    /// The search picks the same segment as walking them in order, so points and tangents stay the
    /// same: on open and closed spines, one segment, curves, and zero-length segments whose anchors
    /// share an arc length; at every anchor, between anchors, at both ends, past them, and for
    /// values that are not finite.
    #[test]
    fn spine_lookup_matches_walking_the_segments() {
        let p = Point::new;
        let bow = SubPath::new(
            vec![Anchor::smooth(p(0.0, 0.0), p(50.0, 50.0)), Anchor::smooth(p(100.0, 0.0), p(150.0, -50.0)), Anchor::corner(p(200.0, 0.0))],
            false,
        );
        let spines = [
            SubPath::polyline(&[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 0.0), p(10.0, 0.0), p(30.0, 5.0), p(30.0, 5.0)], false),
            SubPath::polyline(&[p(0.0, 0.0), p(0.0, 0.0), p(40.0, 0.0), p(40.0, 30.0)], true),
            SubPath::polyline(&[p(0.0, 0.0), p(40.0, 0.0), p(40.0, 30.0), p(0.0, 0.0)], true),
            SubPath::polyline(&[p(0.0, 0.0), p(25.0, 25.0)], false),
            SubPath::polyline(&[p(0.0, 0.0), p(25.0, 25.0)], true),
            bow.clone(),
            SubPath { closed: true, ..bow },
        ];
        for sub in spines {
            let sp = Spine::new(&PathData::single(sub)).unwrap();
            let len = sp.length();
            let mut lengths = vec![0.0, len, -1.0, len + 1.0, -len, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN];
            lengths.extend(sp.anchors.iter().flat_map(|&a| [a, a - 1e-9, a + 1e-9]));
            lengths.extend((0..=400).map(|k| len * k as f64 / 400.0));
            for s in lengths {
                assert_eq!(sp.at_length(s), walked_at_length(&sp, s), "s = {s} on {:?}", sp.anchors);
            }
        }
    }

    /// Finding the segment for an arc length on a spine of thousands of segments takes a few
    /// comparisons, not one for each segment before it.
    #[test]
    fn spine_lookup_compares_a_few_segment_ends() {
        let pts: Vec<Point> = (0..=4096).map(|i| Point::new(i as f64, if i % 2 == 0 { 0.0 } else { 1.0 })).collect();
        let sp = Spine::new(&PathData::single(SubPath::polyline(&pts, false))).unwrap();
        let n = sp.subpath.segment_count();
        assert_eq!(n, 4096);
        for s in [sp.length() - 0.5, sp.length(), sp.length() / 2.0, 0.25] {
            let mut compared = 0;
            let i = segment_at(&sp.anchors, n, s, || compared += 1);
            assert!(compared <= 13, "{compared} comparisons at {s}");
            assert_eq!(sp.at_length(s), walked_at_length(&sp, s));
            assert!(sp.anchors[i] <= s && (i + 1 == n || s <= sp.anchors[i + 1]), "segment {i} at {s}");
        }
    }

    /// [`spread_anchors`] by measuring the whole spine again for each key and walking its segments
    /// from the last, as a reference for the measurement kept up to date between keys.
    fn rebuilt_spread_anchors(path: &mut PathData, k: usize) -> Option<Vec<usize>> {
        let total = Spine::new(path)?.length();
        let closed = path.subpaths.first().is_some_and(|s| s.closed);
        let mut out = vec![0usize];
        for i in 1..k.saturating_sub(1) {
            let s = total * i as f64 / (k - 1) as f64;
            let spine = Spine::new(path)?;
            let sp = path.subpaths.first_mut()?;
            let n = sp.segment_count();
            let seg = (0..n).rev().find(|i| spine.anchor_length(*i).is_some_and(|l| l <= s)).unwrap_or(0);
            let from = spine.anchor_length(seg).unwrap_or(0.0);
            let to = spine.anchor_length(seg + 1).unwrap_or(spine.length());
            out.push(if s - from < 0.01 {
                seg
            } else if to - s < 0.01 {
                (seg + 1) % sp.anchors.len().max(1)
            } else {
                let c = sp.segment(seg);
                let arc = c.arclen(1e-4).max(1e-12);
                let t = c.inv_arclen((s - from) / (to - from).max(1e-12) * arc, 1e-4).clamp(1e-4, 1.0 - 1e-4);
                sp.insert_anchor(seg, t)
            });
        }
        let last = path.subpaths.first().map_or(0, |sp| if closed { 0 } else { sp.anchors.len().saturating_sub(1) });
        out.push(last);
        Some(out)
    }

    /// Spreading keys along a spine measured once and kept up to date gives the same anchors and
    /// the same spine as measuring it again for each key: on open and closed polylines and curves,
    /// zero-length segments, keys exactly on anchors, keys within, at and just past a hundredth of
    /// a point from an anchor, a closed curve whose keys go round past its first anchor, many keys
    /// in one segment, and paths with no segment. The closed spines open as [`pin_spine`] opens
    /// them.
    #[test]
    fn spreading_keys_matches_measuring_the_spine_for_each_key() {
        let p = Point::new;
        let bow = SubPath::new(
            vec![Anchor::smooth(p(0.0, 0.0), p(50.0, 50.0)), Anchor::smooth(p(100.0, 0.0), p(150.0, -50.0)), Anchor::corner(p(200.0, 0.0))],
            false,
        );
        let mut spines = vec![
            PathData::default(),
            PathData::single(SubPath::default()),
            PathData::single(SubPath::polyline(&[p(5.0, 5.0)], false)),
            PathData::single(SubPath::polyline(&[p(5.0, 5.0), p(5.0, 5.0)], false)),
            PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(100.0, 0.0)], false)),
            PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(100.0, 0.0)], true)),
            PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(10.0, 0.0), p(20.0, 0.0), p(30.0, 0.0)], false)),
            PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(10.0, 0.0), p(10.0, 0.0), p(10.0, 0.0), p(30.0, 5.0), p(30.0, 5.0)], false)),
            PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(0.0, 0.0), p(40.0, 0.0), p(40.0, 30.0)], true)),
            PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(40.0, 0.0), p(40.0, 30.0), p(0.0, 0.0)], true)),
            PathData::single(bow.clone()),
            PathData::single(SubPath { closed: true, ..bow }),
            shapes::ellipse(Rect::new(0.0, 0.0, 120.0, 80.0)),
        ];
        // An anchor near where the middle of three keys goes, 10 along a spine 20 long.
        for d in [0.0, 0.005, 0.0099, 0.01, 0.0101, 0.02] {
            for x in [10.0 - d, 10.0 + d] {
                spines.push(PathData::single(SubPath::polyline(&[p(0.0, 0.0), p(x, 0.0), p(20.0, 0.0)], false)));
            }
        }
        let mut compared = 0;
        for spine in &spines {
            for k in (0..=12).chain([17, 50, 200]) {
                let (mut a, mut b) = (spine.clone(), spine.clone());
                let (ka, kb) = (spread_anchors(&mut a, k), rebuilt_spread_anchors(&mut b, k));
                assert_eq!(ka, kb, "{k} keys on {spine:?}");
                assert_eq!(a, b, "{k} keys on {spine:?}");
                if let (Some(mut ka), Some(mut kb)) = (ka, kb) {
                    let (mut sa, mut sb) = (a.subpaths[0].clone(), b.subpaths[0].clone());
                    open_at_start(&mut sa, &mut ka);
                    open_at_start(&mut sb, &mut kb);
                    assert_eq!((sa, ka), (sb, kb), "{k} keys on {spine:?}, opened");
                    compared += 1;
                }
            }
        }
        assert!(compared > 200, "{compared} spreads compared");
    }

    /// Spreading 1,000 keys along a spine of 4,096 segments flattens the spine once and then only
    /// the two halves of each split segment, finds each key's segment in a few comparisons, and
    /// measures about as many segments as the spine ends up with, not the whole spine per key.
    #[test]
    fn spreading_keys_measures_the_spine_once() {
        let pts: Vec<Point> = (0..=4096).map(|i| Point::new(i as f64, if i % 2 == 0 { 0.0 } else { 1.0 })).collect();
        let mut path = PathData::single(SubPath::polyline(&pts, false));
        let (mut whole, mut segments, mut measured, mut compared) = (0, 0, 0, 0);
        let ka = spread_anchors_with(&mut path, 1000, |w| match w {
            SpreadWork::WholePath => whole += 1,
            SpreadWork::Segment => segments += 1,
            SpreadWork::Measure => measured += 1,
            SpreadWork::Compare => compared += 1,
        })
        .unwrap();
        assert_eq!(ka.len(), 1000);
        let n = path.subpaths[0].segment_count();
        assert!(n > 4096 + 900, "{n} segments");
        assert_eq!(whole, 1);
        assert!(segments <= 2 * 998, "{segments} segments flattened");
        assert!(compared <= 998 * 14, "{compared} comparisons");
        assert!(measured <= 4096 + n + 3 * 1000, "{measured} segments measured for {n}");
        let mut reference = PathData::single(SubPath::polyline(&pts, false));
        assert_eq!(rebuilt_spread_anchors(&mut reference, 1000), Some(ka));
        assert_eq!(reference, path);
    }

    /// A spine so large that its length overflows spreads no keys, and is left as it was, rather
    /// than getting an anchor split at a parameter that is not a number. Two keys, which split
    /// nothing, get no anchors either.
    #[test]
    fn spreading_keys_along_a_spine_whose_length_overflows_changes_nothing() {
        let p = Point::new;
        for closed in [false, true] {
            for k in [2, 3, 5] {
                let spine = PathData::single(SubPath::polyline(&[p(-1e308, 0.0), p(0.0, 0.0), p(1e308, 0.0)], closed));
                let mut path = spine.clone();
                assert_eq!(spread_anchors(&mut path, k), None, "closed {closed}, {k} keys");
                assert_eq!(path, spine);
                assert!(path.subpaths[0].anchors.iter().all(|a| a.p.x.is_finite() && a.h_in.x.is_finite() && a.h_out.x.is_finite()));
            }
        }
    }

    /// When a later arc length has no anchor (here one that is not finite, as when a spine's length
    /// overflows only after earlier splits), the splits already made are dropped with it.
    #[test]
    fn a_failed_anchor_keeps_none_of_the_earlier_splits() {
        let sp = SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false);
        for bad in [f64::NAN, f64::INFINITY] {
            let mut split = sp.clone();
            let lengths = SpreadLengths::new(&split, &mut |_| {});
            assert_eq!(anchors_at_lengths(&mut split, lengths, [25.0, 50.0, bad], &mut |_| {}), None);
            assert_eq!(split, sp);
        }
        let mut split = sp.clone();
        let lengths = SpreadLengths::new(&split, &mut |_| {});
        assert_eq!(anchors_at_lengths(&mut split, lengths, [25.0, 50.0], &mut |_| {}), Some(vec![1, 2]));
        assert_eq!(split.anchors.len(), 4);
    }

    #[test]
    fn easing_keeps_the_ends_and_stays_monotone() {
        use crate::live::{BlendEase, BlendEaseType as K};
        for kind in [K::In, K::Out, K::InOut, K::OutIn] {
            for ramp in [0.0, 0.5, 1.0, f64::NAN, f64::INFINITY] {
                let e = BlendEase { kind, ramp };
                assert_eq!((e.apply(0.0), e.apply(1.0)), (0.0, 1.0));
                let ys: Vec<f64> = (0..=100).map(|i| e.apply(i as f64 / 100.0)).collect();
                assert!(ys.windows(2).all(|w| w[0] <= w[1]));
                assert_eq!(e.apply(f64::NAN), 0.0);
                if ramp == 0.0 {
                    assert_eq!(e.apply(0.25), 0.25);
                }
            }
        }
        let e = |kind| BlendEase { kind, ramp: 1.0 };
        assert!(e(K::In).apply(0.25) < 0.25);
        assert!(e(K::Out).apply(0.25) > 0.25);
        assert!(e(K::InOut).apply(0.25) < 0.25 && e(K::InOut).apply(0.75) > 0.75);
        assert!(e(K::OutIn).apply(0.25) > 0.25 && e(K::OutIn).apply(0.75) < 0.75);
    }

    #[test]
    fn colour_easing_moves_colours_apart_from_positions() {
        let rect = |x: f64, c: Color| {
            let mut n = Node::path(NodeId(1), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default());
            n.appearance = Appearance::basic(Paint::solid(c), Paint::None, 0.0).into();
            Arc::new(n)
        };
        let keys = vec![rect(0.0, Color::BLACK), rect(100.0, Color::WHITE)];
        let mid = |spec: &BlendSpec| {
            let out = blend_expand(&keys, spec);
            let n = &out[1];
            (n.geometric_bounds().unwrap().x0, n.appearance.fill_paint())
        };
        // Position easing runs along a spine: Align to Path gives the blend one.
        let base = BlendSpec { spacing: BlendSpacing::Steps(1), orientation: BlendOrientation::AlignToPath, ..Default::default() };
        let (x0, p0) = mid(&base);
        // Colour easing alone leaves the position and moves the colour.
        let colour = BlendSpec { color_shift: 50.0, ..base.clone() };
        let (x1, p1) = mid(&colour);
        assert!((x0 - x1).abs() < 1e-9 && p0 != p1);
        // Position easing alone leaves the colour and moves the position.
        let pos = BlendSpec { ease: crate::live::BlendEase { kind: crate::live::BlendEaseType::In, ramp: 1.0 }, ..base.clone() };
        let (x2, p2) = mid(&pos);
        assert!((x0 - x2).abs() > 1e-3 && p0 == p2);
        assert_eq!(mid(&pos).0, x2, "deterministic");
    }

    #[test]
    fn affine_lerp_turns_the_short_way() {
        let x = Affine::rotate(170f64.to_radians());
        let y = Affine::rotate(-170f64.to_radians());
        let m = lerp_affine(x, y, 0.5);
        let ang = decompose(m)[2].to_degrees();
        assert!((ang.abs() - 180.0).abs() < 1e-6, "{ang}");
        let s = lerp_affine(Affine::scale(1.0), Affine::translate((10.0, 0.0)) * Affine::scale(3.0), 0.5);
        assert!((s.as_coeffs()[0] - 2.0).abs() < 1e-9 && (s.as_coeffs()[4] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn closed_paths_start_where_they_twist_least() {
        let a = shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let mut b = shapes::rectangle(Rect::new(100.0, 0.0, 110.0, 10.0));
        b.subpaths[0].anchors.rotate_left(2);
        let mid = lerp_path(&a, &b, 0.5);
        let r = mid.bounds().unwrap();
        assert!((r.width() - 10.0).abs() < 1e-6 && (r.height() - 10.0).abs() < 1e-6, "no twist: {r:?}");
        // Clicked start points win over the automatic match.
        let pair = PathPair::new(&a, &b, (Some(0), Some(0)));
        assert!(pair.at(0.5).bounds().unwrap().height() < 1e-6, "the clicked anchors pair up");
    }

    /// A blend between two gradients that repeat steps the count; one between a gradient that
    /// pads and one that repeats switches halfway, as the method does.
    #[test]
    fn blend_between_repeating_gradients_steps_the_count() {
        use vectorcraft_color::GradientSpread;
        let spread = |spread, repeats| Paint::Gradient(Box::new(GradientPaint::new(Gradient { spread, repeats, ..Gradient::default() })));
        let at = |a: &Paint, b: &Paint, t: f32| match lerp_paint(a, b, t) {
            Paint::Gradient(g) => (g.gradient.spread, g.gradient.repeat_count()),
            p => panic!("a gradient, not {p:?}"),
        };
        let (two, twenty) = (spread(GradientSpread::Repeat, 2), spread(GradientSpread::Repeat, 20));
        assert_eq!(at(&two, &twenty, 0.25), (GradientSpread::Repeat, 7));
        assert_eq!(at(&two, &twenty, 0.5), (GradientSpread::Repeat, 11));
        assert_eq!(at(&two, &twenty, 1.0), (GradientSpread::Repeat, 20));
        let pad = spread(GradientSpread::Pad, 1);
        assert_eq!(at(&pad, &twenty, 0.25), (GradientSpread::Pad, 1));
        assert_eq!(at(&pad, &twenty, 0.75), (GradientSpread::Repeat, 20));
    }

    #[test]
    fn blend_between_tints_of_one_swatch_keeps_the_link() {
        let tinted = |tint: f32| Paint::Solid { color: Color::cmyk(1.0, 0.7, 0.0, 0.4).tinted(tint), swatch: Some("Spot".into()), tint };
        let Paint::Solid { swatch, tint, .. } = lerp_paint(&tinted(0.2), &tinted(1.0), 0.5) else { panic!() };
        assert_eq!(swatch.as_deref(), Some("Spot"));
        assert!((tint - 0.6).abs() < 1e-6, "{tint}");
        let other = Paint::Solid { color: Color::BLACK, swatch: Some("Other".into()), tint: 1.0 };
        assert!(matches!(lerp_paint(&tinted(0.2), &other, 0.5), Paint::Solid { swatch: None, .. }));

        // Gradient stops on tints of one swatch stay linked too; a process solid unlinks them.
        let spot_gradient = |tint: f32| {
            let mut stop = GradientStop::new(0.0, Color::cmyk(1.0, 0.7, 0.0, 0.4).tinted(tint));
            stop.swatch = Some("Spot".into());
            stop.tint = tint;
            let stops = vec![stop.clone(), GradientStop { offset: 1.0, ..stop }];
            Paint::Gradient(Box::new(GradientPaint::new(Gradient { stops, ..Gradient::default() })))
        };
        let Paint::Gradient(m) = lerp_paint(&spot_gradient(0.2), &spot_gradient(1.0), 0.5) else { panic!() };
        assert!(m.gradient.stops.iter().all(|s| s.swatch.as_deref() == Some("Spot") && (s.tint - 0.6).abs() < 1e-6), "{:?}", m.gradient.stops);
        let Paint::Gradient(m) = lerp_paint(&tinted(0.2), &spot_gradient(1.0), 0.5) else { panic!() };
        assert!(m.gradient.stops.iter().all(|s| s.swatch.as_deref() == Some("Spot") && (s.tint - 0.6).abs() < 1e-6), "{:?}", m.gradient.stops);
        let Paint::Gradient(m) = lerp_paint(&Paint::solid(Color::BLACK), &spot_gradient(1.0), 0.5) else { panic!() };
        assert!(m.gradient.stops.iter().all(|s| s.swatch.is_none()), "{:?}", m.gradient.stops);
    }

    #[test]
    fn resampled_gradient_stops_keep_the_swatch_link() {
        let spot = |offset: f32, tint: f32| {
            let mut stop = GradientStop::new(offset, Color::cmyk(1.0, 0.7, 0.0, 0.4).tinted(tint));
            stop.swatch = Some("Spot".into());
            stop.tint = tint;
            stop
        };
        let paint = |stops: Vec<GradientStop>| Paint::Gradient(Box::new(GradientPaint::new(Gradient { stops, ..Gradient::default() })));
        let two = paint(vec![spot(0.0, 0.2), spot(1.0, 1.0)]);
        let three = paint(vec![spot(0.0, 0.2), spot(0.5, 0.6), spot(1.0, 1.0)]);
        let Paint::Gradient(m) = lerp_paint(&two, &three, 0.5) else { panic!() };
        assert_eq!(m.gradient.stops.len(), 3);
        assert!(m.gradient.stops.iter().all(|s| s.swatch.as_deref() == Some("Spot")), "{:?}", m.gradient.stops);
        let tints: Vec<f32> = m.gradient.stops.iter().map(|s| s.tint).collect();
        assert!((tints[1] - 0.6).abs() < 1e-6 && (tints[2] - 1.0).abs() < 1e-6, "{tints:?}");

        // Spot on one side, process on the other: the stops are unlinked.
        let mut plain = spot(0.5, 1.0);
        plain.swatch = None;
        let mixed = paint(vec![spot(0.0, 1.0), plain, spot(1.0, 1.0)]);
        let Paint::Gradient(m) = lerp_paint(&two, &mixed, 0.5) else { panic!() };
        assert!(m.gradient.stops.iter().any(|s| s.swatch.is_none()), "{:?}", m.gradient.stops);
    }

    #[test]
    fn gradients_with_different_stop_counts_interpolate() {
        let g = |stops: &[(f32, Color)]| {
            let gradient = Gradient { stops: stops.iter().map(|(o, c)| GradientStop::new(*o, *c)).collect(), ..Gradient::default() };
            Paint::Gradient(Box::new(GradientPaint::new(gradient)))
        };
        let a = g(&[(0.0, Color::BLACK), (1.0, Color::BLACK)]);
        let b = g(&[(0.0, Color::WHITE), (0.5, Color::WHITE), (1.0, Color::WHITE)]);
        let Paint::Gradient(m) = lerp_paint(&a, &b, 0.25) else { panic!() };
        assert_eq!(m.gradient.stops.len(), 3);
        assert!(m.gradient.stops.iter().all(|s| (s.color.to_rgb()[0] - 0.25).abs() < 1e-4), "{:?}", m.gradient.stops);
    }

    #[test]
    fn dashes_and_profiles_interpolate() {
        let d = lerp_dash(Some(&Dash { pattern: vec![10.0, 10.0], ..Default::default() }), None, 0.5).unwrap();
        assert_eq!(d.pattern, vec![15.0, 5.0]);
        let d =
            lerp_dash(Some(&Dash { pattern: vec![4.0], ..Default::default() }), Some(&Dash { pattern: vec![2.0, 6.0], ..Default::default() }), 0.5);
        assert_eq!(d.unwrap().pattern, vec![3.0, 5.0]);
        let p = lerp_profile(Some(&WidthProfile::lens()), None, 0.5).unwrap();
        assert_eq!(p.at(0.0), (0.5, 0.5));
        assert_eq!(p.at(0.5), (1.0, 1.0));
    }

    #[test]
    fn pen_pressure_tilt_and_angles_interpolate_for_brushes() {
        let stroked = |pressure: Option<f64>, tilt: Option<f64>, bearing: Option<f64>| {
            let mut ap = Appearance::default();
            ap.set_stroke(Paint::solid(Color::BLACK));
            if let Some(s) = ap.stroke_mut() {
                s.brush = Some("Pressure Round".into());
                s.pressure = pressure.map(|p| PressureProfile { points: vec![(0.0, p), (1.0, p)] });
                s.tilt = tilt.map(|p| PressureProfile { points: vec![(0.0, p), (1.0, p)] });
                s.bearing = bearing.map(|a| AngleProfile { points: vec![(0.0, a), (1.0, a)] });
            }
            ap
        };
        let at = |ap: &Appearance, f: f64| {
            let s = ap.stroke().cloned().unwrap_or_else(|| panic!("a stroke"));
            let read = |p: Option<&PressureProfile>| p.map_or(PressureProfile::MID, |p| p.at(f));
            (read(s.pressure.as_ref()), read(s.tilt.as_ref()), s.bearing.as_ref().map_or(0.0, |p| p.at(f)))
        };
        let (light, heavy) = (stroked(Some(0.0), Some(0.2), Some(170.0)), stroked(Some(1.0), Some(0.6), Some(-170.0)));
        let (p, tilt, bearing) = at(&lerp_appearance(&light, &heavy, 0.25), 0.5);
        assert!((p - 0.25).abs() < 1e-9 && (tilt - 0.3).abs() < 1e-9, "a quarter of the way: {p} {tilt}");
        assert!((bearing - 175.0).abs() < 1e-9, "the angle turns the shorter way round, through 180°: {bearing}");
        assert!((at(&lerp_appearance(&light, &heavy, 0.5), 0.5).2.abs() - 180.0).abs() < 1e-9);
        // A stroke drawn without a recording reads as half pressure, half tilt and 0°.
        let plain = stroked(None, None, None);
        let (p, tilt, bearing) = at(&lerp_appearance(&heavy, &plain, 0.5), 0.0);
        assert!((p - 0.75).abs() < 1e-9 && (tilt - 0.55).abs() < 1e-9 && (bearing + 85.0).abs() < 1e-9, "{p} {tilt} {bearing}");
        // Profiles that vary along the stroke interpolate at each position.
        let mut ramp = stroked(None, None, None);
        if let Some(s) = ramp.stroke_mut() {
            s.pressure = Some(PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] });
        }
        let mid = lerp_appearance(&ramp, &heavy, 0.5);
        assert!((at(&mid, 0.0).0 - 0.5).abs() < 1e-9 && (at(&mid, 0.5).0 - 0.75).abs() < 1e-9, "{:?}", mid.stroke().map(|s| &s.pressure));
        // A step in a recording (two samples at one position) stays a step.
        let mut stepped = stroked(None, None, None);
        if let Some(s) = stepped.stroke_mut() {
            s.pressure = Some(PressureProfile { points: vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)] });
        }
        let mid = lerp_appearance(&stepped, &stepped, 0.5);
        assert!(at(&mid, 0.25).0.abs() < 1e-9 && (at(&mid, 0.75).0 - 1.0).abs() < 1e-9, "{:?}", mid.stroke().map(|s| &s.pressure));
        // A whole turn recorded along a stroke stays a whole turn in the steps.
        let mut turning = stroked(None, None, None);
        if let Some(s) = turning.stroke_mut() {
            s.rotation = Some(AngleProfile { points: vec![(0.0, 0.0), (1.0, 360.0)] });
        }
        let rot = |ap: &Appearance, f: f64| ap.stroke().and_then(|s| s.rotation.as_ref()).map_or(0.0, |p| p.at(f));
        let same = lerp_appearance(&turning, &turning, 0.5);
        assert!((rot(&same, 0.25) - 90.0).abs() < 1e-9 && (rot(&same, 0.5).abs() - 180.0).abs() < 1e-9, "{:?}", same.stroke().map(|s| &s.rotation));
        let half = lerp_appearance(&turning, &plain, 0.5);
        assert!((rot(&half, 0.5) - 90.0).abs() < 1e-9 && (rot(&half, 1.0).abs() - 180.0).abs() < 1e-9, "{:?}", half.stroke().map(|s| &s.rotation));
        // Neither key has a recording: the steps have none either.
        let none = lerp_appearance(&plain, &plain, 0.5);
        let s = none.stroke().cloned().unwrap_or_else(|| panic!("a stroke"));
        assert!(s.pressure.is_none() && s.tilt.is_none() && s.bearing.is_none() && s.rotation.is_none());
    }

    #[test]
    fn a_point_near_the_other_keys_step_keeps_the_step() {
        let flat = PressureProfile { points: vec![(0.0, 0.0), (0.4999995, 0.0), (1.0, 0.0)] };
        let stepped = PressureProfile { points: vec![(0.0, 0.0), (0.5, 0.0), (0.5, 1.0), (1.0, 1.0)] };
        let p = lerp_pressure(Some(&flat), Some(&stepped), 1.0).unwrap_or_default();
        assert!((p.at(0.75) - 1.0).abs() < 1e-9, "{p:?}");
        let narrow = WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.4999995, 1.0, 1.0), (1.0, 1.0, 1.0)] };
        let stepped = WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.5, 1.0, 1.0), (0.5, 0.0, 0.0), (1.0, 0.0, 0.0)] };
        let w = lerp_profile(Some(&narrow), Some(&stepped), 1.0).unwrap_or_default();
        assert!(w.at(0.75).0.abs() < 1e-9, "{w:?}");
        // The other key's step doesn't reshape a smooth profile's curve.
        let early = WidthProfile { points: vec![(0.0, 1.0, 1.0), (0.25, 1.0, 1.0), (0.25, 0.5, 0.5), (1.0, 0.5, 0.5)] };
        let w = lerp_profile(Some(&WidthProfile::lens()), Some(&early), 0.0).unwrap_or_default();
        for u in [0.1, 0.125, 0.2, 0.3, 0.4] {
            let (got, want) = (w.at(u).0, WidthProfile::lens().at(u).0);
            assert!((got - want).abs() < 5e-3, "at {u}: {got} for {want}: {w:?}");
        }
        let lens = WidthProfile::lens();
        assert_eq!(lerp_profile(Some(&lens), Some(&lens), 0.5).and_then(|p| p.preset_id()), Some("lens"));
    }

    #[test]
    fn stroke_profiles_turn_with_a_key_path_started_elsewhere() {
        let keyed = |path: PathData, edit: &dyn Fn(&mut StrokeLayer)| {
            let mut ap = Appearance::default();
            ap.set_stroke(Paint::solid(Color::BLACK));
            if let Some(s) = ap.stroke_mut() {
                edit(s);
            }
            Node::path(NodeId(1), path, ap)
        };
        let stroke = |n: &Node| n.appearance.stroke().cloned().unwrap_or_else(|| panic!("a stroke"));
        let pressure = |s: &StrokeLayer, f: f64| s.pressure.as_ref().map_or(PressureProfile::MID, |p| p.at(f));
        // An open key clicked at its last anchor runs the other way, and so do its pressure, its
        // width profile (whose sides swap) and its arrowheads.
        let line = |x: f64| PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 10.0, 0.0)], false));
        let light = keyed(line(0.0), &|s| s.pressure = Some(PressureProfile { points: vec![(0.0, 0.0), (1.0, 0.0)] }));
        let ramp = keyed(line(100.0), &|s| {
            s.pressure = Some(PressureProfile { points: vec![(0.0, 1.0), (1.0, 0.0)] });
            s.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 0.0), (1.0, 0.0, 0.0)] });
            s.start_arrow = Some(Arrowhead::Triangle);
            s.arrow_scale = (200.0, 100.0);
        });
        let blend = Lerp::new(&light, &ramp, (None, Some(1)));
        let s = stroke(&blend.at(0.25));
        assert!(pressure(&s, 0.0).abs() < 1e-9 && (pressure(&s, 1.0) - 0.25).abs() < 1e-9, "{:?}", s.pressure);
        let s = stroke(&blend.at(1.0));
        assert_eq!(s.profile.as_ref().map(|p| p.at(1.0)), Some((0.0, 1.0)), "{:?}", s.profile);
        assert_eq!((s.start_arrow, s.end_arrow, s.arrow_scale), (None, Some(Arrowhead::Triangle), (100.0, 200.0)));
        // A key whose other subpaths keep their direction keeps its one stroke as it is.
        let two = |x: f64| {
            let sub = |y: f64| SubPath::polyline(&[Point::new(x, y), Point::new(x + 10.0, y)], false);
            PathData::new(vec![sub(0.0), sub(20.0)])
        };
        let ramps = keyed(two(100.0), &|s| s.pressure = Some(PressureProfile { points: vec![(0.0, 1.0), (1.0, 0.0)] }));
        let s = stroke(&Lerp::new(&keyed(two(0.0), &|_| {}), &ramps, (None, Some(1))).at(1.0));
        assert!((pressure(&s, 0.0) - 1.0).abs() < 1e-9, "{:?}", s.pressure);
        // A closed key started half way round starts its recordings there too, keeping the step
        // where its own start was and the whole turn its rotation makes.
        let square = |x: f64| shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0));
        let plain = keyed(square(0.0), &|_| {});
        let turning = keyed(square(100.0), &|s| {
            s.pressure = Some(PressureProfile { points: vec![(0.0, 0.0), (1.0, 1.0)] });
            s.rotation = Some(AngleProfile { points: vec![(0.0, 0.0), (1.0, 360.0)] });
            s.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 0.0, 0.0)] });
        });
        let blend = Lerp::new(&plain, &turning, (Some(0), Some(2)));
        let s = stroke(&blend.at(1.0));
        assert!((pressure(&s, 0.25) - 0.75).abs() < 1e-9 && (pressure(&s, 0.75) - 0.25).abs() < 1e-9, "{:?}", s.pressure);
        let width = s.profile.as_ref().map_or((1.0, 1.0), |p| p.at(0.75));
        assert!((width.0 - 0.75).abs() < 1e-6 && (width.1 - 0.75).abs() < 1e-6, "the width steps where the key started: {:?}", s.profile);
        // A width curve split by the new start keeps its shape.
        let lensed = keyed(square(100.0), &|s| s.profile = Some(WidthProfile::lens()));
        let s = stroke(&Lerp::new(&plain, &lensed, (Some(0), Some(1))).at(1.0));
        for u in [0.05, 0.125, 0.2, 0.6, 0.7, 0.74, 0.76, 0.8, 0.875, 0.95] {
            let (got, want) = (s.profile.as_ref().map_or(1.0, |p| p.at(u).0), WidthProfile::lens().at((u + 0.25) % 1.0).0);
            assert!((got - want).abs() < 2e-3, "at {u}: {got} for {want}: {:?}", s.profile);
        }
        let rot = |t: f64, f: f64| stroke(&blend.at(t)).rotation.as_ref().map_or(0.0, |p| p.unwrapped_at(f));
        assert!((vectorcraft_geom::normalize_deg(rot(1.0, 0.25)) + 90.0).abs() < 1e-6, "{:?}", s.rotation);
        assert!((rot(0.5, 0.499) - rot(0.5, 0.501)).abs() < 1.0, "no jump where the key started: {:?}", stroke(&blend.at(0.5)).rotation);
    }

    #[test]
    fn stroke_gradients_and_dashes_turn_with_a_key_path_started_elsewhere() {
        let keyed = |path: PathData, edit: &dyn Fn(&mut StrokeLayer)| {
            let mut ap = Appearance::default();
            ap.set_stroke(Paint::solid(Color::BLACK));
            if let Some(s) = ap.stroke_mut() {
                edit(s);
            }
            Node::path(NodeId(1), path, ap)
        };
        let stroke = |n: &Node| n.appearance.stroke().cloned().unwrap_or_else(|| panic!("a stroke"));
        // How light the stroke's gradient is at `u` (0 black, 3 white).
        let light = |s: &StrokeLayer, u: f32| match &s.paint {
            Paint::Gradient(g) => g.gradient.sample(u).0.to_rgb().iter().sum::<f32>(),
            p => panic!("a gradient: {p:?}"),
        };
        // Whether dash pattern `d` draws a dash `at` this far along its subpath.
        let dash_at = |d: &Dash, at: f64| {
            let p = if d.pattern.len() % 2 == 1 { d.pattern.repeat(2) } else { d.pattern.clone() };
            let mut x = (at + d.offset).rem_euclid(p.iter().sum());
            for (i, v) in p.iter().enumerate() {
                if x < *v {
                    return i % 2 == 0;
                }
                x -= v;
            }
            false
        };
        let graded = |mode: StrokeGradientMode| {
            move |s: &mut StrokeLayer| {
                s.paint = Paint::Gradient(Box::new(GradientPaint::new(Gradient::default())));
                s.gradient_mode = mode;
                s.dash = Some(Dash { pattern: vec![1.0, 2.0, 3.0, 1.5], offset: 0.75, align_corners: false });
            }
        };
        // An open key clicked at its last anchor runs the other way: a gradient along it or across
        // it (whose sides swap) runs the other way too, and its dashes are read from its end.
        let line = |x: f64| PathData::single(SubPath::polyline(&[Point::new(x, 0.0), Point::new(x + 13.0, 0.0)], false));
        let plain = keyed(line(0.0), &|_| {});
        for mode in [StrokeGradientMode::Along, StrokeGradientMode::Across] {
            let key = keyed(line(100.0), &graded(mode));
            let s = stroke(&Lerp::new(&plain, &key, (None, Some(1))).at(1.0));
            assert!(light(&s, 0.0) < 0.01 && light(&s, 1.0) > 2.99, "{mode:?}: {:?}", s.paint);
            let (own, d) = (stroke(&key).dash.unwrap_or_default(), s.dash.unwrap_or_default());
            for k in 1..130 {
                let u = k as f64 / 10.0 + 0.03;
                assert_eq!(dash_at(&d, u), dash_at(&own, 13.0 - u), "at {u}: {d:?}");
            }
        }
        // A closed key started half way round starts its gradient along it there too, stepping
        // where its own start was, and its dashes start there.
        let square = |x: f64| shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0));
        let key = keyed(square(100.0), &graded(StrokeGradientMode::Along));
        let s = stroke(&Lerp::new(&keyed(square(0.0), &|_| {}), &key, (Some(0), Some(2))).at(1.0));
        for (u, want) in [(0.0, 1.5), (0.25, 0.75), (0.499, 0.0), (0.501, 3.0), (0.75, 2.25), (1.0, 1.5)] {
            assert!((light(&s, u) - want).abs() < 0.02, "at {u}: {} for {want}: {:?}", light(&s, u), s.paint);
        }
        let (own, d) = (stroke(&key).dash.unwrap_or_default(), s.dash.unwrap_or_default());
        for k in 1..200 {
            let u = k as f64 / 10.0 + 0.03;
            assert_eq!(dash_at(&d, u), dash_at(&own, u + 20.0), "at {u}: {d:?}");
        }
        // Stops that reflect twice read the same either way; a link to a swatch stays.
        let reflected = keyed(line(100.0), &|s| {
            let mut g = Gradient { spread: GradientSpread::Reflect, repeats: 2, ..Gradient::default() };
            g.stops[1].swatch = Some("Ink".into());
            s.paint = Paint::Gradient(Box::new(GradientPaint::new(g)));
            s.gradient_mode = StrokeGradientMode::Along;
        });
        assert_eq!(stroke(&Lerp::new(&plain, &reflected, (None, Some(1))).b).paint, stroke(&reflected).paint);
        let linked = keyed(square(100.0), &|s| {
            let mut g = Gradient::default();
            g.stops[1].swatch = Some("Ink".into());
            s.paint = Paint::Gradient(Box::new(GradientPaint::new(g)));
            s.gradient_mode = StrokeGradientMode::Along;
        });
        let s = stroke(&Lerp::new(&keyed(square(0.0), &|_| {}), &linked, (Some(0), Some(2))).b);
        let links = match &s.paint {
            Paint::Gradient(g) => g.gradient.stops.iter().filter(|st| st.swatch.as_deref() == Some("Ink")).map(|st| st.offset).collect::<Vec<_>>(),
            p => panic!("a gradient: {p:?}"),
        };
        assert!(links.contains(&0.5), "the stop the old start ended on keeps its link: {:?}", s.paint);
        // Each stop of a step keeps its own link.
        let step = keyed(square(100.0), &|s| {
            let mut g = Gradient::default();
            let (mut before, mut after) = (GradientStop::new(0.25, Color::BLACK), GradientStop::new(0.25, Color::WHITE));
            (before.swatch, after.swatch) = (Some("Ink".into()), Some("Paper".into()));
            g.stops.splice(1..1, [before, after]);
            s.paint = Paint::Gradient(Box::new(GradientPaint::new(g)));
            s.gradient_mode = StrokeGradientMode::Along;
        });
        let s = stroke(&Lerp::new(&keyed(square(0.0), &|_| {}), &step, (Some(0), Some(2))).b);
        let links = match &s.paint {
            Paint::Gradient(g) => g.gradient.stops.iter().filter(|st| st.offset == 0.75).map(|st| st.swatch.clone()).collect::<Vec<_>>(),
            p => panic!("a gradient: {p:?}"),
        };
        assert_eq!(links, vec![Some("Ink".into()), Some("Paper".into())], "{:?}", s.paint);
        // Blended with a gradient of other stops, the steps keep that step.
        let three = keyed(square(0.0), &|s| {
            let mut g = Gradient::default();
            g.stops.insert(1, GradientStop::new(0.5, Color::BLACK));
            s.paint = Paint::Gradient(Box::new(GradientPaint::new(g)));
            s.gradient_mode = StrokeGradientMode::Along;
        });
        let s = stroke(&Lerp::new(&three, &key, (Some(0), Some(2))).at(0.75));
        assert!(light(&s, 0.501) - light(&s, 0.499) > 1.5, "{:?}", s.paint);
        // Subpaths whose lengths call for different dash offsets keep the offset.
        let rings = |x: f64, turn: usize| {
            let mut inner = shapes::rectangle(Rect::new(x, 20.0, x + 7.0, 27.0)).subpaths.remove(0);
            inner.anchors.rotate_left(turn);
            PathData::new(vec![square(x).subpaths.remove(0), inner])
        };
        let key = keyed(rings(100.0, 2), &graded(StrokeGradientMode::Along));
        let blend = Lerp::new(&keyed(rings(0.0, 0), &|_| {}), &key, (Some(0), Some(2)));
        let turned = match &blend.shape {
            Shape::Path { pair, .. } => pair.orient.1,
            _ => panic!("a path blend"),
        };
        assert!((turned.seam() - 0.5).abs() < 1e-9, "both rings turn half way round: {turned:?}");
        let s = stroke(&blend.at(1.0));
        assert_eq!(s.dash.map(|d| d.offset), Some(0.75));
    }

    #[test]
    fn gradients_of_other_stop_counts_keep_their_steps() {
        let red = Color::Rgb { r: 1.0, g: 0.0, b: 0.0 };
        let stepped = Gradient::new(
            vectorcraft_color::GradientKind::Linear,
            vec![
                GradientStop::new(0.0, Color::WHITE),
                GradientStop::new(0.5, Color::BLACK),
                GradientStop::new(0.5, Color::WHITE),
                GradientStop::new(0.50015, red),
                GradientStop::new(1.0, Color::BLACK),
            ],
        );
        let (a, b) = common_stops(&stepped, &Gradient::default()).unwrap_or_else(|| panic!("one kind"));
        let at = |g: &Gradient| g.stops.iter().map(|s| (s.offset, s.color)).collect::<Vec<_>>();
        assert_eq!(at(&a), vec![(0.0, Color::WHITE), (0.5, Color::BLACK), (0.5, Color::WHITE), (0.50015, red), (1.0, Color::BLACK)]);
        assert_eq!(b.stops.len(), 5);
        assert_eq!(b.stops.get(1).map(|s| s.color), b.stops.get(2).map(|s| s.color), "a gradient without the step paints one color there");
    }

    #[test]
    fn odd_dash_blends_to_solid_with_closed_gaps() {
        let d = lerp_dash(Some(&Dash { pattern: vec![2.0, 3.0, 4.0], ..Default::default() }), None, 1.0).unwrap();
        assert!(d.pattern.iter().skip(1).step_by(2).all(|g| *g == 0.0), "{:?}", d.pattern);
    }

    #[test]
    fn copied_steps_get_new_members() {
        let rect = |x: f64| Arc::new(Node::path(NodeId(7), shapes::rectangle(Rect::new(x, 0.0, x + 10.0, 10.0)), Appearance::default()));
        let a = Node::new(NodeId(1), NodeKind::Group { children: vec![rect(0.0), rect(20.0)], clip: true });
        let b = Node::new(NodeId(2), NodeKind::Group { children: vec![rect(100.0)], clip: true });
        let mid = lerp_node(&a, &b, 0.4);
        let mut ids = vec![];
        mid.walk(&mut |n| ids.push(n.id));
        assert!(ids.iter().all(|i| *i == NodeId(0)), "{ids:?}");
    }

    /// A blend of `a` and `b` spaced by `spacing`.
    fn blend_of(a: Node, b: Node, spacing: BlendSpacing) -> Node {
        let spec = BlendSpec { spacing, ..Default::default() };
        Node::new(NodeId(1), NodeKind::Blend { children: vec![Arc::new(a), Arc::new(b)], spec: Box::new(spec) })
    }

    /// A blend's keys and options.
    fn blend_parts(n: &Node) -> Option<(&Vec<Arc<Node>>, &BlendSpec)> {
        match &n.kind {
            NodeKind::Blend { children, spec } => Some((children, spec)),
            _ => None,
        }
    }

    /// A square of side `side` at (`x`, `y`).
    fn square(x: f64, y: f64, side: f64) -> Node {
        Node::path(NodeId(1), shapes::rectangle(Rect::new(x, y, x + side, y + side)), Appearance::default())
    }

    /// `levels` of blends of 15 steps whose keys are the blends a level down (#3579's input).
    fn nested(levels: u32, x: f64) -> Node {
        if levels == 0 {
            return square(x, 0.0, 4.0);
        }
        let w = 40.0 * f64::from(levels);
        blend_of(nested(levels - 1, x), nested(levels - 1, x + w), BlendSpacing::Steps(15))
    }

    /// The steps drawing `n` makes in all, each blend evaluated as the renderer does: one level at
    /// a time, the blends in its keys and steps evaluated again.
    fn drawn_steps(n: &Node) -> usize {
        match &n.kind {
            NodeKind::Blend { children, spec } => {
                let out = blend_expand(children, spec);
                out.len() - children.len() + out.iter().map(drawn_steps).sum::<usize>()
            }
            _ => n.children().map_or(0, |c| c.iter().map(|c| drawn_steps(c)).sum()),
        }
    }

    /// Steps `levels` of [`nested`] blends of 15 steps make in all: each level its 15 steps,
    /// and again the steps of the level below in each key and each step.
    fn nested_steps(levels: u32) -> usize {
        (17usize.pow(levels) - 1) * 15 / 16
    }

    /// #3579: blends whose keys are blends draw their steps again in every step above them. Five
    /// levels of 15 steps made 1.3 million steps from a 23 KB file. Now a blend makes fewer steps
    /// where they would pass `MAX_BLEND_STEPS`, and none where its keys already fill it, so each
    /// level past that adds only what its keys draw.
    #[test]
    fn nested_blends_keep_to_the_step_budget() {
        assert_eq!(nested_steps(5), 1_331_115);
        let four = nested_steps(4);
        assert!(four < MAX_BLEND_STEPS && 2 * four > MAX_BLEND_STEPS);
        let five = nested(5, 0.0);
        let (children, spec) = blend_parts(&five).unwrap();
        assert_eq!(blend_steps(children, spec), vec![0]);
        assert_eq!(drawn_steps(&five), 2 * four);
        assert_eq!(drawn_steps(&nested(6, 0.0)), 4 * four);
        // A blend whose keys leave room keeps the same share of its steps between each pair.
        let mut three = nested(3, 0.0);
        if let NodeKind::Blend { spec, .. } = &mut three.kind {
            spec.spacing = BlendSpacing::Steps(1000);
        }
        let (children, spec) = blend_parts(&three).unwrap();
        let inside = 2 * nested_steps(2);
        let each = 1 + nested_steps(2);
        let kept = (MAX_BLEND_STEPS - inside) * 1000 / (1000 * each);
        assert_eq!(blend_steps(children, spec), vec![kept]);
        let drawn = drawn_steps(&three);
        assert_eq!(drawn, inside + kept * each);
        assert!(drawn <= MAX_BLEND_STEPS && drawn > MAX_BLEND_STEPS - each, "{drawn}");
    }

    /// Nested blends within the budget make every step their spacing asks for, as before.
    #[test]
    fn nested_blends_within_the_budget_draw_every_step() {
        for levels in 1..=4 {
            let n = nested(levels, 0.0);
            let (children, spec) = blend_parts(&n).unwrap();
            assert_eq!(blend_steps(children, spec), vec![15]);
            assert_eq!(drawn_steps(&n), nested_steps(levels), "{levels}");
        }
        // The keys, then 15 steps, then the last key, each a blend whose keys moved with it.
        let two = nested(2, 0.0);
        let (children, spec) = blend_parts(&two).unwrap();
        let out = blend_expand(children, spec);
        assert_eq!(out.len(), 17);
        assert_eq!(out[0], *children[0]);
        assert!(out.iter().all(|s| matches!(s.kind, NodeKind::Blend { .. })));
        assert_eq!(out[8].geometric_bounds(), children[0].geometric_bounds().map(|b| b + Vec2::new(40.0, 0.0)));
    }

    /// A blend spaced by distance makes more steps scaled up, so a step's copy of one counts the
    /// most its spacing can make. Here a tiny blend of one step is blended with a large blend of
    /// one step: copies of the tiny one, scaled up toward the large one's size, make up to 1000
    /// steps each, which neither key's own count shows.
    #[test]
    fn scaled_copies_of_blends_spaced_by_distance_keep_to_the_budget() {
        let tiny = blend_of(square(0.0, 0.0, 1.0), square(2.0, 0.0, 1.0), BlendSpacing::Distance(1.0));
        let large = blend_of(square(0.0, 100.0, 1000.0), square(2000.0, 100.0, 1000.0), BlendSpacing::Steps(1));
        assert_eq!((drawn_steps(&tiny), drawn_steps(&large)), (1, 1));
        let n = blend_of(tiny, large, BlendSpacing::Steps(1000));
        let (children, spec) = blend_parts(&n).unwrap();
        let steps = blend_steps(children, spec);
        assert!(steps[0] < 1000, "{steps:?}");
        let drawn = drawn_steps(&n);
        assert!(drawn <= MAX_BLEND_STEPS && drawn > 1000, "{drawn}");
    }

    /// A flat blend of many keys shares the budget evenly between its pairs.
    #[test]
    fn many_keys_share_the_step_budget() {
        let keys: Vec<Arc<Node>> = (0..300).map(|i| key(f64::from(i) * 20.0)).collect();
        let spec = BlendSpec { spacing: BlendSpacing::Steps(1000), ..Default::default() };
        let steps = blend_steps(&keys, &spec);
        assert!(steps.iter().all(|&n| n == MAX_BLEND_STEPS * 1000 / (299 * 1000)), "{:?}", &steps[..3]);
        assert_eq!(blend_expand(&keys, &spec).len(), 300 + steps.iter().sum::<usize>());
        // Within the budget, every step stays.
        assert_eq!(blend_steps(&keys[..50], &spec), vec![1000; 49]);
        assert_eq!(fit_steps(&[10, 20], &[1, 1], MAX_BLEND_STEPS), vec![0, 0]);
    }

    /// A key for the Smooth Color tests: a rectangle of `size` (width, height) centered on `c`,
    /// with `fill`, and `stroke` at `width`.
    fn smooth_key(c: (f64, f64), size: (f64, f64), fill: Paint, stroke: Paint, width: f64) -> Node {
        let ((cx, cy), (w, h)) = (c, size);
        let rect = Rect::new(cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0);
        Node::path(NodeId(1), shapes::rectangle(rect), Appearance::basic(fill, stroke, width))
    }

    /// Smooth Color's steps between two filled `size` rectangles centered on `a` and `b`.
    fn smooth(a: (f64, f64), b: (f64, f64), size: (f64, f64), fill_a: Color, fill_b: Color) -> usize {
        let key = |c, fill| smooth_key(c, size, Paint::solid(fill), Paint::None, 0.0);
        blend_step_count(&key(a, fill_a), &key(b, fill_b), BlendSpacing::SmoothColor, a.0.max(b.0) - a.0.min(b.0))
    }

    /// Smooth Color counts 255 steps for a full change of the channel that changes most, a half
    /// step rounding down: RGB values over 256, CMYK and gray in percent, a spot color by its
    /// tint, colors in different spaces by their RGB values over 255, a fill or a stroke,
    /// whichever changes more.
    #[test]
    fn smooth_color_counts_a_step_per_level_of_color_change() {
        let (a, b, size) = ((50.0, 150.0), (250.0, 150.0), (20.0, 20.0));
        let black = Color::rgb8(0, 0, 0);
        for (level, steps) in [(1, 1), (2, 2), (5, 5), (64, 64), (127, 127), (128, 127), (129, 128), (200, 199), (254, 253), (255, 254)] {
            assert_eq!(smooth(a, b, size, black, Color::rgb8(level, 0, 0)), steps, "black to red {level}");
            assert_eq!(smooth(a, b, size, black, Color::rgb8(level, level, level)), steps, "black to gray {level}");
        }
        assert_eq!(smooth(a, b, size, Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255)), 254);
        assert_eq!(smooth(a, b, size, black, Color::rgb8(64, 128, 0)), 127, "the channel that changes most");
        assert_eq!(smooth(a, b, size, Color::rgb8(17, 42, 93), Color::rgb8(91, 147, 220)), 127);
        for (percent, steps) in [(0.2, 1), (1.0, 3), (5.0, 13), (10.0, 25), (25.0, 64), (50.0, 127), (100.0, 254)] {
            let p = percent / 100.0;
            assert_eq!(smooth(a, b, size, Color::cmyk(0.0, 0.0, 0.0, 0.0), Color::cmyk(p, 0.0, 0.0, 0.0)), steps, "cyan {percent}%");
            assert_eq!(smooth(a, b, size, Color::gray(0.0), Color::gray(p)), steps, "gray {percent}%");
        }
        let spot = |tint: f32| Paint::Solid { color: Color::rgb8(255, 0, 0).tinted(tint), swatch: Some("Spot".into()), tint };
        let key = |c, fill| smooth_key(c, size, fill, Paint::None, 0.0);
        assert_eq!(blend_step_count(&key(a, spot(1.0)), &key(b, spot(0.5)), BlendSpacing::SmoothColor, 200.0), 127, "a tint of 50%");
        let stroked = |c, fill: Color, stroke: Color| smooth_key(c, size, Paint::solid(fill), Paint::solid(stroke), 1.0);
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        assert_eq!(blend_step_count(&stroked(a, red, black), &stroked(b, red, blue), BlendSpacing::SmoothColor, 200.0), 254, "the stroke changes");
        let outline = |c, stroke: Color| smooth_key(c, size, Paint::None, Paint::solid(stroke), 1.0);
        assert_eq!(blend_step_count(&outline(a, red), &outline(b, blue), BlendSpacing::SmoothColor, 200.0), 254, "strokes alone");
        assert_eq!(smooth(a, b, size, Color::rgb8(255, 255, 127), Color::cmyk(0.0, 0.0, 0.0, 0.0)), 128, "RGB against CMYK white");
        assert_eq!(smooth(a, b, size, Color::rgb8(255, 255, 127), Color::rgb8(255, 255, 255)), 127, "RGB against RGB white");
    }

    /// Smooth Color reads the colors of group members paired in order and of text runs, and
    /// counts a solid paint against no paint by distance, since such a paint switches halfway
    /// instead of mixing.
    #[test]
    fn smooth_color_reads_the_colors_inside_groups_and_text() {
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        let square = |x: f64, c: Color| Arc::new(smooth_key((x, 150.0), (20.0, 20.0), Paint::solid(c), Paint::None, 0.0));
        let group = |x: f64, top: Color| Node::new(NodeId(1), NodeKind::Group { children: vec![square(x, red), square(x, top)], clip: false });
        let steps = |a: &Node, b: &Node| blend_step_count(a, b, BlendSpacing::SmoothColor, 200.0);
        assert_eq!(steps(&group(50.0, red), &group(250.0, blue)), 254, "groups whose members change color");
        assert_eq!(steps(&group(50.0, red), &group(250.0, red)), 9, "groups of the same colors");
        let text = |x: f64, c: Color| {
            let style = crate::CharStyle { fill: Paint::solid(c), ..Default::default() };
            Node::new(NodeId(1), NodeKind::Text(Box::new(crate::TextObject::point(Point::new(x, 150.0), "Hi", style))))
        };
        assert_eq!(steps(&text(50.0, red), &text(250.0, blue)), 254, "text whose runs change color");
        let mut two_runs = text(250.0, red);
        if let NodeKind::Text(t) = &mut two_runs.kind {
            let style = crate::CharStyle { fill: Paint::solid(blue), ..t.runs[0].style.clone() };
            t.runs.push(crate::text::TextRun::new("Hi", style));
        }
        assert_eq!(steps(&text(50.0, red), &two_runs), 254, "a run that turns blue against one red run");
        let empty = |x: f64| smooth_key((x, 150.0), (20.0, 20.0), Paint::None, Paint::None, 0.0);
        assert_eq!(steps(&smooth_key((50.0, 150.0), (20.0, 20.0), Paint::solid(red), Paint::None, 0.0), &empty(250.0)), 9, "a fill against none");
        let nothing = |c: Color| Node::path(NodeId(1), PathData::new(vec![]), Appearance::basic(Paint::solid(c), Paint::None, 0.0));
        assert_eq!(steps(&nothing(red), &nothing(blue)), 254, "keys without bounds whose colors differ");
        assert_eq!(steps(&nothing(red), &nothing(red)), 1, "keys without bounds of the same color");
    }

    /// Smooth Color makes at most eight steps per point of the largest of the keys' widths and
    /// heights and the distance between their centers along either axis.
    #[test]
    fn smooth_color_makes_at_most_eight_steps_a_point() {
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        for (dx, steps) in [(0.0, 160), (20.0, 160), (20.1, 161), (25.0, 200), (31.5, 252), (40.0, 254)] {
            assert_eq!(smooth((50.0, 150.0), (50.0 + dx, 150.0), (20.0, 20.0), red, blue), steps, "{dx} pt apart");
        }
        assert_eq!(smooth((100.0, 150.0), (100.0, 175.0), (20.0, 20.0), red, blue), 200, "25 pt apart vertically");
        assert_eq!(smooth((100.0, 150.0), (100.0, 150.0), (5.0, 5.0), red, blue), 40, "5 pt squares on each other");
        assert_eq!(smooth((100.0, 150.0), (100.0, 150.0), (25.0, 5.0), red, blue), 200, "the longer side counts");
        assert_eq!(smooth((100.0, 150.0), (100.0, 150.0), (1.0, 1.0), red, blue), 8);
    }

    /// Smooth Color between keys of the same colors counts the largest distance between their
    /// corresponding visible edges in mean key widths or heights, rounded, less one; so do keys
    /// with gradients, whatever their colors.
    #[test]
    fn smooth_color_between_the_same_colors_counts_by_distance() {
        let red = Color::rgb8(255, 0, 0);
        let same = |a, b, size| smooth(a, b, size, red, red);
        for (dx, steps) in [(25.0, 1), (50.0, 2), (100.0, 4), (200.0, 9), (400.0, 19), (800.0, 39)] {
            assert_eq!(same((50.0, 150.0), (50.0 + dx, 150.0), (20.0, 20.0)), steps, "20 pt squares {dx} pt apart");
        }
        for (side, steps) in [(5.0, 39), (10.0, 19), (40.0, 4), (80.0, 2)] {
            assert_eq!(same((50.0, 150.0), (250.0, 150.0), (side, side)), steps, "{side} pt squares");
        }
        assert_eq!(same((50.0, 150.0), (250.0, 150.0), (80.0, 10.0)), 2, "wide");
        assert_eq!(same((50.0, 150.0), (250.0, 150.0), (10.0, 80.0)), 19, "tall");
        assert_eq!(same((50.0, 250.0), (250.0, 50.0), (20.0, 20.0)), 9, "diagonal");
        assert_eq!(same((50.0, 250.0), (250.0, 50.0), (80.0, 10.0)), 19, "wide and diagonal");
        assert_eq!(same((50.0, 150.0), (1050.0, 150.0), (1.0, 1.0)), 999);
        assert_eq!(same((50.0, 150.0), (1052.0, 150.0), (1.0, 1.0)), 1000, "at most 1000");
        assert_eq!(same((50.0, 150.0), (50.0, 150.0), (20.0, 20.0)), 1, "at least one");
        assert_eq!(same((50.0, 150.0), (250.0, 150.0), (0.0, 20.0)), 9, "vertical lines, measured in their height");
        assert_eq!(same((50.0, 150.0), (50.0, 350.0), (20.0, 0.0)), 9, "horizontal lines, measured in their width");
        assert_eq!(same((50.0, 150.0), (60.0, 150.0), (0.0, 0.0)), 9, "points, measured in points");
        let key = |c, size, stroke: f64| smooth_key(c, size, Paint::solid(red), Paint::solid(Color::BLACK), stroke);
        let wide =
            blend_step_count(&key((50.0, 150.0), (10.0, 20.0), 0.0), &key((250.0, 150.0), (80.0, 20.0), 0.0), BlendSpacing::SmoothColor, 200.0);
        assert_eq!(wide, 4, "keys of different widths");
        let strokes =
            blend_step_count(&key((50.0, 150.0), (20.0, 20.0), 1.0), &key((250.0, 150.0), (20.0, 20.0), 10.0), BlendSpacing::SmoothColor, 200.0);
        assert_eq!(strokes, 7, "strokes widen the visible edges");
        let gradient = |a: Color, b: Color| {
            Paint::Gradient(Box::new(GradientPaint::new(Gradient::new(
                Default::default(),
                vec![GradientStop::new(0.0, a), GradientStop::new(1.0, b)],
            ))))
        };
        let shaded = |c, fill| smooth_key(c, (20.0, 20.0), fill, Paint::None, 0.0);
        let blue = Color::rgb8(0, 0, 255);
        for (from, to) in [(gradient(red, blue), gradient(red, blue)), (gradient(red, blue), gradient(blue, red))] {
            assert_eq!(blend_step_count(&shaded((50.0, 150.0), from), &shaded((250.0, 150.0), to), BlendSpacing::SmoothColor, 200.0), 9);
        }
    }

    /// A Smooth Color blend counts each pair of keys on its own.
    #[test]
    fn smooth_color_counts_each_pair_of_keys() {
        let key = |x: f64, c: Color| Arc::new(smooth_key((x, 150.0), (20.0, 20.0), Paint::solid(c), Paint::None, 0.0));
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        let spec = BlendSpec::default();
        assert_eq!(blend_steps(&[key(50.0, red), key(150.0, red), key(250.0, blue)], &spec), vec![4, 254]);
        assert_eq!(blend_steps(&[key(50.0, red), key(150.0, blue), key(250.0, red)], &spec), vec![254, 254]);
    }

    /// A Smooth Color blend between different colors inside another blend's keys counts as the
    /// steps its colors ask for, so the outer blend keeps the steps it asks for within
    /// [`MAX_BLEND_STEPS`]: 200 steps, each a copy making 254 more. The copies keep their keys'
    /// colors, also halfway between a red-to-blue and a blue-to-red blend.
    #[test]
    fn a_smooth_color_blend_in_a_key_counts_its_color_steps() {
        let (red, blue) = (Color::rgb8(255, 0, 0), Color::rgb8(0, 0, 255));
        let key = |x: f64, c: Color| smooth_key((x, 150.0), (20.0, 20.0), Paint::solid(c), Paint::None, 0.0);
        let inner = |x: f64, from: Color, to: Color| blend_of(key(x, from), key(x + 100.0, to), BlendSpacing::SmoothColor);
        let outer = blend_of(inner(0.0, red, blue), inner(500.0, red, blue), BlendSpacing::Steps(200));
        let (keys, spec) = blend_parts(&outer).unwrap();
        assert_eq!(blend_steps(keys, spec), vec![200]);
        let crossed = blend_of(inner(0.0, red, blue), inner(500.0, blue, red), BlendSpacing::Steps(199));
        let (keys, spec) = blend_parts(&crossed).unwrap();
        assert_eq!(blend_steps(keys, spec), vec![199]);
        let drawn = blend_expand(keys, spec);
        let Some(NodeKind::Blend { children, .. }) = drawn.get(100).map(|n| &n.kind) else { panic!("a copy of a blend halfway") };
        let fills: Vec<_> = children.iter().map(|k| k.appearance.fill_paint().color()).collect();
        assert!(fills == [Some(red), Some(blue)] || fills == [Some(blue), Some(red)], "{fills:?}");
    }

    /// A Smooth Color blend between the same colors inside another blend's keys counts as up to
    /// 1000 steps, which a copy turned to a path can ask for: 200 steps of copies of it come
    /// down to 99.
    #[test]
    fn a_smooth_color_blend_of_the_same_colors_in_a_key_counts_up_to_1000_steps() {
        let red = Color::rgb8(255, 0, 0);
        let key = |x: f64| smooth_key((x, 150.0), (20.0, 20.0), Paint::solid(red), Paint::None, 0.0);
        let inner = |x: f64| blend_of(key(x), key(x + 100.0), BlendSpacing::SmoothColor);
        let outer = blend_of(inner(0.0), inner(500.0), BlendSpacing::Steps(200));
        let (keys, spec) = blend_parts(&outer).unwrap();
        assert_eq!(blend_steps(keys, spec), vec![99]);
    }
}
