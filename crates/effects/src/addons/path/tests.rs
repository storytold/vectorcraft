use serde_json::{Value, json};
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Anchor, PathData, Point, Rect, SubPath};

use super::*;
use crate::{apply_geometry, effect_info, is_geometry, is_length, is_raster, length_range};

fn pt(x: f64, y: f64) -> Point {
    Point::new(x, y)
}

fn line(pts: &[(f64, f64)], closed: bool) -> PathData {
    PathData::single(SubPath::polyline(&pts.iter().map(|&(x, y)| pt(x, y)).collect::<Vec<_>>(), closed))
}

fn square() -> PathData {
    line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)], true)
}

/// A curve: a smooth anchor between two ends.
fn curve() -> PathData {
    let mid = Anchor::smooth(pt(50.0, -30.0), pt(70.0, -30.0));
    PathData::single(SubPath::new(
        vec![
            Anchor { h_out: pt(10.0, -20.0), ..Anchor::corner(pt(0.0, 0.0)) },
            mid,
            Anchor { h_in: pt(90.0, -20.0), ..Anchor::corner(pt(100.0, 0.0)) },
        ],
        false,
    ))
}

fn run(id: &str, params: Value, path: &PathData) -> PathData {
    let e = Effect { id: id.into(), params, visible: true };
    apply_geometry(&[e], path, path.bounds().unwrap_or(Rect::ZERO))
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-3
}

const ALL: [&str; 9] = [DASHES, ADD_ANCHORS, EXTEND, CLOSE, SMOOTH, RETRACT, REVERSE, SIMPLIFY, SPLIT];

#[test]
fn the_effects_are_geometry_effects_of_the_path_effects_menu() {
    for id in ALL {
        let info = effect_info(id).unwrap();
        assert!(is_geometry(id) && !is_raster(id), "{id}");
        assert_eq!(info.menu, ["Effect", "Path Effects"], "{id}");
        assert!(id.starts_with(PREFIX));
    }
    assert!(is_length(DASHES, "gap5", false) && is_length(EXTEND, "endLength", false) && !is_length(EXTEND, "endPercent", false));
    assert_eq!(length_range(DASHES, "offset"), Some(-10_000.0..=10_000.0));
}

#[test]
fn dashes_become_open_subpaths_along_the_path() {
    let out = run(DASHES, json!({"dash1": 10, "gap1": 5}), &line(&[(0.0, 0.0), (100.0, 0.0)], false));
    assert_eq!(out.subpaths.len(), 7, "periods of 15 on 100");
    assert!(out.subpaths.iter().all(|s| !s.closed));
    assert!(near(out.subpaths[1].anchors[0].p.x, 15.0) && near(out.subpaths[1].anchors[1].p.x, 25.0));
    // Two pairs, and a pair of zeros between them left out.
    let out = run(DASHES, json!({"dash1": 10, "gap1": 5, "dash3": 20, "gap3": 5}), &line(&[(0.0, 0.0), (100.0, 0.0)], false));
    assert!(near(out.subpaths[1].anchors[0].p.x, 15.0) && near(out.subpaths[1].anchors[1].p.x, 35.0));
    assert!(near(out.subpaths[2].anchors[0].p.x, 40.0));
    // The offset starts part way into the pattern.
    let out = run(DASHES, json!({"dash1": 10, "gap1": 5, "offset": 5}), &line(&[(0.0, 0.0), (100.0, 0.0)], false));
    assert!(near(out.subpaths[0].anchors[1].p.x, 5.0));
}

#[test]
fn aligned_dashes_centre_on_corners_and_ends() {
    let open = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false);
    let out = run(DASHES, json!({"dash1": 10, "gap1": 10, "alignCorners": true}), &open);
    let on = |p: Point| out.subpaths.iter().any(|s| (0..s.segment_count()).any(|i| s.anchors[i].p == p || s.anchors[i + 1].p == p));
    assert!(on(pt(0.0, 0.0)) && on(pt(100.0, 100.0)), "a dash on each end");
    let corner = out.subpaths.iter().find(|s| s.anchors.iter().any(|a| a.p == pt(100.0, 0.0))).expect("a dash over the corner");
    assert!(near(corner.anchors[0].p.distance(pt(100.0, 0.0)), corner.anchors[2].p.distance(pt(100.0, 0.0))), "centred: {corner:?}");
}

#[test]
fn zero_dashes_are_points_and_patterns_of_nothing_leave_the_path() {
    let path = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    let dots = run(DASHES, json!({"dash1": 0, "gap1": 25}), &path);
    assert_eq!(dots.subpaths.len(), 5);
    assert!(dots.subpaths.iter().all(|s| s.anchors.len() == 2 && s.anchors[0].p == s.anchors[1].p));
    for params in [
        json!({"dash1": 0, "gap1": 0}),
        json!({"dash1": -5, "gap1": -5}),
        json!({"dash1": 0.0001, "gap1": 0.0001}),
        json!({"dash1": "x", "gap1": null}),
    ] {
        assert_eq!(run(DASHES, params.clone(), &path), path, "{params}");
    }
}

#[test]
fn add_anchors_spaces_them_and_keeps_to_the_caps() {
    let square = square();
    assert_eq!(run(ADD_ANCHORS, json!({}), &square).anchor_count(), 8, "one on each segment");
    assert_eq!(run(ADD_ANCHORS, json!({"count": 3}), &square).anchor_count(), 16);
    assert_eq!(run(ADD_ANCHORS, json!({"count": 3, "maxCount": 2}), &square).anchor_count(), 12);
    let out = run(ADD_ANCHORS, json!({"spacing": "distance", "distance": 30}), &square);
    assert_eq!(out.anchor_count(), 16, "at 30, 60 and 90 on each side");
    assert!(near(out.subpaths[0].anchors[2].p.x, 60.0));
    assert_eq!(run(ADD_ANCHORS, json!({"spacing": "distance", "distance": 25}), &square).anchor_count(), 16, "none on the segment's end");
    assert_eq!(run(ADD_ANCHORS, json!({"spacing": "distance", "distance": 1e-9, "maxCount": 2}), &square).anchor_count(), 12);
    // Only the segments asked for.
    let mixed = PathData::new(vec![curve().subpaths[0].clone(), square.subpaths[0].clone()]);
    assert_eq!(run(ADD_ANCHORS, json!({"target": "curved"}), &mixed).anchor_count(), 7 + 2);
    assert_eq!(run(ADD_ANCHORS, json!({"target": "straight"}), &mixed).anchor_count(), 7 + 4);
    // The shape stays.
    let out = run(ADD_ANCHORS, json!({"count": 4}), &curve());
    assert_eq!(out.anchor_count(), 3 + 8);
    assert!(near(out.length(), curve().length()));
    assert_eq!(out.bounds().map(|b| (b.y0 * 100.0).round()), curve().bounds().map(|b| (b.y0 * 100.0).round()));
}

#[test]
fn add_anchors_past_the_budget_leaves_the_path() {
    let path = square();
    let most = json!({"count": 1000, "maxCount": 1000});
    assert_eq!(super::apply(ADD_ANCHORS, &most, &path, 4003), path);
    assert_eq!(super::apply(ADD_ANCHORS, &most, &path, 4004).anchor_count(), 4004);
    // Many segments, each with the most anchors: refused before they are built.
    let long = line(&(0..4000).map(|i| (i as f64, 0.0)).collect::<Vec<_>>(), false);
    assert_eq!(run(ADD_ANCHORS, json!({"count": 1000, "maxCount": 1000}), &long).anchor_count(), 4000, "4 million would pass the budget");
}

#[test]
fn extend_lengthens_and_shortens_each_end() {
    let path = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    let out = run(EXTEND, json!({"startLength": 20, "endLength": 10}), &path);
    let xs: Vec<f64> = out.subpaths[0].anchors.iter().map(|a| a.p.x).collect();
    assert_eq!(xs, [-20.0, 0.0, 100.0, 110.0]);
    // Percent of the path's length, adding to the length; negative shortens.
    let out = run(EXTEND, json!({"startLength": 0, "endLength": 5, "endPercent": 10}), &path);
    assert!(out.subpaths[0].anchors.last().is_some_and(|a| near(a.p.x, 115.0)));
    let out = run(EXTEND, json!({"startLength": -10, "endLength": 0, "endPercent": -50}), &path);
    let xs: Vec<f64> = out.subpaths[0].anchors.iter().map(|a| a.p.x).collect();
    assert!(near(xs[0], 10.0) && near(xs[1], 50.0), "{xs:?}");
    // An arc carries on round the end's curve.
    let arc = run(EXTEND, json!({"startLength": 0, "endLength": 40, "endShape": "arc"}), &curve());
    let straight = run(EXTEND, json!({"startLength": 0, "endLength": 40}), &curve());
    assert!(arc.anchor_count() >= straight.anchor_count());
    assert!((arc.length() - straight.length()).abs() < 0.1);
    assert_ne!(arc.subpaths[0].anchors.last().map(|a| a.p), straight.subpaths[0].anchors.last().map(|a| a.p));
}

#[test]
fn extend_leaves_what_it_cannot_change() {
    let closed = square();
    assert_eq!(run(EXTEND, json!({"endLength": 50}), &closed), closed);
    let path = line(&[(0.0, 0.0), (100.0, 0.0)], false);
    assert_eq!(run(EXTEND, json!({"startLength": 0, "endLength": -500}), &path), path, "nothing would be left");
    assert_eq!(run(EXTEND, json!({"startLength": 0, "endLength": 0, "endPercent": -100}), &path), path);
    let one = PathData::single(SubPath::polyline(&[pt(1.0, 1.0)], false));
    assert_eq!(run(EXTEND, json!({}), &one), one);
}

#[test]
fn close_joins_open_subpaths_straight_or_smooth() {
    let open = line(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)], false);
    let out = run(CLOSE, json!({}), &open);
    assert!(out.subpaths[0].closed && out.subpaths[0].segment_is_line(2));
    let out = run(CLOSE, json!({"mode": "smooth"}), &open);
    assert!(out.subpaths[0].closed && !out.subpaths[0].segment_is_line(2));
    assert_eq!(run(CLOSE, json!({}), &square()), square());
    let one = PathData::single(SubPath::polyline(&[pt(1.0, 1.0)], false));
    assert_eq!(run(CLOSE, json!({}), &one), one);
}

#[test]
fn smooth_makes_a_square_round_and_clamps_its_ratio() {
    let out = run(SMOOTH, json!({}), &square());
    assert!(out.subpaths[0].anchors.iter().all(|a| near(a.h_out.distance(a.p), 39.05) && a.has_in()));
    let at = |ratio: f64| run(SMOOTH, json!({ "ratio": ratio }), &square()).subpaths[0].anchors[0];
    assert!(near(at(9.0).h_out.distance(pt(0.0, 0.0)), 50.0));
    assert!(near(at(-9.0).h_out.distance(pt(0.0, 0.0)), 1.0));
}

#[test]
fn retract_takes_all_handles_or_the_short_ones() {
    let out = run(RETRACT, json!({}), &curve());
    assert!(out.subpaths[0].anchors.iter().all(|a| !a.has_in() && !a.has_out()));
    // The ends' handles are about 22 pt long and the middle anchor's 20.
    let out = run(RETRACT, json!({"handles": "shorter", "length": 21}), &curve());
    let a = &out.subpaths[0].anchors;
    assert!(!a[1].has_in() && !a[1].has_out() && a[0].has_out() && a[2].has_in());
    assert_eq!(run(RETRACT, json!({"handles": "shorter", "length": 1}), &curve()), curve());
}

#[test]
fn reverse_runs_the_other_way_and_twice_is_as_it_was() {
    let out = run(REVERSE, json!({}), &curve());
    assert_eq!(out.subpaths[0].anchors[0].p, pt(100.0, 0.0));
    assert_eq!(run(REVERSE, json!({}), &out), curve());
}

#[test]
fn simplify_takes_anchors_within_the_tolerance_and_stops_at_its_cap() {
    let pts: Vec<(f64, f64)> = (0..=40).map(|i| (i as f64, (i as f64 / 40.0 * std::f64::consts::PI).sin() * 3.0)).collect();
    let path = line(&pts, false);
    let out = run(SIMPLIFY, json!({"tolerance": 200}), &path);
    assert!(out.anchor_count() < 20, "{}", out.anchor_count());
    assert_eq!(run(SIMPLIFY, json!({"tolerance": 0}), &path), path);
    // Corners stay unless told otherwise.
    let corner = line(&[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0)], false);
    assert_eq!(run(SIMPLIFY, json!({"tolerance": 200}), &corner), corner);
    assert_eq!(run(SIMPLIFY, json!({"tolerance": 200, "protectCorners": false}), &corner).anchor_count(), 2);
    let long = line(&(0..=MAX_SIMPLIFY_ANCHORS).map(|i| (i as f64, 0.0)).collect::<Vec<_>>(), false);
    assert_eq!(run(SIMPLIFY, json!({}), &long).anchor_count(), long.anchor_count());
}

#[test]
fn split_cuts_at_the_anchors_asked_for() {
    let mixed = PathData::single(SubPath::new(
        vec![
            Anchor::corner(pt(0.0, 0.0)),
            Anchor::smooth(pt(50.0, 0.0), pt(60.0, 0.0)),
            Anchor::corner(pt(100.0, 0.0)),
            Anchor::corner(pt(100.0, 50.0)),
        ],
        false,
    ));
    assert_eq!(run(SPLIT, json!({}), &mixed).subpaths.len(), 3);
    assert_eq!(run(SPLIT, json!({"anchors": "smooth"}), &mixed).subpaths.len(), 2);
    assert_eq!(run(SPLIT, json!({"anchors": "corner"}), &mixed).subpaths.len(), 2);
    let out = run(SPLIT, json!({}), &square());
    assert_eq!(out.subpaths.len(), 4);
    assert!(out.subpaths.iter().all(|s| !s.closed && s.anchors.len() == 2));
    assert!(near(out.length(), 400.0));
    assert_eq!(run(SPLIT, json!({"anchors": "smooth"}), &square()), square(), "no smooth anchors");
}

/// Whatever a file holds: every effect takes wrong types, huge numbers and paths of nothing, of one
/// anchor or past measuring, and keeps to the budget.
#[test]
fn junk_params_and_paths_are_harmless() {
    let junk = [
        json!({}),
        json!(null),
        json!([1, 2]),
        json!("x"),
        json!({"dash1": 1e308, "gap1": 1e308, "dash2": -1e308, "gap2": "x", "offset": 1e308, "alignCorners": "yes"}),
        json!({"dash1": 1e-300, "gap1": 1e-300, "offset": -1e308, "alignCorners": true}),
        json!({"target": 3, "spacing": ["distance"], "count": 1e308, "distance": 1e-308, "maxCount": -1e308}),
        json!({"spacing": "distance", "distance": -1e308, "maxCount": 1e308}),
        json!({"startLength": 1e308, "startPercent": 1e308, "startShape": 7, "endLength": -1e308, "endPercent": -1e308, "endShape": "arc"}),
        json!({"mode": {}, "ratio": 1e308, "handles": "shorter", "length": 1e308, "tolerance": 1e308, "protectCorners": "no", "anchors": 9}),
        json!({"ratio": -1e308, "length": -1e308, "tolerance": -1e308, "anchors": "none"}),
    ];
    let far = line(&[(1e308, -1e308), (-1e308, 1e308), (1e300, 0.0)], false);
    let nan = line(&[(f64::NAN, 0.0), (f64::INFINITY, 1.0)], false);
    let tiny = line(&[(0.0, 0.0), (1e-300, 0.0), (1e-300, 1e-300)], true);
    let same = line(&[(5.0, 5.0), (5.0, 5.0), (5.0, 5.0)], true);
    let empty = PathData::single(SubPath::default());
    let one = PathData::single(SubPath::polyline(&[pt(1.0, 1.0)], true));
    let paths = [square(), curve(), far.clone(), nan.clone(), tiny, same, empty, one, PathData::default()];
    for id in ALL {
        for params in &junk {
            for path in &paths {
                let out = super::apply(id, &crate::merged_params(id, params), path, 100_000);
                assert!(out.anchor_count() <= 100_000.max(path.anchor_count()) + 8 * path.subpaths.len(), "{id} {params}");
            }
        }
        // Compared as text: NaN never equals itself.
        assert_eq!(format!("{:?}", super::apply(id, &json!({}), &nan, 100)), format!("{nan:?}"), "{id}");
        assert_eq!(super::apply(id, &json!({}), &far, 100), far, "{id}");
    }
}

/// The dash generator's limit holds: a pattern too fine for the path leaves it solid.
#[test]
fn too_many_dashes_leave_the_path() {
    let long = line(&[(0.0, 0.0), (1e9, 0.0)], false);
    assert_eq!(run(DASHES, json!({"dash1": 1, "gap1": 1}), &long), long);
}
