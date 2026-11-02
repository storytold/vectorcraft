use std::collections::BTreeSet;

use kurbo::{ParamCurve, ParamCurveArclen, Point, Vec2};
use vectorcraft_geom::{Anchor, AnchorKind, PathData, SubPath};

use super::*;
use crate::addons::simplify_anchors_keep_shape;

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn line(pts: &[(f64, f64)], closed: bool) -> SubPath {
    SubPath::polyline(&pts.iter().map(|&(x, y)| pt(x, y)).collect::<Vec<_>>(), closed)
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn length(sp: &SubPath) -> f64 {
    (0..sp.segment_count()).map(|i| sp.segment(i).arclen(1e-6)).sum()
}

/// The farthest any sampled point of `sp` lies off the circle about `c` of radius `r`.
fn off_circle(sp: &SubPath, c: Point, r: f64) -> f64 {
    (0..sp.segment_count())
        .flat_map(|i| (0..=16).map(move |k| (i, k as f64 / 16.0)))
        .map(|(i, t)| (sp.segment(i).eval(t).distance(c) - r).abs())
        .fold(0.0, f64::max)
}

#[test]
fn auto_smooth_turns_a_square_into_a_circle() {
    let mut sq = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        assert!(auto_smooth(&mut sq, i, SMOOTH_RATIO));
    }
    let a = sq.anchors[1];
    assert_eq!(a.kind, AnchorKind::Smooth);
    // Each handle is the ratio × the side long, at right angles to the corner's bisector.
    assert!(near((a.h_out - a.p).hypot(), 39.05, 1e-9) && near((a.h_in - a.p).hypot(), 39.05, 1e-9));
    assert!(near((a.h_out - a.p).dot(Vec2::new(-1.0, 1.0)), 0.0, 1e-9));
    // The four corners make a circle through them, to within its cubic approximation.
    assert!(off_circle(&sq, pt(50.0, 50.0), 50f64.hypot(50.0)) < 0.2, "{}", off_circle(&sq, pt(50.0, 50.0), 50f64.hypot(50.0)));
    // A smooth anchor stays as it is.
    assert!(!auto_smooth(&mut sq, 1, SMOOTH_RATIO));
}

#[test]
fn auto_smooth_mirrors_a_lone_handle_and_aims_open_ends() {
    let mut sp = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(40.0, -10.0), pt(50.0, 0.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(auto_smooth(&mut sp, 1, SMOOTH_RATIO));
    assert_eq!(sp.anchors[1].h_out, pt(60.0, 10.0));
    // An open end with no handle aims at its neighbour's facing handle.
    assert!(auto_smooth(&mut sp, 0, SMOOTH_RATIO));
    let h = sp.anchors[0].h_out;
    assert!(near(h.distance(pt(0.0, 0.0)), 50.0 * SMOOTH_RATIO, 1e-9));
    assert!(near((h - pt(0.0, 0.0)).cross(pt(40.0, -10.0) - pt(0.0, 0.0)), 0.0, 1e-9), "{h:?}");
    // Two handles out of line are brought into line, keeping their lengths.
    let mut bent = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(40.0, 0.0), pt(50.0, 20.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(auto_smooth(&mut bent, 1, SMOOTH_RATIO));
    let a = bent.anchors[1];
    assert!(
        near((a.h_in - a.p).cross(a.h_out - a.p), 0.0, 1e-9) && near((a.h_out - a.p).hypot(), 20.0, 1e-9) && near((a.h_in - a.p).hypot(), 10.0, 1e-9)
    );
}

#[test]
fn equalise_averages_lengths_and_makes_a_missing_handle() {
    let mut sp = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(40.0, 0.0), pt(80.0, 0.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(equalize_handles(&mut sp, 1));
    assert_eq!((sp.anchors[1].h_in, sp.anchors[1].h_out), (pt(30.0, 0.0), pt(70.0, 0.0)));
    assert!(!equalize_handles(&mut sp, 1), "already equal");
    let mut one = SubPath::new(
        vec![Anchor::corner(pt(0.0, 0.0)), Anchor::with_handles(pt(50.0, 0.0), pt(50.0, 0.0), pt(50.0, 20.0)), Anchor::corner(pt(100.0, 0.0))],
        false,
    );
    assert!(equalize_handles(&mut one, 1));
    assert_eq!(one.anchors[1].h_in, pt(50.0, -20.0));
    assert!(!equalize_handles(&mut one, 0), "no handles to equalise");
}

#[test]
fn make_corner_retracts_or_keeps_handles() {
    let a = Anchor::smooth(pt(50.0, 0.0), pt(60.0, 0.0));
    let mut sp = SubPath::new(vec![Anchor::corner(pt(0.0, 0.0)), a, Anchor::corner(pt(100.0, 0.0))], false);
    assert!(make_corner(&mut sp, 1, false));
    assert_eq!((sp.anchors[1].kind, sp.anchors[1].h_out), (AnchorKind::Corner, pt(60.0, 0.0)));
    assert!(make_corner(&mut sp, 1, true));
    assert!(!sp.anchors[1].has_in() && !sp.anchors[1].has_out());
}

#[test]
fn add_points_splits_segments_into_equal_lengths() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 40.0)], false);
    let (out, added) = add_points(&sp, Some(&BTreeSet::from([0])), 3);
    let xs: Vec<f64> = out.anchors.iter().map(|a| a.p.x).collect();
    for (x, want) in xs.iter().zip([0.0, 25.0, 50.0, 75.0, 100.0, 100.0]) {
        assert!(near(*x, want, 1e-4), "{xs:?}");
    }
    assert_eq!((xs.len(), added), (6, BTreeSet::from([1, 2, 3])));
    // On a curve the pieces have the same length and the shape stays.
    let mut sq = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        auto_smooth(&mut sq, i, SMOOTH_RATIO);
    }
    let (out, added) = add_points(&sq, None, 2);
    assert_eq!((out.anchors.len(), added.len()), (12, 8));
    let lens: Vec<f64> = (0..3).map(|i| out.segment(i).arclen(1e-6)).collect();
    assert!(near(lens[0], lens[1], 1e-3) && near(lens[1], lens[2], 1e-3), "{lens:?}");
    assert!(near(length(&out), length(&sq), 1e-4));
}

#[test]
fn close_joins_ends_three_ways() {
    let open = SubPath::new(
        vec![Anchor::with_handles(pt(0.0, 0.0), pt(-10.0, 10.0), pt(10.0, -10.0)), Anchor::corner(pt(100.0, 0.0)), Anchor::corner(pt(50.0, 80.0))],
        false,
    );
    let mut kept = open.clone();
    assert!(close(&mut kept, CloseMode::Handles, SMOOTH_RATIO));
    assert!(kept.closed && kept.anchors[0].h_in == pt(-10.0, 10.0));
    let mut straight = open.clone();
    assert!(close(&mut straight, CloseMode::Straight, SMOOTH_RATIO));
    assert!(straight.closed && straight.segment_is_line(2));
    let mut smooth = open.clone();
    assert!(close(&mut smooth, CloseMode::Smooth, SMOOTH_RATIO));
    assert!(smooth.anchors[2].has_in() && smooth.anchors[2].has_out(), "the last anchor is made smooth");
    assert!(!close(&mut smooth, CloseMode::Smooth, SMOOTH_RATIO), "closed already");
    assert_eq!(CloseMode::parse("straight"), Some(CloseMode::Straight));
}

#[test]
fn extend_straight_lengthens_and_shortens_either_end() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    let end = extend_open(&sp, false, 50.0, ExtendMode::Straight).unwrap();
    assert_eq!(end.anchors.last().unwrap().p, pt(150.0, 0.0));
    let start = extend_open(&sp, true, 20.0, ExtendMode::Straight).unwrap();
    assert_eq!(start.anchors.first().unwrap().p, pt(-20.0, 0.0));
    let short = extend_open(&sp, false, -30.0, ExtendMode::Straight).unwrap();
    assert_eq!(short.anchors.len(), 2);
    assert!(near(short.anchors[1].p.x, 70.0, 1e-6));
    // Shortening past an anchor drops it.
    let bent = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 10.0)], false);
    let cut = extend_open(&bent, false, -40.0, ExtendMode::Straight).unwrap();
    assert_eq!(cut.anchors.len(), 2);
    assert!(near(cut.anchors[1].p.x, 70.0, 1e-6));
    // Nothing is left of a path shortened by its whole length; closed paths have no ends.
    assert!(extend_open(&sp, false, -100.0, ExtendMode::Straight).is_none());
    assert!(extend_open(&line(&[(0.0, 0.0), (10.0, 0.0), (0.0, 10.0)], true), false, 5.0, ExtendMode::Straight).is_none());
    assert!(extend_open(&sp, false, f64::NAN, ExtendMode::Arc).is_none());
}

/// A quarter of the circle of radius 100 about the origin, from (100, 0) to (0, 100).
fn quarter() -> SubPath {
    let k = 4.0 / 3.0 * (std::f64::consts::FRAC_PI_8).tan() * 100.0;
    SubPath::new(
        vec![Anchor::with_handles(pt(100.0, 0.0), pt(100.0, 0.0), pt(100.0, k)), Anchor::with_handles(pt(0.0, 100.0), pt(k, 100.0), pt(0.0, 100.0))],
        false,
    )
}

#[test]
fn extend_arc_follows_the_circle_the_end_curves_along() {
    let sp = quarter();
    let len = std::f64::consts::FRAC_PI_2 * 100.0;
    let out = extend_open(&sp, false, len, ExtendMode::Arc).unwrap();
    // The circle matches the curvature the (cubic) quarter ends with, within a percent or so of
    // the true circle's.
    assert!(off_circle(&out, Point::ORIGIN, 100.0) < 3.0, "{}", off_circle(&out, Point::ORIGIN, 100.0));
    let end = out.anchors.last().unwrap().p;
    assert!(end.distance(pt(-100.0, 0.0)) < 5.0, "{end:?}");
    assert!(near(length(&out), 2.0 * len, 0.2));
    // At most half way round.
    let long = extend_open(&sp, false, 10.0 * len, ExtendMode::Arc).unwrap();
    assert!(near(length(&long) - len, std::f64::consts::PI * 100.0, 6.0), "{}", length(&long) - len);
    // A straight end carries on straight.
    let straight = extend_open(&line(&[(0.0, 0.0), (10.0, 0.0)], false), false, 5.0, ExtendMode::Arc).unwrap();
    assert_eq!(straight.anchors.last().unwrap().p, pt(15.0, 0.0));
}

#[test]
fn extend_bezier_stretches_the_last_curve_without_new_anchors() {
    let sp = quarter();
    let before = length(&sp);
    let out = extend_open(&sp, false, 30.0, ExtendMode::Bezier).unwrap();
    assert_eq!(out.anchors.len(), 2);
    assert!(near(length(&out), before + 30.0, 1e-3), "{}", length(&out));
    let p = out.segment(0).eval(0.5);
    assert!((p.to_vec2().hypot() - 100.0).abs() < 15.0);
}

#[test]
fn the_length_a_drag_asks_for() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    assert!(near(extend_length_to(&sp, false, pt(130.0, 10.0), ExtendMode::Straight).unwrap(), 30.0, 1e-6));
    assert!(near(extend_length_to(&sp, false, pt(40.0, 5.0), ExtendMode::Straight).unwrap(), -60.0, 1e-6));
    assert!(near(extend_length_to(&sp, true, pt(-25.0, 3.0), ExtendMode::Straight).unwrap(), 25.0, 1e-6));
    // Never the whole path.
    assert!(extend_length_to(&sp, false, pt(-50.0, 0.0), ExtendMode::Straight).unwrap() > -100.0);
    let arc = extend_length_to(&quarter(), false, pt(-100.0, 0.0), ExtendMode::Arc).unwrap();
    assert!(near(arc, std::f64::consts::FRAC_PI_2 * 100.0, 3.0), "{arc}");
    assert!(extend_length_to(&sp, false, pt(f64::NAN, 0.0), ExtendMode::Straight).is_none());
}

#[test]
fn crossings_are_found_on_both_paths() {
    let a = PathData::single(line(&[(0.0, 0.0), (100.0, 100.0)], false));
    let b = PathData::single(line(&[(0.0, 100.0), (100.0, 0.0)], false));
    let hits = crossings(&[&a, &b], false).unwrap();
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].len(), 1);
    assert!(near(hits[0][0].2, 0.5, 1e-6) && near(hits[1][0].2, 0.5, 1e-6));
    // A circle crossed by a line: twice.
    let mut circle = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true);
    for i in 0..4 {
        auto_smooth(&mut circle, i, SMOOTH_RATIO);
    }
    let across = PathData::single(line(&[(-40.0, 50.0), (140.0, 50.0)], false));
    let circle = PathData::single(circle);
    let hits = crossings(&[&circle, &across], false).unwrap();
    assert_eq!((hits[0].len(), hits[1].len()), (2, 2), "{hits:?}");
    // A path doesn't cross itself where its segments meet, and only crosses itself when asked.
    let bow = PathData::single(line(&[(0.0, 0.0), (100.0, 100.0), (100.0, 0.0), (0.0, 100.0)], false));
    assert!(crossings(&[&bow], false).unwrap()[0].is_empty());
    assert_eq!(crossings(&[&bow], true).unwrap()[0].len(), 2, "one crossing, on two of its segments");
    // Two identical paths lying on each other don't tangle the search.
    let sq = PathData::single(circle.subpaths[0].clone());
    assert!(crossings(&[&sq, &circle], false).is_some());
    // A path ending on another meets it there: at its end anchor, and part way along the other.
    let t = PathData::single(line(&[(50.0, 50.0), (50.0, 150.0)], false));
    let hits = crossings(&[&b, &t], false).unwrap();
    assert_eq!((hits[0].len(), hits[1].len()), (1, 1), "{hits:?}");
    assert!(near(hits[1][0].2, 0.0, 1e-6) && near(b.subpaths[0].segment(0).eval(hits[0][0].2).x, 50.0, 1e-6));
}

#[test]
fn insert_at_keeps_the_shape_and_names_the_new_anchors() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false);
    let (out, marks) = insert_at(&sp, &[(1, 0.5), (0, 0.25), (0, 0.75), (1, 1.0), (7, 0.5)]);
    let pts: Vec<Point> = out.anchors.iter().map(|a| a.p).collect();
    let c = sp.segment(0);
    let want = [pt(0.0, 0.0), c.eval(0.25), c.eval(0.75), pt(100.0, 0.0), pt(100.0, 50.0), pt(100.0, 100.0)];
    assert_eq!(pts.len(), want.len());
    assert!(pts.iter().zip(want).all(|(p, w)| p.distance(w) < 1e-9), "{pts:?}");
    assert_eq!(marks, BTreeSet::from([1, 2, 4, 5]));
    // The closing segment of a closed path.
    let tri = line(&[(0.0, 0.0), (100.0, 0.0), (0.0, 100.0)], true);
    let (out, marks) = insert_at(&tri, &[(2, 0.5)]);
    assert_eq!((out.anchors.len(), out.anchors[3].p, marks), (4, pt(0.0, 50.0), BTreeSet::from([3])));
}

#[test]
fn inspect_reads_lengths_area_angles_and_curvature() {
    let rect = PathData::single(line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 50.0), (0.0, 50.0)], true));
    let i = inspect(&rect, pt(50.0, 1.0), 3.0).unwrap();
    assert_eq!((i.closed, i.anchors, i.segment), (true, 4, 0));
    assert!(near(i.length, 300.0, 1e-6) && near(i.area, 5000.0, 1e-6) && near(i.segment_length, 100.0, 1e-6));
    assert_eq!((i.radius, i.anchor.is_none()), (None, true));
    let corner = inspect(&rect, pt(99.0, 1.0), 3.0).unwrap().anchor.unwrap();
    assert_eq!(corner.index, 1);
    assert!(near(corner.angle.unwrap(), 90.0, 1e-9));
    let r = inspect(&PathData::single(quarter()), pt(71.0, 71.0), 1.0).unwrap();
    assert!(near(r.radius.unwrap(), 100.0, 1.0), "{:?}", r.radius);
    assert!(near(r.tangent, -135.0, 1.0), "{}", r.tangent);
    let open = inspect(&PathData::single(quarter()), pt(100.0, 0.0), 1.0).unwrap().anchor.unwrap();
    assert_eq!(open.angle, None, "an open end has no angle");
    assert!(inspect(&PathData::default(), pt(0.0, 0.0), 1.0).is_none());
    // Handle readouts: length and direction counter-clockwise on the page.
    assert_eq!(handle_polar(pt(0.0, 0.0), pt(0.0, -10.0)), Some((10.0, 90.0)));
    assert!(handle_at(pt(0.0, 0.0), 10.0, 90.0).distance(pt(0.0, -10.0)) < 1e-9);
}

#[test]
fn the_brush_kernel_removes_only_marked_anchors() {
    let pts: Vec<(f64, f64)> = (0..=10).map(|i| (i as f64 * 10.0, 0.0)).collect();
    let sp = line(&pts, false);
    let marked: Vec<bool> = (0..=10).map(|i| (3..=5).contains(&i)).collect();
    let out = simplify_anchors_keep_shape(&sp, 0.1, &marked);
    let xs: Vec<f64> = out.anchors.iter().map(|a| a.p.x).collect();
    assert_eq!(xs, [0.0, 10.0, 20.0, 60.0, 70.0, 80.0, 90.0, 100.0]);
    // Off the line, a removal within tolerance only.
    let bump = line(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (30.0, 5.0), (40.0, 0.0)], false);
    assert_eq!(simplify_anchors_keep_shape(&bump, 0.1, &[true; 5]).anchors.len(), 4, "only the anchor on the straight run goes");
    assert_eq!(simplify_anchors_keep_shape(&bump, f64::INFINITY, &[true; 5]).anchors.len(), 2, "unless tolerance is ignored");
}

/// A circle about (0, 0) of radius 100 with `n` smooth anchors, each arc a cubic.
fn circle(n: usize) -> SubPath {
    let step = std::f64::consts::TAU / n as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan() * 100.0;
    let anchors = (0..n).map(|i| {
        let a = i as f64 * step;
        let (p, tan) = (pt(a.cos() * 100.0, a.sin() * 100.0), Vec2::new(-a.sin(), a.cos()));
        Anchor { p, h_in: p - tan * k, h_out: p + tan * k, kind: AnchorKind::Smooth }
    });
    SubPath::new(anchors.collect(), true)
}

#[test]
fn a_slid_anchor_keeps_the_shape_its_count_and_its_index() {
    let sp = circle(8);
    for (index, t) in [(2, 0.5), (2, -0.5), (0, -0.25), (0, 0.25), (7, 0.9), (7, -0.9)] {
        let (out, strayed) = slide_anchor(&sp, index, t).unwrap();
        let seg = if t > 0.0 { sp.segment(index) } else { sp.segment((index + 7) % 8) };
        let want = seg.eval(if t > 0.0 { t } else { 1.0 + t });
        assert_eq!(out.anchors.len(), 8, "{index} {t}");
        assert!(out.anchors[index].p.distance(want) < 1e-9, "{index} {t}: the anchor is where it was sent");
        assert!(strayed < 0.05 && off_circle(&out, pt(0.0, 0.0), 100.0) < 0.05, "{index} {t}: {strayed}");
        // The other anchors stay.
        assert!((0..8).filter(|i| *i != index).all(|i| out.anchors[i].p == sp.anchors[i].p), "{index} {t}");
    }
}

#[test]
fn sliding_along_straight_runs_stays_straight_and_ends_do_not_slide() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0), (200.0, 100.0)], false);
    let (out, strayed) = slide_anchor(&sp, 1, 0.5).unwrap();
    assert_eq!(out.anchors[1].p, pt(150.0, 0.0));
    assert!(strayed < 1e-9 && out.segment_is_line(0) && out.segment_is_line(1));
    assert_eq!(slide_anchor(&sp, 1, 0.0).unwrap().0, sp, "no move");
    assert_eq!(slide_anchor(&sp, 1, 1e-9).unwrap().0, sp, "too small a move");
    for (i, t) in [(0, 0.5), (3, -0.5), (9, 0.5), (1, 0.99), (1, -1.0), (1, f64::NAN), (1, f64::INFINITY)] {
        assert!(slide_anchor(&sp, i, t).is_none(), "{i} {t}");
    }
    let mut bad = sp.clone();
    bad.anchors[1].h_out = pt(f64::NAN, 0.0);
    assert!(slide_anchor(&bad, 1, 0.5).is_none());
}

#[test]
fn the_slide_a_pointer_asks_for_and_where_the_tangent_stops() {
    let sp = line(&[(0.0, 0.0), (100.0, 0.0), (200.0, 0.0)], false);
    assert!(near(slide_t_at(&sp, 1, pt(150.0, 30.0)).unwrap(), 0.5, 1e-6));
    let back = slide_t_at(&sp, 1, pt(75.0, -30.0)).unwrap();
    assert!(back < 0.0 && sp.segment(0).eval(1.0 + back).distance(pt(75.0, 0.0)) < 1e-3, "{back}");
    assert!(near(slide_t_at(&sp, 1, pt(900.0, 0.0)).unwrap(), MAX_SLIDE, 1e-9), "held short of the next anchor");
    assert!(slide_t_at(&sp, 0, pt(50.0, 0.0)).is_none());
    assert!(slide_stops(&sp, 1, 0.0).is_empty(), "a straight run has no tangent to reach");
    let slanted = line(&[(0.0, 0.0), (100.0, 100.0), (250.0, 250.0)], false);
    assert!(slide_stops(&slanted, 1, 0.0).is_empty(), "nor one that lies on a multiple of 45°");
    // Round a circle of 4 anchors the tangent is at a multiple of 45° at each anchor and half way.
    let round = circle(4);
    let stops = slide_stops(&round, 0, 0.0);
    assert_eq!(stops.len(), 2, "{stops:?}");
    assert!(near(stops[0], -0.5, 1e-6) && near(stops[1], 0.5, 1e-6), "{stops:?}");
    // From a constrain angle of 22.5°, between those.
    let stops = slide_stops(&round, 0, 22.5);
    assert_eq!(stops.len(), 4, "{stops:?}");
    assert!(stops.windows(2).all(|w| w[0] < w[1]));
}

#[test]
fn add_anchors_spaces_them_by_count_or_distance() {
    let sq = line(&[(0.0, 0.0), (90.0, 0.0), (90.0, 90.0), (0.0, 90.0)], true);
    let out = add_anchors(&sq, SegmentKind::All, AnchorSpacing::Count(2), 100, 100).unwrap();
    assert_eq!(out.anchors.len(), 12);
    assert!(near(out.anchors[1].p.x, 30.0, 1e-6) && near(out.anchors[2].p.x, 60.0, 1e-6));
    assert!(out.closed);
    let out = add_anchors(&sq, SegmentKind::All, AnchorSpacing::Distance(40.0), 100, 100).unwrap();
    assert_eq!(out.anchors.len(), 12, "at 40 and 80, none past the segment");
    assert!(near(out.anchors[2].p.x, 80.0, 1e-6));
    assert_eq!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Distance(1.0), 3, 100).unwrap().anchors.len(), 16, "the cap");
    assert_eq!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Distance(0.0), 3, 100).unwrap(), sq);
    assert_eq!(add_anchors(&sq, SegmentKind::Curved, AnchorSpacing::Count(1), 3, 100).unwrap(), sq, "no curves");
    assert_eq!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Count(3), 9, 12).unwrap().anchors.len(), 16, "room for all of them");
    assert!(add_anchors(&sq, SegmentKind::All, AnchorSpacing::Count(3), 9, 11).is_none(), "no room");
    // A curve keeps its shape, and a zero-length segment gets nothing.
    let bent = SubPath::new(
        vec![
            Anchor { h_out: pt(0.0, 50.0), ..Anchor::corner(pt(0.0, 0.0)) },
            Anchor { h_in: pt(100.0, 50.0), ..Anchor::corner(pt(100.0, 0.0)) },
            Anchor::corner(pt(100.0, 0.0)),
        ],
        false,
    );
    let out = add_anchors(&bent, SegmentKind::Curved, AnchorSpacing::Count(3), 9, 9).unwrap();
    assert_eq!(out.anchors.len(), 6);
    assert!(near(length(&out), length(&bent), 1e-4));
    assert_eq!(add_anchors(&bent, SegmentKind::Straight, AnchorSpacing::Count(3), 9, 9).unwrap(), bent);
}

#[test]
fn smoothing_every_anchor_reads_the_same_either_way_along_the_path() {
    let zig = line(&[(0.0, 0.0), (30.0, 40.0), (70.0, 0.0), (100.0, 50.0)], false);
    let mut there = zig.clone();
    smooth_anchors(&mut there, SMOOTH_RATIO);
    let mut back = zig.clone();
    back.reverse();
    smooth_anchors(&mut back, SMOOTH_RATIO);
    back.reverse();
    for (a, b) in there.anchors.iter().zip(&back.anchors) {
        assert!(a.h_in.distance(b.h_in) < 1e-9 && a.h_out.distance(b.h_out) < 1e-9, "{a:?} {b:?}");
    }
    assert!(there.anchors[1].kind == AnchorKind::Smooth && there.anchors[0].has_out() && there.anchors[3].has_in());
    // Nothing to smooth.
    for mut sp in [SubPath::default(), line(&[(1.0, 1.0)], false), line(&[(1.0, 1.0)], true)] {
        let before = sp.clone();
        smooth_anchors(&mut sp, SMOOTH_RATIO);
        assert_eq!(sp, before);
    }
}

#[test]
fn splitting_reads_an_anchors_kind_from_its_handles() {
    // Smooth by its handles though it is called a corner, as a path read back from an outline is.
    let run = SubPath::new(
        vec![
            Anchor::corner(pt(0.0, 0.0)),
            Anchor { h_in: pt(40.0, 0.0), h_out: pt(60.0, 0.0), ..Anchor::corner(pt(50.0, 0.0)) },
            Anchor::corner(pt(100.0, 0.0)),
            Anchor::corner(pt(100.0, 50.0)),
        ],
        false,
    );
    assert_eq!(split_at_anchors(&run, false, true).len(), 2);
    assert_eq!(split_at_anchors(&run, true, false).len(), 2);
    assert_eq!(split_at_anchors(&run, true, true).len(), 3);
    assert_eq!(split_at_anchors(&run, false, false), std::slice::from_ref(&run));
    let pieces = split_at_anchors(&run, true, true);
    assert!(near(pieces.iter().map(length).sum::<f64>(), length(&run), 1e-6), "the shape stays");
    // A closed subpath opens at its first cut.
    let sq = line(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], true);
    assert_eq!(split_at_anchors(&sq, true, false).len(), 3);
    assert!(split_at_anchors(&sq, true, false).iter().all(|p| !p.closed));
}
