//! Outline Stroke (`object.path.outlineStroke`) and the live Outline Stroke effect produce the
//! stroke as the canvas paints it: dashes, arrowheads, width profiles, alignment, every stroke
//! item in order, with the stroke's opacity and blend mode moved to the new fill.

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
