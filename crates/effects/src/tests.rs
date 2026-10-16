use serde_json::{Value, json};
use vectorcraft_doc::Effect;
use vectorcraft_geom::{Affine, PathData, Point, Rect, Shape, SubPath, shapes};

use super::*;

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
    assert_eq!(count(&evaluate_container(&g).unwrap()), 1001);
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
        })),
    );
    let mut g = Node::group(id, vec![Arc::new(sq), Arc::new(image)]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 70, "copies": 2})));
    let art = evaluate_container(&g).unwrap();
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
            })),
        ))],
    );
    g.appearance.effects.push(fx("distort.transform", json!({"scaleH": 50, "scaleV": 50, "reference": 8})));
    let art = evaluate_container(&g).unwrap();
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
    assert_eq!(xs(&evaluate_container(&g).unwrap()), [5.0, 75.0, 145.0], "the instance and its two copies, 70 pt apart");
    // Without copies the instance itself takes the transform, as a path does.
    let mut g = Node::group(id, vec![Arc::new(instance())]);
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 70})));
    assert_eq!(xs(&evaluate_container(&g).unwrap()), [75.0]);
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
        })),
    );
    let mut g = Node::group(id, vec![Arc::new(image)]);
    // The largest move: every copy is 1e7 pt further right, so the 100th would end past 1e9.
    g.appearance.effects.push(fx("distort.transform", json!({"moveH": 1.0e7, "copies": 1000})));
    let mut xs = vec![];
    image_xs(&evaluate_container(&g).unwrap(), &mut xs);
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
