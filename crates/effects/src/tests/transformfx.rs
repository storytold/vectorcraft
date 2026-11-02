//! The Transform effect's Transform Objects, Transform Patterns and Scale Strokes & Effects
//! options (#885).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, AppearanceItem, Dash, Document, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::distort::{TransformOptions, transform_map, transform_move};
use crate::{bake_document, expand_outlined, has_transform_art, has_transform_options, reshape};

fn transform(params: Value) -> Effect {
    Effect { id: "distort.transform".into(), params, visible: true }
}

/// A 100 pt square at the origin with a 2 pt black stroke (dashed 4/2) and the Transform effect
/// `params`.
fn stroked(params: Value) -> Node {
    let mut n =
        Node::path(NodeId(7), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0));
    if let Some(s) = n.appearance.stroke_mut() {
        s.dash = Some(Dash { pattern: vec![4.0, 2.0], offset: 0.0, align_corners: false });
    }
    n.appearance.effects.push(transform(params));
    n
}

/// A 100 pt square at the origin filled with pattern "p" placed at the identity, with the
/// Transform effect `params`.
fn patterned(params: Value) -> Node {
    let paint = Paint::Pattern { pattern: "p".into(), xf: Affine::IDENTITY };
    let mut n = Node::path(NodeId(8), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(paint, Paint::None, 0.0));
    n.appearance.effects.push(transform(params));
    n
}

/// The paths under `n`, in document order.
fn leaves(n: &Node) -> Vec<&Node> {
    let mut out = vec![];
    fn walk<'a>(n: &'a Node, out: &mut Vec<&'a Node>) {
        match n.children() {
            Some(ch) if !matches!(n.kind, NodeKind::Compound { .. }) => ch.iter().for_each(|c| walk(c, out)),
            _ => out.push(n),
        }
    }
    walk(n, &mut out);
    out
}

fn stroke_of(n: &Node) -> (f64, Vec<f64>) {
    let s = n.appearance.stroke().unwrap();
    (s.width, s.dash.as_ref().map(|d| d.pattern.clone()).unwrap_or_default())
}

fn pattern_xf(n: &Node) -> Affine {
    match n.appearance.fill_paint() {
        Paint::Pattern { xf, .. } => xf,
        p => panic!("not a pattern: {p:?}"),
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

#[test]
fn the_options_default_to_what_the_effect_did_before() {
    let o = TransformOptions::of(&crate::merged_params("distort.transform", &json!({})));
    assert_eq!(o, TransformOptions { objects: true, patterns: false, strokes: false });
    assert!(!o.reach_appearance());
    // An old effect, with no options stored, still draws as geometry alone.
    let n = stroked(json!({"scaleH": 50, "scaleV": 50, "copies": 2}));
    assert!(!has_transform_options(&n));
    assert!(!has_transform_art(&n));
}

#[test]
fn transform_objects_off_maps_nothing_and_makes_no_copies() {
    let b = Rect::new(0.0, 0.0, 100.0, 100.0);
    let (m, copies) = transform_map(b, &json!({"scaleH": 50, "moveH": 30, "copies": 3, "objects": false}), 0);
    assert_eq!((m, copies), (Affine::IDENTITY, 0));
    // A plain move with Transform Patterns on isn't a plain move any more: the pattern moves too.
    assert!(transform_move(&json!({"moveH": 30})).is_some());
    assert!(transform_move(&json!({"moveH": 30, "patterns": true})).is_none());
    assert!(transform_move(&json!({"moveH": 30, "strokes": true})).is_some(), "a move scales nothing");
    assert!(transform_move(&json!({"scaleH": 50, "objects": false, "strokes": true})).is_none());
}

#[test]
fn scale_strokes_and_effects_scales_the_stroke_and_dash_with_the_object() {
    let n = stroked(json!({"scaleH": 50, "scaleV": 50, "strokes": true}));
    assert!(has_transform_art(&n));
    let art = reshape(None, &n, None).unwrap();
    let paths = leaves(&art);
    assert_eq!(paths.len(), 1);
    let (w, dash) = stroke_of(paths[0]);
    assert!(close(w, 1.0), "{w}");
    assert_eq!(dash, vec![2.0, 1.0]);
    let b = paths[0].geometric_bounds().unwrap();
    assert!(close(b.width(), 50.0), "{b:?}");
}

#[test]
fn each_copy_scales_its_stroke_once_more() {
    let n = stroked(json!({"scaleH": 200, "scaleV": 200, "copies": 2, "strokes": true}));
    let art = reshape(None, &n, None).unwrap();
    let widths: Vec<f64> = leaves(&art).iter().map(|p| stroke_of(p).0).collect();
    assert_eq!(widths, vec![2.0, 4.0, 8.0]);
    // Without the option the copies keep the original's weight.
    let plain = stroked(json!({"scaleH": 200, "scaleV": 200, "copies": 2}));
    assert!(!has_transform_art(&plain));
}

#[test]
fn transform_objects_off_keeps_the_geometry_and_scales_the_stroke_once() {
    let n = stroked(json!({"scaleH": 50, "scaleV": 50, "moveH": 40, "copies": 3, "objects": false, "strokes": true}));
    let art = reshape(None, &n, None).unwrap();
    let paths = leaves(&art);
    assert_eq!(paths.len(), 1, "no copies");
    assert_eq!(paths[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(close(stroke_of(paths[0]).0, 1.0));
}

#[test]
fn transform_patterns_moves_each_copys_pattern() {
    let n = patterned(json!({"moveH": 30, "copies": 2, "patterns": true}));
    assert!(has_transform_options(&n));
    let art = reshape(None, &n, None).unwrap();
    let xs: Vec<f64> = leaves(&art).iter().map(|p| pattern_xf(p).as_coeffs()[4]).collect();
    assert_eq!(xs, vec![0.0, 30.0, 60.0]);
    // Without it, every copy's pattern stays put (the tiles don't follow the copies).
    let n = patterned(json!({"moveH": 30, "patterns": false, "strokes": false, "objects": true}));
    assert!(!has_transform_art(&n));
}

#[test]
fn transform_patterns_with_objects_off_moves_only_the_pattern() {
    let n = patterned(json!({"rotate": 45, "objects": false, "patterns": true}));
    let art = reshape(None, &n, None).unwrap();
    let paths = leaves(&art);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_ne!(pattern_xf(paths[0]), Affine::IDENTITY);
}

#[test]
fn stroke_scaling_across_copies_stays_finite() {
    // A horizontal line stretched vertically barely grows, but its stroke would: the copies stop
    // before a stroke weight runs away.
    let mut n =
        Node::path(NodeId(9), shapes::line((0.0, 0.0).into(), (100.0, 0.0).into()), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
    n.appearance.effects.push(transform(json!({"scaleH": 4000, "scaleV": 4000, "copies": 1000, "strokes": true})));
    if let Some(art) = reshape(None, &n, None) {
        for p in leaves(&art) {
            let w = stroke_of(p).0;
            assert!(w.is_finite() && w <= 1.0e6, "{w}");
        }
    }
}

/// A 100 pt square at the origin with a solid gray fill and a 2 pt black stroke (dashed 4/2) that
/// carries the Transform effect `params` itself.
fn stroke_with_effect(params: Value) -> Node {
    let gray = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    let mut n =
        Node::path(NodeId(10), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(gray, Paint::solid(Color::BLACK), 2.0));
    if let Some(s) = n.appearance.stroke_mut() {
        s.dash = Some(Dash { pattern: vec![4.0, 2.0], offset: 0.0, align_corners: false });
        s.effects.push(transform(params));
    }
    n
}

/// A 100 pt square at the origin filled with pattern "p" placed at the identity, the fill carrying
/// the Transform effect `params` itself.
fn fill_with_effect(params: Value) -> Node {
    let mut n = patterned(json!({}));
    n.appearance.effects.clear();
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.effects.push(transform(params));
    }
    n
}

/// The paths under `n` that a stroke paints.
fn stroked_leaves(n: &Node) -> Vec<&Node> {
    leaves(n).into_iter().filter(|p| p.appearance.stroke().is_some()).collect()
}

/// The paths under `n` that a pattern fills.
fn pattern_leaves(n: &Node) -> Vec<&Node> {
    leaves(n).into_iter().filter(|p| matches!(p.appearance.fill_paint(), Paint::Pattern { .. })).collect()
}

#[test]
fn a_strokes_own_transform_scales_its_weight_and_dashes_with_scale_strokes_and_effects() {
    let n = stroke_with_effect(json!({"scaleH": 200, "scaleV": 200, "strokes": true}));
    assert!(has_transform_options(&n));
    let art = reshape(None, &n, None).unwrap();
    let paths = stroked_leaves(&art);
    assert_eq!(paths.len(), 1);
    let (w, dash) = stroke_of(paths[0]);
    assert!(close(w, 4.0), "{w}");
    assert_eq!(dash, vec![8.0, 4.0]);
    assert!(close(paths[0].geometric_bounds().unwrap().width(), 200.0));
    // The fill stays on the path as it was, below the stroke.
    let all = leaves(&art);
    assert_eq!(all.len(), 2);
    assert!(all[0].appearance.stroke().is_none());
    assert_eq!(all[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    // Off, the stroke keeps its weight and dashes: the effect stays geometry alone.
    let plain = stroke_with_effect(json!({"scaleH": 200, "scaleV": 200, "strokes": false}));
    assert!(!has_transform_art(&plain));
}

#[test]
fn each_copy_of_a_strokes_own_transform_scales_its_weight_once_more() {
    let n = stroke_with_effect(json!({"scaleH": 200, "scaleV": 200, "copies": 2, "strokes": true}));
    let art = reshape(None, &n, None).unwrap();
    let widths: Vec<f64> = stroked_leaves(&art).iter().map(|p| stroke_of(p).0).collect();
    assert_eq!(widths, vec![2.0, 4.0, 8.0]);
}

#[test]
fn a_fills_own_transform_with_scale_strokes_and_effects_scales_its_later_effects() {
    let mut n = stroke_with_effect(json!({}));
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.effects.push(transform(json!({"scaleH": 300, "scaleV": 300, "strokes": true})));
        f.effects.push(Effect { id: "path.offsetPath".into(), params: json!({"offset": 5.0}), visible: true });
    }
    let art = reshape(None, &n, None).unwrap();
    let fill = leaves(&art).into_iter().find(|p| p.appearance.stroke().is_none()).unwrap();
    let AppearanceItem::Fill(f) = &fill.appearance.items[0] else { panic!("not a fill") };
    let ids: Vec<&str> = f.effects.iter().map(|e| e.id.as_str()).collect();
    assert_eq!(ids, vec!["path.offsetPath"]);
    assert!(close(f.effects[0].params["offset"].as_f64().unwrap(), 15.0));
    assert!(close(fill.geometric_bounds().unwrap().width(), 300.0));
}

#[test]
fn a_fills_own_transform_moves_each_copys_pattern_with_transform_patterns() {
    let n = fill_with_effect(json!({"moveH": 30, "copies": 2, "patterns": true}));
    assert!(has_transform_options(&n));
    let art = reshape(None, &n, None).unwrap();
    let xs: Vec<f64> = pattern_leaves(&art).iter().map(|p| pattern_xf(p).as_coeffs()[4]).collect();
    assert_eq!(xs, vec![0.0, 30.0, 60.0]);
    // Off, the pattern stays put.
    assert!(!has_transform_art(&fill_with_effect(json!({"moveH": 30, "copies": 2}))));
}

#[test]
fn a_strokes_own_transform_with_transform_objects_off_keeps_its_geometry() {
    let n = stroke_with_effect(json!({"scaleH": 50, "scaleV": 50, "moveH": 40, "copies": 3, "objects": false, "strokes": true}));
    let art = reshape(None, &n, None).unwrap();
    let paths = stroked_leaves(&art);
    assert_eq!(paths.len(), 1, "no copies");
    assert_eq!(paths[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert!(close(stroke_of(paths[0]).0, 1.0));
    let n = fill_with_effect(json!({"rotate": 45, "objects": false, "patterns": true}));
    let art = reshape(None, &n, None).unwrap();
    let paths = pattern_leaves(&art);
    assert_eq!(paths.len(), 1);
    assert_eq!(paths[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_ne!(pattern_xf(paths[0]), Affine::IDENTITY);
}

#[test]
fn a_strokes_own_transform_draws_bakes_expands_and_measures_alike() {
    let n = stroke_with_effect(json!({"scaleH": 200, "scaleV": 200, "copies": 1, "strokes": true}));
    let drawn = reshape(None, &n, None).unwrap();
    let widths = |art: &Node| stroked_leaves(art).iter().map(|p| stroke_of(p).0).collect::<Vec<f64>>();
    assert_eq!(widths(&drawn), vec![2.0, 4.0]);
    // Expand Appearance.
    assert_eq!(widths(&expand_outlined(None, &n, None).unwrap()), widths(&drawn));
    // Export.
    let mut d = Document::new(800.0, 800.0);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, n.clone()).unwrap();
    let baked = bake_document(&d).unwrap();
    assert_eq!(widths(baked.node(NodeId(10)).unwrap()), widths(&drawn));
    // Use Preview Bounds: the copy's 4 pt stroke (on -50..150) reaches 2 pt past it.
    let symbols = d.symbol_boxes();
    let b = crate::preview::visual_bounds(&symbols, &n).unwrap();
    assert!(close(b.width(), 204.0), "{b:?}");
    assert_eq!(Some(b), drawn.visual_bounds_with(&symbols));
}

/// Three strokes, each with three max-detail Roughens (#3578: nearly one effect budget on their
/// own) before its own Transform effect: their paths keep to one effect budget together, not one
/// each.
#[test]
fn strokes_with_their_own_transform_keep_to_one_effect_budget() {
    use crate::geometry::Budget;
    let rough = Effect { id: "distort.roughen".into(), params: json!({"size": 10000, "relative": false, "detail": 100}), visible: true };
    let mut n = Node::path(
        NodeId(11),
        shapes::rectangle(Rect::new(0.0, 0.0, 1000.0, 1000.0)),
        Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 2.0),
    );
    let Some(AppearanceItem::Stroke(s)) = n.appearance.items.iter().find(|i| matches!(i, AppearanceItem::Stroke(_))).cloned() else {
        panic!("no stroke")
    };
    n.appearance.items.retain(|i| !matches!(i, AppearanceItem::Stroke(_)));
    for _ in 0..3 {
        let mut s = s.clone();
        s.effects = vec![rough.clone(); 3];
        s.effects.push(transform(json!({"scaleH": 50, "scaleV": 50, "strokes": true})));
        n.appearance.items.push(AppearanceItem::Stroke(s));
    }
    let art = reshape(None, &n, None).unwrap();
    let paths = stroked_leaves(&art);
    assert_eq!(paths.len(), 3);
    let cost =
        paths.iter().filter_map(|p| crate::reshape::leaf_geometry(p)).fold(Budget { anchors: 0, lines: 0 }, |c, (g, _)| c.plus(Budget::of(&g)));
    assert!(Budget::EFFECTS.fits(cost) && cost.lines > crate::MAX_EFFECT_LINES / 2, "{cost:?}");
}

/// A path of 600,000 anchors with a fill below a stroke whose own Transform effect makes five
/// copies: the fill's path counts toward the effect budget, so the stroke's copies stop where the
/// whole art would pass it.
#[test]
fn a_fill_below_a_strokes_own_transform_counts_toward_the_effect_budget() {
    use crate::geometry::Budget;
    use vectorcraft_geom::{PathData, Point, SubPath};
    let points: Vec<Point> = (0..600_000).map(|i| Point::new(f64::from(i) * 1.0e-4, f64::from(i % 2))).collect();
    let gray = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    let mut n = Node::path(NodeId(12), PathData::single(SubPath::polyline(&points, false)), Appearance::basic(gray, Paint::solid(Color::BLACK), 1.0));
    if let Some(s) = n.appearance.stroke_mut() {
        s.effects.push(transform(json!({"moveH": 100, "copies": 5, "strokes": true})));
    }
    let art = reshape(None, &n, None).unwrap();
    let cost = leaves(&art)
        .iter()
        .filter_map(|p| crate::reshape::leaf_geometry(p))
        .fold(Budget { anchors: 0, lines: 0 }, |c, (g, _)| c.plus(Budget::of(&g)));
    assert_eq!(stroked_leaves(&art).len(), 2, "{cost:?}");
    assert!(Budget::EFFECTS.fits(cost), "{cost:?}");
}
