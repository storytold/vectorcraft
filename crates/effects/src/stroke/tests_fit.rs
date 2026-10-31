//! Dashes fitted to corners and path ends, and variable-width strokes with dashes and joins.

use kurbo::{BezPath, ParamCurveArclen, PathEl, Point, Rect, Shape};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Dash, LineCap, LineJoin, StrokeLayer, WidthProfile};

use super::*;

fn fitted(pattern: &[f64], offset: f64) -> Dash {
    Dash { pattern: pattern.to_vec(), offset, align_corners: true }
}

fn stroke(width: f64, f: impl FnOnce(&mut StrokeLayer)) -> StrokeLayer {
    let mut st = StrokeLayer::new(Paint::solid(Color::BLACK), width);
    f(&mut st);
    st
}

fn line(x1: f64) -> BezPath {
    let mut b = BezPath::new();
    b.move_to((0.0, 0.0));
    b.line_to((x1, 0.0));
    b
}

fn moves(bp: &BezPath) -> usize {
    bp.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count()
}

fn length(bp: &BezPath) -> f64 {
    bp.segments().map(|s| s.arclen(1e-9)).sum()
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

// ---------------------------------------------------------------- dashes fitted to corners

#[test]
fn every_corner_of_a_rectangle_falls_mid_dash() {
    // 100 × 50 with 12/6: the long sides hold 6 periods (scaled by 100/108), the short ones 3
    // (50/54), so every half dash at a corner is 6 · 100/108 long.
    let r = Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1);
    let d = dash(&r, &fitted(&[12.0, 6.0], 0.0)).unwrap();
    assert_eq!(moves(&d.path), 5 + 2 + 5 + 2 + 4, "inner dashes of each side plus one per corner");
    assert!(near(length(&d.path), 200.0, 1e-6), "two thirds of the perimeter");
    let half = 6.0 * 100.0 / 108.0;
    let o = line_outline(&r, &stroke(2.0, |s| s.dash = Some(fitted(&[12.0, 6.0], 0.0))), 2.0, 1e-3);
    for (c, along) in [
        ((0.0, 0.0), [(1.0, 0.0), (0.0, 1.0)]),
        ((100.0, 0.0), [(-1.0, 0.0), (0.0, 1.0)]),
        ((100.0, 50.0), [(-1.0, 0.0), (0.0, -1.0)]),
        ((0.0, 50.0), [(1.0, 0.0), (0.0, -1.0)]),
    ] {
        let c = Point::from(c);
        assert!(o.contains(c), "corner {c:?} is painted");
        for v in along {
            let v = kurbo::Vec2::from(v);
            assert!(o.contains(c + v * (half - 0.3)), "dash runs on from {c:?} along {v:?}");
            assert!(!o.contains(c + v * (half + 0.3)), "and stops half a dash from {c:?} along {v:?}");
        }
    }
    // The spans of the dashes cover the same length.
    let covered: f64 = d.spans.iter().map(|(a, b)| b - a).sum();
    assert!(near(covered * 300.0, 200.0, 1e-6), "{:?}", d.spans);
}

#[test]
fn open_paths_start_and_end_mid_dash_and_runs_hold_whole_periods() {
    // 100 long, 10/10: five periods, half a dash at each end.
    let d = dash(&line(100.0), &fitted(&[10.0, 10.0], 0.0)).unwrap();
    assert_eq!(moves(&d.path), 6);
    assert!(near(length(&d.path), 50.0, 1e-6));
    let ends: Vec<f64> = d.path.elements().iter().filter_map(|e| e.end_point()).map(|p| p.x).collect();
    assert!(near(ends[0], 0.0, 1e-9) && near(ends[1], 5.0, 1e-9) && near(*ends.last().unwrap(), 100.0, 1e-9), "{ends:?}");
    // 92 long: 4.6 periods round to 5, squeezed to fit.
    let d = dash(&line(92.0), &fitted(&[10.0, 10.0], 0.0)).unwrap();
    assert_eq!(moves(&d.path), 6);
    assert!(near(length(&d.path), 46.0, 1e-6));
    // A run shorter than a period still gets one: half dashes at its ends.
    let d = dash(&line(4.0), &fitted(&[10.0, 10.0], 0.0)).unwrap();
    assert_eq!(moves(&d.path), 2);
    assert!(near(length(&d.path), 2.0, 1e-6));
}

#[test]
fn fitted_dashes_ignore_the_offset_and_keep_exact_dashes_unchanged() {
    let r = Rect::new(0.0, 0.0, 100.0, 50.0).to_path(0.1);
    let a = dash(&r, &fitted(&[12.0, 6.0], 0.0)).unwrap();
    let b = dash(&r, &fitted(&[12.0, 6.0], 5.0)).unwrap();
    assert_eq!(a.path, b.path);
    // Exact dashes still start at the path's start, `offset` into the pattern.
    let exact = dash(&line(100.0), &Dash { pattern: vec![10.0, 10.0], offset: 5.0, align_corners: false }).unwrap();
    let first: Vec<f64> = exact.path.elements().iter().take(2).filter_map(|e| e.end_point()).map(|p| p.x).collect();
    assert_eq!(first, vec![0.0, 5.0]);
    assert_eq!(moves(&exact.path), 6);
}

#[test]
fn smooth_closed_paths_hold_a_whole_number_of_dashes() {
    // A circle has no corners: one run all the way round, 17 periods of 12/6 (314 / 18 ≈ 17.5).
    let c = kurbo::Circle::new((0.0, 0.0), 50.0).to_path(1e-6);
    let d = dash(&c, &fitted(&[12.0, 6.0], 0.0)).unwrap();
    assert_eq!(moves(&d.path), 17, "the dash on the start point is one dash");
    let circumference = 2.0 * std::f64::consts::PI * 50.0;
    assert!(near(length(&d.path), circumference * 12.0 / 18.0, 1e-3));
}

#[test]
fn fitted_dots_sit_on_every_corner_and_end_once() {
    // 60 pt sides with a dot every 7: 9 periods a side (60 / 63 scale), the corner dots shared.
    let sq = Rect::new(0.0, 0.0, 60.0, 60.0).to_path(0.1);
    let d = dash(&sq, &fitted(&[0.0, 7.0], 0.0)).unwrap();
    assert_eq!(d.dots.len(), 36);
    for c in [(0.0, 0.0), (60.0, 0.0), (60.0, 60.0), (0.0, 60.0)] {
        assert_eq!(d.dots.iter().filter(|p| p.at.distance(Point::from(c)) < 1e-6).count(), 1, "one dot on {c:?}");
    }
    let d = dash(&line(60.0), &fitted(&[0.0, 7.0], 0.0)).unwrap();
    assert_eq!(d.dots.len(), 10);
    assert!(near(d.dots[0].at.x, 0.0, 1e-9) && near(d.dots[9].at.x, 60.0, 1e-9));
    assert!(near(d.dots[9].t, 1.0, 1e-9));
}

#[test]
fn fitted_dashes_on_curves_and_degenerate_paths_stay_finite() {
    let mut mixed = BezPath::new();
    mixed.move_to((0.0, 0.0));
    mixed.line_to((0.0, 0.0));
    mixed.curve_to((30.0, -40.0), (70.0, -40.0), (100.0, 0.0));
    mixed.line_to((100.0, 0.0));
    mixed.line_to((50.0, 60.0));
    mixed.close_path();
    mixed.move_to((200.0, 0.0));
    mixed.line_to((200.0, 1e-7));
    for pat in [&[12.0, 6.0][..], &[0.0, 5.0], &[3.0], &[5.0, 0.0], &[1e-12, 4.0, 0.0, 2.0]] {
        for profile in [None, Some(WidthProfile::lens())] {
            let st = stroke(4.0, |s| {
                s.dash = Some(fitted(pat, 0.0));
                s.cap = LineCap::Round;
                s.profile = profile.clone();
            });
            let o = line_outline(&mixed, &st, 4.0, 0.01);
            assert!(o.elements().iter().filter_map(|e| e.end_point()).all(|p| p.x.is_finite() && p.y.is_finite()), "{pat:?}");
        }
    }
}

// ---------------------------------------------------------------- width profiles with dashes and joins

#[test]
fn each_dash_takes_its_width_from_the_profile_along_the_whole_path() {
    let st = stroke(10.0, |s| {
        s.profile = Some(WidthProfile::lens());
        s.dash = Some(Dash { pattern: vec![10.0, 10.0], offset: 0.0, align_corners: false });
    });
    let o = line_outline(&line(100.0), &st, 10.0, 1e-3);
    // The dash over 40..50 is near the lens's widest point; the one over 0..10 near its tip,
    // where the smooth profile is thinner than a straight ramp (a parabola's first tenth).
    assert!(o.contains(Point::new(45.0, 4.0)) && o.contains(Point::new(45.0, -4.0)), "middle dash is wide");
    assert!(o.contains(Point::new(5.0, 0.8)) && !o.contains(Point::new(5.0, 1.2)), "first dash is thin");
    assert!(!o.contains(Point::new(15.0, 0.0)), "gaps stay empty");
    // Dots too: a round dot near the middle is bigger than one near an end.
    let st = stroke(10.0, |s| {
        s.profile = Some(WidthProfile::lens());
        s.cap = LineCap::Round;
        s.dash = Some(Dash { pattern: vec![0.0, 10.0], offset: 0.0, align_corners: false });
    });
    let o = line_outline(&line(100.0), &st, 10.0, 1e-3);
    assert!(o.contains(Point::new(50.0, 4.5)) && o.contains(Point::new(10.0, 1.4)) && !o.contains(Point::new(10.0, 2.0)));
}

/// An L whose corner turns 90° at (100, 0), or a sharp V turning back at (100, 0).
fn bend(sharp: bool) -> BezPath {
    let mut b = line(100.0);
    b.line_to(if sharp { (0.0, 20.0) } else { (100.0, 100.0) });
    b
}

fn uniform(join: LineJoin, miter_limit: f64) -> StrokeLayer {
    stroke(20.0, |s| {
        s.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] });
        s.join = join;
        s.miter_limit = miter_limit;
    })
}

#[test]
fn profile_corners_take_the_stroke_join() {
    let out = |join, limit, sharp| line_outline(&bend(sharp), &uniform(join, limit), 20.0, 1e-3);
    // The L: a miter fills the outer corner, a round join leaves it, a bevel cuts it.
    let corner = Point::new(109.0, -9.0);
    assert!(out(LineJoin::Miter, 10.0, false).contains(corner));
    assert!(!out(LineJoin::Round, 10.0, false).contains(corner));
    assert!(out(LineJoin::Round, 10.0, false).contains(Point::new(106.5, -6.5)), "round join reaches 10 out");
    assert!(!out(LineJoin::Bevel, 10.0, false).contains(Point::new(106.5, -6.5)));
    assert!(out(LineJoin::Bevel, 10.0, false).contains(Point::new(104.5, -4.5)));
    // The inside of the L is filled up to the inner miter and no further.
    let l = out(LineJoin::Round, 10.0, false);
    assert!(l.contains(Point::new(91.0, 9.0)) && !l.contains(Point::new(89.0, 11.0)));
    // The sharp V: its miter (about 10 × the half width) is past the limit of 10, so it bevels;
    // no join spikes out past the round join's reach.
    for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
        let x1 = out(join, 10.0, true).bounding_box().x1;
        assert!(x1 <= 110.0 + 1e-6, "{join:?}: {x1}");
    }
    // A limit above it keeps the full miter.
    let x1 = out(LineJoin::Miter, 20.0, true).bounding_box().x1;
    assert!(x1 > 195.0, "{x1}");
}

#[test]
fn smooth_points_of_curves_stay_smooth_with_any_join() {
    // A circle stroked with a uniform profile is the same ring whatever the join.
    let c = kurbo::Circle::new((0.0, 0.0), 50.0).to_path(1e-4);
    let want = 2.0 * std::f64::consts::PI * 50.0 * 10.0;
    for join in [LineJoin::Miter, LineJoin::Round, LineJoin::Bevel] {
        let st = stroke(10.0, |s| {
            s.profile = Some(WidthProfile { points: vec![(0.0, 1.0, 1.0), (1.0, 1.0, 1.0)] });
            s.join = join;
        });
        let a = line_outline(&c, &st, 10.0, 1e-3).area().abs();
        assert!((a - want).abs() / want < 0.005, "{join:?}: {a} vs {want}");
    }
}
