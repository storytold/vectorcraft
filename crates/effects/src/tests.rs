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

/// The effects that symbol art across 9-slice guides draws once around all its cells are raster
/// effects that do not reshape it.
#[test]
fn nine_slice_painted_effects_are_raster_effects() {
    for id in vectorcraft_doc::PAINTED_EFFECTS {
        assert!(is_raster(id) && !is_geometry(id), "{id}");
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

/// `warp::bends` says a warp moves placed gradients exactly when the warp moves the outline:
/// Arc leaves the outline as it is under a bend of 0.0001 % and moves it above that, and the other
/// styles move it at any bend.
#[test]
fn warp_bends_when_the_warp_moves_the_outline() {
    // A star: Fisheye moves nothing on the edge of the box, where an ellipse or a rectangle lies.
    let oval = shapes::star(Point::new(51.7, 34.3), 40.9, 17.3, 5, 7.0);
    for (s, _) in WARP_STYLES {
        let id = format!("warp.{s}");
        let flat = run(&id, json!({"bend": 0}), &oval);
        for bend in [0.0, 1e-5, -1e-5, 0.9e-4, -0.9e-4, 1.1e-4, -1.1e-4] {
            let p = json!({ "bend": bend });
            let moved = run(&id, p.clone(), &oval) != flat;
            assert_eq!(crate::warp::bends(&id, &p), moved, "{id} at {bend} %");
        }
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

/// A Warp of `version` 1 bends its bounds as its style's Bézier net; one without a version, as
/// saved before the version existed, draws as it always did. Flag at 60 % on a 200 × 100 box: the
/// net moves the top edge up to 6 × 0.6 × 100 × max u(1 - u)(1 - 2u) = 34.64 out, the earlier map
/// moves it 0.6 × 25 = 15.
#[test]
fn a_warp_draws_with_the_map_its_version_names() {
    let rect = shapes::rectangle(Rect::new(10.0, 20.0, 210.0, 120.0));
    // u(1 - u)(1 - 2u) peaks at √3 / 18.
    let peak = 360.0 * 3f64.sqrt() / 18.0;
    for (version, out) in [(json!(1), peak), (json!(null), 15.0), (json!(0), 15.0)] {
        let b = run("warp.flag", json!({"bend": 60, "version": version}), &rect).bounds().unwrap();
        assert!(close(b.y0, 20.0 - out, 0.1) && close(b.y1, 120.0 + out, 0.1), "version {version}: {b:?}");
        assert!(close(b.x0, 10.0, 1e-9) && close(b.x1, 210.0, 1e-9), "version {version}: {b:?}");
    }
}

/// A straight line has a box with no width or no height, and the net bends it as the middle of
/// the patch along that side: under Arc a vertical line stays vertical, and under Bulge, Fisheye,
/// Inflate and Squeeze a horizontal line stays straight.
#[test]
fn a_warp_bends_a_flat_box_as_the_middle_of_its_patch() {
    let anchors = |p: &PathData| p.subpaths.iter().flat_map(|s| s.anchors.clone()).collect::<Vec<_>>();
    let vertical = shapes::line(Point::new(100.0, 0.0), Point::new(100.0, 100.0));
    let out = run("warp.arc", json!({"bend": 50, "version": 1}), &vertical);
    assert!(anchors(&out).iter().all(|a| close(a.p.x, 100.0, 1e-9) && close(a.h_in.x, 100.0, 1e-9) && close(a.h_out.x, 100.0, 1e-9)), "{out:?}");
    let horizontal = shapes::line(Point::new(0.0, 50.0), Point::new(200.0, 50.0));
    for style in ["bulge", "fisheye", "inflate", "squeeze"] {
        let out = run(&format!("warp.{style}"), json!({"bend": 50, "version": 1}), &horizontal);
        assert!(
            anchors(&out).iter().all(|a| close(a.p.y, 50.0, 1e-9) && close(a.h_in.y, 50.0, 1e-9) && close(a.h_out.y, 50.0, 1e-9)),
            "{style}: {out:?}"
        );
    }
}

/// A new Warp takes the nets' version unless one is given, and the catalog's defaults carry none,
/// so a saved warp's params don't gain one.
#[test]
fn new_warps_take_the_nets_version() {
    for (s, _) in WARP_STYLES {
        let id = format!("warp.{s}");
        let Some(e) = crate::new_effect(&id, &json!({"bend": 20})) else { panic!("{id}") };
        assert_eq!(e.params["version"], json!(crate::WARP_NET), "{id}");
        let Some(e) = crate::new_effect(&id, &json!({"version": 0})) else { panic!("{id}") };
        assert_eq!(e.params["version"], json!(0), "{id}");
        assert!(crate::merged_params(&id, &json!({"bend": 20})).get("version").is_none(), "{id}");
    }
    let Some(e) = crate::new_effect("distort.twist", &json!({})) else { panic!("twist") };
    assert!(e.params.get("version").is_none());
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

/// A 40 × 40 px image placed at (0, 0) at one point a pixel, with id `id`.
fn warp_test_image(id: u64) -> std::sync::Arc<Node> {
    use vectorcraft_doc::{ImageObject, NodeId};
    let im = ImageObject { key: "k".into(), width: 40, height: 40, xf: Affine::IDENTITY, link: None, placement: Default::default(), pixels: None };
    std::sync::Arc::new(Node::new(NodeId(id), NodeKind::Image(Box::new(im))))
}

/// How many images `art` holds, and the keys they have.
fn warp_test_images(art: &Node) -> (usize, std::collections::BTreeSet<String>) {
    let (mut n, mut keys) = (0, std::collections::BTreeSet::new());
    art.walk(&mut |c| {
        if let NodeKind::Image(im) = &c.kind {
            n += 1;
            keys.insert(im.key.clone());
        }
    });
    (n, keys)
}

#[test]
fn warped_images_stay_within_the_object_budget() {
    use std::rc::Rc;
    use std::sync::Arc;

    use vectorcraft_doc::NodeId;
    let lift: crate::reshape::WarpMap = Rc::new(|p: Point| Point::new(p.x, p.y - p.x * 0.1));
    let cost = vectorcraft_doc::live::mesh_warp_cost(crate::reshape::WARP_IMAGE_CELLS);
    let none = std::collections::HashSet::new();
    let k = || std::collections::BTreeSet::from(["k".to_string()]);
    let warp = |art: &mut Node, warps: usize| {
        let mut warped = crate::reshape::WarpedImages::new();
        for _ in 0..warps {
            crate::reshape::mark_warped_images(art, &lift, &none, false, &mut 0, &mut warped);
        }
        warped
    };
    // The art has room for one image's pieces only: neither image is cut, and each gets its own
    // key back.
    let mut art = Node::group(NodeId(1), vec![warp_test_image(2), warp_test_image(3)]);
    let warped = warp(&mut art, 1);
    let max = art.count() + cost;
    crate::reshape::cut_warped_images_within(&mut art, &warped, max);
    assert_eq!(warp_test_images(&art), (2, k()));
    // With room for both, each is cut into 200 pieces once, however many warps it took.
    let mut art = Node::group(NodeId(1), vec![warp_test_image(2), warp_test_image(3)]);
    let warped = warp(&mut art, 3);
    let max = art.count() + 2 * cost;
    crate::reshape::cut_warped_images_within(&mut art, &warped, max);
    assert_eq!(warp_test_images(&art), (400, k()));
    // Images clipped by their frames: the second frame (the second path) kept its shape, so its
    // image stays whole, and the first is cut. The third's frame is a group of two paths, the
    // second of them kept, with an image after them, and its image sits in a group of its own:
    // both stay whole too.
    let frame = |id: u64| Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)), Default::default()));
    let clip_group = |id: u64, children: Vec<Arc<Node>>| Arc::new(Node::new(NodeId(id), NodeKind::Group { children, clip: true }));
    let nested = clip_group(
        4,
        vec![
            Arc::new(Node::group(NodeId(4), vec![frame(4), frame(4), warp_test_image(5)])),
            Arc::new(Node::group(NodeId(4), vec![warp_test_image(4)])),
        ],
    );
    let mut art =
        Node::group(NodeId(1), vec![clip_group(2, vec![frame(2), warp_test_image(2)]), clip_group(3, vec![frame(3), warp_test_image(3)]), nested]);
    let (mut at, mut warped) = (0, crate::reshape::WarpedImages::new());
    crate::reshape::mark_warped_images(&mut art, &lift, &std::collections::HashSet::from([1, 3]), false, &mut at, &mut warped);
    assert_eq!(at, 4);
    crate::reshape::cut_warped_images_within(&mut art, &warped, usize::MAX);
    assert_eq!(warp_test_images(&art), (203, k()));
}

/// An image key from the document that looks like a key of the table is not read as one, and a
/// hidden image stays out of the bounds the later effects take, as it does before it is warped.
#[test]
fn warped_images_keep_document_keys_and_hidden_images_apart() {
    use std::rc::Rc;
    use std::sync::Arc;

    use vectorcraft_doc::NodeId;
    let lift: crate::reshape::WarpMap = Rc::new(|p: Point| Point::new(p.x, p.y - p.x * 0.1));
    let none = std::collections::HashSet::new();
    let mut warped = crate::reshape::WarpedImages::new();
    let lookalike = format!("{}0", warped.prefix());
    let mut b = (*warp_test_image(3)).clone();
    if let NodeKind::Image(im) = &mut b.kind {
        im.key = lookalike.clone();
    }
    let mut art = Node::group(NodeId(1), vec![warp_test_image(2), Arc::new(b)]);
    warped.reserve(&art);
    crate::reshape::mark_warped_images(&mut art, &lift, &none, false, &mut 0, &mut warped);
    crate::reshape::cut_warped_images_within(&mut art, &warped, usize::MAX);
    assert_eq!(warp_test_images(&art), (400, std::collections::BTreeSet::from(["k".to_string(), lookalike])));
    // A layer of a 10 × 10 square and a hidden 40 × 40 image: the warped image is not measured.
    let square = Arc::new(Node::path(NodeId(4), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default()));
    let mut hidden = (*warp_test_image(5)).clone();
    hidden.visible = false;
    let mut layer = Node::layer(NodeId(6), "Layer 1", vectorcraft_doc::LayerColor::Preset(0));
    if let Some(children) = layer.children_mut() {
        *children = vec![square, Arc::new(hidden)];
    }
    let before = layer.geometric_bounds();
    let mut warped = crate::reshape::WarpedImages::new();
    warped.reserve(&layer);
    crate::reshape::mark_warped_images(&mut layer, &lift, &none, false, &mut 0, &mut warped);
    assert_eq!(warped.bounds(&layer), before);
    // A shown image is measured by the pieces it is cut into.
    let mut art = Node::group(NodeId(1), vec![warp_test_image(2)]);
    let mut warped = crate::reshape::WarpedImages::new();
    warped.reserve(&art);
    crate::reshape::mark_warped_images(&mut art, &lift, &none, false, &mut 0, &mut warped);
    let measured = warped.bounds(&art);
    crate::reshape::cut_warped_images_within(&mut art, &warped, usize::MAX);
    assert_eq!(measured, art.geometric_bounds());
    assert!(measured.is_some_and(|b| b.y0 < -3.9), "{measured:?}");
    // An image placed at a tiny scale (a placement with a tiny determinant) is cut too; one placed
    // with no area is not.
    for (scale, pieces) in [(1e-7, 200), (0.0, 1)] {
        let mut image = (*warp_test_image(2)).clone();
        if let NodeKind::Image(im) = &mut image.kind {
            im.xf = Affine::scale(scale);
        }
        let mut art = Node::group(NodeId(1), vec![Arc::new(image)]);
        let mut warped = crate::reshape::WarpedImages::new();
        crate::reshape::mark_warped_images(&mut art, &lift, &none, false, &mut 0, &mut warped);
        crate::reshape::cut_warped_images_within(&mut art, &warped, usize::MAX);
        assert_eq!(warp_test_images(&art).0, pieces, "{scale}");
    }
}

/// An image whose frame keeps its shape because the warp would pass the anchor budget keeps its
/// pixels as they are; within the budget, both bend.
#[test]
fn an_image_whose_frame_passes_the_budget_stays_whole() {
    use std::sync::Arc;

    use vectorcraft_doc::NodeId;
    let clipped = || {
        let mut frame = Node::path(NodeId(1), shapes::rectangle(Rect::new(0.0, 0.0, 40.0, 40.0)), Appearance::default());
        if let NodeKind::Path { clipping, .. } = &mut frame.kind {
            *clipping = true;
        }
        Node::new(NodeId(1), NodeKind::Group { children: vec![Arc::new(frame), warp_test_image(1)], clip: true })
    };
    let arc = fx("warp.arc", json!({"bend": 50, "version": 1}));
    let k = || std::collections::BTreeSet::from(["k".to_string()]);
    let mut art = clipped();
    crate::reshape::reshape_leaves_within(&mut art, &[&arc], Budget { anchors: 0, lines: 0 }, None);
    assert_eq!(art.children().and_then(|c| c.first()).and_then(|c| c.path_data()).map(|p| p.anchor_count()), Some(4));
    assert_eq!(warp_test_images(&art), (1, k()));
    let mut art = clipped();
    crate::reshape::reshape_leaves_within(&mut art, &[&arc], Budget::EFFECTS, None);
    assert!(art.children().and_then(|c| c.first()).and_then(|c| c.path_data()).is_some_and(|p| p.anchor_count() > 4));
    assert_eq!(warp_test_images(&art), (200, k()));
}

/// The first image under `n` (the first piece of a cut image).
fn first_image(n: &Node) -> Option<vectorcraft_doc::ImageObject> {
    let mut first = None;
    n.walk(&mut |c| {
        if first.is_none()
            && let NodeKind::Image(im) = &c.kind
        {
            first = Some((**im).clone());
        }
    });
    first
}

#[test]
fn effects_after_a_warp_reshape_an_images_frame_and_not_its_pieces() {
    let image = |effects: Vec<Effect>| {
        let mut n = (*warp_test_image(1)).clone();
        n.appearance.effects = effects;
        n
    };
    // Each piece's clipping path, and the image's frame (the outer clip group's clipping path).
    let clips = |art: &Node| {
        let (mut pieces, mut frame) = (vec![], None);
        art.walk(&mut |c| {
            if let NodeKind::Group { children, clip: true } = &c.kind
                && let Some(path) = children.first().and_then(|p| p.path_data())
            {
                if matches!(children.get(1).map(|i| &i.kind), Some(NodeKind::Image(_))) && path.anchor_count() == 3 {
                    pieces.push(path.anchor_count());
                } else if frame.is_none() {
                    frame = Some(path.anchor_count());
                }
            }
        });
        (pieces, frame)
    };
    let arc = fx("warp.arc", json!({"bend": 50, "version": 1}));
    for after in [
        fx("distort.roughen", json!({"size": 5, "detail": 10})),
        fx("stylize.roundCorners", json!({"radius": 5})),
        fx("path.offsetPath", json!({"offset": -2})),
    ] {
        let art = crate::reshape::reshape(None, &image(vec![arc.clone(), after.clone()]), None).unwrap();
        let (pieces, frame) = clips(&art);
        assert_eq!(pieces.len(), 200, "{}", after.id);
        assert!(frame.is_some_and(|n| n > 4), "{}: {frame:?}", after.id);
        assert_eq!(warp_test_images(&art), (200, std::collections::BTreeSet::from(["k".to_string()])), "{}", after.id);
    }
    // Two warps cut the image once, through both maps; a Transform after them moves the pieces.
    let flag = fx("warp.flag", json!({"bend": 30, "version": 1}));
    let mv = fx("distort.transform", json!({"moveH": 100, "moveV": 0}));
    let art = crate::reshape::reshape(None, &image(vec![arc.clone(), flag.clone()]), None).unwrap();
    assert_eq!(warp_test_images(&art).0, 200);
    let moved = crate::reshape::reshape(None, &image(vec![arc.clone(), flag.clone(), mv]), None).unwrap();
    assert_eq!(warp_test_images(&moved).0, 200);
    let box1 = Rect::new(0.0, 0.0, 40.0, 40.0);
    let f1 = crate::warp::warp_map(box1, crate::warp::WarpStyle::Arc, &merged_params("warp.arc", &arc.params));
    // The flag bends the box the arc leaves, which the arc's frame measures.
    let box2 = crate::reshape::reshape(None, &image(vec![arc.clone()]), None).unwrap().geometric_bounds().unwrap();
    let f2 = crate::warp::warp_map(box2, crate::warp::WarpStyle::Flag, &merged_params("warp.flag", &flag.params));
    // The first piece's corners: the pixels (0, 0), (4, 0) and (4, 4).
    for (art, by) in [(&art, 0.0), (&moved, 100.0)] {
        let piece = first_image(art).unwrap();
        for p in [Point::new(0.0, 0.0), Point::new(4.0, 0.0), Point::new(4.0, 4.0)] {
            let want = f2(f1(p)) + vectorcraft_geom::Vec2::new(by, 0.0);
            assert!((piece.xf * p).distance(want) < 1e-6, "{p:?}: {:?} {want:?}", piece.xf * p);
        }
    }
}

/// A warp with no version (VectorCraft's earlier map) bends an image's pixels with that map, as it
/// bends the frame.
#[test]
fn an_images_pixels_take_the_map_their_warps_version_names() {
    for version in [json!(null), json!(0), json!(1)] {
        let mut params = json!({"bend": 50});
        if !version.is_null() {
            params["version"] = version.clone();
        }
        let mut n = (*warp_test_image(1)).clone();
        n.appearance.effects = vec![fx("warp.arc", params.clone())];
        let art = crate::reshape::reshape(None, &n, None).unwrap();
        let f = crate::warp::warp_map(Rect::new(0.0, 0.0, 40.0, 40.0), crate::warp::WarpStyle::Arc, &merged_params("warp.arc", &params));
        let piece = first_image(&art).unwrap();
        for p in [Point::new(0.0, 0.0), Point::new(4.0, 0.0), Point::new(4.0, 4.0)] {
            assert!((piece.xf * p).distance(f(p)) < 1e-6, "{version}: {p:?}");
        }
        // The frame has an anchor where the map takes each corner.
        let frame = art.children().and_then(|c| c.first()).and_then(|c| c.path_data()).unwrap();
        for corner in [Point::new(0.0, 0.0), Point::new(40.0, 0.0), Point::new(40.0, 40.0), Point::new(0.0, 40.0)] {
            let near = frame.subpaths.iter().flat_map(|s| &s.anchors).any(|a| a.p.distance(f(corner)) < 1e-6);
            assert!(near, "{version}: {corner:?}");
        }
    }
}

/// Under a group's Warp effect, a nested group's own placed gradient follows the nested group's
/// bounds to where its image's pieces draw them.
#[test]
fn a_nested_groups_gradient_follows_its_warped_image() {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
    use vectorcraft_doc::{Appearance, AppearanceItem, FillLayer, NodeId};
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let geom = GradientGeom { start: (5.0, 20.0).into(), end: (35.0, 20.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..GradientPaint::new(Gradient::new(GradientKind::Linear, stops)) }));
    let mut inner = Node::group(NodeId(5), vec![warp_test_image(1)]);
    inner.appearance.items.push(AppearanceItem::Fill(FillLayer::new(placed)));
    let square = Node::path(NodeId(2), shapes::rectangle(Rect::new(60.0, 0.0, 100.0, 40.0)), Appearance::default());
    let mut g = Node::group(NodeId(10), vec![Arc::new(inner), Arc::new(square)]);
    g.appearance.effects.push(fx("warp.arc", json!({"bend": 50, "version": 1})));
    let art = crate::group::evaluate_container(None, &g).unwrap();
    assert_eq!(warp_test_images(&art).0, 200);
    let (from, mut to) = (Rect::new(0.0, 0.0, 40.0, 40.0), None);
    art.walk(&mut |c| {
        if c.id == NodeId(5) && to.is_none() {
            to = c.geometric_bounds();
        }
    });
    let to = to.unwrap();
    assert!(to.y0 < from.y0 - 1.0, "the pieces draw the image bent: {to:?}");
    let follow = |p: Point| Point::new(to.x0 + (p.x - from.x0) / from.width() * to.width(), to.y0 + (p.y - from.y0) / from.height() * to.height());
    let mut checked = 0;
    art.walk(&mut |c| {
        if c.id != NodeId(5) {
            return;
        }
        for item in &c.appearance.items {
            let Paint::Gradient(gp) = item.paint() else { continue };
            let got = gp.geom.unwrap();
            assert!(got.start.distance(follow(geom.start)) < 1e-6 && got.end.distance(follow(geom.end)) < 1e-6, "{got:?} in {to:?}");
            checked += 1;
        }
    });
    assert!(checked > 0);
}

/// An image under a geometry effect other than a Warp draws as the image does: colorized by
/// a grayscale image's fill color, and smoothed or not.
#[test]
fn an_image_under_a_geometry_effect_keeps_its_color_and_smoothing() {
    let mut n = (*warp_test_image(1)).clone();
    n.appearance.effects = vec![fx("distort.roughen", json!({"size": 5, "detail": 10}))];
    n.extras_mut().colorize = Some(Box::new(vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb(1.0, 0.0, 0.0))));
    n.extras_mut().pixelated = true;
    let art = crate::reshape::reshape(None, &n, None).unwrap();
    let mut seen = vec![];
    art.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Image(_)) {
            seen.push((c.extras().colorize.clone(), c.extras().pixelated));
        }
    });
    assert_eq!(seen, vec![(n.extras().colorize.clone(), true)]);
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
    crate::reshape::reshape_leaves_within(&mut art, &[&rough, &rough, &rough], Budget { anchors: 5_000, lines: usize::MAX }, None);
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

/// A path that keeps its shape because its Transform effect would pass the budget keeps the
/// gradients that paint the art as a whole in place too, also with Scale Strokes & Effects on.
#[test]
fn art_past_the_budget_keeps_its_own_gradients_in_place() {
    use std::sync::Arc;

    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    use vectorcraft_doc::{AppearanceItem, FillLayer, NodeId, color::Paint};
    let id = NodeId(9);
    let stops = vec![GradientStop::new(0.0, vectorcraft_color::Color::BLACK), GradientStop::new(1.0, vectorcraft_color::Color::WHITE)];
    let geom = GradientGeom { start: (0.0, 50.0).into(), end: (100.0, 50.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..GradientPaint::new(Gradient::new(GradientKind::Linear, stops)) }));
    let circle = || Arc::new(Node::path(id, shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default()));
    // The group's own fill, a gradient placed across the circle, after `carry` moves it.
    let own_end = |carry: &crate::Carry| {
        let mut ap = Appearance::default();
        ap.items.push(AppearanceItem::Fill(FillLayer::new(placed.clone())));
        carry.apply(&mut ap);
        match ap.fill_paint() {
            Paint::Gradient(g) => g.geom.map(|g| g.end.x),
            _ => None,
        }
    };
    for params in [json!({"scaleH": 1000, "scaleV": 1000}), json!({"scaleH": 1000, "scaleV": 1000, "strokes": true})] {
        let grow = fx("distort.transform", params.clone());
        let mut art = Node::group(id, vec![circle()]);
        let carry = crate::reshape::reshape_leaves_within(&mut art, &[&grow], Budget::EFFECTS, None);
        let end = own_end(&carry).unwrap();
        assert!((end - 550.0).abs() < 1e-6, "{params}: within the budget the group's gradient grows with the circle: {end}");
        // No room beyond what the circle costs: scaled up, it would be drawn with more lines.
        let mut art = Node::group(id, vec![circle()]);
        let carry = crate::reshape::reshape_leaves_within(&mut art, &[&grow], Budget { anchors: usize::MAX, lines: 0 }, None);
        let b = art.geometric_bounds().unwrap();
        assert!(b.width() < 101.0 && b.height() < 101.0, "{params}: the circle keeps its shape: {b:?}");
        assert_eq!(own_end(&carry), Some(100.0), "{params}: the group's gradient stays where it was placed");
        // A group inside the art whose square took the effect moves its own gradient all the same.
        let square = Arc::new(Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default()));
        let mut inner = Node::group(NodeId(2), vec![square]);
        inner.appearance.items.push(AppearanceItem::Fill(FillLayer::new(placed.clone())));
        let mut art = Node::group(id, vec![circle(), Arc::new(inner)]);
        let carry = crate::reshape::reshape_leaves_within(&mut art, &[&grow], Budget { anchors: usize::MAX, lines: 0 }, None);
        assert_eq!(own_end(&carry), Some(100.0), "{params}: the art's own gradient stays");
        let inner = &art.children().unwrap()[1];
        let Paint::Gradient(g) = inner.appearance.fill_paint() else { panic!("a gradient") };
        assert!((g.geom.unwrap().end.x - 550.0).abs() < 1e-6, "{params}: the inner group's gradient: {:?}", g.geom);
    }
}

/// A hidden path in a layer that keeps its shape past the budget does not hold the layer's
/// gradients in place: the layer's bounds leave it out, so the layer's own gradient moves with
/// the visible art under a Transform effect and Free Distort, as does the art's when the layer is
/// the art.
#[test]
fn a_hidden_path_past_the_budget_leaves_a_layers_gradients_free_to_move() {
    use std::sync::Arc;

    use vectorcraft_color::{Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop};
    use vectorcraft_doc::{AppearanceItem, FillLayer, LayerColor, NodeId, NodeKind, color::Paint};
    let stops = vec![GradientStop::new(0.0, vectorcraft_color::Color::BLACK), GradientStop::new(1.0, vectorcraft_color::Color::WHITE)];
    let geom = GradientGeom { start: (0.0, 50.0).into(), end: (100.0, 50.0).into(), aspect: 1.0, focal: None };
    let placed = Paint::Gradient(Box::new(GradientPaint { geom: Some(geom), ..GradientPaint::new(Gradient::new(GradientKind::Linear, stops)) }));
    let end = |ap: &Appearance| match ap.fill_paint() {
        Paint::Gradient(g) => g.geom.map(|g| g.end.x),
        _ => None,
    };
    // A layer with a visible square and a hidden circle, both 0..100, and a gradient fill of its own.
    let layer = || {
        let square = Arc::new(Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default()));
        let mut circle = Node::path(NodeId(4), shapes::ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::default());
        circle.visible = false;
        let mut layer = Node::layer(NodeId(2), "Layer 1", LayerColor::Preset(0));
        if let NodeKind::Layer { children, .. } = &mut layer.kind {
            *children = vec![square, Arc::new(circle)];
        }
        layer.appearance.items.push(AppearanceItem::Fill(FillLayer::new(placed.clone())));
        layer
    };
    // No room beyond what the paths cost: the square takes each effect, the circle would be drawn
    // with more lines and keeps its shape.
    let budget = Budget { anchors: usize::MAX, lines: 0 };
    // Each scales the art ten times about its center: 0..100 becomes -450..550.
    let corners = json!({"corners": [[-4.5, -4.5], [5.5, -4.5], [5.5, 5.5], [-4.5, 5.5]]});
    for grow in [fx("distort.transform", json!({"scaleH": 1000, "scaleV": 1000})), fx("distort.freeDistort", corners)] {
        let mut art = layer();
        let carry = crate::reshape::reshape_leaves_within(&mut art, &[&grow], budget, None);
        let circle = art.children().unwrap()[1].geometric_bounds().unwrap();
        assert!(circle.width() < 101.0, "{}: the hidden circle keeps its shape: {circle:?}", grow.id);
        let square = art.children().unwrap()[0].geometric_bounds().unwrap();
        assert!((square.x1 - 550.0).abs() < 1e-6, "{}: the square takes the effect: {square:?}", grow.id);
        let mut own = Appearance::clone(&art.appearance);
        carry.apply(&mut own);
        assert_eq!(end(&own), Some(550.0), "{}: the layer's gradient, as the art", grow.id);
        let mut art = Node::group(NodeId(9), vec![Arc::new(layer())]);
        crate::reshape::reshape_leaves_within(&mut art, &[&grow], budget, None);
        assert_eq!(end(&art.children().unwrap()[0].appearance), Some(550.0), "{}: the layer's gradient, inside the art", grow.id);
    }
}

/// A freeform gradient whose points were edited (its vector still fitted to the bounds) moves its
/// points with a Transform effect and Free Distort, on a path, on a group inside the art and on the
/// art itself, as a placed gradient does.
#[test]
fn edited_freeform_points_follow_a_transform_effect_and_free_distort() {
    use std::sync::Arc;

    use vectorcraft_color::{Color, Freeform, FreeformPoint, Gradient, GradientKind, GradientPaint};
    use vectorcraft_doc::{AppearanceItem, FillLayer, NodeId, color::Paint};
    let mut ramp = GradientPaint::new(Gradient { kind: GradientKind::Freeform, ..Gradient::default() });
    let pt = |x: f64, c| FreeformPoint::new(Point::new(x, 50.0), c);
    ramp.set_freeform(Freeform { points: vec![pt(0.0, Color::BLACK), pt(100.0, Color::WHITE)], ..Default::default() });
    assert!(ramp.geom.is_none() && !ramp.is_placed());
    let freeform = Paint::Gradient(Box::new(ramp));
    let xs = |ap: &Appearance| match ap.fill_paint() {
        Paint::Gradient(g) => g.freeform.as_ref().map(|f| f.points.iter().map(|p| p.at.x).collect::<Vec<_>>()),
        _ => None,
    };
    let square = |ap: Appearance| Arc::new(Node::path(NodeId(3), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), ap));
    // Each moves the art 30 pt right.
    let corners = json!({"corners": [[0.3, 0.0], [1.3, 0.0], [1.3, 1.0], [0.3, 1.0]]});
    for e in [fx("distort.transform", json!({"moveH": 30})), fx("distort.freeDistort", corners)] {
        let mut inner = Node::group(NodeId(2), vec![square(Appearance::default())]);
        inner.appearance.items.push(AppearanceItem::Fill(FillLayer::new(freeform.clone())));
        let mut art = Node::group(NodeId(9), vec![square(Appearance::basic(freeform.clone(), Paint::None, 0.0)), Arc::new(inner)]);
        art.appearance.items.push(AppearanceItem::Fill(FillLayer::new(freeform.clone())));
        let carry = crate::reshape::reshape_leaves_within(&mut art, &[&e], Budget::EFFECTS, None);
        let members = art.children().unwrap();
        assert_eq!(xs(&members[0].appearance), Some(vec![30.0, 130.0]), "{}: the path's points", e.id);
        assert_eq!(xs(&members[1].appearance), Some(vec![30.0, 130.0]), "{}: the inner group's points", e.id);
        let mut own = Appearance::clone(&art.appearance);
        carry.apply(&mut own);
        assert_eq!(xs(&own), Some(vec![30.0, 130.0]), "{}: the art's own points", e.id);
    }
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
    crate::reshape::reshape_leaves_within(&mut art, &[&transform, &offset], Budget::EFFECTS, None);
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
    crate::reshape::reshape_leaves_within(&mut art_evenodd, &[&transform, &offset], Budget::EFFECTS, None);
    let p = art_evenodd.children().unwrap()[0].path_data().unwrap();
    assert_eq!(p.subpaths.len(), 1, "EvenOdd does not swap; copies union into 1 outer subpath");

    // 2. Non-terminal Offset Path: followed by Round Corners, must preserve order.
    let leaf_round = Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default()));
    let mut art_round = Node::group(id, vec![leaf_round]);
    let round_corners = fx("stylize.roundCorners", json!({"radius": 2.0}));
    crate::reshape::reshape_leaves_within(&mut art_round, &[&transform, &offset, &round_corners], Budget::EFFECTS, None);
    let p = art_round.children().unwrap()[0].path_data().unwrap();
    assert_eq!(p.subpaths.len(), 1, "Non-terminal Offset Path does not swap; copies union into 1 outer subpath");

    // 3. Non-round join (e.g. miter): preserving order is required as corners differ (#4145).
    let leaf_miter = Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 20.0, 20.0)), Appearance::default()));
    let mut art_miter = Node::group(id, vec![leaf_miter]);
    let offset_miter = fx("path.offsetPath", json!({"offset": 4.0, "joins": "miter"}));
    crate::reshape::reshape_leaves_within(&mut art_miter, &[&transform, &offset_miter], Budget::EFFECTS, None);
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
fn twist_turns_clockwise_for_positive_angles() {
    // A positive angle turns clockwise, a negative one counterclockwise
    // (https://helpx.adobe.com/illustrator/desktop/manage-objects/reshape-transform-objects/distort-and-transform-effects-in-illustrator.html).
    // The document's y runs down, so clockwise on the page takes a point right of the center down.
    let right = PathData::single(SubPath::polyline(&[Point::new(55.0, 50.0), Point::new(56.0, 50.0)], false));
    let bounds = Rect::new(0.0, 0.0, 100.0, 100.0);
    let turned = |angle: f64| apply_geometry(&[fx("distort.twist", json!({"angle": angle}))], &right, bounds).subpaths[0].anchors[0].p;
    assert!(turned(45.0).y > 50.5, "{:?}", turned(45.0));
    assert!(turned(-45.0).y < 49.5, "{:?}", turned(-45.0));
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
    d.symbols.push(Symbol { dynamic: false, name: "Grid".into(), art: repeat(grid(unit(), 500)) });
    d.symbols.push(Symbol { dynamic: false, name: "Grouped".into(), art: Arc::new(Node::group(NodeId(0), vec![repeat(grid(unit(), 500))])) });
    d.symbols.push(Symbol { dynamic: false, name: "Loop".into(), art: Arc::new(Node::group(NodeId(0), vec![unit(), instance("Loop")])) });
    d.symbols.push(Symbol { dynamic: false, name: "Small".into(), art: repeat(grid(unit(), 3)) });
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
    assert_eq!(length_range("stylize.glowingEdges", "edgeWidth"), Some(1.0..=14.0));
    // A distance documented without a range, and unknown effects and parameters, have none.
    assert_eq!(length_range("path.offsetPath", "offset"), None);
    assert_eq!(length_range("stylize.glowingEdges", "edgeBrightness"), None);
    assert_eq!(length_range("no.such", "edgeWidth"), None);
}

#[test]
fn color_halftones_radius_is_in_raster_pixels_not_a_distance() {
    // Pixels of the Document Raster Effects resolution, as public documentation gives it (#3260):
    // no unit setting changes it, so it isn't a distance in points.
    assert_eq!(raster_px_range("pixelate.colorHalftone", "maxRadius"), Some(4.0..=127.0));
    assert!(!is_length("pixelate.colorHalftone", "maxRadius", false));
    assert_eq!(length_range("pixelate.colorHalftone", "maxRadius"), None);
    assert_eq!(raster_px_range("pixelate.colorHalftone", "channel1"), None);
    assert_eq!(raster_px_range("stylize.glowingEdges", "edgeWidth"), None);
    // The effect applies the range's floor to a radius below it.
    assert_eq!(
        crate::pixel::parse("pixelate.colorHalftone", &json!({"maxRadius": 1})),
        crate::pixel::parse("pixelate.colorHalftone", &json!({"maxRadius": 4}))
    );
    // Scale Strokes & Effects scales it with the object, as it does distances.
    let mut e = crate::new_effect("pixelate.colorHalftone", &json!({"maxRadius": 10})).unwrap();
    crate::scale_effect(&mut e, 2.0);
    assert_eq!((e.params["maxRadius"].as_f64(), e.params["channel1"].as_f64()), (Some(20.0), Some(108.0)));
}
