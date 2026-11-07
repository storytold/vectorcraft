//! Scale Strokes & Effects and Scale Corners: what scales with the art, from the `strokes` and
//! `corners` params or the preferences, and what the journal records.

use serde_json::{Value, json};
use vectorcraft_doc::{LiveShape, Node, NodeId, NodeKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// A selected 100 pt square with a 4 pt stroke dashed [6, 2] from offset 3 and a drop shadow.
fn shadowed_dashed_square(s: &mut Session) -> NodeId {
    let id = id_of(&run(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100})));
    run(s, "stroke.set", json!({"weight": 4, "dash": [6, 2], "dashOffset": 3}));
    run(s, "effect.apply", json!({"effect": "stylize.dropShadow", "params": {"x": 7, "y": 5}}));
    id
}

fn shadow(n: &Node, key: &str) -> f64 {
    n.appearance.effects[0].params[key].as_f64().unwrap()
}

#[test]
fn scaling_200_percent_doubles_the_stroke_dashes_and_shadow() {
    let mut s = session();
    let id = shadowed_dashed_square(&mut s);
    run(&mut s, "object.scale", json!({"sx": 200, "strokes": true}));
    let n = node(&s, id);
    let st = n.appearance.stroke().unwrap();
    let dash = st.dash.as_ref().unwrap();
    assert_eq!((st.width, dash.pattern.clone(), dash.offset), (8.0, vec![12.0, 4.0], 6.0));
    assert_eq!((shadow(&n, "x"), shadow(&n, "y")), (14.0, 10.0));
    // The blur was left at its default (5 pt): it scales too.
    assert_eq!(shadow(&n, "blur"), 10.0);
}

#[test]
fn off_nothing_visual_scales() {
    let mut s = session();
    let id = shadowed_dashed_square(&mut s);
    let before = node(&s, id).appearance;
    run(&mut s, "object.scale", json!({"sx": 200, "strokes": false}));
    let n = node(&s, id);
    assert_eq!(n.appearance.stroke(), before.stroke());
    assert_eq!(n.appearance.effects, before.effects);
    assert!((n.geometric_bounds().unwrap().width() - 200.0).abs() < 1e-9);
}

#[test]
fn the_preference_applies_when_the_param_is_absent() {
    let mut s = session();
    let id = shadowed_dashed_square(&mut s);
    run(&mut s, "prefs.set", json!({"key": "scaleStrokes", "value": false}));
    run(&mut s, "object.scale", json!({"sx": 50}));
    assert_eq!(node(&s, id).appearance.stroke_width(), 4.0);
    run(&mut s, "prefs.set", json!({"key": "scaleStrokes", "value": true}));
    run(&mut s, "object.scale", json!({"sx": 50}));
    assert_eq!(node(&s, id).appearance.stroke_width(), 2.0);
}

#[test]
fn relative_distances_and_fill_and_stroke_effects() {
    let mut s = session();
    let id = id_of(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100})));
    // Relative Roughen is a percentage of the size: it stays; an absolute one is a distance.
    run(&mut s, "effect.apply", json!({"effect": "distort.roughen", "params": {"size": 5, "relative": true}}));
    run(&mut s, "effect.apply", json!({"effect": "distort.zigZag", "params": {"size": 4, "relative": false}}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 3}, "item": 1}));
    run(&mut s, "object.transformEach", json!({"scaleH": 300, "scaleV": 300, "strokes": true}));
    let a = node(&s, id).appearance;
    assert_eq!(a.effects[0].params["size"], json!(5));
    assert_eq!(a.effects[1].params["size"], json!(12.0));
    assert_eq!(a.items[1].effects()[0].params["offset"], json!(9.0), "the stroke's own effect");
}

#[test]
fn transform_each_without_strokes_keeps_them() {
    let mut s = session();
    let id = shadowed_dashed_square(&mut s);
    run(&mut s, "object.transformEach", json!({"scaleH": 200, "scaleV": 200, "strokes": false}));
    let n = node(&s, id);
    assert_eq!((n.appearance.stroke_width(), shadow(&n, "x")), (4.0, 7.0));
    assert!((n.geometric_bounds().unwrap().width() - 200.0).abs() < 1e-9);
}

#[test]
fn off_compensates_the_type_stroke_on_scales_it_with_the_type() {
    let mut s = session();
    let id = id_of(&run(&mut s, "text.create", json!({"x": 20, "y": 100, "text": "Scale", "size": 24})));
    run(&mut s, "select.set", json!({ "ids": [id.0] }));
    run(&mut s, "paint.setStroke", json!({"color": "#ff0000"}));
    run(&mut s, "stroke.set", json!({"weight": 2, "dash": [4, 2], "dashOffset": 1}));
    let style = |s: &Session| match &node(s, id).kind {
        NodeKind::Text(t) => (t.xf.as_coeffs()[0], t.runs[0].style.clone()),
        _ => panic!("not type"),
    };
    run(&mut s, "object.scale", json!({"sx": 200, "strokes": false}));
    let (k, st) = style(&s);
    // Drawn in text space (×2), the weight and dashes look as before.
    assert_eq!(k, 2.0);
    let dash = st.stroke_dash.unwrap();
    assert_eq!((st.stroke_width * k, dash.pattern[0] * k, dash.offset * k), (2.0, 4.0, 1.0));
    run(&mut s, "object.scale", json!({"sx": 200, "strokes": true}));
    let (k, st) = style(&s);
    assert_eq!((k, st.stroke_width * k), (4.0, 4.0), "the type's scale doubles the stroke");
}

#[test]
fn group_appearance_scales_with_its_members() {
    let mut s = session();
    let a = id_of(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50})));
    let b = id_of(&run(&mut s, "shape.rectangle", json!({"x": 60, "y": 0, "width": 50, "height": 50})));
    run(&mut s, "select.set", json!({ "ids": [a.0, b.0] }));
    let g = id_of(&run(&mut s, "object.group", json!({})));
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "stylize.dropShadow", "params": {"x": 4, "y": 4}}));
    run(&mut s, "object.scale", json!({"sx": 50, "strokes": true}));
    let n = node(&s, g);
    assert_eq!(shadow(&n, "x"), 2.0);
    let NodeKind::Group { children, .. } = &n.kind else { panic!("not a group") };
    assert_eq!(children[0].appearance.stroke_width(), 0.5);
}

#[test]
fn scale_corners_keeps_or_scales_live_corner_radii() {
    let radius = |s: &Session, id| match node(s, id).live_shape() {
        Some(LiveShape::Rectangle { radii, xf, .. }) => radii[0] * xf.as_coeffs()[0],
        _ => panic!("not a live rectangle"),
    };
    let mut s = session();
    let id = id_of(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100, "radius": 10})));
    run(&mut s, "object.scale", json!({"sx": 200, "corners": false}));
    assert!((radius(&s, id) - 10.0).abs() < 1e-9, "corners keep their size");
    run(&mut s, "object.scale", json!({"sx": 200, "corners": true}));
    assert!((radius(&s, id) - 20.0).abs() < 1e-9);
    // The path follows the live shape.
    let n = node(&s, id);
    assert!((n.geometric_bounds().unwrap().width() - 400.0).abs() < 1e-6);
}

/// The extent along x and y of each corner arc of an upright live rounded rectangle's path.
fn corner_spans(n: &Node) -> Vec<(f64, f64)> {
    let NodeKind::Path { path, .. } = &n.kind else { panic!("not a path") };
    let a = &path.subpaths[0].anchors;
    assert_eq!(a.len(), 8, "a rounded rectangle");
    [(1, 2), (3, 4), (5, 6), (7, 0)].iter().map(|(i, j)| ((a[*j].p.x - a[*i].p.x).abs(), (a[*j].p.y - a[*i].p.y).abs())).collect()
}

/// Every corner is a circular arc of radius `r`.
fn round_corners(n: &Node, r: f64) -> bool {
    corner_spans(n).iter().all(|(x, y)| (x - r).abs() < 1e-9 && (y - r).abs() < 1e-9)
}

#[test]
fn live_corners_stay_circular_when_a_rectangle_is_scaled_unevenly() {
    // #291: a rounded square widened into a long rectangle keeps round corners.
    let mut s = session();
    let id = id_of(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 100, "height": 100, "radius": 10})));
    run(&mut s, "object.scale", json!({"sx": 300, "sy": 100, "corners": false}));
    let n = node(&s, id);
    assert!((n.geometric_bounds().unwrap().width() - 300.0).abs() < 1e-9);
    assert!(round_corners(&n, 10.0), "{:?}", corner_spans(&n));
    let Some(LiveShape::Rectangle { w, h, radii, .. }) = n.live_shape() else { panic!("not live") };
    assert!((w - 300.0).abs() < 1e-9 && (h - 100.0).abs() < 1e-9 && (radii[0] - 10.0).abs() < 1e-9, "the panels read document units");
    // Scale Corners on: the radius scales by the mean scale (√(2 × 0.5) = 1 here, 2 below).
    run(&mut s, "object.scale", json!({"sx": 200, "sy": 50, "corners": true}));
    assert!(round_corners(&node(&s, id), 10.0), "{:?}", corner_spans(&node(&s, id)));
    run(&mut s, "object.scale", json!({"sx": 100, "sy": 400, "corners": true}));
    assert!(round_corners(&node(&s, id), 20.0), "{:?}", corner_spans(&node(&s, id)));
    // Undo brings each step back.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert!(round_corners(&node(&s, id), 10.0));
    run(&mut s, "edit.undo", json!({}));
    assert!(round_corners(&node(&s, id), 10.0));
    assert!((node(&s, id).geometric_bounds().unwrap().width() - 100.0).abs() < 1e-9);
}

#[test]
fn a_bounding_box_drag_keeps_live_corners_circular() {
    // The Selection tool's handle drag previews `object.transform` from the shape as it began.
    let mut s = session();
    let id = id_of(&run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100, "radius": 25})));
    s.begin_interaction("Scale").unwrap();
    for sx in [1.5, 2.5, 4.0] {
        // Dragging the right-middle handle: scaled about the left edge.
        s.preview("object.transform", &json!({"matrix": [sx, 0, 0, 1, 50.0 - 50.0 * sx, 0]})).unwrap();
    }
    s.commit_interaction().unwrap();
    let n = node(&s, id);
    let b = n.geometric_bounds().unwrap();
    assert!((b.x0 - 50.0).abs() < 1e-9 && (b.width() - 400.0).abs() < 1e-9 && (b.height() - 100.0).abs() < 1e-9, "{b:?}");
    assert!(round_corners(&n, 25.0), "{:?}", corner_spans(&n));
}

#[test]
fn the_journal_records_the_scale_options_used() {
    let mut s = session();
    shadowed_dashed_square(&mut s);
    run(&mut s, "prefs.set", json!({"values": {"scaleStrokes": false, "scaleCorners": true}}));
    run(&mut s, "object.scale", json!({"sx": 150}));
    let (cmd, p) = s.journal.last().unwrap().clone();
    assert_eq!(cmd, "object.scale");
    assert_eq!(p, json!({"sx": 150, "strokes": false, "corners": true}));
    // An explicit value stays; moves don't scale, so they record nothing.
    run(&mut s, "object.scale", json!({"sx": 150, "strokes": true}));
    assert_eq!(s.journal.last().unwrap().1["strokes"], json!(true));
    run(&mut s, "object.move", json!({"dx": 5, "dy": 0}));
    assert_eq!(s.journal.last().unwrap().1, json!({"dx": 5, "dy": 0}));
    // A drag's preview is journaled at commit with the options too.
    s.begin_interaction("Scale").unwrap();
    s.preview("object.transform", &json!({"matrix": [2, 0, 0, 2, 0, 0]})).unwrap();
    s.commit_interaction().unwrap();
    let (cmd, p) = s.journal.last().unwrap().clone();
    assert_eq!((cmd.as_str(), &p["strokes"], &p["corners"]), ("object.transform", &json!(false), &json!(true)));
}

#[test]
fn replaying_the_journal_entry_scales_the_same_way() {
    let mut s = session();
    let id = shadowed_dashed_square(&mut s);
    run(&mut s, "prefs.set", json!({"key": "scaleStrokes", "value": true}));
    run(&mut s, "object.scale", json!({"sx": 200}));
    let (cmd, p) = s.journal.last().unwrap().clone();
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "prefs.set", json!({"key": "scaleStrokes", "value": false}));
    run(&mut s, &cmd, p);
    assert_eq!(node(&s, id).appearance.stroke_width(), 8.0);
}
