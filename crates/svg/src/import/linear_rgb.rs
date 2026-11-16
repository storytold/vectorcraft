//! Gradients drawn with `color-interpolation="linearRGB"` (SVG 1.1 §11.7.1,
//! <https://www.w3.org/TR/SVG11/painting.html#ColorInterpolationProperty>): their stops mix in
//! linear light, which a document gradient (mixed in display RGB) shows through extra stops.

use super::{Color, GradientStop, HashMap, HashSet, css, roxmltree};
use vectorcraft_color::cms::lab::{linear_to_srgb, srgb_to_linear};

/// How far (display RGB, 0..1) the straight line between neighbouring baked stops may stray from
/// the linear-light mix: under half a colour level.
const TOLERANCE: f32 = 0.4 / 255.0;
/// Most times a span between two stops is halved to keep within [`TOLERANCE`].
const MAX_DEPTH: u32 = 8;
/// Most stops a baked gradient has: a file with very many stops keeps the rest of its spans as
/// they are rather than growing without bound.
pub(super) const MAX_STOPS: usize = 4096;

/// The ids (as usvg names them) of the gradients whose `color-interpolation` is `linearRGB`.
///
/// usvg gives a gradient used by several objects a made-up id (`linearGradient2`…) for each user
/// but the last; those are found by their stops and geometry, matching an original gradient's
/// only when no other original gradient with the same stops and geometry interpolates otherwise.
pub(super) fn linear_rgb_ids(xml: &roxmltree::Document, tree: &usvg::Tree) -> HashSet<String> {
    if !xml.input_text().contains("linearRGB") {
        return HashSet::new();
    }
    let styles = css::Styles::new(xml);
    let mut linear = HashSet::new();
    let mut named = HashSet::new();
    for g in xml.descendants().filter(|n| matches!(n.tag_name().name(), "linearGradient" | "radialGradient")) {
        let Some(id) = g.attribute("id") else { continue };
        named.insert(id);
        if styles.prop(g, "color-interpolation").as_deref() == Some("linearRGB") {
            linear.insert(id.to_string());
        }
    }
    if linear.is_empty() {
        return linear;
    }
    let gradients = || {
        let lin = tree.linear_gradients().iter().map(|g| (g.id(), signature(g, &[g.x1(), g.y1(), g.x2(), g.y2()])));
        let rad = tree.radial_gradients().iter().map(|g| (g.id(), signature(g, &[g.cx(), g.cy(), g.r().get(), g.fx(), g.fy(), g.fr().get()])));
        lin.chain(rad)
    };
    // Each original gradient's look → whether all the originals that look so are linearRGB.
    let mut by_look: HashMap<Vec<u32>, bool> = HashMap::new();
    for (id, sig) in gradients().filter(|(id, _)| named.contains(id)) {
        let is_linear = linear.contains(id);
        by_look.entry(sig).and_modify(|v| *v &= is_linear).or_insert(is_linear);
    }
    let copies: Vec<String> =
        gradients().filter(|(id, sig)| !named.contains(id) && by_look.get(sig) == Some(&true)).map(|(id, _)| id.to_string()).collect();
    linear.extend(copies);
    linear
}

/// What tells two usvg gradients apart but for their id, units and transform (what usvg changes
/// on the copies it makes): their geometry `geom`, spread method and stops.
fn signature(g: &usvg::BaseGradient, geom: &[f32]) -> Vec<u32> {
    let spread = match g.spread_method() {
        usvg::SpreadMethod::Pad => 0,
        usvg::SpreadMethod::Reflect => 1,
        usvg::SpreadMethod::Repeat => 2,
    };
    let mut sig: Vec<u32> = geom.iter().map(|v| v.to_bits()).collect();
    sig.push(spread);
    for s in g.stops() {
        let c = s.color();
        sig.extend([s.offset().get().to_bits(), u32::from_be_bytes([0, c.red, c.green, c.blue]), s.opacity().get().to_bits()]);
    }
    sig
}

/// `stops`, mixed in linear light: between each two stops, stops where the display-RGB line
/// strays from the linear-light mix (opacity still mixes evenly). Spans with a moved midpoint
/// (only our own export writes those, never with linearRGB) keep their midpoint unbaked.
pub(super) fn bake(stops: Vec<GradientStop>) -> Vec<GradientStop> {
    let mut out: Vec<GradientStop> = Vec::with_capacity(stops.len());
    let mut it = stops.into_iter().peekable();
    while let Some(a) = it.next() {
        let next = it.peek().cloned();
        out.push(a.clone());
        let Some(b) = next else { break };
        if b.offset <= a.offset || (a.midpoint - 0.5).abs() > 1e-6 || a.color.to_rgb() == b.color.to_rgb() {
            continue;
        }
        let mut ts = Vec::new();
        steps(&a.color, &b.color, (0.0, 1.0), 1, &mut ts);
        let room = MAX_STOPS.saturating_sub(out.len() + it.len());
        for t in ts.into_iter().take(room) {
            let opacity = a.opacity + (b.opacity - a.opacity) * t;
            out.push(GradientStop { opacity, ..GradientStop::new(a.offset + (b.offset - a.offset) * t, mix(&a.color, &b.color, t)) });
        }
    }
    out
}

/// `a` and `b` mixed at `t` (0..1) in linear light, as display RGB.
fn mix(a: &Color, b: &Color, t: f32) -> Color {
    let (p, q) = (a.to_rgb(), b.to_rgb());
    let ch = |k: usize| {
        let (x, y) = (srgb_to_linear(p[k]), srgb_to_linear(q[k]));
        let v = linear_to_srgb(x + (y - x) * t);
        if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 }
    };
    Color::rgb(ch(0), ch(1), ch(2))
}

/// The mix positions (after `t0`, before `t1`) between which display RGB mixed straight stays
/// within [`TOLERANCE`] of the linear-light mix of `a` and `b`, in order.
fn steps(a: &Color, b: &Color, (t0, t1): (f32, f32), depth: u32, out: &mut Vec<f32>) {
    let at = |t: f32| mix(a, b, t).to_rgb();
    let (c0, c1) = (at(t0), at(t1));
    let stray = [0.25, 0.5, 0.75]
        .into_iter()
        .flat_map(|u: f32| {
            let c = at(t0 + (t1 - t0) * u);
            (0..3).map(move |k| (c0[k] + (c1[k] - c0[k]) * u - c[k]).abs())
        })
        .fold(0.0, f32::max);
    if depth <= MAX_DEPTH && stray > TOLERANCE {
        let m = (t0 + t1) / 2.0;
        steps(a, b, (t0, m), depth + 1, out);
        out.push(m);
        steps(a, b, (m, t1), depth + 1, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(stops: &[GradientStop], x: f32) -> [f32; 3] {
        vectorcraft_color::Gradient::new(vectorcraft_color::GradientKind::Linear, stops.to_vec()).sample(x).0.to_rgb()
    }

    #[test]
    fn black_to_white_is_brighter_mid_way() {
        let baked = bake(vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::WHITE)]);
        assert!(baked.len() > 2 && baked.len() < 300, "{}", baked.len());
        // A balanced linear-light mix of black and white encodes to about 188/255.
        let mid = sample(&baked, 0.5)[0] * 255.0;
        assert!((mid - 188.0).abs() < 1.0, "{mid}");
        for i in 0..=100 {
            let x = i as f32 / 100.0;
            let want = linear_to_srgb(x);
            assert!((sample(&baked, x)[0] - want).abs() < 1.0 / 255.0, "at {x}");
        }
        assert!(baked.windows(2).all(|w| w[0].offset <= w[1].offset));
    }

    #[test]
    fn equal_colours_and_moved_midpoints_stay_as_they_are() {
        let same = vec![GradientStop::new(0.0, Color::BLACK), GradientStop::new(1.0, Color::BLACK)];
        assert_eq!(bake(same.clone()), same);
        let moved = vec![GradientStop { midpoint: 0.3, ..GradientStop::new(0.0, Color::BLACK) }, GradientStop::new(1.0, Color::WHITE)];
        assert_eq!(bake(moved.clone()), moved);
    }

    #[test]
    fn opacity_mixes_evenly_and_the_stop_count_is_capped() {
        let a = GradientStop { opacity: 0.0, ..GradientStop::new(0.0, Color::BLACK) };
        let baked = bake(vec![a, GradientStop::new(1.0, Color::WHITE)]);
        assert!(baked.iter().all(|s| (s.opacity - s.offset).abs() < 1e-5));
        let many: Vec<GradientStop> =
            (0..3000).map(|i| GradientStop::new(i as f32 / 2999.0, if i % 2 == 0 { Color::BLACK } else { Color::WHITE })).collect();
        let baked = bake(many);
        assert_eq!(baked.len(), MAX_STOPS);
        assert_eq!(baked.last().map(|s| s.offset), Some(1.0), "every original stop is kept");
    }
}
