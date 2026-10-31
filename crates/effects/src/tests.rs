use serde_json::{Value, json};
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, PathData, Point, Rect, Shape, SubPath, shapes};

use super::*;

mod scale;
mod transformfx;

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// Every image under `n`, in document order (its placement's x).
fn image_xs(n: &Node, out: &mut Vec<f64>) {
    if let NodeKind::Image(im) = &n.kind {
        out.push(im.xf.as_coeffs()[4]);
    }
    for c in n.children().into_iter().flatten() {
        image_xs(c, out);
    }
}

/// Every symbol instance under `n`, in document order (its placement's x).
fn instance_xs(n: &Node, out: &mut Vec<f64>) {
    if let NodeKind::SymbolInstance { xf, .. } = &n.kind {
        out.push(xf.as_coeffs()[4]);
    }
    for c in n.children().into_iter().flatten() {
        instance_xs(c, out);
    }
}

fn square() -> PathData {
    shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0))
}

fn run(id: &str, params: Value, p: &PathData) -> PathData {
    apply_geometry(&[fx(id, params)], p, p.bounds().unwrap())
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn all_finite(p: &PathData) -> bool {
    p.subpaths.iter().flat_map(|s| &s.anchors).all(|a| a.p.x.is_finite() && a.p.y.is_finite() && a.h_in.x.is_finite() && a.h_out.y.is_finite())
}

/// Every geometry effect with non-trivial parameters.
fn geometry_cases() -> Vec<(&'static str, Value)> {
    let mut v = vec![
        ("distort.freeDistort", json!({"corners": [[0.1, 0.0], [0.9, 0.1], [1.0, 1.0], [0.0, 0.8]]})),
        ("distort.puckerBloat", json!({"amount": 40})),
        ("distort.puckerBloat", json!({"amount": -40})),
        ("distort.roughen", json!({})),
        ("distort.roughen", json!({"points": "corner", "size": 3, "relative": false})),
        ("distort.transform", json!({"scaleH": 50, "rotate": 30, "moveH": 10})),
        ("distort.tweak", json!({})),
        ("distort.twist", json!({"angle": 90})),
        ("distort.zigZag", json!({})),
        ("distort.zigZag", json!({"points": "corner", "ridges": 3})),
        ("path.offsetPath", json!({"offset": 5})),
        ("path.outlineStroke", json!({"width": 4})),
        ("convertToShape.rectangle", json!({})),
        ("convertToShape.roundedRectangle", json!({})),
        ("convertToShape.ellipse", json!({})),
        ("stylize.roundCorners", json!({"radius": 10})),
    ];
    for (s, _) in WARP_STYLES {
        let id: &'static str = Box::leak(format!("warp.{s}").into_boxed_str());
        v.push((id, json!({"bend": 50})));
    }
    v
}

#[test]
fn catalog_complete_and_unique() {
    let cat = effect_catalog();
    let mut ids: Vec<_> = cat.iter().map(|e| e.id).collect();
    ids.sort();
    ids.dedup();
    assert_eq!(ids.len(), cat.len());
    assert!(cat.len() >= 34);
    for e in &cat {
        assert!(e.defaults.is_object(), "{}", e.id);
        assert!(!e.params.is_empty());
        assert_eq!(e.menu[0], "Effect");
        assert_eq!(is_raster(e.id), e.raster);
    }
}

#[test]
fn defaults_match_dialogs() {
    let d = default_params("stylize.dropShadow").unwrap();
    assert_eq!(d["opacity"], json!(75.0));
    assert_eq!(d["x"], json!(7.0));
    assert_eq!(d["mode"], json!("multiply"));
    let r = default_params("distort.roughen").unwrap();
    assert_eq!(r["size"], json!(5.0));
    assert_eq!(r["detail"], json!(10.0));
    assert!(default_params("nope").is_none());
}

#[test]
fn merged_params_override_defaults() {
    let e = new_effect("distort.twist", &json!({"angle": 45})).unwrap();
    assert_eq!(e.params["angle"], json!(45));
    let e = new_effect("path.offsetPath", &json!({})).unwrap();
    assert_eq!(e.params["joins"], json!("miter"));
    assert!(new_effect("bogus.effect", &json!({})).is_none());
}

#[test]
fn every_geometry_effect_changes_geometry_and_keeps_closedness() {
    let sq = square();
    for (id, params) in geometry_cases() {
        let out = run(id, params.clone(), &sq);
        assert!(!out.is_empty(), "{id} produced nothing");
        assert!(out.is_closed(), "{id} opened the path");
        assert!(all_finite(&out), "{id} produced NaN");
        assert_ne!(out.to_bezpath(), sq.to_bezpath(), "{id} did nothing");
    }
}

#[test]
fn geometry_effects_are_deterministic() {
    let sq = square();
    for (id, params) in geometry_cases() {
        let a = run(id, params.clone(), &sq);
        let b = run(id, params, &sq);
        assert_eq!(a, b, "{id} is not deterministic");
    }
}

#[test]
fn open_paths_stay_open() {
    let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(50.0, 20.0), Point::new(100.0, 0.0)], false));
    for id in ["distort.roughen", "distort.zigZag", "distort.twist", "distort.tweak", "warp.arc", "warp.flag", "stylize.roundCorners"] {
        let out = run(id, json!({}), &line);
        assert!(!out.is_closed(), "{id}");
        assert_eq!(out.subpaths.len(), 1);
    }
}

#[test]
fn warp_keeps_bounds_sane() {
    let sq = square();
    let d = 100.0f64.hypot(100.0);
    for (s, _) in WARP_STYLES {
        for bend in [-100.0, -50.0, 50.0, 100.0] {
            for orient in ["horizontal", "vertical"] {
                let out = run(&format!("warp.{s}"), json!({"bend": bend, "horizontal": 30, "vertical": -30, "orientation": orient}), &sq);
                let b = out.bounds().unwrap();
                assert!(all_finite(&out), "warp.{s}");
                assert!(b.width() > 1.0 && b.height() > 1.0, "warp.{s} collapsed: {b:?}");
                assert!(b.width() < 3.0 * d && b.height() < 3.0 * d, "warp.{s} exploded: {b:?}");
                assert!(b.center().distance(Point::new(50.0, 50.0)) < d, "warp.{s} drifted");
            }
        }
    }
}

#[test]
fn warp_zero_bend_is_identity_ish() {
    let sq = square();
    for (s, _) in WARP_STYLES {
        let out = run(&format!("warp.{s}"), json!({"bend": 0}), &sq);
        let b = out.bounds().unwrap();
        assert!(close(b.x0, 0.0, 1e-6) && close(b.x1, 100.0, 1e-6) && close(b.y1, 100.0, 1e-6), "warp.{s}: {b:?}");
    }
}

#[test]
fn warp_arc_bends_the_top_edge() {
    let out = run("warp.arc", json!({"bend": 50}), &square());
    // The ends droop below the original bottom while the middle keeps the top.
    let b = out.bounds().unwrap();
    assert!(b.y1 > 101.0 && close(b.y0, 0.0, 0.5), "{b:?}");
    let out = run("warp.arc", json!({"bend": 50, "orientation": "vertical"}), &square());
    let b = out.bounds().unwrap();
    assert!(b.x0 < -1.0 || b.x1 > 101.0, "{b:?}");
}

#[test]
fn transform_copies_and_move() {
    let out = run("distort.transform", json!({"moveH": 20, "copies": 3}), &square());
    assert_eq!(out.subpaths.len(), 4);
    let b = out.bounds().unwrap();
    assert!(close(b.x1, 160.0, 1e-9));
    let r = run("distort.transform", json!({"scaleH": 50, "scaleV": 50}), &square()).bounds().unwrap();
    assert!(close(r.width(), 50.0, 1e-9) && close(r.center().x, 50.0, 1e-9));
    let f = run("distort.transform", json!({"reflectX": true, "moveV": 5}), &square()).bounds().unwrap();
    assert!(close(f.y0, 5.0, 1e-9));
}

#[test]
fn transform_scales_and_rotates_about_its_reference_point() {
    // Bottom right (8) stays put while the square halves; the top left (0) while it rotates.
    let r = run("distort.transform", json!({"scaleH": 50, "scaleV": 50, "reference": 8}), &square()).bounds().unwrap();
    assert!(close(r.x0, 50.0, 1e-9) && close(r.y0, 50.0, 1e-9) && close(r.x1, 100.0, 1e-9) && close(r.y1, 100.0, 1e-9), "{r:?}");
    let r = run("distort.transform", json!({"rotate": 90, "reference": 0}), &square()).bounds().unwrap();
    assert!(close(r.x0, 0.0, 1e-9) && close(r.x1, 100.0, 1e-9) && close(r.y0, -100.0, 1e-9) && close(r.y1, 0.0, 1e-9), "{r:?}");
    // Reflect X about the right edge's middle (5): a mirrored copy beside the original.
    let out = run("distort.transform", json!({"reflectX": true, "reference": 5, "copies": 1}), &square());
    assert_eq!(out.bounds().map(|b| (b.x0.round(), b.x1.round())), Some((0.0, 200.0)));
    // A point off the grid clamps to it.
    assert_eq!(run("distort.transform", json!({"scaleH": 50, "reference": 99}), &square()).bounds().map(|b| b.x0), Some(50.0));
}

#[test]
fn random_transform_is_stable_per_object_and_within_the_values() {
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    let bounds = |params: Value, seed| {
        apply_geometry_with(&[fx("distort.transform", params)], &square(), b, &GeomContext { seed, ..Default::default() }).bounds().unwrap()
    };
    let random = |seed| bounds(json!({"scaleH": 50, "moveH": 40, "random": true}), seed);
    let first = random(7);
    assert_eq!(random(7), first, "the same object always gets the same result");
    assert_ne!(random(8), first, "another object varies its own way");
    for seed in 0..50 {
        let r = random(seed);
        // Scale between 100 % and 50 %, move between 0 and 40 pt.
        assert!(r.width() >= 50.0 - 1e-9 && r.width() <= 100.0 + 1e-9, "{r:?}");
        assert!(r.center().x >= 50.0 - 1e-9 && r.center().x <= 90.0 + 1e-9, "{r:?}");
    }
    // Without Random the values apply in full whatever the seed.
    let r = bounds(json!({"scaleH": 50, "moveH": 40}), 3);
    assert!(close(r.width(), 50.0, 1e-9) && close(r.center().x, 90.0, 1e-9), "{r:?}");
}

/// #1705: stacked Transform effects multiply their copies; the anchors they make are bounded,
/// whatever each one's count.
#[test]
fn stacked_transform_copies_stay_within_the_anchor_budget() {
    let p = json!({"copies": 1000});
    let b = square().bounds().unwrap();
    let mut path = square();
    let mut counts = vec![];
    for _ in 0..3 {
        path = crate::distort::transform_within(&path, b, &p, 0, 100);
        counts.push(path.anchor_count());
    }
    assert_eq!(counts, [100, 100, 100], "25 squares, then no room for a copy of them");
    // Within the budget every copy is made, and the effect's own budget takes a thousand copies.
    assert_eq!(crate::distort::transform_within(&square(), b, &json!({"copies": 24}), 0, 100).subpaths.len(), 25);
    assert_eq!(run("distort.transform", p, &square()).subpaths.len(), 1001);
}

/// #2042: stacked Transform effects multiply their image copies as they do their anchors; the
/// image nodes they make are bounded, whatever each one's count.
#[test]
fn stacked_transform_copies_stay_within_the_image_budget() {
    use std::sync::Arc;

    use vectorcraft_doc::{ImageObject, NodeId};
    let id = NodeId(9);
    let image = || {
        Node::new(
            id,
            NodeKind::Image(Box::new(ImageObject {
                key: "k".into(),
                width: 40,
                height: 40,
                xf: Affine::translate((10.0, 0.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        )
    };
    let count = |art: &Node| {
        let mut xs = vec![];
        image_xs(art, &mut xs);
        xs.len()
    };
    let m = Affine::translate((70.0, 0.0));
    let mut art = Node::group(id, vec![Arc::new(image())]);
    let mut counts = vec![];
    for _ in 0..3 {
        crate::reshape::transform_placed_within(&mut art, m, 1000, 100);
        counts.push(count(&art));
    }
    assert_eq!(counts, [100, 100, 100], "99 copies, then no room for a copy of them");
    // Within the budget every copy is made, and the effect's own budget takes a thousand copies.
    let mut art = Node::group(id, vec![Arc::new(image())]);
    crate::reshape::transform_placed_within(&mut art, m, 24, 100);
    assert_eq!(count(&art), 25);
    let mut g = Node::group(id, vec![Arc::new(image())]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 70, "copies": 1000})));
    assert_eq!(count(&evaluate_container(None, &g).unwrap()), 1001);
}

/// What `effects` make of `path` within `budget`.
fn within(effects: &[Effect], path: &PathData, budget: Budget) -> PathData {
    apply_geometry_within(effects, path, path.bounds().unwrap(), &GeomContext::default(), budget)
}

/// What `effects` make of `path` within a budget of `max` anchors (and any number of lines).
fn anchors_within(effects: &[Effect], path: &PathData, max: usize) -> PathData {
    within(effects, path, Budget { anchors: max, lines: usize::MAX })
}

/// #3578: stacked Roughens multiply their points (up to 2000 a segment); the anchors they make
/// stay within the budget, each Roughen spacing its points further apart where it must.
#[test]
fn stacked_roughens_stay_within_the_anchor_budget() {
    let rough = fx("distort.roughen", json!({"size": 10000, "relative": false, "detail": 100}));
    let outs: Vec<PathData> = (1..=3).map(|n| anchors_within(&vec![rough.clone(); n], &square(), 10_000)).collect();
    let counts: Vec<usize> = outs.iter().map(PathData::anchor_count).collect();
    // 100 pt sides at 100 points per inch: 139 points each, within the budget.
    assert_eq!(counts[0], 4 * 139);
    assert_eq!(outs[0], run("distort.roughen", rough.params.clone(), &square()));
    assert!(counts[1..].iter().all(|&c| c > 5_000 && c <= 10_000), "{counts:?}");
    // The third Roughen still roughens, with its points spaced to fit.
    assert_ne!(outs[2], outs[1]);
    assert!(outs.iter().all(all_finite));
}

/// #3578: stacked Zig Zags multiply their points (up to 101 a segment); each segment gets as many
/// ridges as fit the budget.
#[test]
fn stacked_zig_zags_stay_within_the_anchor_budget() {
    let zig = fx("distort.zigZag", json!({"ridges": 100, "points": "corner"}));
    let counts: Vec<usize> = (1..=3).map(|n| anchors_within(&vec![zig.clone(); n], &square(), 10_000).anchor_count()).collect();
    // 4 × 101 points; then 23 ridges a segment fit (404 × 24); then none, each point displaced.
    assert_eq!(counts, [404, 9_696, 9_696]);
    let twice = anchors_within(&vec![zig.clone(); 2], &square(), 10_000);
    assert_ne!(anchors_within(&vec![zig.clone(); 3], &square(), 10_000), twice);
    // An open path's end point counts as well.
    let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false));
    assert_eq!(anchors_within(&[zig], &line, 30).anchor_count(), 30, "the start, 28 ridges and the end");
}

/// #3578: every effect that multiplies anchors builds within the budget, stacked or not, and one
/// that doesn't (Round Corners doubles them) is left out where it would pass it.
#[test]
fn stacked_geometry_effects_stay_within_the_anchor_budget() {
    let star = shapes::star(Point::new(150.0, 100.0), 70.0, 32.0, 5, 0.0);
    for (id, params) in [
        ("distort.roughen", json!({"size": 50, "detail": 100})),
        ("distort.zigZag", json!({"ridges": 100, "size": 30})),
        ("distort.twist", json!({"angle": 3600})),
        ("distort.freeDistort", json!({"corners": [[0.3, -2], [1, 0], [4, 1], [0, 3]]})),
        ("warp.twist", json!({"bend": 100})),
        ("distort.transform", json!({"copies": 1000, "moveH": 1})),
    ] {
        for n in 1..=4 {
            let out = anchors_within(&vec![fx(id, params.clone()); n], &star, 2_000);
            assert!(out.anchor_count() <= 2_000 && all_finite(&out), "{id} × {n}: {}", out.anchor_count());
        }
    }
    let zig = fx("distort.zigZag", json!({"ridges": 100, "points": "corner"}));
    let full = anchors_within(&[zig.clone(), zig.clone()], &square(), 10_000);
    let round = fx("stylize.roundCorners", json!({"radius": 1}));
    assert_eq!(anchors_within(&[zig.clone(), zig.clone(), round.clone()], &square(), 10_000), full);
    assert_ne!(anchors_within(&[zig, round], &square(), 10_000), run("distort.zigZag", json!({"ridges": 100, "points": "corner"}), &square()));
}

/// #3578: a path with more anchors than the budget keeps its count as its budget: effects that
/// add none apply, and the others add none.
#[test]
fn a_path_past_the_anchor_budget_still_takes_effects() {
    let star = shapes::star(Point::new(150.0, 100.0), 70.0, 32.0, 25, 0.0);
    let n = star.anchor_count();
    for (id, params) in [
        ("distort.tweak", json!({})),
        ("distort.roughen", json!({"detail": 100})),
        ("distort.zigZag", json!({"ridges": 100})),
        ("distort.twist", json!({"angle": 90})),
    ] {
        let out = anchors_within(&[fx(id, params)], &star, 10);
        assert_eq!(out.anchor_count(), n, "{id}");
        assert_ne!(out, star, "{id}");
    }
}

/// #3578: the effects on type, symbols and images reshape all their paths within one budget.
#[test]
fn stacked_effects_on_art_stay_within_the_anchor_budget() {
    use std::sync::Arc;

    use vectorcraft_doc::NodeId;
    let id = NodeId(9);
    let leaf = |x: f64| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0)), Appearance::default()));
    let mut art = Node::group(id, (0..10).map(|k| leaf(k as f64 * 120.0)).collect());
    let rough = fx("distort.roughen", json!({"size": 50, "detail": 100}));
    crate::reshape::reshape_leaves_within(&mut art, &[&rough, &rough, &rough], Budget { anchors: 5_000, lines: usize::MAX });
    let mut total = 0;
    let mut reshaped = 0;
    for c in art.children().into_iter().flatten() {
        let path = c.path_data().unwrap();
        total += path.anchor_count();
        reshaped += usize::from(path.anchor_count() > 4);
    }
    // One Roughen alone makes 10 × 556: the budget binds, and goes to the paths in turn.
    assert!(total > 4_000 && total <= 5_000, "{total}");
    assert_eq!(reshaped, 10);
}

/// #3578: a large Roughen draws each of its points as many lines; they stay within the budget,
/// the Roughen built again with fewer points where they wouldn't.
#[test]
fn large_roughens_stay_within_the_line_budget() {
    let rough = fx("distort.roughen", json!({"size": 10000, "relative": false, "detail": 100}));
    let budget = Budget { anchors: 100_000, lines: 200_000 };
    let once = within(std::slice::from_ref(&rough), &square(), budget);
    assert_eq!(once, run("distort.roughen", rough.params.clone(), &square()), "within both budgets");
    let twice = within(&[rough.clone(), rough], &square(), budget);
    let cost = Budget::of(&twice);
    assert!(budget.fits(cost) && cost.lines > 100_000, "{cost:?}");
    assert!(cost.anchors < 10_000 && twice != once, "the second Roughen with fewer points: {cost:?}");
    // Art of small curves draws its anchors as a few lines each: the anchors bind first.
    let zig = fx("distort.zigZag", json!({"ridges": 100, "size": 2}));
    let out = within(&[zig.clone(), zig], &square(), Budget { anchors: 10_000, lines: 100_000 });
    assert!(out.anchor_count() > 9_000 && util::line_estimate(&out) < 50_000, "{}", util::line_estimate(&out));
}

/// Straight segments are one line each, and a curve as many as Wang's formula takes at 0.25 px.
#[test]
fn line_estimate_counts_the_lines_a_path_is_drawn_with() {
    assert_eq!(util::line_estimate(&square()), 4);
    let circle = shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_eq!(util::line_estimate(&circle), 4 * 9, "a quarter circle of 50 pt in 9 lines");
    let big = shapes::ellipse(Rect::new(0.0, 0.0, 10_000.0, 10_000.0));
    assert_eq!(util::line_estimate(&big), 4 * 84, "100 times as large: about 10 times the lines");
}

/// #3578: the issue's input, a large rectangle with three max-detail Roughens of the largest
/// size, at the real budgets: two million anchors of it were 141 million lines to draw.
#[test]
fn three_max_detail_roughens_stay_within_the_effect_budget() {
    let rough = fx("distort.roughen", json!({"size": 10000, "relative": false, "detail": 100}));
    let rect = shapes::rectangle(Rect::new(0.0, 0.0, 1000.0, 1000.0));
    let out = apply_geometry(&vec![rough.clone(); 3], &rect, rect.bounds().unwrap());
    let cost = Budget::of(&out);
    assert!(Budget::EFFECTS.fits(cost) && cost.lines > MAX_EFFECT_LINES / 2, "{cost:?}");
    assert_ne!(out, apply_geometry(&[rough], &rect, rect.bounds().unwrap()), "more than one Roughen applies");
}

#[test]
fn transform_copies_before_offset_path_evaluates_without_ram_spike() {
    let effects = [
        fx("distort.transform", json!({"moveH": -0.66, "moveV": 0.66, "copies": 30})),
        fx("path.offsetPath", json!({"offset": 10.0, "joins": "round"})),
    ];
    let path = shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0));
    let start = std::time::Instant::now();
    let out = apply_geometry(&effects, &path, path.bounds().unwrap());
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(2), "elapsed: {:?}", elapsed);
    assert_eq!(out.subpaths.len(), 31);
}

#[test]
fn reshape_leaves_within_optimizes_transform_before_terminal_offset_path() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeId};

    let id = NodeId(10);
    let leaf = |x: f64| Arc::new(Node::path(id, shapes::rectangle(Rect::new(x, 0.0, x + 20.0, 20.0)), Appearance::default()));
    let mut art = Node::group(id, (0..5).map(|k| leaf(k as f64 * 30.0)).collect());
    let transform = fx("distort.transform", json!({"moveH": 2.0, "moveV": 2.0, "copies": 10}));
    let offset = fx("path.offsetPath", json!({"offset": 4.0, "joins": "round"}));

    let start = std::time::Instant::now();
    crate::reshape::reshape_leaves_within(&mut art, &[&transform, &offset], Budget::EFFECTS);
    let elapsed = start.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(2));

    let children = art.children().unwrap();
    assert_eq!(children.len(), 5);
    for c in children {
        let (p, _) = crate::reshape::leaf_geometry(c).unwrap();
        // Base rectangle offset once, then copied 10 times -> 11 subpaths
        assert_eq!(p.subpaths.len(), 11);
    }
}

#[test]
fn reshape_leaves_within_preserves_order_when_not_terminal_or_not_style_safe() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};

    let id = NodeId(11);
    // 1. EvenOdd fill rule: preserving order causes Transform copies to union in Offset Path into 1 subpath.
    let mut leaf = Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default());
    if let NodeKind::Path { rule, .. } = &mut leaf.kind {
        *rule = FillRule::EvenOdd;
    }
    let mut art_evenodd = Node::group(id, vec![Arc::new(leaf)]);
    let transform = fx("distort.transform", json!({"moveH": 2.0, "moveV": 2.0, "copies": 2}));
    let offset = fx("path.offsetPath", json!({"offset": 4.0}));
    crate::reshape::reshape_leaves_within(&mut art_evenodd, &[&transform, &offset], Budget::EFFECTS);
    let p = art_evenodd.children().unwrap()[0].path_data().unwrap();
    assert_eq!(p.subpaths.len(), 1, "EvenOdd does not swap; copies union into 1 outer subpath");

    // 2. Non-terminal Offset Path: followed by Round Corners, must preserve order.
    let leaf_round = Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default()));
    let mut art_round = Node::group(id, vec![leaf_round]);
    let round_corners = fx("stylize.roundCorners", json!({"radius": 2.0}));
    crate::reshape::reshape_leaves_within(&mut art_round, &[&transform, &offset, &round_corners], Budget::EFFECTS);
    let p = art_round.children().unwrap()[0].path_data().unwrap();
    assert_eq!(p.subpaths.len(), 1, "Non-terminal Offset Path does not swap; copies union into 1 outer subpath");

    // 3. Non-round join (e.g. miter): preserving order is required as corners differ (#4145).
    let leaf_miter = Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default()));
    let mut art_miter = Node::group(id, vec![leaf_miter]);
    let offset_miter = fx("path.offsetPath", json!({"offset": 4.0, "joins": "miter"}));
    crate::reshape::reshape_leaves_within(&mut art_miter, &[&transform, &offset_miter], Budget::EFFECTS);
    let p = art_miter.children().unwrap()[0].path_data().unwrap();
    assert_eq!(p.subpaths.len(), 1, "Miter join does not swap; copies union into 1 outer subpath");
}

#[test]
fn consecutive_transforms_before_terminal_offset_path_all_move_after() {
    let t1 = fx("distort.transform", json!({"moveH": 1.0, "moveV": 0.0, "copies": 2}));
    let t2 = fx("distort.transform", json!({"moveH": 0.0, "moveV": 1.0, "copies": 3}));
    let offset = fx("path.offsetPath", json!({"offset": 4.0, "joins": "round"}));

    let opt = crate::geometry::optimize_geometry_effects(&[&t1, &t2, &offset], true);
    assert_eq!(opt.len(), 3);
    assert_eq!(opt[0].id, "path.offsetPath");
    assert_eq!(opt[1].id, "distort.transform");
    assert_eq!(opt[2].id, "distort.transform");
}

#[test]
fn transform_copies_stop_before_they_overflow() {
    let out = run("distort.transform", json!({"scaleH": 100000, "scaleV": 100000, "copies": 1000}), &square());
    assert!(all_finite(&out) && out.subpaths.len() < 10, "{}", out.subpaths.len());
}

/// #2023: a Transform effect on a group moves, scales and copies its images like its paths: the
/// map goes into each image's placement, and every copy is an image node of its own.
#[test]
fn transform_on_a_group_moves_and_copies_its_images() {
    use std::sync::Arc;

    use vectorcraft_doc::{Appearance, ImageObject, Node, NodeId, NodeKind};
    let id = NodeId(9);
    let sq = Node::path(id, square(), Appearance::default());
    let image = Node::new(
        id,
        NodeKind::Image(Box::new(ImageObject {
            key: "k".into(),
            width: 40,
            height: 40,
            xf: Affine::translate((10.0, 0.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    let mut g = Node::group(id, vec![Arc::new(sq), Arc::new(image)]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 70, "copies": 2})));
    let art = evaluate_container(None, &g).unwrap();
    let mut xs = vec![];
    image_xs(&art, &mut xs);
    xs.sort_by(f64::total_cmp);
    assert_eq!(xs, [10.0, 80.0, 150.0], "the image and its two copies, 70 pt apart");
    // Without copies the image itself takes the transform, as a path does.
    let mut g = Node::group(
        id,
        vec![Arc::new(Node::new(
            id,
            NodeKind::Image(Box::new(ImageObject {
                key: "k".into(),
                width: 40,
                height: 40,
                xf: Affine::translate((10.0, 0.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            })),
        ))],
    );
    g.appearance.effects.push(fx("distort.transform", json!({"scaleH": 50, "scaleV": 50, "reference": 8})));
    let art = evaluate_container(None, &g).unwrap();
    let mut xs = vec![];
    image_xs(&art, &mut xs);
    // The bounds are the image itself, so the bottom-right reference is its own: it halves in place.
    assert_eq!(xs, [30.0], "scaled about its own bottom-right corner");
}

/// #2023: a Transform effect on a group moves and copies its symbol instances as it does its paths
/// and images: the map goes into each instance's placement, and every copy is an instance of its
/// own.
#[test]
fn transform_on_a_group_moves_and_copies_its_symbol_instances() {
    use std::sync::Arc;

    use vectorcraft_doc::{Appearance, Node, NodeId, NodeKind};
    let id = NodeId(9);
    let instance = || Node::new(id, NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::translate((5.0, 40.0)) });
    let xs = |art: &Node| {
        let mut xs = vec![];
        instance_xs(art, &mut xs);
        xs.sort_by(f64::total_cmp);
        xs
    };
    let mut g = Node::group(id, vec![Arc::new(Node::path(id, square(), Appearance::default())), Arc::new(instance())]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 70, "copies": 2})));
    assert_eq!(xs(&evaluate_container(None, &g).unwrap()), [5.0, 75.0, 145.0], "the instance and its two copies, 70 pt apart");
    // Without copies the instance itself takes the transform, as a path does.
    let mut g = Node::group(id, vec![Arc::new(instance())]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 70})));
    assert_eq!(xs(&evaluate_container(None, &g).unwrap()), [75.0]);
}

/// Images and symbol instances share the copy budget: copies of one leave less room for the other.
#[test]
fn images_and_symbol_instances_share_the_copy_budget() {
    use std::sync::Arc;

    use vectorcraft_doc::{ImageObject, NodeId};
    let id = NodeId(9);
    let image = Node::new(
        id,
        NodeKind::Image(Box::new(ImageObject {
            key: "k".into(),
            width: 40,
            height: 40,
            xf: Affine::translate((10.0, 0.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    let instance = Node::new(id, NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
    let mut art = Node::group(id, vec![Arc::new(image), Arc::new(instance)]);
    crate::reshape::transform_placed_within(&mut art, Affine::translate((70.0, 0.0)), 1000, 100);
    let (mut images, mut instances) = (vec![], vec![]);
    image_xs(&art, &mut images);
    instance_xs(&art, &mut instances);
    // The image takes the 98 copies there is room for; none is left for the instance.
    assert_eq!((images.len(), instances.len()), (99, 1));
}

/// As a path's do, a Transform effect's copies of an image stop before one reaches further than
/// `MAX_REACH` from the origin.
#[test]
fn transform_copies_of_images_stop_at_the_reach_limit() {
    use std::sync::Arc;

    use vectorcraft_doc::{ImageObject, NodeId};
    let id = NodeId(9);
    let image = Node::new(
        id,
        NodeKind::Image(Box::new(ImageObject {
            key: "k".into(),
            width: 40,
            height: 40,
            xf: Affine::translate((10.0, 0.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    let mut g = Node::group(id, vec![Arc::new(image)]);
    // The largest move: every copy is 1e7 pt further right, so the 100th would end past 1e9.
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 1.0e7, "copies": 1000})));
    let mut xs = vec![];
    image_xs(&evaluate_container(None, &g).unwrap(), &mut xs);
    assert_eq!(xs.len(), 100, "the image and the 99 copies that stay within reach");
    assert!(xs.iter().all(|x| x + 40.0 <= 1.0e9));
}

#[test]
fn pucker_bloat_moves_anchors_in_opposite_directions() {
    let bloat = run("distort.puckerBloat", json!({"amount": 50}), &square());
    let a = bloat.subpaths[0].anchors[0].p;
    assert!(a.x > 0.0 && a.y > 0.0, "bloat pulls corners in: {a:?}");
    let pucker = run("distort.puckerBloat", json!({"amount": -50}), &square());
    let a = pucker.subpaths[0].anchors[0].p;
    assert!(a.x < 0.0 && a.y < 0.0, "pucker pushes corners out: {a:?}");
    assert_eq!(run("distort.puckerBloat", json!({"amount": 0}), &square()), square());
}

#[test]
fn roughen_seed_changes_result_and_detail_adds_points() {
    let a = run("distort.roughen", json!({"seed": 1}), &square());
    let b = run("distort.roughen", json!({"seed": 2}), &square());
    assert_ne!(a, b);
    let lo = run("distort.roughen", json!({"detail": 1}), &square());
    let hi = run("distort.roughen", json!({"detail": 50}), &square());
    assert!(hi.anchor_count() > lo.anchor_count());
    // Displacement is bounded by size (5% of 100 = 5 pt).
    let bb = a.bounds().unwrap();
    assert!(bb.x0 > -8.0 && bb.x1 < 108.0);
}

#[test]
fn catmull_rom_keeps_a_turn_back_smooth() {
    // #1643: the path runs out to (10, 0) and turns back to (0.5, 0.2). The Catmull-Rom tangent
    // `next - prev` is almost zero there; the anchor gets handles along the chords' bisector
    // (here nearly perpendicular to them), collinear and as long as half the longer chord / 6.
    let pts = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(0.5, 0.2), Point::new(10.0, 5.0)];
    let sp = util::catmull_rom(&pts, false, 1.0);
    let a = sp.anchors[1];
    let (hin, hout) = (a.h_in - a.p, a.h_out - a.p);
    assert!(close(hin.x, -hout.x, 1e-9) && close(hin.y, -hout.y, 1e-9), "collinear: {a:?}");
    assert!(close(hout.hypot(), 0.5 * 10.0 / 6.0, 1e-3), "{a:?}");
    assert!(hout.x.abs() < 0.05 * hout.hypot(), "across the chords: {a:?}");
    // A regular turn keeps the plain Catmull-Rom tangent.
    let pts = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0)];
    let a = util::catmull_rom(&pts, false, 1.0).anchors[1];
    let (got, want) = (a.h_out - a.p, (pts[2] - pts[0]) / 6.0);
    assert!(close(got.x, want.x, 1e-12) && close(got.y, want.y, 1e-12), "{got:?} vs {want:?}");
}

#[test]
fn roughen_smooth_makes_no_cusps() {
    // #1643: wherever Roughen's jitter turns the path back, the smooth points stay smooth: no
    // anchor's handles shrink below 1/12 of its longer chord (a cusp that PDF viewers miter).
    let star = shapes::star(Point::new(150.0, 100.0), 70.0, 32.0, 5, 0.0);
    for seed in 0..50 {
        let out = run("distort.roughen", json!({"size": 3, "detail": 10, "seed": seed}), &star);
        for sp in &out.subpaths {
            let n = sp.anchors.len();
            for i in 0..n {
                let a = sp.anchors[i];
                let (prev, next) = (sp.anchors[(i + n - 1) % n].p, sp.anchors[(i + 1) % n].p);
                let chord = (a.p - prev).hypot().max((next - a.p).hypot());
                assert!((a.h_out - a.p).hypot() >= chord / 12.0 - 1e-9, "seed {seed}: cusp at {:?}", a.p);
            }
        }
    }
}

#[test]
fn zigzag_ridge_count_and_amplitude() {
    let out = run("distort.zigZag", json!({"ridges": 4, "points": "corner", "size": 10}), &square());
    assert_eq!(out.anchor_count(), 4 * 5);
    let b = out.bounds().unwrap();
    assert!(b.width() > 100.0 && b.width() <= 120.0 + 1e-6, "{b:?}");
}

#[test]
fn zigzag_spaces_ridges_evenly_along_a_straight_line() {
    // #1554: on a straight line every ridge is the same distance from the line and as far from
    // the next one, whether the points are corners or smooth.
    let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.0, 500.0)], false));
    for points in ["corner", "smooth"] {
        let out = run("distort.zigZag", json!({"ridges": 4, "points": points, "size": 20}), &line);
        let ps: Vec<Point> = out.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(ps.len(), 6, "{points}");
        for (k, q) in ps.iter().enumerate() {
            assert!(close(q.x.abs(), 20.0, 1e-6), "{points} {k}: {q:?}");
            assert!(close(q.y, 100.0 * k as f64, 1e-3), "{points} {k}: {q:?}");
        }
    }
}

/// #2343: Smooth points make a continuous wave, not a zigzag with rounded tips. Every peak and
/// valley is a smooth point whose handles run parallel to the line, and the curve between them
/// follows a cosine, even where the ridges are much taller than they are apart.
#[test]
fn zigzag_smooth_is_a_wave() {
    use vectorcraft_geom::kurbo::ParamCurve;
    let line = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(100.0, 0.0)], false));
    for size in [3.0, 30.0] {
        let out = run("distort.zigZag", json!({"ridges": 9, "points": "smooth", "size": size}), &line);
        let sp = &out.subpaths[0];
        assert_eq!(sp.anchors.len(), 11);
        for a in &sp.anchors {
            assert!(close(a.h_in.y, a.p.y, 1e-9) && close(a.h_out.y, a.p.y, 1e-9), "size {size}: handles not level at {a:?}");
        }
        for i in 0..sp.segment_count() {
            let c = sp.segment(i);
            for k in 0..=20 {
                let q = c.eval(k as f64 / 20.0);
                let want = -size * (std::f64::consts::PI * q.x / 10.0).cos();
                assert!((q.y - want).abs() < 0.03 * size, "size {size}: {q:?} off the wave ({want})");
            }
        }
    }
}

/// #1554 on curves: a circle's ridges sit at equal angles, at the radius plus and minus the size in
/// turn all the way round.
#[test]
fn zigzag_on_a_circle_is_regular() {
    let circle = shapes::ellipse(Rect::new(-50.0, -50.0, 50.0, 50.0));
    let out = run("distort.zigZag", json!({"ridges": 3, "points": "corner", "size": 10}), &circle);
    let pts: Vec<Point> = out.subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts.len(), 16);
    let start = pts[0].y.atan2(pts[0].x);
    let mut radii = vec![];
    for (i, q) in pts.iter().enumerate() {
        let turn = (q.y.atan2(q.x) - start).to_degrees().abs() / 22.5;
        assert!(close(turn, turn.round(), 0.02), "point {i} at {turn} sixteenths");
        radii.push(q.to_vec2().hypot().round());
    }
    assert!(radii.iter().all(|r| *r == 60.0 || *r == 40.0) && (0..16).all(|i| radii[i] != radii[(i + 1) % 16]), "{radii:?}");
}

/// #1554 on an uneven curve: the ridges divide its length, not its parameter, into equal parts.
#[test]
fn zigzag_spaces_ridges_by_length_on_an_uneven_curve() {
    use vectorcraft_geom::kurbo::{ParamCurve, ParamCurveArclen, ParamCurveNearest};
    let (a, b) = (Point::new(0.0, 0.0), Point::new(300.0, 0.0));
    let curve = PathData::single(SubPath::new(
        vec![
            vectorcraft_geom::Anchor::with_handles(a, a, Point::new(10.0, 40.0)),
            vectorcraft_geom::Anchor::with_handles(b, Point::new(100.0, 200.0), b),
        ],
        false,
    ));
    let c = curve.subpaths[0].segment(0);
    let len = c.arclen(1e-6);
    let out = run("distort.zigZag", json!({"ridges": 3, "points": "corner", "size": 0}), &curve);
    let along: Vec<f64> = out.subpaths[0].anchors.iter().map(|q| c.subsegment(0.0..c.nearest(q.p, 1e-9).t).arclen(1e-6) / len).collect();
    assert_eq!(along.len(), 5);
    for (k, share) in along.iter().enumerate() {
        assert!(close(*share, k as f64 / 4.0, 1e-3), "{along:?}");
    }
}

#[test]
fn offset_path_grows_and_shrinks() {
    let grow = run("path.offsetPath", json!({"offset": 10, "joins": "miter"}), &square()).bounds().unwrap();
    assert!(close(grow.width(), 120.0, 0.1), "{grow:?}");
    let shrink = run("path.offsetPath", json!({"offset": -10}), &square()).bounds().unwrap();
    assert!(close(shrink.width(), 80.0, 0.1), "{shrink:?}");
}

#[test]
fn outline_stroke_uses_context_stroke() {
    let st = vectorcraft_doc::StrokeLayer::new(vectorcraft_color::Paint::solid(vectorcraft_color::Color::BLACK), 10.0);
    let ctx = GeomContext { stroke: Some(&st), ..Default::default() };
    let out = apply_geometry_with(&[fx("path.outlineStroke", json!({}))], &square(), Rect::new(0.0, 0.0, 100.0, 100.0), &ctx);
    let b = out.bounds().unwrap();
    assert!(close(b.width(), 110.0, 0.1), "{b:?}");
    // An explicit width overrides the weight; without a stroke a plain 1 pt one is outlined.
    let out = apply_geometry_with(&[fx("path.outlineStroke", json!({"width": 4}))], &square(), Rect::new(0.0, 0.0, 100.0, 100.0), &ctx);
    assert!(close(out.bounds().unwrap().width(), 104.0, 0.1));
    let out = run("path.outlineStroke", json!({}), &square());
    assert!(close(out.bounds().unwrap().width(), 101.0, 0.1));
}

#[test]
fn outline_object_is_a_geometry_effect_that_keeps_the_path() {
    let info = effect_info("path.outlineObject").unwrap();
    assert_eq!((info.label, info.menu, &info.defaults, info.raster), ("Outline Object", &["Effect", "Path"][..], &json!({}), false));
    assert!(is_geometry("path.outlineObject"));
    assert_eq!(run("path.outlineObject", json!({}), &square()), square());
    // The effects after it reshape the same geometry.
    let b = square().bounds().unwrap();
    let offset = fx("path.offsetPath", json!({"offset": 5}));
    assert_eq!(apply_geometry(&[fx("path.outlineObject", json!({})), offset.clone()], &square(), b), apply_geometry(&[offset], &square(), b));
}

#[test]
fn convert_to_shape_sizes() {
    let r = run("convertToShape.rectangle", json!({"extraW": 20, "extraH": 10}), &square()).bounds().unwrap();
    assert!(close(r.width(), 120.0, 1e-9) && close(r.height(), 110.0, 1e-9));
    let e = run("convertToShape.ellipse", json!({"relative": false, "width": 40, "height": 20}), &square());
    let b = e.bounds().unwrap();
    assert!(close(b.width(), 40.0, 1e-6) && close(b.center().x, 50.0, 1e-9));
    assert_eq!(e.anchor_count(), 4);
}

#[test]
fn round_corners_replaces_corners() {
    let out = run("stylize.roundCorners", json!({"radius": 10}), &square());
    assert_eq!(out.anchor_count(), 8);
    // Area shrinks by 4 × (r² − πr²/4) ≈ 85.8 for r = 10.
    let area = out.to_bezpath().area().abs();
    assert!(close(area, 10000.0 - 85.84, 2.0), "{area}");
    // An ellipse has no sharp corners → unchanged.
    let el = shapes::ellipse(Rect::new(0.0, 0.0, 50.0, 30.0));
    assert_eq!(run("stylize.roundCorners", json!({}), &el), el);
}

/// A D: a straight back, a curved bowl, and two corners where the bowl meets the straight
/// sides (the anchors there carry the bowl's handles).
fn d_shape() -> PathData {
    use vectorcraft_geom::Anchor;
    let p = Point::new;
    PathData::new(vec![SubPath::new(
        vec![
            Anchor::corner(p(0.0, 0.0)),
            Anchor::with_handles(p(50.0, 0.0), p(50.0, 0.0), p(80.0, 20.0)),
            Anchor::with_handles(p(50.0, 100.0), p(80.0, 80.0), p(50.0, 100.0)),
            Anchor::corner(p(0.0, 100.0)),
        ],
        true,
    )])
}

#[test]
fn round_corners_also_rounds_where_a_curve_meets_a_line() {
    let d = d_shape();
    let out = run("stylize.roundCorners", json!({"radius": 8}), &d);
    // All four corners are cut, the two curve-line ones too: two anchors each.
    assert_eq!(out.anchor_count(), 8);
    assert!(all_finite(&out));
    // Every cut leaves both sides smoothly: no corner is left anywhere.
    assert!(vectorcraft_geom::corners::path_corners(&out).is_empty(), "{out:?}");
    let (before, after) = (d.to_bezpath().area().abs(), out.to_bezpath().area().abs());
    assert!(after < before && after > before - 4.0 * 8.0 * 8.0, "{before} -> {after}");
    // The cut at a curve-line corner meets the straight side 8 from the corner.
    let p = Point::new;
    assert!(out.subpaths[0].anchors.iter().any(|a| a.p.distance(p(42.0, 0.0)) < 1e-9), "{out:?}");
}

#[test]
fn round_corners_keeps_straight_corners_as_they_were() {
    // The D's straight corners are rounded as before, next to the new cuts: same anchors, same
    // quarter-circle handles, and the path still starts at its first corner's rounding.
    use vectorcraft_geom::Anchor;
    let p = Point::new;
    let k = 8.0 * shapes::KAPPA;
    let out = stylize::round_corners(&d_shape(), 8.0);
    let a = &out.subpaths[0].anchors;
    assert_eq!(a[0], Anchor::with_handles(p(0.0, 8.0), p(0.0, 8.0), p(0.0, 8.0 - k)));
    assert_eq!(a[1], Anchor::with_handles(p(8.0, 0.0), p(8.0 - k, 0.0), p(8.0, 0.0)));
    // A path of straight corners only is rounded exactly as before.
    assert_eq!(stylize::round_corners(&square(), 10.0).subpaths[0].anchors.len(), 8);
}

#[test]
fn round_corners_keeps_where_a_closed_path_starts() {
    // The D starting at a curve-line corner: the result starts where that corner's cut begins,
    // on the side coming from the previous corner, as a straight corner's rounding does.
    let mut d = d_shape();
    d.subpaths[0].anchors.rotate_left(1);
    let out = stylize::round_corners(&d, 8.0);
    let start = out.subpaths[0].anchors[0].p;
    assert!(start.distance(Point::new(42.0, 0.0)) < 1e-9, "{start:?}");
    // An infinite radius rounds as much as fits, curve-line corners too.
    assert_eq!(stylize::round_corners(&d, f64::INFINITY), stylize::round_corners(&d, 1e9));
}

#[test]
fn twist_fixes_the_outer_circle_and_rotates_inside() {
    let out = run("distort.twist", json!({"angle": 0}), &square());
    assert_eq!(out, square());
    let small = PathData::single(SubPath::polyline(&[Point::new(45.0, 50.0), Point::new(55.0, 50.0)], false));
    let out = apply_geometry(&[fx("distort.twist", json!({"angle": 90}))], &small, Rect::new(0.0, 0.0, 100.0, 100.0));
    let a = out.subpaths[0].anchors[0].p;
    assert!((a.y - 50.0).abs() > 1.0, "near-centre points rotate: {a:?}");
}

#[test]
fn scribble_does_not_cross_holes() {
    // #1648: a U (opening at the top, the notch x 30..70, y 0..60). Rows (y 2.5, 7.5, ...) through the prongs have
    // two spans; their zig-zags must stay apart instead of crossing the notch.
    let u = [(0.0, 0.0), (30.0, 0.0), (30.0, 60.0), (70.0, 60.0), (70.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)];
    let u = PathData::single(SubPath::polyline(&u.map(|(x, y)| Point::new(x, y)), true));
    let params = json!({"angle": 0, "variation": 0, "curviness": 0, "strokeWidth": 2, "spacing": 5});
    let out = run("stylize.scribble", params, &u).to_bezpath();
    for x in [40.0, 50.0, 60.0] {
        for y in [12.5, 32.5, 52.5] {
            assert_eq!(vectorcraft_geom::kurbo::Shape::winding(&out, Point::new(x, y)), 0, "scribble covers the notch at ({x}, {y})");
        }
    }
    // The bar below the notch is still hatched.
    assert_ne!(vectorcraft_geom::kurbo::Shape::winding(&out, Point::new(50.0, 82.5)), 0);
}

#[test]
fn scribble_produces_filled_outline_inside_bounds() {
    let out = run("stylize.scribble", json!({}), &square());
    assert!(!out.is_empty());
    let b = out.bounds().unwrap();
    assert!(b.x0 > -10.0 && b.x1 < 110.0 && b.y0 > -10.0 && b.y1 < 110.0, "{b:?}");
    assert_eq!(out, run("stylize.scribble", json!({}), &square()));
}

#[test]
fn scribble_variations_are_separate() {
    // At 0° the lines run along x: the Path Overlap variation moves their ends in x only, the
    // Spacing variation moves them in y only, and the Curviness variation curves some of the turns.
    let base =
        json!({"angle": 0, "curviness": 0, "strokeWidth": 0.5, "spacing": 10, "overlapVariation": 0, "spacingVariation": 0, "curvinessVariation": 0});
    let with = |key: &str, v: f64| {
        let mut p = base.clone();
        p[key] = json!(v);
        run("stylize.scribble", p, &square())
    };
    let plain = run("stylize.scribble", base.clone(), &square());
    let b = plain.bounds().unwrap();
    let moved = |a: f64, b: f64| (a - b).abs() > 0.5;
    let along = with("overlapVariation", 4.0).bounds().unwrap();
    assert!(close(along.y0, b.y0, 0.06) && close(along.y1, b.y1, 0.06), "{along:?} {b:?}");
    assert!(moved(along.x0, b.x0) || moved(along.x1, b.x1), "{along:?} {b:?}");
    let across = with("spacingVariation", 4.0).bounds().unwrap();
    assert!(close(across.x0, b.x0, 0.06) && close(across.x1, b.x1, 0.06), "{across:?} {b:?}");
    assert!(moved(across.y0, b.y0) || moved(across.y1, b.y1), "{across:?} {b:?}");
    assert_ne!(with("curvinessVariation", 50.0), plain);
    // On a curviness of its own, the variation changes how much each turn curves.
    let mut p = base.clone();
    p["curviness"] = json!(20);
    let even = run("stylize.scribble", p.clone(), &square());
    p["curvinessVariation"] = json!(20);
    assert_ne!(run("stylize.scribble", p, &square()), even);
}

#[test]
fn scribble_reads_its_earlier_single_variation() {
    // Before the separate variations, one `variation` moved the ends both along and across.
    let old = run("stylize.scribble", json!({"variation": 3}), &square());
    assert_eq!(old, run("stylize.scribble", json!({"overlapVariation": 3, "spacingVariation": 3}), &square()));
    assert_eq!(upgrade_params("stylize.scribble", &json!({"variation": 3})), json!({"overlapVariation": 3, "spacingVariation": 3}));
    // A current key given with it wins.
    assert_eq!(
        upgrade_params("stylize.scribble", &json!({"variation": 3, "spacingVariation": 1})),
        json!({"overlapVariation": 3, "spacingVariation": 1})
    );
    assert!(default_params("stylize.scribble").unwrap().get("variation").is_none());
}

#[test]
fn scribble_settings_presets_are_distinct_and_recognized() {
    let presets = crate::scribble_settings();
    assert_eq!(presets.len(), 11);
    assert_eq!(presets[0].id, "default");
    assert_eq!(crate::scribble_setting_id(&presets[0].params), Some("default"));
    assert_eq!(crate::scribble_setting_id(&presets[9].params), Some("tight"));
    let custom = json!({"angle": 30.0, "overlap": 0.0, "overlapVariation": 0.5, "strokeWidth": 3.0, "curviness": 5.0, "curvinessVariation": 0.0, "spacing": 4.0, "spacingVariation": 0.5});
    assert_eq!(crate::scribble_setting_id(&custom), None);
}

#[test]
fn scribble_angle_is_stored_above_minus_180_and_up_to_180() {
    for (given, stored) in [(270.0, -90.0), (-180.0, 180.0), (180.0, 180.0), (540.0, 180.0), (-270.0, 90.0), (-200.0, 160.0)] {
        assert_eq!(new_effect("stylize.scribble", &json!({"angle": given})).unwrap().params["angle"].as_f64(), Some(stored), "{given}");
    }
    // An angle in range is kept as given; other effects' angles are not changed.
    assert_eq!(upgrade_params("stylize.scribble", &json!({"angle": 45})), json!({"angle": 45}));
    assert_eq!(upgrade_params("distort.twist", &json!({"angle": 270})), json!({"angle": 270}));
}

#[test]
fn effects_chain_in_order_and_skip_hidden_and_raster() {
    let chain = [fx("distort.transform", json!({"moveH": 10})), fx("distort.transform", json!({"scaleH": 200}))];
    let b = apply_geometry(&chain, &square(), Rect::new(0.0, 0.0, 100.0, 100.0)).bounds().unwrap();
    // Move first (10..110), then scale around the new centre (60): -40..160.
    assert!(close(b.x0, -40.0, 1e-9) && close(b.x1, 160.0, 1e-9), "{b:?}");
    let mut hidden = fx("distort.twist", json!({"angle": 90}));
    hidden.visible = false;
    let raster = fx("stylize.dropShadow", json!({}));
    assert_eq!(apply_geometry(&[hidden, raster.clone()], &square(), Rect::new(0.0, 0.0, 100.0, 100.0)), square());
    assert!(!has_geometry(&[raster]));
}

#[test]
fn raster_effects_parse_and_outset() {
    let list = [
        fx("stylize.dropShadow", json!({"x": 10, "y": -4, "blur": 6, "color": "#ff0000", "opacity": 50})),
        fx("stylize.outerGlow", json!({})),
        fx("stylize.innerGlow", json!({"source": "center"})),
        fx("stylize.feather", json!({})),
        fx("blur.gaussian", json!({"radius": 2})),
        fx("distort.twist", json!({})),
    ];
    let r = raster_effects(&list);
    assert_eq!(r.len(), 5);
    match &r[0] {
        RasterFx::DropShadow { mode, opacity, dx, dy, blur, color, darkness } => {
            assert_eq!(*mode, vectorcraft_doc::color::BlendMode::Multiply);
            assert!(close(*opacity as f64, 0.5, 1e-6));
            assert_eq!((*dx, *dy, *blur), (10.0, -4.0, 6.0));
            assert_eq!(color.to_rgb(), [1.0, 0.0, 0.0]);
            assert_eq!(*darkness, None);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(r[2], RasterFx::InnerGlow { center: true, .. }));
    assert!(close(outset(&list, vectorcraft_geom::Rect::ZERO), 10.0 + 9.0, 1e-9));
    assert_eq!(outset(&[], vectorcraft_geom::Rect::ZERO), 0.0);
}

#[test]
fn drop_shadow_darkness_replaces_its_color_when_chosen() {
    let darkness = |params: Value| match raster_effects(&[fx("stylize.dropShadow", params)]).as_slice() {
        [RasterFx::DropShadow { darkness, .. }] => *darkness,
        other => panic!("{other:?}"),
    };
    assert_eq!(darkness(json!({"shade": "darkness", "darkness": 60})), Some(0.6));
    assert_eq!(darkness(json!({"shade": "darkness"})), Some(1.0));
    assert_eq!(darkness(json!({"shade": "darkness", "darkness": 250})), Some(1.0));
    // Color is the default: a Darkness value alone leaves the shadow its color.
    assert_eq!(darkness(json!({"darkness": 60})), None);
    assert_eq!(darkness(json!({})), None);
}

#[test]
fn rasterize_compose_leaves_the_raster_alone_when_the_sizes_do_not_fit() {
    let r = match raster_effects(&[fx(RASTERIZE, json!({}))]).as_slice() {
        [RasterFx::Rasterize(r)] => *r,
        other => panic!("{other:?}"),
    };
    let space = PixelSpace {
        to_doc: Affine::IDENTITY,
        px: 1.0,
        center: Point::new(1.0, 1.0),
        channels: pixel::Channels::Rgb,
        line: 1.0,
        extent: std::f64::consts::SQRT_2,
    };
    let compose = |len: usize, w: usize, h: usize, image: &[u8], cols: usize, rows: usize, rect: Rect| {
        let mut data = vec![7; len];
        r.compose(&mut data, w, h, &space, image, cols, rows, rect);
        data
    };
    // A 2 × 2 red image over a 2 × 2 raster of one-point pixels: each pixel takes the image's color.
    let red = [255, 0, 0, 255].repeat(4);
    let mut long = red.clone();
    long.extend([255, 0]);
    let (img, long, area) = (red.as_slice(), long.as_slice(), Rect::new(0.0, 0.0, 2.0, 2.0));
    assert_eq!(compose(16, 2, 2, img, 2, 2, area), red);
    let cases = [
        ("w × h is not the raster's pixel count", 16, 1, 2, img, 2, 2, area),
        ("the raster's length is not a multiple of 4", 18, 2, 2, img, 2, 2, area),
        ("cols × rows is not the image's pixel count", 16, 2, 2, img, 1, 2, area),
        ("the image's length is not a multiple of 4", 16, 2, 2, long, 2, 2, area),
        ("the image has no width", 16, 2, 2, img, 2, 2, Rect::new(0.0, 0.0, 0.0, 2.0)),
        ("the image has no height", 16, 2, 2, img, 2, 2, Rect::new(0.0, 0.0, 2.0, 0.0)),
        ("the image has a negative width", 16, 2, 2, img, 2, 2, Rect::new(2.0, 0.0, 0.0, 2.0)),
        ("the image has a NaN width", 16, 2, 2, img, 2, 2, Rect::new(0.0, 0.0, f64::NAN, 2.0)),
        ("the image has a NaN height", 16, 2, 2, img, 2, 2, Rect::new(0.0, 0.0, 2.0, f64::NAN)),
    ];
    let failures: Vec<_> = cases
        .iter()
        .filter(|&&(_, len, w, h, image, cols, rows, rect)| compose(len, w, h, image, cols, rows, rect) != vec![7; len])
        .map(|c| c.0)
        .collect();
    assert!(failures.is_empty(), "{failures:?}");
}

#[test]
fn blend_mode_names() {
    use vectorcraft_doc::color::BlendMode;
    assert_eq!(raster::blend_mode("Color-Burn"), BlendMode::ColorBurn);
    assert_eq!(raster::blend_mode("screen"), BlendMode::Screen);
    assert_eq!(raster::blend_mode("???"), BlendMode::Normal);
}

#[test]
fn free_distort_identity_and_corner_move() {
    let id = run("distort.freeDistort", json!({}), &square()).bounds().unwrap();
    assert!(close(id.x0, 0.0, 1e-9) && close(id.x1, 100.0, 1e-9));
    let out = run("distort.freeDistort", json!({"corners": [[0, 0], [1.5, 0], [1, 1], [0, 1]]}), &square());
    let b = out.bounds().unwrap();
    assert!(close(b.x1, 150.0, 1e-6), "{b:?}");
}

#[test]
fn bad_params_fall_back_to_defaults() {
    let out = run("distort.roughen", json!({"size": "abc", "detail": null, "points": 7}), &square());
    assert_eq!(out, run("distort.roughen", json!({}), &square()));
    let out = run("path.offsetPath", json!({"offset": "12 pt"}), &square());
    assert!(close(out.bounds().unwrap().width(), 124.0, 0.1));
}

#[test]
#[ignore]
fn bench_catalog_lookup() {
    let t = std::time::Instant::now();
    let mut n = 0;
    for _ in 0..10_000 {
        n += crate::is_geometry("distort.roughen") as usize;
        n += crate::merged_params("stylize.dropShadow", &serde_json::Value::Null).as_object().unwrap().len();
    }
    eprintln!("10k lookups: {:?} ({n})", t.elapsed());
}

#[test]
fn a_repeat_of_a_symbol_whose_art_is_a_repeat_shares_the_instance_budget() {
    use std::sync::Arc;
    use vectorcraft_doc::{Document, NodeId, RepeatKind, RepeatSpec, Symbol};
    let unit = || {
        Arc::new(Node::path(
            NodeId(0),
            PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0)], true)),
            Appearance::default(),
        ))
    };
    let grid = |source: Arc<Node>, n: u32| {
        let mut g = RepeatSpec::grid(vec![source], 0.0, 0.0);
        if let RepeatKind::Grid { rows, cols, .. } = &mut g.kind {
            *rows = n;
            *cols = n;
        }
        g
    };
    let repeat = |r: RepeatSpec| Arc::new(Node::new(NodeId(0), NodeKind::Repeat(r)));
    let instance = |name: &str| Arc::new(Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: name.into(), xf: Affine::IDENTITY }));
    let mut d = Document::new(100.0, 100.0);
    // A 500 × 500 grid as a symbol's art, directly and inside a group; a symbol holding itself.
    d.symbols.push(Symbol { name: "Grid".into(), art: repeat(grid(unit(), 500)) });
    d.symbols.push(Symbol { name: "Grouped".into(), art: Arc::new(Node::group(NodeId(0), vec![repeat(grid(unit(), 500))])) });
    d.symbols.push(Symbol { name: "Loop".into(), art: Arc::new(Node::group(NodeId(0), vec![unit(), instance("Loop")])) });
    d.symbols.push(Symbol { name: "Small".into(), art: repeat(grid(unit(), 3)) });
    // A 3 × 3 grid of an instance of each: each copy draws 250,000 instances, so one copy fits.
    for name in ["Grid", "Grouped"] {
        let outer = Node::new(NodeId(0), NodeKind::Repeat(grid(instance(name), 3)));
        let kept = expand_live(Some(&d), &outer).len();
        assert!(kept <= 1, "{name}: {kept} copies of 250,000 instances");
    }
    // A symbol holding itself counts no Repeats, and small nests keep every instance.
    let looped = Node::new(NodeId(0), NodeKind::Repeat(grid(instance("Loop"), 3)));
    assert_eq!(expand_live(Some(&d), &looped).len(), 9);
    let small = Node::new(NodeId(0), NodeKind::Repeat(RepeatSpec::mirror(vec![instance("Small")], 90.0, 5.0)));
    assert_eq!(expand_live(Some(&d), &small).len(), 2);
}

#[test]
fn a_distance_parameters_range_comes_from_its_documentation() {
    assert_eq!(length_range("pixelate.colorHalftone", "maxRadius"), Some(4.0..=127.0));
    // The effect applies the range's floor to a radius below it.
    assert_eq!(
        crate::pixel::parse("pixelate.colorHalftone", &json!({"maxRadius": 1})),
        crate::pixel::parse("pixelate.colorHalftone", &json!({"maxRadius": 4}))
    );
    // A distance documented without a range, and unknown effects and parameters, have none.
    assert_eq!(length_range("path.offsetPath", "offset"), None);
    assert_eq!(length_range("pixelate.colorHalftone", "channel1"), None);
    assert_eq!(length_range("no.such", "maxRadius"), None);
}
