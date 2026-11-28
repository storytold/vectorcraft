//! The Transform effect's Transform Objects and Scale Strokes & Effects options (#885) through
//! commands: as the renderer draws them and as Expand Appearance bakes them, and files saved
//! before the options existed looking the same.

use serde_json::{Value, json};
use vectorcraft_doc::Node;

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

/// A selected 100 pt square at (50, 50) with no fill and a 4 pt black stroke.
fn stroked_square(s: &mut Session) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"none": true}));
    run(s, "paint.setStroke", json!({"color": "#000000"}));
    run(s, "stroke.set", json!({"weight": 4}));
    id
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

fn dark(px: [u8; 4]) -> bool {
    px[0] < 128 && px[3] > 128
}

/// Scale Strokes & Effects: the stroke of a square halved by the effect is drawn 2 pt wide with
/// the option, 4 pt without. The halved square's left edge is at x = 75: a 4 pt stroke covers the
/// pixel column at 73, a 2 pt one doesn't.
#[test]
fn scale_strokes_and_effects_scales_the_drawn_stroke() {
    for strokes in [false, true] {
        let mut s = session();
        stroked_square(&mut s);
        run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"scaleH": 50, "scaleV": 50, "strokes": strokes}}));
        let img = render(&s);
        assert!(dark(img.pixel(75, 100)), "strokes={strokes}: the edge itself is stroked");
        assert_eq!(dark(img.pixel(73, 100)), !strokes, "strokes={strokes}: a 4 pt stroke reaches x = 73, a 2 pt one doesn't");
        assert!(!dark(img.pixel(50, 100)), "strokes={strokes}: the original edge is gone");
    }
}

/// Expand Appearance bakes the option: the outlined stroke is 2 pt wide (bounds 74..126) with
/// it, 4 pt (73..127) without.
#[test]
fn expand_appearance_bakes_the_scaled_stroke() {
    for strokes in [false, true] {
        let mut s = session();
        let id = stroked_square(&mut s);
        run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"scaleH": 50, "scaleV": 50, "strokes": strokes}}));
        run(&mut s, "effect.expandAppearance", json!({}));
        let b = node(&s, id).visual_bounds().unwrap();
        let want = if strokes { 52.0 } else { 54.0 };
        assert!((b.width() - want).abs() < 1e-6 && (b.x0 - (100.0 - want / 2.0)).abs() < 1e-6, "strokes={strokes}: {b:?}");
    }
}

/// Transform Objects off: the geometry stays where it is (the effect then only reaches patterns
/// and strokes).
#[test]
fn transform_objects_off_keeps_the_geometry_in_place() {
    for objects in [true, false] {
        let mut s = session();
        let id = stroked_square(&mut s);
        run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 60, "objects": objects}}));
        let img = render(&s);
        assert_eq!(dark(img.pixel(50, 100)), !objects, "objects={objects}: the original left edge");
        assert_eq!(dark(img.pixel(110, 100)), objects, "objects={objects}: the moved left edge");
        // The object itself never moves: the effect is live.
        assert_eq!(node(&s, id).geometric_bounds().unwrap().x0, 50.0);
    }
}

/// An effect saved before the options existed (no `objects`, `patterns`, `strokes` keys) draws
/// exactly as one with the defaults written out, and the dialog defaults carry them.
#[test]
fn old_transform_effects_draw_as_before() {
    let mut old = session();
    stroked_square(&mut old);
    run(&mut old, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 20, "scaleH": 150, "rotate": 15, "copies": 2}}));
    let mut explicit = session();
    stroked_square(&mut explicit);
    run(
        &mut explicit,
        "effect.apply",
        json!({"effect": "distort.transform", "params": {"moveH": 20, "scaleH": 150, "rotate": 15, "copies": 2, "objects": true, "patterns": false, "strokes": false}}),
    );
    assert_eq!(render(&old).pixels, render(&explicit).pixels);
    let list = run(&mut old, "effect.list", json!({}));
    let entry = list["catalog"].as_array().unwrap().iter().find(|e| e["id"] == "distort.transform").unwrap();
    assert_eq!(
        (&entry["defaults"]["objects"], &entry["defaults"]["patterns"], &entry["defaults"]["strokes"]),
        (&json!(true), &json!(false), &json!(false))
    );
    assert!(entry["params"].as_str().unwrap().contains("Scale Strokes & Effects"));
}

/// A group's Transform effect with Scale Strokes & Effects scales its members' strokes (#885
/// review): two 40 pt squares with 4 pt strokes halved as one piece about the group's centre;
/// the first square's left edge lands at x = 75, where a 4 pt stroke covers x = 73 and a 2 pt one
/// doesn't.
#[test]
fn a_group_transform_scales_its_members_strokes() {
    for strokes in [false, true] {
        let mut s = session();
        for x in [50, 110] {
            run(&mut s, "shape.rectangle", json!({"x": x, "y": 50, "width": 40, "height": 40}));
            run(&mut s, "paint.setFill", json!({"none": true}));
            run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
            run(&mut s, "stroke.set", json!({"weight": 4}));
        }
        run(&mut s, "select.all", json!({}));
        run(&mut s, "object.group", json!({}));
        run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"scaleH": 50, "scaleV": 50, "strokes": strokes}}));
        let img = render(&s);
        assert!(dark(img.pixel(75, 70)), "strokes={strokes}: the moved edge is stroked");
        assert_eq!(dark(img.pixel(73, 70)), !strokes, "strokes={strokes}: a 4 pt stroke reaches x = 73, a 2 pt one doesn't");
        assert!(!dark(img.pixel(50, 70)), "strokes={strokes}: the original edge is gone");
    }
}

/// The canvas and Expand Appearance (what export bakes) paint copies with scaled strokes alike
/// (#885 review): a 2 pt stroke, 150 % per copy, two copies.
#[test]
fn live_copies_and_their_expanded_art_paint_alike() {
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 20, "y": 20, "width": 30, "height": 30}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setFill", json!({"none": true}));
    run(&mut s, "paint.setStroke", json!({"color": "#000000"}));
    run(&mut s, "stroke.set", json!({"weight": 2}));
    run(
        &mut s,
        "effect.apply",
        json!({"effect": "distort.transform", "params": {"scaleH": 150, "scaleV": 150, "moveH": 30, "moveV": 30, "copies": 2, "reference": 0, "strokes": true}}),
    );
    let live = render(&s);
    // The copies' strokes are 2, 3 and 4.5 pt: the last copy's edge at (20 + 30·2.5, …) is wider.
    run(&mut s, "effect.expandAppearance", json!({}));
    let expanded = render(&s);
    let differ = live.pixels.chunks(4).zip(expanded.pixels.chunks(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 96)).count();
    let inked = live.pixels.chunks(4).filter(|p| p[0] < 128).count();
    assert!(inked > 500, "{inked} dark pixels");
    assert!(differ * 50 < inked, "{differ} of {inked} pixels differ");
    // Each copy's outlined stroke is an object of its own (a compound path), whatever groups hold
    // them.
    fn pieces(n: &Node) -> usize {
        match n.children() {
            Some(c) if !matches!(n.kind, vectorcraft_doc::NodeKind::Compound { .. }) => c.iter().map(|c| pieces(c)).sum(),
            _ => 1,
        }
    }
    assert_eq!(pieces(&node(&s, id)), 3, "one piece per copy");
}
