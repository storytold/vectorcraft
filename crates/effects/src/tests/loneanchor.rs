//! Geometry effects on a path with a lone anchor work from the path's bounds with that anchor
//! included ([`PathData::bounds`]), as the selection's box, Align and Transform Each measure the
//! path, wherever the effects are applied.

use std::sync::Arc;

use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Node, NodeId, NodeKind, Symbol};

use super::*;

/// A 10 pt line from the origin and a lone anchor at (100, 50): its bounds are (0, 0) to
/// (100, 50), and the line alone measures (0, 0) to (10, 0).
fn marked() -> PathData {
    PathData::new(vec![
        SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(10.0, 0.0)], false),
        SubPath::polyline(&[Point::new(100.0, 50.0)], false),
    ])
}

/// The Transform effect at 200%, about the center of the box it is given.
fn scale() -> Effect {
    fx("distort.transform", json!({"scaleH": 200, "scaleV": 200}))
}

/// `marked` with a 1 pt black stroke, and `scale` on the object (`on_stroke` false) or on its
/// stroke.
fn node(on_stroke: bool) -> Node {
    let mut n = Node::path(NodeId(5), marked(), Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0));
    if on_stroke {
        if let Some(s) = n.appearance.stroke_mut() {
            s.effects.push(scale());
        }
    } else {
        n.appearance.effects.push(scale());
    }
    n
}

/// The first point of each subpath of each path under `n`.
fn starts(n: &Node, out: &mut Vec<Point>) {
    if let NodeKind::Path { path, .. } = &n.kind {
        out.extend(path.subpaths.iter().filter_map(|s| s.anchors.first()).map(|a| a.p));
    }
    for c in n.children().into_iter().flatten() {
        starts(c, out);
    }
}

/// Does a path under `n` start where the line scaled about (50, 25) starts, (-50, -25), and
/// none where the line scaled about its own center (5, 0) starts, (-5, 0)?
fn scaled_about_the_bounds(n: &Node) -> bool {
    let mut pts = vec![];
    starts(n, &mut pts);
    let at = |x: f64, y: f64| pts.iter().any(|p| close(p.x, x, 1e-6) && close(p.y, y, 1e-6));
    assert_eq!(apply_geometry(&[scale()], &marked(), marked().bounds().unwrap()).subpaths[0].anchors[0].p, Point::new(-50.0, -25.0));
    at(-50.0, -25.0) && !at(-5.0, 0.0)
}

#[test]
fn expand_appearance_scales_about_bounds_with_the_lone_anchor() {
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    d.insert(Some(l), 0, node(false)).unwrap();
    let baked = bake_document(&d).expect("the effect bakes");
    let layer = &baked.layers[0];
    assert!(scaled_about_the_bounds(layer), "{layer:?}");
}

/// A stroke's Transform effect with Scale Strokes & Effects on makes the stroke a path of its own
/// when the object's own effects are reshaped, its geometry after the stroke's effects before it.
#[test]
fn a_stroke_effect_scales_about_bounds_with_the_lone_anchor() {
    let mut n = node(true);
    n.appearance.effects = vec![fx("distort.transform", json!({}))];
    if let Some(s) = n.appearance.stroke_mut() {
        s.effects.push(fx("distort.transform", json!({"strokes": true})));
    }
    let art = reshape(None, &n, None).expect("the effects make art");
    assert!(scaled_about_the_bounds(&art), "{art:?}");
}

#[test]
fn shape_art_scales_about_bounds_with_the_lone_anchor() {
    let art = shape_art(None, &node(false), None).expect("the effect makes art");
    assert!(scaled_about_the_bounds(&art), "{art:?}");
}

#[test]
fn preview_art_scales_about_bounds_with_the_lone_anchor() {
    let d = Document::new(200.0, 200.0);
    let symbols = d.symbol_boxes();
    for on_stroke in [false, true] {
        let art = crate::preview::drawn_art(&symbols, &node(on_stroke)).expect("the effect makes art");
        assert!(scaled_about_the_bounds(&art), "on the stroke {on_stroke}: {art:?}");
    }
}

#[test]
fn an_instance_paints_its_art_scaled_about_bounds_with_the_lone_anchor() {
    let mut d = Document::new(200.0, 200.0);
    d.symbols.push(Symbol { dynamic: false, name: "s".into(), art: Arc::new(node(true)) });
    let mut inst = Node::new(NodeId(9), NodeKind::SymbolInstance { symbol: "s".into(), xf: Affine::IDENTITY });
    inst.appearance = Appearance::basic(Paint::None, Paint::solid(Color::BLACK), 1.0).into();
    if let Some(s) = inst.appearance.stroke_mut() {
        s.effects.push(fx("distort.transform", json!({})));
    }
    let painted = image_paint(Some(&d), &inst);
    let pts: Vec<Point> = painted
        .iter()
        .filter_map(|(bp, _, _)| match bp.elements().first() {
            Some(vectorcraft_geom::PathEl::MoveTo(p)) => Some(*p),
            _ => None,
        })
        .collect();
    assert!(pts.iter().any(|p| close(p.x, -50.0, 1e-6) && close(p.y, -25.0, 1e-6)), "{pts:?}");
    assert!(!pts.iter().any(|p| close(p.x, -5.0, 1e-6) && close(p.y, 0.0, 1e-6)), "{pts:?}");
}
