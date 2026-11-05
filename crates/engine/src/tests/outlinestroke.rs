//! Outline Stroke (`object.path.outlineStroke`) and the live Outline Stroke effect produce the
//! stroke as the canvas paints it: dashes, arrowheads, width profiles, alignment, every stroke
//! item in order, with the stroke's opacity and blend mode moved to the new fill. Outline Stroke,
//! Expand's Stroke option and Flatten Transparency's Convert All Strokes to Outlines apply the
//! object's geometry effects and each stroke's own to the path before outlining that stroke.

use serde_json::{Value, json};
use vectorcraft_color::BlendMode;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind};
use vectorcraft_geom::{FillRule, PathData, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// A horizontal 100 pt line from (0, 50), black, no fill, with stroke options `opts`.
fn line(s: &mut Session, opts: Value) -> NodeId {
    let r = s.execute("shape.line", &json!({"x1": 0, "y1": 50, "x2": 100, "y2": 50})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    s.execute("stroke.set", &opts).unwrap();
    id
}

pub(super) fn outline(s: &mut Session) -> Node {
    let r = s.execute("object.path.outlineStroke", &json!({})).unwrap();
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

pub(super) fn path_of(n: &Node) -> PathData {
    match &n.kind {
        NodeKind::Path { path, .. } => path.clone(),
        NodeKind::Compound { children, .. } => PathData::new(children.iter().flat_map(|c| c.path_data().unwrap().subpaths.clone()).collect()),
        _ => panic!("not a path: {:?}", n.kind),
    }
}

fn area(n: &Node) -> f64 {
    vectorcraft_pathops::area(&path_of(n), FillRule::NonZero)
}

fn near(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

fn close(a: Rect, b: Rect, tol: f64) -> bool {
    near(a.x0, b.x0, tol) && near(a.y0, b.y0, tol) && near(a.x1, b.x1, tol) && near(a.y1, b.y1, tol)
}

#[test]
fn a_dashed_line_gives_one_subpath_per_dash() {
    let mut s = session();
    line(&mut s, json!({"weight": 4, "dash": [10, 10]}));
    let n = outline(&mut s);
    assert_eq!(path_of(&n).subpaths.len(), 5, "dashes at 0, 20, 40, 60 and 80");
    assert!(near(area(&n), 5.0 * 10.0 * 4.0, 0.5), "{}", area(&n));
    assert!(close(n.geometric_bounds().unwrap(), Rect::new(0.0, 48.0, 90.0, 52.0), 0.01));
}

#[test]
fn arrowheads_are_part_of_the_outline() {
    let mut s = session();
    line(&mut s, json!({"weight": 2, "endArrow": "Triangle"}));
    let n = outline(&mut s);
    // The 2 pt line plus an 8 × 8 triangle whose tip sits past the end point.
    let b = n.geometric_bounds().unwrap();
    assert!(b.x1 > 103.0 && near(b.height(), 8.0, 0.05), "{b:?}");
    assert!(area(&n) > 200.0 + 20.0, "{}", area(&n));
    assert_eq!(path_of(&n).subpaths.len(), 1, "line and head are one shape");
}

#[test]
fn a_lens_profile_outlines_to_half_the_rectangle() {
    let mut s = session();
    line(&mut s, json!({"weight": 10, "profile": "lens"}));
    let n = outline(&mut s);
    assert!(near(area(&n), 0.5 * 10.0 * 100.0, 2.0), "{}", area(&n));
}

#[test]
fn inside_alignment_keeps_the_outline_inside_the_path() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4, "align": "inside"})).unwrap();
    let g = outline(&mut s);
    let ring = &g.children().unwrap()[1];
    assert!(close(ring.geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0), 0.01));
    assert!(near(area(ring), 100.0 * 100.0 - 92.0 * 92.0, 1.0), "{}", area(ring));
}

#[test]
fn stroke_opacity_and_blend_move_to_the_new_fill() {
    let mut s = session();
    line(&mut s, json!({"weight": 4}));
    s.execute("appearance.setItem", &json!({"index": 1, "opacity": 50, "blend": "multiply"})).unwrap();
    let n = outline(&mut s);
    let Some(AppearanceItem::Fill(f)) = n.appearance.items.first() else { panic!("{:?}", n.appearance) };
    assert!(near(f.opacity as f64, 0.5, 1e-6) && f.blend == BlendMode::Multiply, "{f:?}");
    assert!(n.opacity == 1.0 && n.blend == BlendMode::Normal);
}

#[test]
fn every_stroke_is_outlined_in_paint_order() {
    let mut s = session();
    s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 100, "height": 100})).unwrap();
    s.execute("appearance.addStroke", &json!({})).unwrap();
    // Items: fill, 1 pt stroke, then the new top stroke at 10 pt red.
    s.execute("appearance.setItem", &json!({"index": 2, "weight": 10, "color": "#ff0000"})).unwrap();
    let g = outline(&mut s);
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 3, "the fill, then both outlines");
    assert!(ch[0].appearance.items.iter().all(AppearanceItem::is_fill));
    assert!(close(ch[0].geometric_bounds().unwrap(), Rect::new(0.0, 0.0, 100.0, 100.0), 1e-9));
    assert!(close(ch[1].geometric_bounds().unwrap(), Rect::new(-0.5, -0.5, 100.5, 100.5), 0.01));
    assert!(close(ch[2].geometric_bounds().unwrap(), Rect::new(-5.0, -5.0, 105.0, 105.0), 0.01));
    assert_eq!(ch[2].appearance.fill().unwrap().paint.color().unwrap().to_hex(), "#ff0000");
    // One undo step restores the stroked rectangle.
    s.execute("edit.undo", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert_eq!(d.layers[0].children().unwrap()[0].appearance.items.len(), 3);
}

/// The art that stands for an outlined path keeps the path's hidden, locked and knockout-shape
/// states, so a hidden member of a selected group stays hidden.
#[test]
fn outlined_art_keeps_the_hidden_locked_and_knockout_shape_states() {
    let mut s = session();
    // A filled and stroked square (a fill copy and an outline) and a stroked line (one outline).
    let sq = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 100, "width": 50, "height": 50})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    let ln = line(&mut s, json!({"weight": 4}));
    s.execute("select.set", &json!({"ids": [sq.0, ln.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.edit("t", |d, _| {
        d.node_mut(sq).unwrap().visible = false;
        let l = d.node_mut(ln).unwrap();
        l.locked = true;
        l.knockout_shape = true;
        Ok(())
    })
    .unwrap();
    let g = outline(&mut s);
    let ch = g.children().unwrap();
    assert_eq!(ch.len(), 2);
    let (sq, ln) = (&ch[0], &ch[1]);
    assert!(!sq.visible && !sq.locked, "the square's art stays hidden: {sq:?}");
    assert_eq!(sq.children().map(|c| c.len()), Some(2), "a fill copy and the outline");
    assert!(sq.children().unwrap().iter().all(|c| c.visible && !c.locked), "showing it shows all of it: {sq:?}");
    assert!(ln.visible && ln.locked && ln.knockout_shape, "the line's outline stays locked: {ln:?}");
}

/// The live effect on a stroke item outlines that stroke (its own weight), not the top stroke.
#[test]
fn the_live_effect_outlines_the_stroke_it_sits_on() {
    let mut s = session();
    let id = line(&mut s, json!({"weight": 20}));
    // Items: the line's empty fill, its 20 pt stroke, then a new 2 pt top stroke.
    s.execute("appearance.addStroke", &json!({})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 2, "weight": 2})).unwrap();
    // The lower (20 pt) stroke gets the effect.
    s.execute("effect.apply", &json!({"effect": "path.outlineStroke", "item": 1})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let n = doc.node(id).unwrap();
    assert!(matches!(&n.appearance.items[1], AppearanceItem::Stroke(st) if st.width == 20.0 && st.effects.len() == 1), "{:?}", n.appearance.items);
    let baked = vectorcraft_render::effects::bake_document(doc).unwrap();
    let parts = baked.node(id).unwrap().children().unwrap().to_vec();
    assert!(near(parts[1].geometric_bounds().unwrap().height(), 20.0, 0.01), "{:?}", parts[1].geometric_bounds());
    assert!(near(parts[2].geometric_bounds().unwrap().height(), 0.0, 1e-9), "the top stroke keeps the line");
    // On the object, the effect outlines the top stroke.
    s.execute("effect.apply", &json!({"effect": "path.outlineStroke", "item": null})).unwrap();
    let doc = &s.doc().unwrap().doc;
    let n = doc.node(id).unwrap();
    let ctx = vectorcraft_render::effects::GeomContext::of(n);
    let g = vectorcraft_render::effects::apply_geometry_with(&n.appearance.effects, n.path_data().unwrap(), Rect::new(0.0, 50.0, 100.0, 50.0), &ctx);
    assert!(close(g.bounds().unwrap(), Rect::new(0.0, 49.0, 100.0, 51.0), 0.01), "{:?}", g.bounds());
}

fn render(s: &Session) -> vectorcraft_render::Rendered {
    let doc = s.doc().unwrap().doc.clone();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&doc, 400, 300, vectorcraft_geom::Affine::IDENTITY, &opts)
}

/// Pixels whose color differs by more than `tol` in any channel.
fn differing(a: &vectorcraft_render::Rendered, b: &vectorcraft_render::Rendered, tol: u8) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > tol))
        .count()
}

/// Does `n` or anything in it have a geometry effect, its own or a fill's or stroke's?
fn has_geometry_effects(n: &Node) -> bool {
    let geometry = |e: &vectorcraft_doc::Effect| vectorcraft_render::effects::is_geometry(&e.id);
    let mut found = false;
    n.walk(&mut |m| found |= m.appearance.effects.iter().any(geometry) || m.appearance.items.iter().any(|i| i.effects().iter().any(geometry)));
    found
}

/// A selected 100 pt square at (150, 100) with a 4 pt black stroke and fill `fill` (paint.setFill's),
/// and `effects` (effect.apply's `effect` and `params`) on appearance item `item` (`null`: on the
/// object).
fn square_with(s: &mut Session, fill: Value, item: Value, effects: &[(&str, Value)]) {
    s.execute("shape.rectangle", &json!({"x": 150, "y": 100, "width": 100, "height": 100})).unwrap();
    s.execute("paint.setFill", &fill).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 4})).unwrap();
    for (effect, params) in effects {
        s.execute("effect.apply", &json!({"effect": effect, "params": params, "item": item})).unwrap();
    }
}

/// The geometry effects the tests put on the stroke or on the object.
fn geometry_effects() -> [(&'static str, Value); 2] {
    [("path.offsetPath", json!({"offset": 10})), ("distort.zigZag", json!({"size": 6, "ridges": 3}))]
}

/// The commands that outline strokes: Outline Stroke, Expand with Stroke alone, and Flatten
/// Transparency with Convert All Strokes to Outlines.
fn outlining() -> [(&'static str, Value); 3] {
    [
        ("object.path.outlineStroke", json!({})),
        ("object.expand", json!({"object": false, "fill": false, "stroke": true})),
        ("object.flattenTransparency", json!({"strokesToOutlines": true})),
    ]
}

/// What `command` with `params` leaves in place of the selected object.
fn outlined_by(s: &mut Session, command: &str, params: &Value) -> Node {
    let r = s.execute(command, params).unwrap_or_else(|e| panic!("{command}: {e}"));
    let id = NodeId(r["ids"][0].as_u64().unwrap());
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

/// For each command in [`outlining`] and each of [`geometry_effects`] on `item` of a square filled
/// with `fill`: the pixels that differ by more than 64 from the canvas before, and whether a
/// geometry effect is left on the result.
fn outlined_as_drawn(fill: Value, item: Value) -> Vec<(&'static str, &'static str, usize, bool)> {
    let mut results = vec![];
    for (command, p) in outlining() {
        for (effect, params) in geometry_effects() {
            let mut s = session();
            square_with(&mut s, fill.clone(), item.clone(), &[(effect, params)]);
            let before = render(&s);
            let n = outlined_by(&mut s, command, &p);
            results.push((command, effect, differing(&before, &render(&s), 64), has_geometry_effects(&n)));
        }
    }
    results
}

/// What [`outlined_as_drawn`] gives when every outline looks as the canvas drew the object.
fn as_drawn() -> Vec<(&'static str, &'static str, usize, bool)> {
    outlining().iter().flat_map(|(command, _)| geometry_effects().map(|(effect, _)| (*command, effect, 0, false))).collect()
}

/// A stroke's own geometry effects reshape the path it strokes, as on the canvas: the outline is
/// the stroke of the offset or zig-zagged path, not the stroke's outline offset or zig-zagged.
#[test]
fn a_geometry_effect_on_the_stroke_shapes_the_path_it_outlines() {
    assert_eq!(outlined_as_drawn(json!({"none": true}), json!(1)), as_drawn());
    // The stroke of the square offset by 10: 4 pt wide around a 120 pt square.
    let mut s = session();
    square_with(&mut s, json!({"none": true}), json!(1), &[("path.offsetPath", json!({"offset": 10}))]);
    let n = outline(&mut s);
    assert!(close(n.geometric_bounds().unwrap(), Rect::new(138.0, 88.0, 262.0, 212.0), 0.01), "{:?}", n.geometric_bounds());
    assert!(near(area(&n), 124.0 * 124.0 - 116.0 * 116.0, 2.0), "{}", area(&n));
}

/// A stroke whose geometry effect leaves no path paints nothing on the canvas, and outlining it
/// replaces the live stroke with an empty group.
#[test]
fn a_stroke_its_effects_empty_is_outlined_to_nothing() {
    for (command, p) in outlining() {
        let mut s = session();
        square_with(&mut s, json!({"none": true}), json!(1), &[("path.offsetPath", json!({"offset": -60}))]);
        let before = render(&s);
        let n = outlined_by(&mut s, command, &p);
        assert!(matches!(&n.kind, NodeKind::Group { children, .. } if children.is_empty()), "{command}: {n:?}");
        let mut stroked = false;
        for layer in &s.doc().unwrap().doc.layers {
            layer.walk(&mut |m| stroked |= m.appearance.items.iter().any(|i| matches!(i, AppearanceItem::Stroke(_))));
        }
        assert!(!stroked, "{command} left a live stroke");
        assert_eq!(differing(&before, &render(&s), 64), 0, "{command}");
    }
}

/// A drop shadow on a stroke with a geometry effect stays on the outline, which draws as the
/// stroke did.
#[test]
fn the_outline_of_a_reshaped_stroke_keeps_its_drop_shadow() {
    let mut s = session();
    square_with(&mut s, json!({"none": true}), json!(1), &[("path.offsetPath", json!({"offset": 10})), ("stylize.dropShadow", json!({}))]);
    let before = render(&s);
    let n = outline(&mut s);
    let kept: Vec<&str> = n.appearance.items.iter().flat_map(|i| i.effects()).map(|e| e.id.as_str()).collect();
    assert_eq!(kept, ["stylize.dropShadow"]);
    assert_eq!(differing(&before, &render(&s), 64), 0, "the shadowed outline draws as the stroke did");
}

/// The brush art of a brushed stroke with a geometry effect follows the path the effect makes.
#[test]
fn a_brush_draws_along_the_path_the_stroke_effects_make() {
    let mut s = session();
    square_with(&mut s, json!({"none": true}), json!(1), &[("path.offsetPath", json!({"offset": 10}))]);
    s.execute("brush.apply", &json!({"name": "Tapered Stroke"})).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    let st = s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().cloned().unwrap();
    assert!(st.brush.is_some() && st.effects.len() == 1, "{st:?}");
    let before = render(&s);
    let n = outline(&mut s);
    assert!(!vectorcraft_brush::has_brush(&n) && !has_geometry_effects(&n), "brush art: {n:?}");
    assert_eq!(differing(&before, &render(&s), 64), 0, "the brush art draws as the brushed stroke did");
}

/// The object's geometry effects reshape the path its fills paint and its strokes outline, as on
/// the canvas, and do not stay on the result.
#[test]
fn a_geometry_effect_on_the_object_shapes_the_path_it_outlines() {
    assert_eq!(outlined_as_drawn(json!({"color": "#33aa55"}), Value::Null), as_drawn());
}

/// The canvas paints a gradient fill or stroke on each copy that the object's Transform effect
/// makes by itself; the outlined result draws the same, with the gradient fitted to each copy.
#[test]
fn each_transform_copy_keeps_its_own_gradient() {
    let mut differ = vec![];
    for on_stroke in [false, true] {
        for (command, p) in outlining() {
            let mut s = session();
            let fill = if on_stroke { json!({"none": true}) } else { json!({"gradient": {"kind": "linear"}}) };
            square_with(&mut s, fill, Value::Null, &[("distort.transform", json!({"moveH": 120, "copies": 1}))]);
            if on_stroke {
                s.execute("paint.setStroke", &json!({"gradient": {"kind": "linear"}})).unwrap();
                s.execute("stroke.set", &json!({"weight": 12})).unwrap();
            }
            let before = render(&s);
            outlined_by(&mut s, command, &p);
            differ.push((on_stroke, command, differing(&before, &render(&s), 64)));
        }
    }
    let none_differ: Vec<_> = [false, true].into_iter().flat_map(|on_stroke| outlining().map(|(command, _)| (on_stroke, command, 0))).collect();
    assert_eq!(differ, none_differ);
}

/// With Scale Strokes & Effects on, the canvas draws the stroke of a solid square that its
/// Transform effect scales to 200% at 8 pt; the outlined result draws the same.
#[test]
fn a_transform_that_scales_strokes_outlines_the_scaled_stroke() {
    let scale = json!({"scaleH": 200, "scaleV": 200, "strokes": true});
    let mut differ = vec![];
    for (command, p) in outlining() {
        let mut s = session();
        square_with(&mut s, json!({"color": "#33aa55"}), Value::Null, &[("distort.transform", scale.clone())]);
        let before = render(&s);
        outlined_by(&mut s, command, &p);
        differ.push((command, differing(&before, &render(&s), 64)));
    }
    let none_differ: Vec<_> = outlining().map(|(command, _)| (command, 0)).into_iter().collect();
    assert_eq!(differ, none_differ);
}

/// Random in a Transform effect takes its seed from the object's id. The outline draws as the
/// canvas drew the object with Random in a Transform effect that copies a gradient square, in a
/// Transform effect on the stroke of such a square, and in a Transform effect on the fill of a
/// plain square.
#[test]
fn outlines_keep_the_random_transforms_the_canvas_drew() {
    let random = json!({"moveH": 30, "moveV": 30, "rotate": 40, "random": true});
    let cases = [
        ("random copies", json!({"gradient": {"kind": "linear"}}), json!({"moveH": 120, "rotate": 30, "copies": 1, "random": true}), None),
        ("copies, random stroke", json!({"gradient": {"kind": "linear"}}), json!({"moveH": 120, "copies": 1}), Some(1)),
        ("random fill", json!({"color": "#33aa55"}), json!({}), Some(0)),
    ];
    let mut differ = vec![];
    for (case, fill, object, item) in &cases {
        for (command, p) in outlining() {
            let mut s = session();
            square_with(&mut s, fill.clone(), Value::Null, &[]);
            if object != &json!({}) {
                s.execute("effect.apply", &json!({"effect": "distort.transform", "params": object})).unwrap();
            }
            s.execute("stroke.set", &json!({"weight": 12})).unwrap();
            if let Some(item) = item {
                s.execute("effect.apply", &json!({"effect": "distort.transform", "params": random, "item": item})).unwrap();
            }
            let before = render(&s);
            outlined_by(&mut s, command, &p);
            differ.push((*case, command, differing(&before, &render(&s), 64)));
        }
    }
    let none_differ: Vec<_> = cases.iter().flat_map(|(case, ..)| outlining().map(|(command, _)| (*case, command, 0))).collect();
    assert_eq!(differ, none_differ);
}

/// For each command in [`outlining`] on a stroke-only square with `effects` on appearance item
/// `item`: the pixels that differ by more than 64 from the canvas before, whether a geometry
/// effect is left on the result, and (for Outline Stroke) the outline's box and area.
fn outlined_square(item: Value, effects: &[(&str, Value)], hide: bool) -> (Vec<(&'static str, usize, bool)>, Rect, f64) {
    let mut results = vec![];
    let mut shape = (Rect::ZERO, 0.0);
    for (command, p) in outlining() {
        let mut s = session();
        square_with(&mut s, json!({"none": true}), item.clone(), effects);
        if hide {
            for index in 0..effects.len() {
                s.execute("effect.setParams", &json!({"index": index, "visible": false, "item": item})).unwrap();
            }
        }
        let before = render(&s);
        let n = outlined_by(&mut s, command, &p);
        if command == "object.path.outlineStroke" {
            shape = (n.geometric_bounds().unwrap(), area(&n));
        }
        results.push((command, differing(&before, &render(&s), 64), has_geometry_effects(&n)));
    }
    (results, shape.0, shape.1)
}

/// What [`outlined_square`] gives for each command when the outline draws as the canvas did and
/// keeps no geometry effect.
fn drawn_plain() -> Vec<(&'static str, usize, bool)> {
    outlining().map(|(command, _)| (command, 0, false)).into_iter().collect()
}

/// A path with no effects outlines its own stroke: 4 pt wide around the 100 pt square.
#[test]
fn a_path_without_effects_outlines_its_own_stroke() {
    let (results, bounds, a) = outlined_square(Value::Null, &[], false);
    assert_eq!(results, drawn_plain());
    assert!(close(bounds, Rect::new(148.0, 98.0, 252.0, 202.0), 0.01), "{bounds:?}");
    assert!(near(a, 104.0 * 104.0 - 96.0 * 96.0, 0.5), "{a}");
}

/// Round Corners on the object rounds the path whose stroke is outlined, with the radius in
/// points as the canvas draws it: corners of radius 12 outside and 8 inside.
#[test]
fn round_corners_on_the_object_round_the_stroke_it_outlines() {
    let (results, bounds, a) = outlined_square(Value::Null, &[("stylize.roundCorners", json!({"radius": 10}))], false);
    assert_eq!(results, drawn_plain());
    assert!(close(bounds, Rect::new(148.0, 98.0, 252.0, 202.0), 0.01), "{bounds:?}");
    let rounded = 104.0 * 104.0 - 96.0 * 96.0 - (4.0 - std::f64::consts::PI) * (12.0 * 12.0 - 8.0 * 8.0);
    assert!(near(a, rounded, 2.0), "{a}, rounded {rounded}");
}

/// A hidden geometry effect, on the object or on the stroke, does not draw on the canvas and does
/// not shape the outline.
#[test]
fn a_hidden_geometry_effect_does_not_shape_the_outline() {
    for item in [Value::Null, json!(1)] {
        let (results, bounds, a) = outlined_square(item.clone(), &geometry_effects(), true);
        let drawn: Vec<_> = results.iter().map(|(command, differ, _)| (*command, *differ)).collect();
        assert_eq!(drawn, drawn_plain().into_iter().map(|(command, differ, _)| (command, differ)).collect::<Vec<_>>(), "item {item}");
        assert!(close(bounds, Rect::new(148.0, 98.0, 252.0, 202.0), 0.01), "item {item}: {bounds:?}");
        assert!(near(a, 104.0 * 104.0 - 96.0 * 96.0, 0.5), "item {item}: {a}");
    }
}

/// An effect on the object that is not a geometry effect, such as a Gaussian Blur, stays on the
/// result of Outline Stroke and Expand while the geometry effect shapes the path. Flatten
/// Transparency renders raster effects into an image instead.
#[test]
fn an_effect_that_is_not_geometry_stays_on_the_result() {
    for (command, p) in outlining().into_iter().filter(|(command, _)| *command != "object.flattenTransparency") {
        let mut s = session();
        square_with(
            &mut s,
            json!({"color": "#33aa55"}),
            Value::Null,
            &[("distort.zigZag", json!({"size": 6, "ridges": 3})), ("blur.gaussian", json!({}))],
        );
        let n = outlined_by(&mut s, command, &p);
        let kept: Vec<&str> = n.appearance.effects.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(kept, ["blur.gaussian"], "{command}");
        assert!(!has_geometry_effects(&n), "{command}: {n:?}");
    }
}

/// Undo after outlining a path with geometry effects on the object and on its stroke gives back
/// the path as it was, effects included, and its selection.
#[test]
fn undo_gives_back_the_path_with_its_effects() {
    for (command, p) in outlining() {
        let mut s = session();
        square_with(&mut s, json!({"color": "#33aa55"}), Value::Null, &[("distort.zigZag", json!({"size": 6, "ridges": 3}))]);
        s.execute("effect.apply", &json!({"effect": "path.offsetPath", "params": {"offset": 10}, "item": 1})).unwrap();
        let before = s.doc().unwrap().doc.layers.clone();
        let selected = s.doc().unwrap().selection.objects.clone();
        outlined_by(&mut s, command, &p);
        assert_ne!(s.doc().unwrap().doc.layers, before, "{command} changed nothing");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().doc.layers, before, "{command}");
        assert_eq!(s.doc().unwrap().selection.objects, selected, "{command}");
    }
}

#[test]
fn brushed_strokes_outline_to_their_brush_art() {
    let mut s = session();
    line(&mut s, json!({"weight": 2}));
    s.execute("brush.apply", &json!({"name": "Tapered Stroke"})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 1, "opacity": 50})).unwrap();
    let n = outline(&mut s);
    let doc = &s.doc().unwrap().doc;
    let mut fills = 0;
    n.walk(&mut |m| fills += m.appearance.items.iter().filter(|i| i.is_fill()).count());
    assert!(fills > 0 && n.appearance.stroke().is_none(), "brush art: {n:?}");
    assert!(near(n.opacity as f64, 0.5, 1e-6), "the stroke opacity stays on the art");
    assert!(!doc.layers[0].children().unwrap().iter().any(|c| vectorcraft_brush::has_brush(c)));
}

#[test]
fn a_brushed_stroke_keeps_its_effects_on_its_brush_art() {
    let mut s = session();
    line(&mut s, json!({"weight": 2}));
    s.execute("brush.apply", &json!({"name": "Tapered Stroke"})).unwrap();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "item": 1})).unwrap();
    s.execute("effect.apply", &json!({"effect": "path.offsetPath", "params": {"offset": 3}, "item": 1})).unwrap();
    let n = outline(&mut s);
    let count = |id: &str| {
        let mut k = 0;
        n.walk(&mut |m| k += m.appearance.effects.iter().filter(|e| e.id == id).count());
        k
    };
    assert_eq!(count("stylize.dropShadow"), 1, "the drop shadow is on the brush art: {n:?}");
    // A geometry effect changes the path the brush follows; it is not applied to the brush art.
    assert_eq!(count("path.offsetPath"), 0);
}
