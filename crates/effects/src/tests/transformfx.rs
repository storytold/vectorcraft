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
    assert_eq!(widths, vec![8.0, 4.0, 2.0], "the original on top");
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
    assert_eq!(xs, vec![60.0, 30.0, 0.0], "the original on top");
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
    assert_eq!(widths, vec![8.0, 4.0, 2.0], "the original on top");
}

#[test]
fn a_fills_own_transform_with_scale_strokes_and_effects_leaves_its_later_effects() {
    let mut n = stroke_with_effect(json!({}));
    if let Some(AppearanceItem::Fill(f)) = n.appearance.items.first_mut() {
        f.effects.push(transform(json!({"scaleH": 300, "scaleV": 300, "strokes": true})));
        f.effects.push(Effect { id: "path.offsetPath".into(), params: json!({"offset": 5.0}), visible: true });
    }
    let art = reshape(None, &n, None).unwrap();
    let fill = leaves(&art).into_iter().find(|p| p.appearance.stroke().is_none()).unwrap();
    let AppearanceItem::Fill(f) = &fill.appearance.items[0] else { panic!("not a fill") };
    // The Offset Path applies to the transformed fill's path at its own 5 pt.
    assert!(f.effects.is_empty(), "{:?}", f.effects);
    let w = fill.geometric_bounds().unwrap().width();
    assert!(close(w, 310.0), "{w}");
}

#[test]
fn a_later_transform_on_a_stroke_turns_its_copies_about_their_combined_center() {
    for later in [json!({"rotate": 90}), json!({"rotate": 90, "strokes": true})] {
        let mut n = stroke_with_effect(json!({"moveH": 200, "copies": 1, "strokes": true}));
        if let Some(s) = n.appearance.stroke_mut() {
            s.effects.push(transform(later.clone()));
        }
        let art = reshape(None, &n, None).unwrap();
        let paths = stroked_leaves(&art);
        assert_eq!(paths.len(), 2, "{later}");
        // The copies span 0 to 300 by 0 to 100 before the turn; about the center (150, 50) both
        // go to x 100 to 200, one above the other.
        let boxes: Vec<Rect> = paths.iter().map(|p| p.geometric_bounds().unwrap()).collect();
        for b in &boxes {
            assert!(close(b.x0, 100.0) && close(b.x1, 200.0), "{later}: {boxes:?}");
        }
        let top = boxes.iter().map(|b| b.y0).fold(f64::INFINITY, f64::min);
        let bottom = boxes.iter().map(|b| b.y1).fold(f64::NEG_INFINITY, f64::max);
        assert!(close(top, -100.0) && close(bottom, 200.0), "{later}: {boxes:?}");
        for p in &paths {
            assert!(p.appearance.stroke().unwrap().effects.is_empty(), "{later}");
        }
    }
}

#[test]
fn a_fills_own_transform_moves_each_copys_pattern_with_transform_patterns() {
    let n = fill_with_effect(json!({"moveH": 30, "copies": 2, "patterns": true}));
    assert!(has_transform_options(&n));
    let art = reshape(None, &n, None).unwrap();
    let xs: Vec<f64> = pattern_leaves(&art).iter().map(|p| pattern_xf(p).as_coeffs()[4]).collect();
    assert_eq!(xs, vec![60.0, 30.0, 0.0], "the original on top");
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
    assert_eq!(widths(&drawn), vec![4.0, 2.0], "the original on top");
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

/// A path of 600,000 anchors with a stroke whose own Transform effect makes two copies, below a
/// fill: the stroke's copies leave room for the fill painted after them.
#[test]
fn a_strokes_own_transform_leaves_room_for_a_fill_above_it() {
    use crate::geometry::Budget;
    use vectorcraft_geom::{PathData, Point, SubPath};
    let points: Vec<Point> = (0..600_000).map(|i| Point::new(f64::from(i) * 1.0e-4, f64::from(i % 2))).collect();
    let gray = Paint::solid(Color::rgb(0.5, 0.5, 0.5));
    let mut n = Node::path(NodeId(13), PathData::single(SubPath::polyline(&points, false)), Appearance::basic(gray, Paint::solid(Color::BLACK), 1.0));
    if let Some(s) = n.appearance.stroke_mut() {
        s.effects.push(transform(json!({"moveH": 100, "copies": 2, "strokes": true})));
    }
    n.appearance.items.reverse();
    assert!(matches!(n.appearance.items.first(), Some(AppearanceItem::Stroke(_))));
    let art = reshape(None, &n, None).unwrap();
    let cost = leaves(&art)
        .iter()
        .filter_map(|p| crate::reshape::leaf_geometry(p))
        .fold(Budget { anchors: 0, lines: 0 }, |c, (g, _)| c.plus(Budget::of(&g)));
    assert_eq!(stroked_leaves(&art).len(), 2, "{cost:?}");
    assert!(Budget::EFFECTS.fits(cost), "{cost:?}");
}

/// A 100 pt square at the origin painted with `fill` and a 1 pt white stroke, with the Transform
/// effect `params`.
fn filled_and_stroked(fill: Paint, params: Value) -> Node {
    let mut n = Node::path(NodeId(9), shapes::rectangle(Rect::new(0.0, 0.0, 100.0, 100.0)), Appearance::basic(fill, Paint::solid(Color::WHITE), 1.0));
    n.appearance.effects.push(transform(params));
    n
}

#[test]
fn copies_with_a_fill_and_a_stroke_are_drawn_one_below_another() {
    // Black fill, white stroke, each copy scaled and turned (#5920, #6316): each copy is an object
    // of its own below the one before, so the fills above cover the strokes of the copies below.
    let params = json!({"scaleH": 97, "scaleV": 97, "rotate": 10, "copies": 25});
    let n = filled_and_stroked(Paint::solid(Color::BLACK), params.clone());
    assert!(has_transform_art(&n));
    let art = reshape(None, &n, None).unwrap();
    let paths = leaves(&art);
    assert_eq!(paths.len(), 26, "the original and 25 copies");
    for p in &paths {
        assert!(matches!(p.kind, NodeKind::Path { .. }), "one shape each, not one path of every copy");
        assert!(!p.appearance.fill_paint().is_none() && p.appearance.stroke().is_some());
    }
    // The smallest copy is at the bottom, the original on top.
    assert_eq!(paths[25].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    let bottom = paths[0].geometric_bounds().unwrap();
    assert!(bottom.width() < 100.0 && bottom.height() < 100.0, "{bottom:?}");
    // Drawn as Expand Appearance makes it.
    assert_eq!(leaves(&expand_outlined(None, &n, None).unwrap()).len(), 26);

    // One opaque stroke, or one nonzero fill, paints the copies alike as one path: no art.
    assert!(!has_transform_art(&filled_and_stroked(Paint::None, params.clone())));
    let mut fill_only = filled_and_stroked(Paint::solid(Color::BLACK), params.clone());
    fill_only.appearance.items.retain(AppearanceItem::is_fill);
    assert!(!has_transform_art(&fill_only));
    // A partly transparent fill paints where copies overlap twice.
    if let Some(AppearanceItem::Fill(f)) = fill_only.appearance.items.first_mut() {
        f.opacity = 0.5;
    }
    assert!(has_transform_art(&fill_only));
    // No copies: nothing to stack.
    assert!(!has_transform_art(&filled_and_stroked(Paint::solid(Color::BLACK), json!({"rotate": 10}))));
}

/// A group of two squares side by side, each with a black fill and a white stroke, with the
/// Transform effect `params`.
fn two_squares(params: Value) -> Node {
    let square = |id, x| {
        let a = Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::WHITE), 4.0);
        std::sync::Arc::new(Node::path(NodeId(id), shapes::rectangle(Rect::new(x, 0.0, x + 100.0, 100.0)), a))
    };
    let mut g = Node::group(NodeId(20), vec![square(21, 0.0), square(22, 120.0)]);
    g.appearance.effects.push(transform(params));
    g
}

#[test]
fn a_groups_copies_are_whole_groups_below_the_original() {
    let art = crate::evaluate_container(None, &two_squares(json!({"scaleH": 95, "scaleV": 95, "copies": 10}))).unwrap();
    let copies = art.children().unwrap();
    assert_eq!(copies.len(), 11, "the original and 10 copies");
    for c in copies {
        assert_eq!(c.children().map(Vec::len), Some(2), "each copy holds both squares");
    }
    // The smallest copy is at the bottom, the original on top with its members in their order.
    let width = |i: usize| copies[i].geometric_bounds().unwrap().width();
    assert!((0..10).all(|i| width(i) < width(i + 1)), "{:?}", (0..11).map(width).collect::<Vec<_>>());
    assert_eq!(copies[10].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 220.0, 100.0));
    let members = copies[10].children().unwrap();
    assert_eq!(members[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0));
    assert_eq!(members[1].geometric_bounds().unwrap(), Rect::new(120.0, 0.0, 220.0, 100.0));

    // Without copies the members stay members of the art.
    let art = crate::evaluate_container(None, &two_squares(json!({"scaleH": 50, "scaleV": 50}))).unwrap();
    assert_eq!(art.children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>(), [NodeId(21), NodeId(22)]);

    // A group's own fill paints the shape of all its copies, so each member keeps its place.
    let mut own_fill = two_squares(json!({}));
    own_fill.appearance = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0).into();
    let copies = transform(json!({"moveH": 300, "copies": 2}));
    crate::reshape::reshape_leaves_within(&mut own_fill, &[&copies], crate::geometry::Budget::EFFECTS, None);
    assert_eq!(own_fill.children().map(Vec::len), Some(2), "a stack of copies per member");

    // A guide in the group stays one guide.
    let mut guided = two_squares(json!({}));
    let mut guide = Node::path(NodeId(23), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
    if let NodeKind::Path { guide: on, .. } = &mut guide.kind {
        *on = true;
    }
    if let Some(members) = guided.children_mut() {
        members.push(std::sync::Arc::new(guide));
    }
    guided.appearance.effects.clear();
    crate::reshape::reshape_leaves_within(&mut guided, &[&copies], crate::geometry::Budget::EFFECTS, None);
    assert_eq!(guided.children().map(Vec::len), Some(3), "the original and 2 copies");
    let mut guides = 0;
    guided.walk(&mut |n| guides += usize::from(matches!(n.kind, NodeKind::Path { guide: true, .. })));
    assert_eq!(guides, 1);

    // A clipping group's first member stays its clip.
    let mut clipping = two_squares(json!({}));
    if let NodeKind::Group { clip, .. } = &mut clipping.kind {
        *clip = true;
    }
    clipping.appearance.effects.clear();
    crate::reshape::reshape_leaves_within(&mut clipping, &[&copies], crate::geometry::Budget::EFFECTS, None);
    assert_eq!(clipping.children().map(Vec::len), Some(2), "the clip and the clipped member");
}

#[test]
fn a_groups_copies_stop_at_the_node_limit() {
    // Empty groups cost no anchors, so only the count of nodes stops their copies.
    let empties = (0..5_000).map(|i| std::sync::Arc::new(Node::group(NodeId(100 + i), vec![])));
    let mut g = two_squares(json!({}));
    if let Some(members) = g.children_mut() {
        members.extend(empties);
    }
    g.appearance.effects.clear();
    let copies = transform(json!({"moveH": 1, "copies": 1000}));
    crate::reshape::reshape_leaves_within(&mut g, &[&copies], crate::geometry::Budget::EFFECTS, None);
    let mut nodes = 0usize;
    g.walk(&mut |_| nodes += 1);
    assert!(nodes <= crate::distort::MAX_COPY_OBJECTS + 1, "{nodes}");
    assert!(g.children().unwrap().len() < 1001, "the limit stopped the copies");
}

#[test]
fn a_groups_copies_count_the_art_of_opacity_masks() {
    let empties = (0..5_000).map(|i| std::sync::Arc::new(Node::group(NodeId(100 + i), vec![]))).collect();
    let mut g = two_squares(json!({}));
    if let Some(members) = g.children_mut() {
        let mask = vectorcraft_doc::OpacityMask::new(Node::group(NodeId(99), empties), true);
        std::sync::Arc::make_mut(&mut members[0]).extras_mut().mask = Some(Box::new(mask));
    }
    g.appearance.effects.clear();
    let copies = transform(json!({"moveH": 1, "copies": 1000}));
    crate::reshape::reshape_leaves_within(&mut g, &[&copies], crate::geometry::Budget::EFFECTS, None);
    let nodes = g.count_with_masks();
    assert!(nodes <= crate::distort::MAX_COPY_OBJECTS + 1, "{nodes}");
}

/// The anchors `n` draws with every effect inside it evaluated, group by group and member by
/// member as drawing and baking evaluate them. The count stops once it passes `limit`.
fn drawn_anchors(n: &Node, limit: usize) -> usize {
    fn count(n: &Node, limit: usize, total: &mut usize) {
        if *total > limit {
            return;
        }
        if n.children().is_some() && !matches!(n.kind, NodeKind::Compound { .. }) {
            let art = crate::evaluate_container(None, n).unwrap_or_else(|| n.clone());
            art.children().into_iter().flatten().for_each(|c| count(c, limit, total));
            return;
        }
        if let Some(art) = reshape(None, n, None) {
            count(&art, limit, total);
            return;
        }
        let Some((path, _)) = crate::reshape::leaf_geometry(n) else { return };
        let Some(bounds) = path.bounds() else { return };
        let own = crate::apply_geometry(&n.appearance.effects, &path, bounds);
        let items: Vec<usize> = n
            .appearance
            .items
            .iter()
            .filter(|i| crate::has_geometry(i.effects()))
            .filter_map(|i| Some(crate::apply_geometry(i.effects(), &own, own.bounds()?).anchor_count()))
            .collect();
        *total += if items.is_empty() { own.anchor_count() } else { items.iter().sum() };
    }
    let mut total = 0;
    count(n, limit, &mut total);
    total
}

#[test]
fn a_groups_copies_keep_what_the_effects_inside_them_make_to_the_budget() {
    let most = crate::geometry::MAX_EFFECT_ANCHORS;
    let copies = |key: &str| transform(json!({key: 1, "copies": 1000}));

    // Copies of a group whose member group makes copies of its own.
    let inner = two_squares(json!({"moveH": 1, "copies": 1000}));
    let mut outer = Node::group(NodeId(30), vec![std::sync::Arc::new(inner)]);
    outer.appearance.effects.push(copies("moveV"));
    let drawn = drawn_anchors(&outer, 2 * most);
    assert!(drawn <= most, "a group inside: {drawn}");

    // Copies of a group whose member path makes copies of its own.
    let mut own = two_squares(json!({"moveH": 1, "copies": 1000}));
    if let Some(members) = own.children_mut() {
        std::sync::Arc::make_mut(&mut members[0]).appearance.effects.push(copies("moveV"));
    }
    let drawn = drawn_anchors(&own, 2 * most);
    assert!(drawn <= most, "a path's own effect: {drawn}");

    // Copies of a group whose member's fill makes copies of its own.
    let mut item = two_squares(json!({"moveH": 1, "copies": 1000}));
    if let Some(members) = item.children_mut() {
        std::sync::Arc::make_mut(&mut members[0]).appearance.items[0].effects_mut().push(copies("moveV"));
    }
    let drawn = drawn_anchors(&item, 2 * most);
    assert!(drawn <= most, "a fill's own effect: {drawn}");
}

#[test]
fn copies_with_scaled_effects_keep_what_each_one_makes_to_the_budget() {
    // Each copy's Roughen is twice as large as the last's, and the rough edges take more lines.
    let roughen = Effect { id: "distort.roughen".into(), params: json!({"size": 40, "relative": false, "detail": 20}), visible: true };
    let lines = |budget: crate::geometry::Budget| {
        let mut g = two_squares(json!({}));
        if let Some(members) = g.children_mut() {
            let m = std::sync::Arc::make_mut(&mut members[0]);
            m.appearance.items.retain(AppearanceItem::is_fill);
            m.appearance.effects.push(roughen.clone());
            members.truncate(1);
        }
        g.appearance.effects.clear();
        let copies = transform(json!({"scaleH": 200, "scaleV": 200, "copies": 5, "strokes": true}));
        crate::reshape::reshape_leaves_within(&mut g, &[&copies], budget, None);
        let (mut count, mut lines) = (0, 0);
        for leaf in leaves(&g) {
            count += 1;
            let Some((path, _)) = crate::reshape::leaf_geometry(leaf) else { continue };
            let drawn = crate::apply_geometry(&leaf.appearance.effects, &path, path.bounds().unwrap());
            lines += crate::geometry::Budget::of(&drawn).lines;
        }
        (count, lines)
    };
    let (all, most) = lines(crate::geometry::Budget::EFFECTS);
    assert_eq!(all, 6, "the original and 5 copies");
    let cap = most / 2;
    let budget = crate::geometry::Budget { anchors: crate::geometry::MAX_EFFECT_ANCHORS, lines: cap };
    let (count, drawn) = lines(budget);
    assert!(count < all, "the budget stopped the copies");
    assert!(drawn <= cap, "{drawn} > {cap}");
}

#[test]
fn a_groups_copies_are_whole_only_when_nothing_inside_makes_more_art() {
    let copies = transform(json!({"moveH": 300, "copies": 2}));
    let effect = |id: &str| Effect { id: id.into(), params: json!({}), visible: true };
    let roughen = effect("distort.roughen");
    // Two squares under a group with `change` made to the first member.
    let whole = |change: &dyn Fn(&mut Node)| {
        let mut g = two_squares(json!({}));
        if let Some(members) = g.children_mut() {
            change(std::sync::Arc::make_mut(&mut members[0]));
        }
        g.appearance.effects.clear();
        crate::reshape::reshape_leaves_within(&mut g, &[&copies], crate::geometry::Budget::EFFECTS, None);
        g.children().unwrap().len() == 3
    };
    // A group of one square with effect `e`, in place of the member.
    let in_group = |m: &mut Node, e: &Effect| {
        let square = Node::path(NodeId(40), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Appearance::default());
        let mut g = Node::group(NodeId(41), vec![std::sync::Arc::new(square), std::sync::Arc::new(m.clone())]);
        g.appearance.effects.push(e.clone());
        *m = g;
    };
    assert!(whole(&|_| {}));
    // Effects that only change pixels, and effects turned off, are copied with the art.
    assert!(whole(&|m| m.appearance.effects.push(effect("stylize.dropShadow"))));
    assert!(whole(&|m| m.appearance.effects.push(Effect { visible: false, ..roughen.clone() })));
    // Effects that draw nothing are copied with the art as well.
    assert!(
        whole(&|m| {
            m.visible = false;
            m.appearance.effects.push(roughen.clone());
        }),
        "a hidden member's effect"
    );
    assert!(
        whole(&|m| {
            *m.appearance.items[0].paint_mut() = Paint::None;
            m.appearance.items[0].effects_mut().push(roughen.clone());
        }),
        "the effect of a fill that paints nothing"
    );
    assert!(whole(&|m| m.appearance.effects.push(effect("no.such.effect"))), "an unknown effect");
    // Effects that make more art than a copy holds keep each member's copies in its place.
    assert!(!whole(&|m| m.appearance.effects.push(roughen.clone())), "a member's own effect");
    assert!(!whole(&|m| m.appearance.items[0].effects_mut().push(roughen.clone())), "a fill's own effect");
    assert!(!whole(&|m| in_group(m, &roughen)), "a group's own effect");
    assert!(!whole(&|m| in_group(m, &effect("pathfinder.add"))), "a Pathfinder effect");
    assert!(
        !whole(&|m| {
            let mut art = Node::group(NodeId(43), vec![]);
            in_group(&mut art, &roughen);
            m.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(art, true)));
        }),
        "an effect in an opacity mask"
    );
}
