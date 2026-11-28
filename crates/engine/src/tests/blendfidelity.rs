//! Blend fidelity (M8.20): start points that don't twist, strokes, groups, compound paths, text
//! and symbols interpolating, knockout on the canvas, and Expand / Release keeping what the
//! blend carries.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::{Knockout, Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    id_of(&s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn steps(s: &Session, g: NodeId) -> Vec<Node> {
    vectorcraft_doc::live::expand_live(&node(s, g))
}

fn make(s: &mut Session, ids: &[NodeId], steps: u32) -> NodeId {
    id_of(&s.execute("object.blend.make", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "steps": steps})).unwrap())
}

#[test]
fn closed_shapes_start_where_they_dont_twist() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    // The same square drawn from its bottom right corner.
    let b = id_of(&s.execute("path.create", &json!({"d": "M240 40 L200 40 L200 0 L240 0 Z"})).unwrap());
    let g = make(&mut s, &[a, b], 1);
    let mid = steps(&s, g)[1].geometric_bounds().unwrap();
    assert!((mid.width() - 40.0).abs() < 1e-6 && (mid.height() - 40.0).abs() < 1e-6, "no twist: {mid:?}");
}

#[test]
fn dashes_caps_and_width_profiles_interpolate() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    s.execute("stroke.set", &json!({"ids": [a.0], "weight": 2, "dash": [10, 10], "cap": "butt"})).unwrap();
    let b = rect(&mut s, 200.0, 0.0, 40.0, 40.0);
    s.execute("stroke.set", &json!({"ids": [b.0], "weight": 6, "cap": "round", "profile": "lens"})).unwrap();
    let g = make(&mut s, &[a, b], 3);
    let st = steps(&s, g);
    let stroke = |n: &Node| n.appearance.stroke().cloned().unwrap();
    let (first, last) = (stroke(&st[1]), stroke(&st[3]));
    assert_eq!(first.dash.as_ref().unwrap().pattern, vec![12.5, 7.5], "the gaps close towards the solid key");
    assert!(last.dash.as_ref().unwrap().pattern[1] < first.dash.as_ref().unwrap().pattern[1]);
    assert_eq!((first.cap, last.cap), (vectorcraft_doc::LineCap::Butt, vectorcraft_doc::LineCap::Round), "caps switch halfway");
    assert_eq!(stroke(&st[2]).profile.unwrap().at(0.0), (0.5, 0.5), "the lens profile comes in gradually");
}

#[test]
fn groups_with_different_member_counts_interpolate_member_by_member() {
    let mut s = session();
    let a1 = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let a2 = rect(&mut s, 0.0, 40.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [a1.0, a2.0]})).unwrap();
    let ga = id_of(&s.execute("object.group", &json!({})).unwrap());
    let b1 = rect(&mut s, 200.0, 0.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [b1.0]})).unwrap();
    let gb = id_of(&s.execute("object.group", &json!({})).unwrap());
    let g = make(&mut s, &[ga, gb], 1);
    let mid = &steps(&s, g)[1];
    let NodeKind::Group { children, .. } = &mid.kind else { panic!("a group step") };
    assert_eq!(children.len(), 2, "the member only one group has grows out of the other's centre");
    let c = children[0].geometric_bounds().unwrap();
    assert!((c.x0 - 100.0).abs() < 1e-6 && (c.width() - 20.0).abs() < 1e-6, "{c:?}");
    assert!(children[1].geometric_bounds().unwrap().width() < 15.0);
}

#[test]
fn compound_steps_stay_compound_paths() {
    let mut s = session();
    let mk = |s: &mut Session, x: f64| {
        let o = rect(s, x, 0.0, 40.0, 40.0);
        let i = rect(s, x + 10.0, 10.0, 20.0, 20.0);
        s.execute("select.set", &json!({"ids": [o.0, i.0]})).unwrap();
        id_of(&s.execute("object.compoundPath.make", &json!({})).unwrap())
    };
    let (a, b) = (mk(&mut s, 0.0), mk(&mut s, 200.0));
    let g = make(&mut s, &[a, b], 1);
    let mid = &steps(&s, g)[1];
    assert!(matches!(&mid.kind, NodeKind::Compound { children, .. } if children.len() == 2), "{}", mid.kind_label());
}

#[test]
fn text_moves_and_recolours_while_its_words_switch_halfway() {
    let mut s = session();
    let a = id_of(&s.execute("text.create", &json!({"x": 0, "y": 100, "text": "Hi", "size": 20, "color": "#000000"})).unwrap());
    let b = id_of(&s.execute("text.create", &json!({"x": 200, "y": 100, "text": "Yo", "size": 40, "color": "#ffffff"})).unwrap());
    let g = make(&mut s, &[a, b], 3);
    let st = steps(&s, g);
    let NodeKind::Text(t) = &st[2].kind else { panic!("a text step") };
    assert!((t.xf.as_coeffs()[4] - 100.0).abs() < 1e-6, "halfway along");
    let run = &t.runs[0];
    assert!((run.style.size - 30.0).abs() < 1e-6);
    assert!((run.style.fill.color().unwrap().to_rgb()[0] - 0.5).abs() < 1e-3, "colour halfway");
    let NodeKind::Text(t1) = &st[1].kind else { panic!() };
    assert_eq!(t1.plain_text(), "Hi", "the words switch only halfway");
}

#[test]
fn symbol_instances_interpolate_their_transforms() {
    let mut s = session();
    let r = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [r.0]})).unwrap();
    let a = id_of(&s.execute("symbol.new", &json!({"name": "Dot"})).unwrap());
    let b = id_of(&s.execute("symbol.place", &json!({"name": "Dot", "x": 210, "y": 10})).unwrap());
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    let g = make(&mut s, &[a, b], 1);
    let mid = &steps(&s, g)[1];
    let NodeKind::SymbolInstance { xf, .. } = &mid.kind else { panic!("{}", mid.kind_label()) };
    let c = xf.as_coeffs();
    assert!((c[1].atan2(c[0]).to_degrees().abs() - 45.0).abs() < 1e-6, "turned halfway: {c:?}");
    assert!((mid.geometric_bounds().unwrap().center().x - 110.0).abs() < 1e-6);
}

#[test]
fn knockout_blends_draw_as_they_export() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 40.0, 40.0);
    let b = rect(&mut s, 20.0, 0.0, 40.0, 40.0);
    for id in [a, b] {
        s.execute("paint.setFill", &json!({"ids": [id.0], "color": "#ff0000"})).unwrap();
        s.execute("paint.setStroke", &json!({"ids": [id.0], "color": null})).ok();
        s.execute("object.setProps", &json!({"ids": [id.0], "opacity": 50})).unwrap();
    }
    let g = make(&mut s, &[a, b], 3);
    assert_eq!(node(&s, g).knockout, Knockout::On, "blends knock out by default");
    let px = |s: &Session| {
        vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 60.0, 40.0), 1.0, true).pixel(30, 20)
    };
    let on = px(&s);
    assert!((i32::from(on[1]) - 128).abs() <= 3, "one layer of 50 % red over white: {on:?}");
    s.execute("object.setProps", &json!({"ids": [g.0], "knockout": "off"})).unwrap();
    let off = px(&s);
    assert!(off[1] < 100, "the steps build up without knockout: {off:?}");
    // Opaque steps don't need a knockout group in the outputs.
    let mut t = session();
    let (c, d) = (rect(&mut t, 0.0, 0.0, 10.0, 10.0), rect(&mut t, 100.0, 0.0, 10.0, 10.0));
    let h = make(&mut t, &[c, d], 3);
    assert_eq!(vectorcraft_doc::live::expanded_group(&node(&t, h), None).knockout, Knockout::Off);
}

#[test]
fn expand_and_release_keep_what_the_blend_carries() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0, 10.0));
    let g = make(&mut s, &[a, b], 2);
    s.execute("object.setProps", &json!({"ids": [g.0], "name": "Fade", "opacity": 40, "isolate": true})).unwrap();
    s.edit("mask", |d, _| {
        let art = Node::path(d.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(0.0, 0.0, 50.0, 10.0)), Default::default());
        let mask = vectorcraft_doc::OpacityMask::new(art, false);
        d.node_mut(g).unwrap().extras_mut().mask = Some(Box::new(mask));
        Ok(())
    })
    .unwrap();
    let before = node(&s, g);
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.execute("object.blend.expand", &json!({})).unwrap();
    let e = node(&s, g);
    assert!(matches!(e.kind, NodeKind::Group { .. }));
    assert_eq!((e.name.as_deref(), e.opacity, e.isolate, e.knockout), (Some("Fade"), before.opacity, true, Knockout::On));
    assert_eq!(e.extras().mask, before.extras().mask);
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    let grp = NodeId(r["groups"][0].as_u64().unwrap());
    let n = node(&s, grp);
    assert_eq!((n.name.as_deref(), n.opacity, n.isolate), (Some("Fade"), before.opacity, true));
    assert_eq!(n.extras().mask, before.extras().mask);
    assert_eq!(n.children().unwrap().len(), 3, "spine and keys inside");
}

#[test]
fn object_expand_expands_blends_inside_the_selection() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0, 0.0, 10.0, 10.0), rect(&mut s, 100.0, 0.0, 10.0, 10.0));
    let g = make(&mut s, &[a, b], 2);
    let c = rect(&mut s, 0.0, 100.0, 10.0, 10.0);
    s.execute("select.set", &json!({"ids": [g.0, c.0]})).unwrap();
    let outer = id_of(&s.execute("object.group", &json!({})).unwrap());
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(true));
    s.execute("object.expand", &json!({"object": true, "fill": false, "stroke": false})).unwrap();
    let inner = node(&s, g);
    assert!(matches!(inner.kind, NodeKind::Group { .. }), "the blend inside the group expanded");
    assert_eq!(inner.children().unwrap().len(), 4);
    assert!(s.doc().unwrap().doc.node(outer).is_some());
    let _ = Paint::None;
    let _ = Point::ZERO;
}
