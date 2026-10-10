use serde_json::{Value, json};
use vectorcraft_doc::Effect;
use vectorcraft_geom::{PathData, Point, Rect, Shape, SubPath, shapes};

use super::*;

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
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

#[test]
fn transform_copies_stop_before_they_overflow() {
    let out = run("distort.transform", json!({"scaleH": 100000, "scaleV": 100000, "copies": 1000}), &square());
    assert!(all_finite(&out) && out.subpaths.len() < 10, "{}", out.subpaths.len());
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
fn zigzag_ridge_count_and_amplitude() {
    let out = run("distort.zigZag", json!({"ridges": 4, "points": "corner", "size": 10}), &square());
    assert_eq!(out.anchor_count(), 4 * 5);
    let b = out.bounds().unwrap();
    assert!(b.width() > 100.0 && b.width() <= 120.0 + 1e-6, "{b:?}");
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
fn scribble_produces_filled_outline_inside_bounds() {
    let out = run("stylize.scribble", json!({}), &square());
    assert!(!out.is_empty());
    let b = out.bounds().unwrap();
    assert!(b.x0 > -10.0 && b.x1 < 110.0 && b.y0 > -10.0 && b.y1 < 110.0, "{b:?}");
    assert_eq!(out, run("stylize.scribble", json!({}), &square()));
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
        RasterFx::DropShadow { mode, opacity, dx, dy, blur, color } => {
            assert_eq!(*mode, vectorcraft_doc::color::BlendMode::Multiply);
            assert!(close(*opacity as f64, 0.5, 1e-6));
            assert_eq!((*dx, *dy, *blur), (10.0, -4.0, 6.0));
            assert_eq!(color.to_rgb(), [1.0, 0.0, 0.0]);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(r[2], RasterFx::InnerGlow { center: true, .. }));
    assert!(close(outset(&list, vectorcraft_geom::Rect::ZERO), 10.0 + 9.0, 1e-9));
    assert_eq!(outset(&[], vectorcraft_geom::Rect::ZERO), 0.0);
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
