//! `spreadMethod="reflect"` and `"repeat"`: gradients only pad past their ends, so the stops are
//! repeated (every other period mirrored, for reflect) as many times as the painted area needs,
//! and the gradient stretched over those periods.

use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientStop};
use vectorcraft_geom::{Point, Rect};

/// Most periods a gradient is expanded to.
pub(super) const MAX_PERIODS: usize = 64;

/// What [`expand`] did.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Expanded {
    /// Every period the area shows.
    Whole,
    /// [`MAX_PERIODS`] of them: the gradient pads past those.
    Capped,
}

/// Expand `stops` and `geom` (document space) so the gradient paints `area` as the spread method
/// does: `reflect` mirrors every other period, else they repeat.
pub(super) fn expand(kind: GradientKind, geom: &mut GradientGeom, stops: &mut Vec<GradientStop>, reflect: bool, area: Rect) -> Expanded {
    let corners = [Point::new(area.x0, area.y0), Point::new(area.x1, area.y0), Point::new(area.x1, area.y1), Point::new(area.x0, area.y1)];
    let ts = corners.map(|p| geom.param_at(kind, p));
    if ts.iter().any(|t| !t.is_finite()) || stops.is_empty() {
        return Expanded::Whole;
    }
    let radial = kind == GradientKind::Radial;
    // The parameter's range over the area: a corner holds its extremes (radial levels are nested
    // convex rings from the focal point, where the parameter is 0).
    let lo = if radial { 0.0 } else { ts.iter().copied().fold(f64::INFINITY, f64::min).floor() };
    let hi = ts.iter().copied().fold(f64::NEG_INFINITY, f64::max).ceil().max(lo + 1.0);
    if lo >= 0.0 && hi <= 1.0 {
        return Expanded::Whole;
    }
    let capped = hi - lo > MAX_PERIODS as f64;
    // Capped, the periods kept start no further back than half of them.
    let lo = if capped { lo.max(-(MAX_PERIODS as f64) / 2.0) } else { lo };
    let n = (hi - lo).min(MAX_PERIODS as f64);
    let period = whole_period(stops);
    let mut out = Vec::with_capacity(period.len() * n as usize);
    for j in 0..n as usize {
        let mirrored = reflect && (lo as i64 + j as i64).rem_euclid(2) == 1;
        let one = if mirrored { mirror(&period) } else { period.clone() };
        out.extend(one.into_iter().map(|s| GradientStop { offset: ((j as f64 + s.offset as f64) / n) as f32, ..s }));
    }
    *stops = out;
    let u = geom.end - geom.start;
    if radial {
        // Level t of an off-centre radial is the extent ellipse scaled by t about the line from the
        // focal point to the centre; level n·t' of the new one: the centre moves away from the
        // focal point n times as far, and the radius grows n times.
        let Some(frame) = geom.unit_frame() else { return Expanded::Whole };
        let f = geom.focal.map_or(Point::ORIGIN, |f| frame.inverse() * f);
        geom.start = frame * (f.to_vec2() * (1.0 - n)).to_point();
        geom.end = geom.start + u * n;
    } else {
        let start = geom.start;
        geom.start = start + u * lo;
        geom.end = start + u * (lo + n);
    }
    if capped { Expanded::Capped } else { Expanded::Whole }
}

/// The stops of one period, from offset 0 to 1: the end colours held out to the ends.
fn whole_period(stops: &[GradientStop]) -> Vec<GradientStop> {
    let mut out = stops.to_vec();
    if let Some(first) = out.first().filter(|s| s.offset > 0.0).cloned() {
        out.insert(0, GradientStop { offset: 0.0, ..first });
    }
    if let Some(last) = out.last_mut() {
        // The last stop's midpoint leads nowhere; the copy at 1 takes over.
        if last.offset < 1.0 {
            let end = GradientStop { offset: 1.0, ..last.clone() };
            out.push(end);
        }
    }
    out
}

/// A period mirrored: the stops in reverse, each midpoint measured from the other side.
fn mirror(period: &[GradientStop]) -> Vec<GradientStop> {
    let mut g = Gradient::new(GradientKind::Linear, period.to_vec());
    g.reverse();
    g.stops
}

#[cfg(test)]
mod tests {
    use super::*;
    use vectorcraft_color::Color;

    fn stops() -> Vec<GradientStop> {
        let mut a = GradientStop::new(0.0, Color::BLACK);
        a.midpoint = 0.3;
        vec![a, GradientStop::new(1.0, Color::WHITE)]
    }

    #[test]
    fn a_linear_gradient_repeats_over_the_area() {
        let mut geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None };
        let mut s = stops();
        let r = expand(GradientKind::Linear, &mut geom, &mut s, false, Rect::new(-5.0, 0.0, 25.0, 10.0));
        assert_eq!(r, Expanded::Whole);
        // From -10 to 30: four periods.
        assert_eq!((geom.start, geom.end), (Point::new(-10.0, 0.0), Point::new(30.0, 0.0)));
        let offsets: Vec<f32> = s.iter().map(|s| s.offset).collect();
        assert_eq!(offsets, [0.0, 0.25, 0.25, 0.5, 0.5, 0.75, 0.75, 1.0]);
        assert!(s.iter().step_by(2).all(|s| s.color == Color::BLACK && (s.midpoint - 0.3).abs() < 1e-6));
    }

    #[test]
    fn reflect_mirrors_every_other_period_and_its_midpoints() {
        let mut geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None };
        let mut s = stops();
        expand(GradientKind::Linear, &mut geom, &mut s, true, Rect::new(0.0, 0.0, 20.0, 10.0));
        let colours: Vec<Color> = s.iter().map(|s| s.color).collect();
        assert_eq!(colours, [Color::BLACK, Color::WHITE, Color::WHITE, Color::BLACK]);
        // Black → white with the midpoint at 0.3 mirrors to white → black with it at 0.7.
        assert!((s[2].midpoint - 0.7).abs() < 1e-6, "{s:?}");
    }

    #[test]
    fn stops_inside_the_period_hold_their_colours_to_its_ends() {
        let mut geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(10.0, 0.0), aspect: 1.0, focal: None };
        let mut s = vec![GradientStop::new(0.25, Color::BLACK), GradientStop::new(0.75, Color::WHITE)];
        expand(GradientKind::Linear, &mut geom, &mut s, false, Rect::new(0.0, 0.0, 20.0, 10.0));
        let offsets: Vec<f32> = s.iter().map(|s| s.offset).collect();
        assert_eq!(offsets, [0.0, 0.125, 0.375, 0.5, 0.5, 0.625, 0.875, 1.0]);
    }

    #[test]
    fn a_radial_gradient_grows_about_its_focal_point() {
        let c = Point::new(50.0, 50.0);
        let mut geom = GradientGeom { start: c, end: Point::new(60.0, 50.0), aspect: 1.0, focal: Some(Point::new(55.0, 50.0)) };
        let before = geom.param_at(GradientKind::Radial, Point::new(72.0, 61.0));
        let mut s = stops();
        expand(GradientKind::Radial, &mut geom, &mut s, false, Rect::new(0.0, 0.0, 100.0, 100.0));
        let n = s.len() / 2;
        assert!(n >= 5, "{n}");
        assert_eq!(geom.focal, Some(Point::new(55.0, 50.0)));
        // Every point keeps its level: the new parameter is the old one over n.
        let after = geom.param_at(GradientKind::Radial, Point::new(72.0, 61.0));
        assert!((after * n as f64 - before).abs() < 1e-9, "{before} {after}");
    }

    #[test]
    fn far_repeats_are_capped() {
        let mut geom = GradientGeom { start: Point::new(0.0, 0.0), end: Point::new(0.01, 0.0), aspect: 1.0, focal: None };
        let mut s = stops();
        assert_eq!(expand(GradientKind::Linear, &mut geom, &mut s, false, Rect::new(0.0, 0.0, 100.0, 1.0)), Expanded::Capped);
        assert_eq!(s.len(), 2 * MAX_PERIODS);
    }
}
