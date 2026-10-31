//! The Transform effect's Transform Objects, Transform Patterns and Scale Strokes & Effects
//! options (#885).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Dash, Effect, Node, NodeId, NodeKind};
use vectorcraft_geom::{Affine, Rect, shapes};

use crate::distort::{TransformOptions, transform_map, transform_move};
use crate::{has_transform_art, has_transform_options, reshape};

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
