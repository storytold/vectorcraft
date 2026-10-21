//! Use Preview Bounds: the Transform panel values, the bounding box, Align and Distribute measure
//! visual bounds (strokes and geometry effects included) when it is on.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, Document, Effect, Node, NodeId, NodeKind, Symbol};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_tools::{Cursor, Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// A rectangle with a `weight` pt stroke (0: no stroke).
fn rect(s: &mut Session, x: f64, w: f64, weight: f64) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 100, "width": w, "height": 50}))["id"].as_u64().unwrap());
    if weight > 0.0 {
        run(s, "stroke.set", json!({"weight": weight}));
    } else {
        run(s, "paint.setStroke", json!({"none": true}));
    }
    id
}

fn geometric(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

fn preview(s: &mut Session, on: bool) {
    run(s, "prefs.set", json!({"key": "usePreviewBounds", "value": on}));
}

/// Two rectangles: one with a 10 pt stroke at x = 100, an unstroked one at x = 20; align left.
fn aligned_left(on: bool, p: Value) -> f64 {
    let mut s = session();
    let a = rect(&mut s, 100.0, 50.0, 10.0);
    rect(&mut s, 20.0, 50.0, 0.0);
    preview(&mut s, on);
    run(&mut s, "select.all", json!({}));
    let mut p = p;
    p["horizontal"] = json!("left");
    run(&mut s, "object.align", p);
    geometric(&s, a).x0
}

#[test]
fn align_left_differs_by_half_the_stroke() {
    assert_eq!(aligned_left(false, json!({})), 20.0);
    // The 10 pt stroke's outer edge lines up with the other object: the path sits 5 pt in.
    assert_eq!(aligned_left(true, json!({})), 25.0);
    // `bounds` overrides the preference either way.
    assert_eq!(aligned_left(true, json!({"bounds": "geometric"})), 20.0);
    assert_eq!(aligned_left(false, json!({"bounds": "preview"})), 25.0);
    let mut s = session();
    rect(&mut s, 0.0, 10.0, 0.0);
    assert!(s.execute("object.align", &json!({"horizontal": "left", "bounds": "visual"})).is_err());
}

#[test]
fn distribute_spacing_measures_preview_bounds() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 50.0, 10.0);
    let b = rect(&mut s, 200.0, 50.0, 10.0);
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "object.distributeSpacing", json!({"axis": "horizontal", "spacing": 0}));
    // The strokes touch: the paths are a stroke weight apart.
    assert_eq!(geometric(&s, b).x0 - geometric(&s, a).x1, 10.0);
}

/// A 100 pt square at (`x`, 100) without a stroke, with Offset Path 10.
fn offset_square(s: &mut Session, x: f64) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 100, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setStroke", json!({"none": true}));
    run(s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "ids": [id.0], "item": null}));
    id
}

/// Point type `text` (48 pt) at (`x`, 100) with Outline Object.
fn outlined_type(s: &mut Session, x: f64, text: &str) -> NodeId {
    let id = NodeId(run(s, "text.create", json!({"x": x, "y": 100, "text": text, "size": 48}))["id"].as_u64().unwrap());
    run(s, "effect.apply", json!({"effect": "path.outlineObject", "ids": [id.0], "item": null}));
    id
}

/// The box of the glyph outlines that type `id` with Outline Object is drawn as.
fn glyphs(s: &Session, id: NodeId) -> Rect {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    vectorcraft_render::effects::reshape(&n, None).unwrap().geometric_bounds().unwrap()
}

fn near(a: Rect, b: Rect, tolerance: f64) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() <= tolerance)
}

#[test]
fn a_square_with_offset_path_is_measured_by_the_offset_shape() {
    let mut s = session();
    let a = offset_square(&mut s, 100.0);
    preview(&mut s, true);
    assert!(near(s.transform_bounds(&[a]).unwrap(), Rect::new(90.0, 90.0, 210.0, 210.0), 1e-6), "{:?}", s.transform_bounds(&[a]));
    // Align left with an unstroked rectangle at x 20: the offset shape's left edge lines up with it.
    rect(&mut s, 20.0, 50.0, 0.0);
    run(&mut s, "select.all", json!({}));
    run(&mut s, "object.align", json!({"horizontal": "left"}));
    assert!((geometric(&s, a).x0 - 30.0).abs() < 1e-6, "{:?}", geometric(&s, a));
    // Distribute Spacing 0: the offset shapes touch, so the squares are 20 pt apart.
    let mut s = session();
    let (a, b) = (offset_square(&mut s, 0.0), offset_square(&mut s, 300.0));
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "object.distributeSpacing", json!({"axis": "horizontal", "spacing": 0}));
    assert!((geometric(&s, b).x0 - geometric(&s, a).x1 - 20.0).abs() < 1e-6);
}

#[test]
fn a_stroke_with_offset_path_of_its_own_is_measured_where_it_paints() {
    // A 100 pt square with a 10 pt stroke whose own Offset Path 10 moves it 10 pt out: the stroke
    // paints from 15 pt outside the square to 5 pt outside it.
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "stroke.set", json!({"weight": 10}));
    let items = s.doc().unwrap().doc.node(id).unwrap().appearance.items.clone();
    let stroke = items.iter().position(|i| matches!(i, vectorcraft_doc::AppearanceItem::Stroke(_))).unwrap();
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "ids": [id.0], "item": stroke}));
    preview(&mut s, true);
    assert!(near(s.transform_bounds(&[id]).unwrap(), Rect::new(85.0, 85.0, 215.0, 215.0), 1e-6), "{:?}", s.transform_bounds(&[id]));
}

#[test]
fn a_groups_own_stroke_with_offset_path_is_measured_where_it_paints() {
    // A group of an unstroked 100 pt square, with a 10 pt stroke of its own whose Offset Path 10
    // moves it 10 pt out: the stroke paints from 15 pt outside the square to 5 pt outside it.
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addStroke", json!({"ids": [g.0]}));
    run(&mut s, "paint.setStroke", json!({"ids": [g.0], "item": 0, "color": "#000000"}));
    run(&mut s, "stroke.set", json!({"ids": [g.0], "item": 0, "weight": 10}));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "ids": [g.0], "item": 0}));
    preview(&mut s, true);
    assert!(near(s.transform_bounds(&[g]).unwrap(), Rect::new(85.0, 85.0, 215.0, 215.0), 1e-6), "{:?}", s.transform_bounds(&[g]));
}

#[test]
fn a_groups_own_fill_is_measured_on_its_members_where_they_are() {
    // A group with a fill of its own around a 100 pt square whose fill a Transform effect moves
    // 100 pt right: the group's fill paints the square where it is, and the square's own fill
    // paints 100 pt to the right of it.
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let items = s.doc().unwrap().doc.node(id).unwrap().appearance.items.clone();
    let fill = items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 100}, "ids": [id.0], "item": fill}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "appearance.addFill", json!({"ids": [g.0]}));
    run(&mut s, "paint.setFill", json!({"ids": [g.0], "item": 0, "color": "#000000"}));
    preview(&mut s, true);
    assert!(near(s.transform_bounds(&[g]).unwrap(), Rect::new(100.0, 100.0, 300.0, 200.0), 1e-6), "{:?}", s.transform_bounds(&[g]));
}

#[test]
fn a_clip_group_is_measured_where_its_clipping_paths_stroke_paints() {
    // A 200 pt square clipped by a 100 pt square at (100, 100) with Offset Path 20. The clipped art
    // is clipped by the square without the offset, so the group is measured by that square.
    let mut s = session();
    let content = NodeId(run(&mut s, "shape.rectangle", json!({"x": 50, "y": 50, "width": 200, "height": 200}))["id"].as_u64().unwrap());
    let path = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [content.0, path.0]}));
    let clip = NodeId(run(&mut s, "object.clippingMask.make", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 20}, "ids": [path.0], "item": null}));
    preview(&mut s, true);
    let square = Rect::new(100.0, 100.0, 200.0, 200.0);
    assert!(near(s.transform_bounds(&[clip]).unwrap(), square, 1e-6), "{:?}", s.transform_bounds(&[clip]));
    // A 4 pt stroke on the clipping path paints along the offset shape, over the clipped art, from
    // 78 to 222.
    run(&mut s, "paint.setStroke", json!({"ids": [path.0], "color": "#000000"}));
    run(&mut s, "stroke.set", json!({"ids": [path.0], "weight": 4}));
    let stroked = Rect::new(78.0, 78.0, 222.0, 222.0);
    assert!(near(s.transform_bounds(&[clip]).unwrap(), stroked, 1e-6), "{:?}", s.transform_bounds(&[clip]));
    // A fill of the clip group's own keeps that box.
    run(&mut s, "appearance.addFill", json!({"ids": [clip.0]}));
    run(&mut s, "paint.setFill", json!({"ids": [clip.0], "item": 0, "color": "#000000"}));
    assert!(near(s.transform_bounds(&[clip]).unwrap(), stroked, 1e-6), "with a fill: {:?}", s.transform_bounds(&[clip]));
    // Turned 30°, the box keeps that size.
    run(&mut s, "select.set", json!({"ids": [clip.0]}));
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let turned = s.transform_box(&[clip]).unwrap();
    assert!(
        (turned.angle - 30.0).abs() < 1e-9 && (turned.rect.width() - 144.0).abs() < 1e-6 && (turned.rect.height() - 144.0).abs() < 1e-6,
        "{turned:?}"
    );
    // In a group, the clip group is measured the same way.
    run(&mut s, "object.rotate", json!({"angle": -30}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    assert!(near(s.transform_bounds(&[group]).unwrap(), stroked, 1e-6), "{:?}", s.transform_bounds(&[group]));
    // Without Use Preview Bounds, the clipping path's geometry is measured.
    preview(&mut s, false);
    assert!(near(s.transform_bounds(&[clip]).unwrap(), square, 1e-6), "{:?}", s.transform_bounds(&[clip]));
}

#[test]
fn only_fills_and_strokes_that_paint_are_measured() {
    let item = |s: &Session, id: NodeId, fill: bool| {
        let items = s.doc().unwrap().doc.node(id).unwrap().appearance.items.clone();
        items.iter().position(|i| i.is_fill() == fill).unwrap()
    };
    // A 100 pt square without a stroke whose fill a Transform effect moves 100 pt right is measured
    // where the fill paints, not where the path is.
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "paint.setStroke", json!({"none": true}));
    let fill = item(&s, id, true);
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"moveH": 100}, "ids": [id.0], "item": fill}));
    preview(&mut s, true);
    assert!(near(s.transform_bounds(&[id]).unwrap(), Rect::new(200.0, 100.0, 300.0, 200.0), 1e-6), "{:?}", s.transform_bounds(&[id]));
    // A hidden 10 pt stroke with Offset Path 10 of its own adds nothing to the box of the square,
    // which Offset Path 5 of the object's own takes 5 pt out.
    let mut s = session();
    let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "stroke.set", json!({"weight": 10}));
    let stroke = item(&s, id, false);
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 10}, "ids": [id.0], "item": stroke}));
    run(&mut s, "appearance.setItem", json!({"index": stroke, "visible": false, "ids": [id.0]}));
    preview(&mut s, true);
    assert!(near(s.transform_bounds(&[id]).unwrap(), Rect::new(100.0, 100.0, 200.0, 200.0), 1e-6), "{:?}", s.transform_bounds(&[id]));
    run(&mut s, "effect.apply", json!({"effect": "path.offsetPath", "params": {"offset": 5}, "ids": [id.0], "item": null}));
    assert!(near(s.transform_bounds(&[id]).unwrap(), Rect::new(95.0, 95.0, 205.0, 205.0), 1e-6), "{:?}", s.transform_bounds(&[id]));
}

#[test]
fn type_with_outline_object_aligns_by_its_glyphs() {
    // "ace" has no ascender or descender; "Tg" has a capital and a descender. Aligned top, their
    // glyph tops line up; aligned bottom, the bottom of the g lines up with the bottom of "ace".
    let edge = |vertical: &str| {
        let mut s = session();
        let a = outlined_type(&mut s, 20.0, "ace");
        let b = outlined_type(&mut s, 200.0, "Tg");
        preview(&mut s, true);
        run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
        run(&mut s, "object.align", json!({"vertical": vertical}));
        let (a, b) = (glyphs(&s, a), glyphs(&s, b));
        if vertical == "top" { (a.y0, b.y0) } else { (a.y1, b.y1) }
    };
    for vertical in ["top", "bottom"] {
        let (a, b) = edge(vertical);
        assert!((a - b).abs() < 1e-6, "{vertical}: {a} and {b}");
    }
}

#[test]
fn type_with_outline_object_measures_as_its_outlines() {
    let mut s = session();
    let id = outlined_type(&mut s, 40.0, "Outline");
    preview(&mut s, true);
    let measured = s.transform_bounds(&[id]).unwrap();
    assert!(near(measured, glyphs(&s, id), 1e-9), "{measured:?}");
    // The type's Create Outlines copy has the same visual box.
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let outlines: Vec<NodeId> =
        run(&mut s, "type.createOutlines", json!({}))["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    let copy = s.doc().unwrap().doc.bounds_of(&outlines, true).unwrap();
    assert!(near(measured, copy, 0.01), "{measured:?} {copy:?}");
}

#[test]
fn set_bounds_sizes_the_glyphs_of_type_with_outline_object() {
    let mut s = session();
    let id = outlined_type(&mut s, 40.0, "Outline");
    preview(&mut s, true);
    let before = glyphs(&s, id);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.setBounds", json!({"width": 100, "reference": 0}));
    let after = glyphs(&s, id);
    assert!((after.width() - 100.0).abs() < 1e-6, "{after:?}");
    assert!((after.x0 - before.x0).abs() < 1e-6 && (after.y0 - before.y0).abs() < 1e-6, "{before:?} {after:?}");
    assert!(near(s.transform_bounds(&[id]).unwrap(), after, 1e-9));
}

#[test]
fn set_bounds_sizes_the_offset_shape_of_a_square_with_offset_path() {
    // The offset shape is 120 pt wide. Set to 220 wide from its top-left corner, it is 220 wide and
    // stays 120 high, with Scale Strokes & Effects off (the offset stays 10 pt) and on (the offset
    // scales with the square).
    for strokes in [false, true] {
        let mut s = session();
        let id = offset_square(&mut s, 100.0);
        preview(&mut s, true);
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "object.setBounds", json!({"width": 220, "reference": 0, "strokes": strokes}));
        let v = s.transform_bounds(&[id]).unwrap();
        assert!(near(v, Rect::new(90.0, 90.0, 310.0, 210.0), 1e-6), "strokes {strokes}: {v:?}");
    }
    // Set to 10 x 10, smaller than the 20 pt the offset adds, with Scale Strokes & Effects on: the
    // offset shrinks with the square, and the offset shape is 10 x 10.
    let mut s = session();
    let id = offset_square(&mut s, 100.0);
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.setBounds", json!({"width": 10, "height": 10, "reference": 0, "strokes": true}));
    let v = s.transform_bounds(&[id]).unwrap();
    assert!(near(v, Rect::new(90.0, 90.0, 100.0, 100.0), 1e-6), "{v:?}");
}

#[test]
fn set_bounds_sizes_art_that_a_transform_effect_turns() {
    // A 100 x 50 rectangle that a Transform effect turns 90° is drawn 50 wide and 100 high. Set to
    // 100 x 100, it is drawn 100 x 100: its width stays 100 and its height doubles.
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 0.0);
    run(&mut s, "effect.apply", json!({"effect": "distort.transform", "params": {"rotate": 90}, "ids": [id.0], "item": null}));
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.setBounds", json!({"width": 100, "height": 100}));
    let v = s.transform_bounds(&[id]).unwrap();
    assert!((v.width() - 100.0).abs() < 1e-6 && (v.height() - 100.0).abs() < 1e-6, "{v:?}");
    let g = geometric(&s, id);
    assert!((g.width() - 100.0).abs() < 1e-6 && (g.height() - 100.0).abs() < 1e-6, "{g:?}");
}

#[test]
fn a_symbol_instance_is_measured_with_the_geometry_effects_in_its_art() {
    let mut s = session();
    offset_square(&mut s, 100.0);
    let id = NodeId(run(&mut s, "symbol.new", json!({"name": "Offset"}))["id"].as_u64().unwrap());
    preview(&mut s, true);
    let v = s.transform_bounds(&[id]).unwrap();
    assert!(near(v, Rect::new(90.0, 90.0, 210.0, 210.0), 1e-6), "{v:?}");
}

#[test]
fn blends_envelopes_and_repeats_are_measured_with_the_geometry_effects_of_their_objects() {
    // A grid repeat of a 100 pt square without a stroke, with Offset Path 10: each copy is drawn
    // 10 pt out from its square.
    let mut s = session();
    let a = offset_square(&mut s, 100.0);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let id = NodeId(run(&mut s, "object.repeat.grid", json!({}))["id"].as_u64().unwrap());
    preview(&mut s, true);
    let (g, v) = (geometric(&s, id), s.transform_bounds(&[id]).unwrap());
    assert!(g.width() > 200.0 && near(v, g.inflate(10.0, 10.0), 1e-6), "{v:?} {g:?}");
    // A blend between two such squares, 100 pt apart.
    let mut s = session();
    let (a, b) = (offset_square(&mut s, 0.0), offset_square(&mut s, 200.0));
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let id = NodeId(run(&mut s, "object.blend.make", json!({"steps": 1}))["id"].as_u64().unwrap());
    preview(&mut s, true);
    let v = s.transform_bounds(&[id]).unwrap();
    assert!(near(v, Rect::new(-10.0, 90.0, 310.0, 210.0), 1e-6), "{v:?}");
    // An envelope around one, with a warp that leaves it as it is.
    let mut s = session();
    let a = offset_square(&mut s, 100.0);
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let id = NodeId(run(&mut s, "object.envelope.makeWithWarp", json!({"style": "arc", "bend": 0}))["id"].as_u64().unwrap());
    preview(&mut s, true);
    let v = s.transform_bounds(&[id]).unwrap();
    assert!(near(v, Rect::new(90.0, 90.0, 210.0, 210.0), 1e-6), "{v:?}");
}

#[test]
fn an_object_has_the_art_of_at_most_4096_symbol_instances_drawn() {
    // Symbol S0 is a filled 10 pt square with Offset Path 1, drawn from -1 to 11. Each of S1 to S12
    // holds two instances of the symbol before it, the second 20 × 2^(k - 1) pt to the right, so an
    // instance of S12 shows 4,096 squares in a row, 20 pt apart, through 8,191 instances. The art of
    // the first 4,096 is drawn: the instance itself and the 4,095 in the left half. The squares in
    // the right half are measured without the offset.
    let mut d = Document::new(400.0, 400.0);
    let mut filled = Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0);
    filled.effects.push(Effect { id: "path.offsetPath".into(), params: json!({"offset": 1}), visible: true });
    let square = Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), filled);
    d.symbols.push(Symbol { name: "S0".into(), art: Arc::new(square) });
    for k in 1..=12 {
        let mut instance =
            |dx: f64| Arc::new(Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: format!("S{}", k - 1), xf: Affine::translate((dx, 0.0)) }));
        let pair = vec![instance(0.0), instance(20.0 * f64::from(1 << (k - 1)))];
        let art = Node::group(d.alloc_id(), pair);
        d.symbols.push(Symbol { name: format!("S{k}"), art: Arc::new(art) });
    }
    let instance = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "S12".into(), xf: Affine::IDENTITY });
    let (layer, id) = (d.layers[0].id, instance.id);
    d.insert(Some(layer), 0, instance).unwrap();
    let mut s = Session::new();
    s.add_document(d, None);
    preview(&mut s, true);
    let v = s.transform_bounds(&[id]).unwrap();
    assert!(near(v, Rect::new(-1.0, -1.0, 81910.0, 11.0), 1e-6), "{v:?}");
}

#[test]
fn a_turned_type_objects_box_follows_its_glyphs() {
    let mut s = session();
    let id = outlined_type(&mut s, 40.0, "Outline");
    preview(&mut s, true);
    let before = s.transform_box(&[id]).unwrap();
    assert!(near(before.rect, glyphs(&s, id), 1e-9), "{before:?}");
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let after = s.transform_box(&[id]).unwrap();
    assert!((after.angle - 30.0).abs() < 1e-9, "{after:?}");
    assert!(
        (after.rect.width() - before.rect.width()).abs() < 1e-6 && (after.rect.height() - before.rect.height()).abs() < 1e-6,
        "{before:?} {after:?}"
    );
    assert!(after.center().distance(before.center()) < 1e-6, "{before:?} {after:?}");
}

#[test]
fn objects_selected_whole_with_anchors_turn_about_the_box_of_their_offset_shape() {
    // A square with Offset Path 10 (drawn from 90 to 210) selected whole with the top-left anchor
    // of an unstroked square at (300, 100); a half turn about the center of the box around both.
    let center_after = |on: bool| {
        let mut s = session();
        let a = offset_square(&mut s, 100.0);
        let b = NodeId(run(&mut s, "shape.rectangle", json!({"x": 300, "y": 100, "width": 50, "height": 50}))["id"].as_u64().unwrap());
        run(&mut s, "paint.setStroke", json!({"none": true}));
        preview(&mut s, on);
        run(&mut s, "select.set", json!({"ids": [a.0]}));
        run(&mut s, "select.anchors", json!({"id": b.0, "anchors": [[0, 0]], "mode": "add"}));
        run(&mut s, "object.rotate", json!({"angle": 180}));
        geometric(&s, a).center()
    };
    // On, the box runs from (90, 90) to (300, 210); off, from (100, 100) to (300, 200).
    assert!(center_after(true).distance(vectorcraft_geom::Point::new(240.0, 150.0)) < 1e-6, "{:?}", center_after(true));
    assert!(center_after(false).distance(vectorcraft_geom::Point::new(250.0, 150.0)) < 1e-6, "{:?}", center_after(false));
}

#[test]
fn without_preview_bounds_geometry_effects_are_not_measured() {
    let mut s = session();
    let ids = [offset_square(&mut s, 100.0), outlined_type(&mut s, 40.0, "Outline")];
    let d = &s.doc().unwrap().doc;
    for id in ids {
        assert_eq!(s.transform_bounds(&[id]), d.bounds_of(&[id], false));
        assert_eq!(s.transform_box(&[id]), d.oriented_bounds(&[id], false));
    }
}

#[test]
fn the_w_field_sets_the_visual_width() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 10.0);
    preview(&mut s, true);
    assert_eq!(s.transform_bounds(&[id]).unwrap().width(), 110.0);
    // Strokes scale: the whole visual box doubles, from its left edge.
    run(&mut s, "object.setBounds", json!({"width": 220, "reference": 0, "proportional": true, "strokes": true}));
    let v = s.transform_bounds(&[id]).unwrap();
    assert!((v.width() - 220.0).abs() < 1e-9 && (v.x0 - 95.0).abs() < 1e-9, "{v:?}");
    assert!((geometric(&s, id).width() - 200.0).abs() < 1e-9);
    run(&mut s, "edit.undo", json!({}));
    // Strokes kept: the path takes the difference.
    run(&mut s, "object.setBounds", json!({"width": 220, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&[id]).unwrap();
    assert!((v.width() - 220.0).abs() < 1e-9 && (v.x0 - 95.0).abs() < 1e-9, "{v:?}");
    assert!((geometric(&s, id).width() - 210.0).abs() < 1e-9);
    // X places the visual box's reference point.
    run(&mut s, "object.setBounds", json!({"x": 0, "reference": 0}));
    assert!((s.transform_bounds(&[id]).unwrap().x0).abs() < 1e-9);
    assert!((geometric(&s, id).x0 - 5.0).abs() < 1e-9);
}

#[test]
fn a_size_below_what_a_kept_stroke_adds_gives_the_path_no_width_or_height() {
    // The 10 pt stroke adds 10 pt to the width and the height. With strokes kept, a width or height
    // of 0 or more below 10 pt gives the path no width or height, and the visual box is the
    // stroke's 10 pt: the path does not turn over. A size of 10 pt or more scales the path to it,
    // and a negative size turns the path over.
    let mut wrong = vec![];
    for (key, size, path, visual) in [
        ("width", 5.0, 0.0, 10.0),
        ("width", 0.0, 0.0, 10.0),
        ("width", 10.0, 0.0, 10.0),
        ("width", 30.0, 20.0, 30.0),
        ("width", -10.0, 20.0, 30.0),
        ("height", 5.0, 0.0, 10.0),
        ("height", 0.0, 0.0, 10.0),
        ("height", 10.0, 0.0, 10.0),
        ("height", 30.0, 20.0, 30.0),
        ("height", -10.0, 20.0, 30.0),
    ] {
        let mut s = session();
        let id = rect(&mut s, 100.0, 100.0, 10.0);
        preview(&mut s, true);
        run(&mut s, "object.setBounds", json!({key: size, "reference": 0, "strokes": false}));
        let (g, v) = (geometric(&s, id), s.transform_bounds(&[id]).unwrap());
        let (g, v) = if key == "width" { (g.width(), v.width()) } else { (g.height(), v.height()) };
        if (g - path).abs() > 1e-9 || (v - visual).abs() > 1e-9 {
            wrong.push(format!("{key} {size}: path {g}, visual box {v}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn set_bounds_gives_one_path_sizes_out_of_proportion_when_strokes_scale() {
    // The 100 × 50 rectangle with a 10 pt stroke: its visual box runs from (95, 95) to (205, 155).
    // The stroke scales by the square root of the product of the two scales, so the width it adds
    // depends on the height too. Set from the top-left corner, the visual box is 220 × 60, and
    // 5 × 1000 with the path the same way round.
    let mut wrong = vec![];
    for (width, height) in [(220.0, 60.0), (5.0, 1000.0)] {
        let mut s = session();
        let id = rect(&mut s, 100.0, 100.0, 10.0);
        preview(&mut s, true);
        run(&mut s, "object.setBounds", json!({"width": width, "height": height, "reference": 0, "strokes": true}));
        let v = s.transform_bounds(&[id]).unwrap();
        let Some(vectorcraft_doc::LiveShape::Rectangle { xf, .. }) = s.doc().unwrap().doc.node(id).unwrap().live_shape() else {
            panic!("the rectangle")
        };
        if !close(v, Rect::new(95.0, 95.0, 95.0 + width, 95.0 + height)) || xf.as_coeffs()[0] <= 0.0 {
            wrong.push(format!("{width} × {height}: {v:?}, {xf:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-9)
}

/// Two rectangles 50 pt high at y 100, both selected with Use Preview Bounds on: one from x 0 to
/// 10 with a 2 pt stroke, which sets the left edge of the paths, and one from x 3 to 10 with a
/// 10 pt stroke, which sets every edge of the visual box, (-2, 95) to (15, 155).
fn thin_and_thick() -> (Session, [NodeId; 2]) {
    let mut s = session();
    let ids = [rect(&mut s, 0.0, 10.0, 2.0), rect(&mut s, 3.0, 7.0, 10.0)];
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [ids[0].0, ids[1].0]}));
    assert!(close(s.transform_bounds(&ids).unwrap(), Rect::new(-2.0, 95.0, 15.0, 155.0)));
    (s, ids)
}

#[test]
fn set_bounds_measures_the_margins_on_each_side_object_by_object() {
    // Set to 37 wide from the left edge with strokes kept, the paths scale by 3.1, and the left edge
    // of the visual box stays at -2.
    let (mut s, ids) = thin_and_thick();
    run(&mut s, "object.setBounds", json!({"width": 37, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&ids).unwrap();
    assert!(close(v, Rect::new(-2.0, 95.0, 35.0, 155.0)), "{v:?}");
    assert!(close(geometric(&s, ids[0]), Rect::new(-1.0, 100.0, 30.0, 150.0)), "{:?}", geometric(&s, ids[0]));
    assert!(close(geometric(&s, ids[1]), Rect::new(8.3, 100.0, 30.0, 150.0)), "{:?}", geometric(&s, ids[1]));
}

#[test]
fn set_bounds_scales_both_ways_at_once_when_strokes_scale_and_the_sizes_are_out_of_proportion() {
    // Each stroke scales by the square root of the product of the two scales, so the width it
    // adds depends on the height too. The visual box is 40 × 100.
    let (mut s, ids) = thin_and_thick();
    run(&mut s, "object.setBounds", json!({"width": 40, "height": 100, "reference": 0, "strokes": true}));
    let v = s.transform_bounds(&ids).unwrap();
    assert!(close(v, Rect::new(-2.0, 95.0, 38.0, 195.0)), "{v:?}");
}

#[test]
fn set_bounds_measures_paths_turned_together_square_to_their_box() {
    // Turned 30° together, the paths are measured square to their turned box: it is 37 wide, and
    // its top-left corner stays where it was.
    let (mut s, ids) = thin_and_thick();
    run(&mut s, "object.rotate", json!({"angle": 30}));
    let before = s.transform_box(&ids).unwrap();
    run(&mut s, "object.setBounds", json!({"width": 37, "reference": 0, "strokes": false}));
    let after = s.transform_box(&ids).unwrap();
    assert!((after.angle - 30.0).abs() < 1e-9 && (after.rect.width() - 37.0).abs() < 1e-9, "{after:?}");
    assert!((after.rect.height() - 60.0).abs() < 1e-9, "{after:?}");
    assert!(after.reference_point(0).distance(before.reference_point(0)) < 1e-9, "{before:?} {after:?}");
}

#[test]
fn a_size_below_what_the_kept_strokes_of_several_paths_add_gives_them_no_width() {
    // With no width, the paths have a visual box 10 pt wide, the width of the thick stroke. A width
    // of 0 or more below 10 gives both paths no width, and the left edge of the visual box stays at
    // -2. A negative width turns the paths over, as it does one path.
    let mut wrong = vec![];
    for (width, paths, visual) in [
        (8.0, [Rect::new(3.0, 100.0, 3.0, 150.0); 2], Rect::new(-2.0, 95.0, 8.0, 155.0)),
        (0.0, [Rect::new(3.0, 100.0, 3.0, 150.0); 2], Rect::new(-2.0, 95.0, 8.0, 155.0)),
        (-37.0, [Rect::new(-44.0, 100.0, 0.0, 150.0), Rect::new(-44.0, 100.0, -13.2, 150.0)], Rect::new(-49.0, 95.0, 1.0, 155.0)),
    ] {
        let (mut s, ids) = thin_and_thick();
        run(&mut s, "object.setBounds", json!({"width": width, "reference": 0, "strokes": false}));
        let (g, v) = (ids.map(|id| geometric(&s, id)), s.transform_bounds(&ids).unwrap());
        if !close(g[0], paths[0]) || !close(g[1], paths[1]) || !close(v, visual) {
            wrong.push(format!("width {width}: paths {g:?}, visual box {v:?}"));
        }
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn set_bounds_leaves_the_height_of_lines_with_no_height() {
    // Horizontal lines at y 100 with square caps, from x 0 to 10 with a 2 pt stroke and from x 3 to
    // 10 with a 10 pt stroke: the visual box is (-2, 95) to (15, 105). They have no height to
    // scale, so the height stays 10; the width becomes 37 from the left edge.
    let mut s = session();
    let mut ids = vec![];
    for (x, weight) in [(0.0, 2.0), (3.0, 10.0)] {
        ids.push(NodeId(run(&mut s, "shape.line", json!({"x1": x, "y1": 100, "x2": 10, "y2": 100}))["id"].as_u64().unwrap()));
        run(&mut s, "stroke.set", json!({"weight": weight, "cap": "square"}));
    }
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [ids[0].0, ids[1].0]}));
    assert!(close(s.transform_bounds(&ids).unwrap(), Rect::new(-2.0, 95.0, 15.0, 105.0)), "{:?}", s.transform_bounds(&ids));
    run(&mut s, "object.setBounds", json!({"width": 37, "height": 30, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&ids).unwrap();
    assert!(close(v, Rect::new(-2.0, 95.0, 35.0, 105.0)), "{v:?}");
    assert!(close(geometric(&s, ids[1]), Rect::new(8.3, 100.0, 30.0, 100.0)), "{:?}", geometric(&s, ids[1]));
}

/// The rectangles of [`thin_and_thick`] in a group, the thick one in a group of its own inside it
/// when `nested`, selected with an unstroked rectangle from x 100 to 110 when `plain`. Returns the
/// selected objects.
fn grouped(nested: bool, plain: bool) -> (Session, Vec<NodeId>) {
    let (mut s, [thin, thick]) = thin_and_thick();
    let mut members = vec![thin.0, thick.0];
    if nested {
        run(&mut s, "select.set", json!({"ids": [thick.0]}));
        members[1] = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    }
    run(&mut s, "select.set", json!({"ids": members}));
    let mut ids = vec![NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap())];
    if plain {
        ids.push(rect(&mut s, 100.0, 10.0, 0.0));
    }
    run(&mut s, "select.set", json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}));
    (s, ids)
}

#[test]
fn set_bounds_measures_the_members_of_groups_one_by_one() {
    // With the unstroked rectangle, the visual box runs from (-2, 95) to (110, 155). Set to 224 wide
    // from its left edge, it is 224 wide and its left edge stays at -2, with strokes kept or
    // scaling, and with the thick rectangle in a group of its own. The group alone, (-2, 95) to
    // (15, 155), set to 37 wide is 37 wide.
    let mut wrong = vec![];
    for (nested, plain, width, strokes) in
        [(false, true, 224.0, false), (false, true, 224.0, true), (true, true, 224.0, false), (false, false, 37.0, false)]
    {
        let (mut s, ids) = grouped(nested, plain);
        run(&mut s, "object.setBounds", json!({"width": width, "reference": 0, "strokes": strokes}));
        let v = s.transform_bounds(&ids).unwrap();
        if !close(v, Rect::new(-2.0, 95.0, width - 2.0, 155.0)) {
            wrong.push(format!("nested {nested}, plain {plain}, strokes {strokes}: {v:?}"));
        }
    }
    // The group with a 1 pt stroke of its own, which reaches 5 pt at its miter limit of 10, around
    // each member: the visual box runs from (-7, 90) to (110, 160). Set to 224 wide from its left
    // edge, it is 224 wide and its left edge stays at -7, with strokes kept or scaling.
    for strokes in [false, true] {
        let (mut s, _) = thin_and_thick();
        let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
        run(&mut s, "appearance.addStroke", json!({"ids": [group.0]}));
        let ids = [group, rect(&mut s, 100.0, 10.0, 0.0)];
        run(&mut s, "select.set", json!({"ids": [ids[0].0, ids[1].0]}));
        assert!(close(s.transform_bounds(&ids).unwrap(), Rect::new(-7.0, 90.0, 110.0, 160.0)), "{:?}", s.transform_bounds(&ids));
        run(&mut s, "object.setBounds", json!({"width": 224, "reference": 0, "strokes": strokes}));
        let v = s.transform_bounds(&ids).unwrap();
        if !close(v, Rect::new(-7.0, 90.0, 217.0, 160.0)) {
            wrong.push(format!("the group with a stroke, strokes {strokes}: {v:?}"));
        }
    }
    // A group of a rectangle from x 0 to 100 with a 2 pt stroke and one from x 40 to 60 with a 10 pt
    // stroke: its visual box, (-1, 95) to (101, 155), set to 11 wide is 11 wide.
    let mut s = session();
    let ids = [rect(&mut s, 0.0, 100.0, 2.0), rect(&mut s, 40.0, 20.0, 10.0)];
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [ids[0].0, ids[1].0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "object.setBounds", json!({"width": 11, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&[group]).unwrap();
    if !close(v, Rect::new(-1.0, 95.0, 10.0, 155.0)) {
        wrong.push(format!("the 0 to 100 and 40 to 60 group: {v:?}"));
    }
    assert!(wrong.is_empty(), "{wrong:?}");
}

#[test]
fn set_bounds_measures_clip_groups_and_groups_with_effects_as_one_object() {
    // A clip group is measured by its clipping path, a rectangle from x 0 to 10, though its content
    // with a 10 pt stroke reaches past it: with the unstroked rectangle from x 100 to 110, set to
    // 224 wide from the left edge, the visual box runs from x 0 to 224.
    let mut s = session();
    let content = rect(&mut s, 3.0, 17.0, 10.0);
    let path = rect(&mut s, 0.0, 10.0, 0.0);
    run(&mut s, "select.set", json!({"ids": [content.0, path.0]}));
    let clip = NodeId(run(&mut s, "object.clippingMask.make", json!({}))["id"].as_u64().unwrap());
    let plain = rect(&mut s, 100.0, 10.0, 0.0);
    preview(&mut s, true);
    run(&mut s, "select.set", json!({"ids": [clip.0, plain.0]}));
    run(&mut s, "object.setBounds", json!({"width": 224, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&[clip, plain]).unwrap();
    assert!(close(v, Rect::new(0.0, 100.0, 224.0, 150.0)), "the clip group: {v:?}");
    // A group with a drop shadow of its own is measured as one object, as before. Its 2 pt margin on
    // the left is measured from the group's left edge, though the stroke of the thick rectangle at
    // x 3 paints it, so the visual box is 223 wide and its left edge moves to -1.
    let (mut s, ids) = grouped(false, true);
    run(&mut s, "effect.apply", json!({"effect": "stylize.dropShadow", "ids": [ids[0].0]}));
    run(&mut s, "select.set", json!({"ids": [ids[0].0, ids[1].0]}));
    run(&mut s, "object.setBounds", json!({"width": 224, "reference": 0, "strokes": false}));
    let v = s.transform_bounds(&ids).unwrap();
    assert!(close(v, Rect::new(-1.0, 95.0, 222.0, 155.0)), "the group with a shadow: {v:?}");
}

#[test]
fn the_bounding_box_handles_sit_on_the_preview_bounds() {
    // Drag from the visual right edge (10 pt outside the path) 100 pt to the right.
    let drag = |on: bool| {
        let mut s = session();
        let id = rect(&mut s, 100.0, 100.0, 20.0);
        preview(&mut s, on);
        let v = ViewInfo::default();
        s.select_tool("selection", v).unwrap();
        for (k, x) in [(PointerKind::Down, 210.0), (PointerKind::Drag, 310.0), (PointerKind::Up, 310.0)] {
            s.pointer(&PointerEvent::new(k, x, 125.0), v).unwrap();
        }
        geometric(&s, id)
    };
    assert!(drag(true).width() > 150.0, "a scale from the visual edge");
    assert_eq!(drag(false).width(), 100.0, "off, the point is no handle");
}

#[test]
fn the_bounding_box_handles_sit_on_the_offset_shape() {
    // Drag from the right edge of the offset shape (10 pt outside the square) 100 pt to the right.
    let drag = |on: bool| {
        let mut s = session();
        let id = offset_square(&mut s, 100.0);
        preview(&mut s, on);
        let v = ViewInfo::default();
        s.select_tool("selection", v).unwrap();
        for (k, x) in [(PointerKind::Down, 210.0), (PointerKind::Drag, 310.0), (PointerKind::Up, 310.0)] {
            s.pointer(&PointerEvent::new(k, x, 150.0), v).unwrap();
        }
        geometric(&s, id)
    };
    assert!(drag(true).width() > 150.0, "a scale from the edge of the offset shape");
    assert_eq!(drag(false).width(), 100.0, "off, the point is no handle");
}

/// The box `id` is drawn in (strokes and geometry effects included) and its geometric box after a
/// Selection tool drag from `from` to `to` with `mods`, with Smart Guides off.
fn drag_box(s: &mut Session, id: NodeId, from: (f64, f64), to: (f64, f64), mods: Mods) -> (Rect, Rect) {
    let v = ViewInfo { smart_guides: false, ..ViewInfo::default() };
    s.select_tool("selection", v).unwrap();
    for (k, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
        s.pointer(&PointerEvent { mods, ..PointerEvent::new(k, x, y) }, v).unwrap();
    }
    let drawn = vectorcraft_render::effects::preview::bounds_of(&s.doc().unwrap().doc, &[id], true).unwrap();
    (drawn, geometric(s, id))
}

/// Drags of `make`'s object with Use Preview Bounds on, and Scale Strokes & Effects off and on: from
/// the handle at the first point to the second point, with the modifiers, giving the visual box.
type HandleDrags = [((f64, f64), (f64, f64), Mods, Rect); 4];

fn assert_handle_drags(make: fn(&mut Session) -> NodeId, drags: HandleDrags) {
    for strokes in [false, true] {
        for (from, to, mods, want) in drags {
            let mut s = session();
            let id = make(&mut s);
            preview(&mut s, true);
            s.prefs.scale_strokes = strokes;
            let (v, _) = drag_box(&mut s, id, from, to, mods);
            assert!(near(v, want, 1e-6), "strokes {strokes}, {from:?} to {to:?}, {mods:?}: {v:?}");
        }
    }
}

const SHIFT: Mods = Mods { shift: true, alt: false, cmd: false, ctrl: false, space: false };
const ALT: Mods = Mods { shift: false, alt: true, cmd: false, ctrl: false, space: false };

#[test]
fn a_handle_drag_keeps_the_opposite_visual_edge_of_a_stroked_path() {
    // A 100 x 50 rectangle at (100, 100) with a 20 pt stroke, drawn from (90, 90) to (210, 160): the
    // opposite edge of the visual box stays and the dragged edge goes to the pointer, with Shift in
    // proportion, with Alt about the center.
    let make = |s: &mut Session| rect(s, 100.0, 100.0, 20.0);
    assert_handle_drags(
        make,
        [
            ((210.0, 125.0), (310.0, 125.0), Mods::default(), Rect::new(90.0, 90.0, 310.0, 160.0)),
            ((210.0, 160.0), (260.0, 200.0), Mods::default(), Rect::new(90.0, 90.0, 260.0, 200.0)),
            ((210.0, 160.0), (330.0, 170.0), SHIFT, Rect::new(90.0, 90.0, 330.0, 230.0)),
            ((210.0, 125.0), (260.0, 125.0), ALT, Rect::new(40.0, 90.0, 260.0, 160.0)),
        ],
    );
    // Off, the handles sit on the path, and the path scales from its own opposite edge.
    for strokes in [false, true] {
        let mut s = session();
        let id = make(&mut s);
        s.prefs.scale_strokes = strokes;
        let (_, g) = drag_box(&mut s, id, (200.0, 125.0), (300.0, 125.0), Mods::default());
        assert_eq!(g, Rect::new(100.0, 100.0, 300.0, 150.0), "strokes {strokes}");
        let (_, g) = drag_box(&mut s, id, (300.0, 150.0), (350.0, 200.0), Mods::default());
        assert_eq!(g, Rect::new(100.0, 100.0, 350.0, 200.0), "strokes {strokes}");
    }
}

#[test]
fn transform_again_repeats_the_transform_of_a_handle_drag_with_preview_bounds() {
    // The drag takes the visual right edge of the stroked rectangle from 210 to 310, so the path,
    // with its stroke kept, goes from 100..200 to 100..300: x becomes 2x - 100.
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 20.0);
    preview(&mut s, true);
    s.prefs.scale_strokes = false;
    let (_, g) = drag_box(&mut s, id, (210.0, 125.0), (310.0, 125.0), Mods::default());
    assert!(near(g, Rect::new(100.0, 100.0, 300.0, 150.0), 1e-6), "{g:?}");
    run(&mut s, "object.transformAgain", json!({}));
    assert!(near(geometric(&s, id), Rect::new(100.0, 100.0, 500.0, 150.0), 1e-6), "{:?}", geometric(&s, id));
}

#[test]
fn a_handle_drag_with_preview_bounds_scales_a_compound_paths_member_alone() {
    // A ring: a 100 pt square at (100, 100) with a 20 pt square hole at (140, 140), the hole selected
    // alone in the isolated compound path.
    let mut s = session();
    let square = |s: &mut Session, at: f64, size: f64| {
        NodeId(run(s, "shape.rectangle", json!({"x": at, "y": at, "width": size, "height": size}))["id"].as_u64().unwrap())
    };
    let (outer, hole) = (square(&mut s, 100.0, 100.0), square(&mut s, 140.0, 20.0));
    run(&mut s, "select.set", json!({"ids": [outer.0, hole.0]}));
    let c = run(&mut s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "object.isolate", json!({"id": c}));
    run(&mut s, "select.set", json!({"ids": [hole.0]}));
    preview(&mut s, true);
    let (b, around) = (s.transform_bounds(&[hole]).unwrap(), s.transform_bounds(&[outer]));
    let (v, _) = drag_box(&mut s, hole, (b.x1, b.center().y), (b.x1 + 10.0, b.center().y), Mods::default());
    assert_eq!(s.transform_bounds(&[outer]), around, "the square around the hole stays");
    assert!(near(v, Rect::new(b.x0, b.y0, b.x1 + 10.0, b.y1), 1e-6), "the hole: {v:?} from {b:?}");
}

#[test]
fn a_handle_drag_with_preview_bounds_leaves_the_art_some_width() {
    // Dragging the visual right edge of the rectangle with a kept 20 pt stroke (drawn from 90 to 210)
    // to 95 asks for less width than the stroke adds: the path keeps 1e-4 of its width.
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 20.0);
    preview(&mut s, true);
    s.prefs.scale_strokes = false;
    let (_, g) = drag_box(&mut s, id, (210.0, 125.0), (95.0, 125.0), Mods::default());
    assert!((g.width() - 0.01).abs() < 1e-9, "{g:?}");
}

#[test]
fn a_handle_drag_keeps_the_opposite_edge_of_the_offset_shape() {
    // A 100 pt square at (100, 100) with Offset Path 10, drawn from (90, 90) to (210, 210).
    let make = |s: &mut Session| offset_square(s, 100.0);
    assert_handle_drags(
        make,
        [
            ((210.0, 150.0), (310.0, 150.0), Mods::default(), Rect::new(90.0, 90.0, 310.0, 210.0)),
            ((210.0, 210.0), (260.0, 250.0), Mods::default(), Rect::new(90.0, 90.0, 260.0, 250.0)),
            ((210.0, 210.0), (330.0, 220.0), SHIFT, Rect::new(90.0, 90.0, 330.0, 330.0)),
            ((210.0, 150.0), (260.0, 150.0), ALT, Rect::new(40.0, 90.0, 260.0, 210.0)),
        ],
    );
    // Off, the handles sit on the square, and the square scales from its own opposite edge.
    for strokes in [false, true] {
        let mut s = session();
        let id = make(&mut s);
        s.prefs.scale_strokes = strokes;
        let (_, g) = drag_box(&mut s, id, (200.0, 150.0), (300.0, 150.0), Mods::default());
        assert_eq!(g, Rect::new(100.0, 100.0, 300.0, 200.0), "strokes {strokes}");
        let (_, g) = drag_box(&mut s, id, (300.0, 200.0), (350.0, 250.0), Mods::default());
        assert_eq!(g, Rect::new(100.0, 100.0, 350.0, 250.0), "strokes {strokes}");
    }
}

#[test]
fn hovering_over_an_unchanged_document_measures_nothing_again() {
    // A selected square with Offset Path 10, drawn from 90 to 210, with Use Preview Bounds and Smart
    // Guides on: the bounding box and the center Smart Guides label are measured once.
    let mut s = session();
    let id = offset_square(&mut s, 100.0);
    preview(&mut s, true);
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let hover = |s: &mut Session, x: f64| {
        s.pointer(&PointerEvent::new(PointerKind::Move, x, 150.0), v).unwrap();
        s.cursor(Point::new(x, 150.0), Mods::default(), v)
    };
    let xs = [210.0, 150.0, 151.0, 180.0, 250.0];
    for x in xs {
        hover(&mut s, x);
    }
    let measured = s.preview_memo.measurements();
    assert!(measured > 0, "the first hover measures");
    for x in xs {
        hover(&mut s, x);
    }
    assert_eq!(s.preview_memo.measurements(), measured, "hovering again measures nothing");
    assert_eq!(hover(&mut s, 210.0), Cursor::ResizeH, "the right edge of the offset shape");
    assert_ne!(hover(&mut s, 230.0), Cursor::ResizeH);

    // Offset Path 30 draws the square from 70 to 230: the box is measured again.
    run(&mut s, "effect.setParams", json!({"index": 0, "params": {"offset": 30}, "ids": [id.0], "item": null}));
    assert_eq!(hover(&mut s, 230.0), Cursor::ResizeH, "the right edge of the new offset shape");
    let measured_again = s.preview_memo.measurements();
    assert!(measured_again > measured);

    // Moved 50 pt to the right, the square is drawn from 120 to 280.
    run(&mut s, "object.move", json!({"dx": 50, "dy": 0}));
    assert_eq!(hover(&mut s, 280.0), Cursor::ResizeH, "the right edge of the moved offset shape");
    assert!(s.preview_memo.measurements() > measured_again);
}
