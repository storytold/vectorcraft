//! Pathfinder panel › compound shapes (`object.compoundShape.*`): Make Compound Shape in each
//! shape mode, a member's mode, Release, Expand, Undo, saving and exporting; blends, envelopes and
//! warped objects as members.

use serde_json::{Value, json};
use vectorcraft_color::{BlendMode, Color};
use vectorcraft_doc::{Node, ShapeMode};
use vectorcraft_render::effects;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn area(path: &vectorcraft_geom::PathData) -> f64 {
    vectorcraft_pathops::area(path, vectorcraft_geom::FillRule::NonZero).abs()
}

/// The area a path or compound path fills.
fn node_area(n: &Node) -> f64 {
    area(&crate::cmd::pathops::node_path(n).unwrap().0)
}

fn square(s: &mut Session, x: f64, color: &str) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 0, "width": 100, "height": 100}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"color": color}));
    id
}

/// A red square (0–100) behind a blue one (50–150), both 100 high and selected → (session, back, front).
fn two_squares() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let back = square(&mut s, 0.0, "#ff0000");
    let front = square(&mut s, 50.0, "#0000ff");
    run(&mut s, "select.set", json!({"ids": [back.0, front.0]}));
    (s, back, front)
}

fn selected(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.objects.clone()
}

/// Make Compound Shape (Alt-clicking a Shape Mode gives its mode) keeps both squares as members in
/// the mode, takes the paint of the topmost member not subtracting, draws the area the modes make,
/// and Expand turns it into that area as one path; Undo brings each step back.
#[test]
fn make_and_expand_each_shape_mode() {
    for (mode, want, fill, kind) in [
        ("add", 15_000.0, Color::rgb(0.0, 0.0, 1.0), "Path"),
        ("subtract", 5_000.0, Color::rgb(1.0, 0.0, 0.0), "Path"),
        ("intersect", 5_000.0, Color::rgb(0.0, 0.0, 1.0), "Path"),
        ("exclude", 10_000.0, Color::rgb(0.0, 0.0, 1.0), "Compound Path"),
    ] {
        let (mut s, back, front) = two_squares();
        let before = s.doc().unwrap().doc.clone();
        let id = NodeId(run(&mut s, "object.compoundShape.make", json!({"mode": mode}))["id"].as_u64().unwrap());
        assert_eq!(selected(&s), vec![id]);
        let shape = node(&s, id);
        assert!(shape.is_compound_shape(), "{mode}");
        assert_eq!(shape.kind_label(), "Compound Shape");
        let members: Vec<NodeId> = shape.children().unwrap().iter().map(|m| m.id).collect();
        assert_eq!(members, vec![back, front], "the members, in their stacking order");
        assert!(shape.children().unwrap().iter().all(|m| m.extras().shape_mode.name() == mode));
        assert_eq!(shape.appearance.fill_paint().color(), Some(fill), "{mode}");
        let drawn = effects::compound_shape_area(&shape, None).unwrap();
        assert!((area(&drawn) - want).abs() < 0.5, "{mode}: {}", area(&drawn));
        // Expand: the path, or compound path, of the area, painted as the compound shape.
        let made = run(&mut s, "object.compoundShape.expand", json!({}));
        let path = NodeId(made["ids"][0].as_u64().unwrap());
        let n = node(&s, path);
        assert_eq!(n.kind_label(), kind, "{mode}");
        assert!((node_area(&n) - want).abs() < 0.5, "{mode}");
        assert_eq!(n.appearance.fill_paint().color(), Some(fill));
        assert_eq!(selected(&s), vec![path]);
        assert!(s.doc().unwrap().doc.node(back).is_none() && s.doc().unwrap().doc.node(id).is_none());
        run(&mut s, "edit.undo", json!({}));
        assert!(node(&s, id).is_compound_shape());
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc().unwrap().doc.layers, before.layers, "{mode}: Undo leaves the squares as they were");
    }
}

/// Make Compound Shape from the panel menu gives every member Add; Release gives the members back
/// as they were, selected, in one undo step.
#[test]
fn release_returns_the_members_as_they_were() {
    let (mut s, back, front) = two_squares();
    let before = s.doc().unwrap().doc.clone();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({}))["id"].as_u64().unwrap());
    assert!(node(&s, id).children().unwrap().iter().all(|m| m.extras().shape_mode == ShapeMode::Add));
    run(&mut s, "object.compoundShape.setMode", json!({"mode": "subtract", "ids": [front.0]}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let undo = s.doc().unwrap().history.undo.len();
    let out = run(&mut s, "object.compoundShape.release", json!({}));
    assert_eq!(out["ids"], json!([back.0, front.0]));
    assert_eq!(selected(&s), vec![back, front]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    assert_eq!(s.doc().unwrap().doc.layers, before.layers, "the members' paint and place, no shape modes");
    // Nothing selected is in a compound shape: Release and Expand are off.
    assert!(s.execute("object.compoundShape.release", &json!({})).is_err());
    assert!(s.execute("object.compoundShape.expand", &json!({})).is_err());
    assert!(s.execute("object.compoundShape.setMode", &json!({"mode": "add"})).is_err());
}

/// A member selected (Direct Selection) takes the mode of the Shape Mode clicked; the area follows,
/// and Undo puts the mode back. Release and Expand act on the compound shape it is in.
#[test]
fn a_selected_member_takes_a_shape_mode() {
    let (mut s, _, front) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [front.0]}));
    assert_eq!(cmd::selected_member_modes(&s), vec![ShapeMode::Add]);
    run(&mut s, "object.compoundShape.setMode", json!({"mode": "intersect"}));
    assert_eq!(cmd::selected_member_modes(&s), vec![ShapeMode::Intersect]);
    assert!((area(&effects::compound_shape_area(&node(&s, id), None).unwrap()) - 5_000.0).abs() < 0.5);
    // The same mode again is no edit.
    let undo = s.doc().unwrap().history.undo.len();
    run(&mut s, "object.compoundShape.setMode", json!({"mode": "intersect"}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    assert!(s.execute("object.compoundShape.setMode", &json!({"mode": "divide"})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, front).extras().shape_mode, ShapeMode::Add);
    run(&mut s, "select.set", json!({"ids": [front.0]}));
    let made = run(&mut s, "object.compoundShape.expand", json!({}));
    assert!((node_area(&node(&s, NodeId(made["ids"][0].as_u64().unwrap()))) - 15_000.0).abs() < 0.5);
}

/// A compound shape holds its paint: a fill given to it (or to a member) paints the compound shape,
/// whose members keep theirs.
#[test]
fn paint_goes_to_the_compound_shape() {
    let (mut s, back, front) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({}))["id"].as_u64().unwrap());
    let out = run(&mut s, "paint.setFill", json!({"color": "#00ff00"}));
    assert_eq!(out["objects"], 1);
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)));
    run(&mut s, "paint.setFill", json!({"color": "#ffff00", "ids": [front.0]}));
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(1.0, 1.0, 0.0)));
    assert_eq!(node(&s, back).appearance.fill_paint().color(), Some(Color::rgb(1.0, 0.0, 0.0)));
    assert_eq!(node(&s, front).appearance.fill_paint().color(), Some(Color::rgb(0.0, 0.0, 1.0)));
}

/// Make Compound Shape needs two or more objects it can take.
#[test]
fn make_needs_two_objects_it_can_take() {
    let (mut s, back, _) = two_squares();
    run(&mut s, "select.set", json!({"ids": [back.0]}));
    assert!(s.execute("object.compoundShape.make", &json!({})).is_err());
    let (mut s, back, front) = two_squares();
    assert!(s.execute("object.compoundShape.make", &json!({"mode": "divide"})).is_err());
    assert_eq!(selected(&s), vec![back, front], "nothing changed");
}

/// Saved and opened again, a compound shape keeps its members and their modes; a mode a file names
/// that isn't one reads as Add.
#[test]
fn compound_shapes_survive_a_save() {
    let (mut s, _, front) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({"mode": "subtract"}))["id"].as_u64().unwrap());
    let doc = s.doc().unwrap().doc.clone();
    let bytes = vectorcraft_format::save(&doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.node(id), doc.node(id));
    assert!(back.node(id).unwrap().is_compound_shape());
    assert_eq!(back.node(front).unwrap().extras().shape_mode, ShapeMode::Subtract);
    let text = String::from_utf8(bytes).unwrap().replace("\"shape_mode\":\"subtract\"", "\"shape_mode\":\"divide\"");
    let odd = vectorcraft_format::load(text.as_bytes()).unwrap();
    assert_eq!(odd.node(front).unwrap().extras().shape_mode, ShapeMode::Add);
}

/// SVG export draws the area, painted as the compound shape, not the members.
#[test]
fn svg_export_draws_the_area() {
    let (mut s, _, _) = two_squares();
    run(&mut s, "object.compoundShape.make", json!({"mode": "subtract"}));
    let svg = vectorcraft_svg::export(&s.doc().unwrap().doc, &Default::default());
    assert!(svg.contains("#ff0000"), "the back square's red paints the area");
    assert!(!svg.contains("#0000ff"), "the front square isn't drawn: {svg}");
}

/// The Shape Modes and Pathfinders take a compound shape as the area it draws.
#[test]
fn pathfinder_operations_take_the_area() {
    let (mut s, _, _) = two_squares();
    let id = NodeId(run(&mut s, "object.compoundShape.make", json!({"mode": "subtract"}))["id"].as_u64().unwrap());
    let r = NodeId(run(&mut s, "shape.rectangle", json!({"x": 200, "y": 0, "width": 50, "height": 100}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [id.0, r.0]}));
    let out = run(&mut s, "object.pathfinder.unite", json!({}));
    let n = node(&s, NodeId(out["ids"][0].as_u64().unwrap()));
    assert!((node_area(&n) - 10_000.0).abs() < 0.5, "the 50 × 100 area and the 50 × 100 rectangle: {}", node_area(&n));
}

fn rect(s: &mut Session, x: f64, y: f64, width: f64, height: f64) -> NodeId {
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": y, "width": width, "height": height}))["id"].as_u64().unwrap())
}

fn make(s: &mut Session, mode: &str) -> NodeId {
    NodeId(run(s, "object.compoundShape.make", json!({"mode": mode}))["id"].as_u64().unwrap())
}

/// The area compound shape `id` draws, symbol instances among its members as their symbols' art.
fn drawn(s: &Session, id: NodeId) -> f64 {
    let d = &s.doc().unwrap().doc;
    let hook = |c: &Node| effects::outline_object(Some(d), c);
    area(&effects::compound_shape_area(&node(s, id), Some(&hook)).unwrap())
}

/// A bar (0–100 across, 10–30 down) behind a blend of two 20 pt squares at 0 and 80 (0–20 down)
/// with one step between them, both selected → (session, bar, blend).
fn bar_and_blend() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let b = rect(&mut s, 80.0, 0.0, 20.0, 20.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let blend = NodeId(run(&mut s, "object.blend.make", json!({"steps": 1}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [bar.0, blend.0]}));
    (s, bar, blend)
}

/// A blend is a member as its steps: the area takes each step in its shape mode, follows a change
/// of the blend's steps, survives a save, and Expand turns it into that area; Release gives the
/// blend back as a blend.
#[test]
fn a_blend_is_a_member_as_its_steps() {
    // Each square covers 20 × 10 of the bar; five squares 20 apart cover all 100 of it.
    for (mode, want, more) in [("add", 2_600.0, 3_000.0), ("subtract", 1_400.0, 1_000.0)] {
        let (mut s, bar, blend) = bar_and_blend();
        let id = make(&mut s, mode);
        let members: Vec<NodeId> = node(&s, id).children().unwrap().iter().map(|m| m.id).collect();
        assert_eq!(members, vec![bar, blend]);
        assert!((drawn(&s, id) - want).abs() < 0.5, "{mode}: {}", drawn(&s, id));
        run(&mut s, "select.set", json!({"ids": [blend.0]}));
        run(&mut s, "object.blend.options", json!({"spacing": "steps", "value": 3}));
        assert!((drawn(&s, id) - more).abs() < 0.5, "{mode}, three steps: {}", drawn(&s, id));
        let doc = s.doc().unwrap().doc.clone();
        let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
        assert_eq!(back.node(id), doc.node(id));
        assert!((area(&effects::compound_shape_area(back.node(id).unwrap(), None).unwrap()) - more).abs() < 0.5);
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        let made = run(&mut s, "object.compoundShape.expand", json!({}));
        assert!((node_area(&node(&s, NodeId(made["ids"][0].as_u64().unwrap()))) - more).abs() < 0.5, "{mode}");
        run(&mut s, "edit.undo", json!({}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "object.compoundShape.release", json!({}));
        assert_eq!(selected(&s), vec![bar, blend]);
        assert!(matches!(node(&s, blend).kind, NodeKind::Blend { .. }), "{mode}: still a blend");
    }
}

/// An envelope is a member as its distorted art: moving a point of its mesh reshapes the area, and
/// Release gives the envelope back.
#[test]
fn an_envelope_is_a_member_as_its_distorted_art() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let content = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    run(&mut s, "select.set", json!({"ids": [content.0]}));
    let env = NodeId(run(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [env.0, dot.0]}));
    let id = make(&mut s, "add");
    assert!((drawn(&s, id) - 10_100.0).abs() < 1.0, "{}", drawn(&s, id));
    // The corner at (100, 100) moved 100 to the right: a trapezoid of (100 + 200) / 2 × 100.
    let NodeKind::Envelope { kind, .. } = node(&s, env).kind else { panic!("an envelope") };
    let vectorcraft_doc::live::EnvelopeKind::Mesh { points, .. } = *kind else { panic!("a mesh envelope") };
    let index = points.iter().position(|p| (p.x - 100.0).abs() < 1e-6 && (p.y - 100.0).abs() < 1e-6).unwrap();
    run(&mut s, "object.envelope.setMeshPoint", json!({"id": env.0, "index": index, "x": 200, "y": 100}));
    assert!((drawn(&s, id) - 15_100.0).abs() < 1.0, "{}", drawn(&s, id));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.compoundShape.release", json!({}));
    assert_eq!(selected(&s), vec![env, dot]);
    assert!(matches!(node(&s, env).kind, NodeKind::Envelope { .. }), "still an envelope");
}

/// An object with a warp is a member as it draws: the area follows the warp's bend, and Release
/// gives the object back with its warp.
#[test]
fn a_warped_object_is_a_member_as_it_draws() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let r = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    run(&mut s, "select.set", json!({"ids": [r.0]}));
    run(&mut s, "effect.apply", json!({"effect": "warp.bulge", "params": {"bend": 50}}));
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [r.0, dot.0]}));
    let id = make(&mut s, "add");
    // The rectangle as the canvas draws it.
    let warped = |s: &Session| {
        let n = node(s, r);
        let (path, _) = crate::cmd::pathops::node_path(&n).unwrap();
        let bounds = vectorcraft_geom::Shape::bounding_box(&path.to_bezpath());
        area(&effects::apply_geometry_with(&n.appearance.effects, &path, bounds, &effects::GeomContext::of(&n)))
    };
    let bulged = warped(&s);
    assert!((bulged - 5_000.0).abs() > 100.0, "the warp changes the area: {bulged}");
    assert!((drawn(&s, id) - (bulged + 100.0)).abs() < 1.0, "{} vs {}", drawn(&s, id), bulged + 100.0);
    run(&mut s, "effect.setParams", json!({"ids": [r.0], "index": 0, "item": null, "params": {"bend": -50}}));
    let pinched = warped(&s);
    assert!((pinched - bulged).abs() > 100.0, "{pinched} vs {bulged}");
    assert!((drawn(&s, id) - (pinched + 100.0)).abs() < 1.0, "{} vs {}", drawn(&s, id), pinched + 100.0);
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.compoundShape.release", json!({}));
    assert_eq!(selected(&s), vec![r, dot]);
    assert_eq!(node(&s, r).appearance.effects.first().map(|e| e.id.as_str()), Some("warp.bulge"), "it keeps its warp");
}

/// A session with a green 20 pt square (0–20 across and down) made into the symbol "Dot", its
/// instance selected → (session, instance).
fn dot_symbol() -> (Session, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    run(&mut s, "paint.setFill", json!({"color": "#00ff00"}));
    let a = NodeId(run(&mut s, "symbol.new", json!({"name": "Dot"}))["id"].as_u64().unwrap());
    (s, a)
}

/// Symbol instances count as their symbols' art: a blend of two instances is a member as its
/// steps and gives the compound shape their paint, an envelope around an instance is a member as
/// its distorted art, and an instance inside a group as its art; Expand makes the same area.
#[test]
fn symbol_instances_in_members_count_as_their_art() {
    for (mode, want) in [("add", 2_600.0), ("subtract", 1_400.0)] {
        let (mut s, a) = dot_symbol();
        let b = NodeId(run(&mut s, "symbol.place", json!({"name": "Dot", "x": 90, "y": 10}))["id"].as_u64().unwrap());
        run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
        let blend = NodeId(run(&mut s, "object.blend.make", json!({"steps": 1}))["id"].as_u64().unwrap());
        let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
        run(&mut s, "select.set", json!({"ids": [bar.0]}));
        run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
        run(&mut s, "object.arrange.sendToBack", json!({}));
        run(&mut s, "select.set", json!({"ids": [bar.0, blend.0]}));
        let id = make(&mut s, mode);
        assert!((drawn(&s, id) - want).abs() < 0.5, "blend, {mode}: {}", drawn(&s, id));
        if mode == "add" {
            assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)), "the blend's paint");
        }
        assert!((expanded(&mut s, id) - want).abs() < 0.5, "expanded, {mode}");
    }
    let (mut s, _) = dot_symbol();
    let env = NodeId(run(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [env.0, dot.0]}));
    let id = make(&mut s, "add");
    assert!((drawn(&s, id) - 500.0).abs() < 1.0, "envelope: {}", drawn(&s, id));
    // The corner at (20, 20) moved 20 to the right: a trapezoid of (20 + 40) / 2 × 20.
    let NodeKind::Envelope { kind, .. } = node(&s, env).kind else { panic!("an envelope") };
    let vectorcraft_doc::live::EnvelopeKind::Mesh { points, .. } = *kind else { panic!("a mesh envelope") };
    let index = points.iter().position(|p| (p.x - 20.0).abs() < 1e-6 && (p.y - 20.0).abs() < 1e-6).unwrap();
    run(&mut s, "object.envelope.setMeshPoint", json!({"id": env.0, "index": index, "x": 40, "y": 20}));
    assert!((drawn(&s, id) - 700.0).abs() < 1.0, "envelope, distorted: {}", drawn(&s, id));
    assert!((expanded(&mut s, id) - 700.0).abs() < 1.0, "envelope, expanded");
    let (mut s, _) = dot_symbol();
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    assert!((drawn(&s, id) - 500.0).abs() < 1.0, "group: {}", drawn(&s, id));
    assert!((expanded(&mut s, id) - 500.0).abs() < 1.0, "group, expanded");
}

/// The 20 pt square symbol "Dot", a symbol "Ring" whose art is an instance of "Dot" and a symbol
/// "Halo" whose art is an instance of "Ring", with an instance of "Halo" in the square's place →
/// (session, the instance of "Halo").
fn halo_symbol() -> (Session, NodeId) {
    let (mut s, dot) = dot_symbol();
    let ring = NodeId(run(&mut s, "symbol.new", json!({"name": "Ring", "ids": [dot.0]}))["id"].as_u64().unwrap());
    let halo = NodeId(run(&mut s, "symbol.new", json!({"name": "Halo", "ids": [ring.0]}))["id"].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    for (outer, inner) in [("Halo", "Ring"), ("Ring", "Dot")] {
        let Some(sym) = d.symbols.iter().find(|x| x.name == outer) else { panic!("the symbol {outer}") };
        let mut holds = false;
        sym.art.walk(&mut |c| holds |= matches!(&c.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == inner));
        assert!(holds, "{outer} holds an instance of {inner}");
    }
    (s, halo)
}

/// Instances of a symbol whose art holds instances of other symbols, nested three deep, count as
/// the innermost symbol's art: in a blend and inside a group.
#[test]
fn nested_symbol_instances_in_members_count_as_their_art() {
    for (mode, want) in [("add", 2_600.0), ("subtract", 1_400.0)] {
        let (mut s, a) = halo_symbol();
        let b = NodeId(run(&mut s, "symbol.place", json!({"name": "Halo", "x": 90, "y": 10}))["id"].as_u64().unwrap());
        run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
        let blend = NodeId(run(&mut s, "object.blend.make", json!({"steps": 1}))["id"].as_u64().unwrap());
        let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
        run(&mut s, "select.set", json!({"ids": [bar.0]}));
        run(&mut s, "object.arrange.sendToBack", json!({}));
        run(&mut s, "select.set", json!({"ids": [bar.0, blend.0]}));
        let id = make(&mut s, mode);
        assert!((drawn(&s, id) - want).abs() < 0.5, "blend, {mode}: {}", drawn(&s, id));
        assert!((expanded(&mut s, id) - want).abs() < 0.5, "blend, expanded, {mode}");
    }
    let (mut s, halo) = halo_symbol();
    run(&mut s, "select.set", json!({"ids": [halo.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    assert!((drawn(&s, id) - 500.0).abs() < 1.0, "group: {}", drawn(&s, id));
    assert!((expanded(&mut s, id) - 500.0).abs() < 1.0, "group, expanded");
}

/// A symbol whose art holds two instances of itself counts as its art once, in a blend and inside
/// a group, without evaluating forever.
#[test]
fn a_symbol_holding_itself_counts_once() {
    let holding_itself = |s: &mut Session| {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let Some(sym) = d.symbols.iter_mut().find(|x| x.name == "Dot") else { panic!("the symbol Dot") };
        let me = |id| std::sync::Arc::new(Node::new(NodeId(id), NodeKind::SymbolInstance { symbol: "Dot".into(), xf: Default::default() }));
        sym.art = std::sync::Arc::new(Node::group(NodeId(9_997), vec![sym.art.clone(), me(9_998), me(9_999)]));
    };
    let (mut s, a) = dot_symbol();
    let b = NodeId(run(&mut s, "symbol.place", json!({"name": "Dot", "x": 90, "y": 10}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let blend = NodeId(run(&mut s, "object.blend.make", json!({"steps": 1}))["id"].as_u64().unwrap());
    let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
    run(&mut s, "select.set", json!({"ids": [bar.0, blend.0]}));
    let id = make(&mut s, "add");
    holding_itself(&mut s);
    assert!((drawn(&s, id) - 2_600.0).abs() < 0.5, "blend: {}", drawn(&s, id));
    let (mut s, a) = dot_symbol();
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    holding_itself(&mut s);
    assert!((drawn(&s, id) - 500.0).abs() < 1.0, "group: {}", drawn(&s, id));
}

/// Symbols whose art holds two instances of the next symbol, 32 deep (2³¹ squares in all), stop at
/// the symbol art budget: the area of a compound shape with an instance of the first in a member
/// is left as past its budget, and the member's shapes stop there.
#[test]
fn branching_symbol_art_stops_at_its_budget() {
    let (mut s, a) = dot_symbol();
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    {
        let d = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
        let Some(first) = d.symbols.iter().position(|x| x.name == "Dot") else { panic!("the symbol Dot") };
        let square = d.symbols[first].art.clone();
        let two = |level: u64| {
            let one = |k: u64| {
                let symbol = format!("Level {level}");
                std::sync::Arc::new(Node::new(NodeId(20_000 + 2 * level + k), NodeKind::SymbolInstance { symbol, xf: Default::default() }))
            };
            std::sync::Arc::new(Node::group(NodeId(10_000 + level), vec![one(0), one(1)]))
        };
        d.symbols[first].art = two(1);
        for level in 1..32 {
            let mut sym = d.symbols[first].clone();
            sym.name = format!("Level {level}");
            sym.art = if level == 31 { square.clone() } else { two(level + 1) };
            d.symbols.push(sym);
        }
    }
    let d = &s.doc().unwrap().doc;
    let hook = |c: &Node| effects::outline_object(Some(d), c);
    assert!(effects::compound_shape_area(&node(&s, id), Some(&hook)).is_none(), "past the budget");
    let leaves = effects::member_leaves(&node(&s, group), Some(&hook));
    assert!(!leaves.is_empty() && leaves.len() as u64 <= effects::SYMBOL_ART_MAX_NODES, "{} shapes", leaves.len());
}

/// A group whose frontmost member is a blend or a symbol instance gives the compound shape the
/// paint of that member's art, not that of a member below it.
#[test]
fn a_group_of_a_blend_or_an_instance_gives_its_paint() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
    let b = rect(&mut s, 80.0, 0.0, 20.0, 20.0);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    run(&mut s, "paint.setFill", json!({"color": "#00ff00"}));
    run(&mut s, "object.blend.make", json!({"steps": 1}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [bar.0, group.0]}));
    let id = make(&mut s, "add");
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)), "the blend's paint");
    let (mut s, a) = dot_symbol();
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
    run(&mut s, "paint.setFill", json!({"color": "#ff0000"}));
    run(&mut s, "object.arrange.sendToBack", json!({}));
    run(&mut s, "select.set", json!({"ids": [bar.0, group.0]}));
    let id = make(&mut s, "add");
    assert_eq!(node(&s, id).appearance.fill_paint().color(), Some(Color::rgb(0.0, 1.0, 0.0)), "the instance's paint");
}

/// A topmost blend, envelope or group with its own opacity and blend mode gives them to the
/// compound shape, multiplied by the opacity of the shape inside it.
#[test]
fn a_topmost_blend_envelope_or_group_gives_its_transparency() {
    for member in ["blend", "envelope", "group"] {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 400, "height": 300}));
        let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
        let a = rect(&mut s, 0.0, 0.0, 20.0, 20.0);
        let ids = if member == "blend" {
            let b = rect(&mut s, 80.0, 0.0, 20.0, 20.0);
            json!([a.0, b.0])
        } else {
            json!([a.0])
        };
        run(&mut s, "select.set", json!({"ids": ids}));
        let top = match member {
            "blend" => run(&mut s, "object.blend.make", json!({"steps": 1})),
            "envelope" => run(&mut s, "object.envelope.makeWithMesh", json!({"rows": 1, "cols": 1})),
            _ => {
                run(&mut s, "transparency.set", json!({"item": null, "opacity": 80}));
                run(&mut s, "object.group", json!({}))
            }
        };
        let top = NodeId(top["id"].as_u64().unwrap());
        run(&mut s, "select.set", json!({"ids": [top.0]}));
        run(&mut s, "transparency.set", json!({"item": null, "opacity": 50, "blend": "multiply"}));
        run(&mut s, "select.set", json!({"ids": [bar.0, top.0]}));
        let id = make(&mut s, "add");
        let n = node(&s, id);
        let want = if member == "group" { 0.4 } else { 0.5 };
        assert!((n.opacity - want).abs() < 1e-6, "{member}: {}", n.opacity);
        assert_eq!(n.blend, BlendMode::Multiply, "{member}");
    }
}

/// A symbol instance with a warp is a member as it draws: its symbol's art reshaped by the warp,
/// inside a group and as the shapes the instance gives alone.
#[test]
fn a_warped_symbol_instance_is_a_member_as_it_draws() {
    let (mut s, a) = dot_symbol();
    run(&mut s, "effect.apply", json!({"effect": "warp.bulge", "params": {"bend": 50}}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    let d = &s.doc().unwrap().doc;
    let Some(sym) = d.symbols.iter().find(|x| x.name == "Dot") else { panic!("the symbol Dot") };
    let Some(art) = effects::reshape(Some(d), &node(&s, a), Some(&sym.art)) else { panic!("the warped art") };
    let mut warped = 0.0;
    art.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Path { .. }) {
            warped += node_area(c);
        }
    });
    assert!((warped - 400.0).abs() > 10.0, "the warp changes the area: {warped}");
    assert!((drawn(&s, id) - (warped + 100.0)).abs() < 1.0, "{} vs {}", drawn(&s, id), warped + 100.0);
    let hook = |c: &Node| effects::outline_object(Some(d), c);
    let alone: f64 = effects::member_leaves(&node(&s, a), Some(&hook)).iter().map(node_area).sum();
    assert!((alone - warped).abs() < 1.0, "the instance alone: {alone} vs {warped}");
}

/// A symbol instance gives the compound shape the transparency its art is drawn with: that of an
/// instance of another symbol inside the art, within the instance's own.
#[test]
fn an_instance_gives_the_transparency_of_its_art() {
    let (mut s, a) = dot_symbol();
    run(&mut s, "transparency.set", json!({"item": null, "opacity": 50, "blend": "multiply"}));
    let outer = NodeId(run(&mut s, "symbol.new", json!({"name": "Outer", "ids": [a.0]}))["id"].as_u64().unwrap());
    run(&mut s, "select.set", json!({"ids": [outer.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let bar = rect(&mut s, 0.0, 10.0, 100.0, 20.0);
    run(&mut s, "object.arrange.sendToBack", json!({}));
    run(&mut s, "select.set", json!({"ids": [bar.0, group.0]}));
    let NodeKind::SymbolInstance { .. } = node(&s, outer).kind else { panic!("an instance of Outer") };
    let id = make(&mut s, "add");
    let n = node(&s, id);
    assert!((n.opacity - 0.5).abs() < 1e-6, "{}", n.opacity);
    assert_eq!(n.blend, BlendMode::Multiply);
}

/// The area the paths and compound paths in `art` fill, at every depth of its groups.
fn art_area(art: &Node) -> f64 {
    match &art.kind {
        NodeKind::Path { .. } | NodeKind::Compound { .. } => node_area(art),
        _ => art.children().map_or(0.0, |ch| ch.iter().map(|c| art_area(c)).sum()),
    }
}

/// A session with the type "Hello bo" made into the symbol "Word", its instance selected →
/// (session, instance).
fn word_symbol() -> (Session, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    run(&mut s, "text.create", json!({"x": 10, "y": 50, "text": "Hello bo", "size": 36}));
    let a = NodeId(run(&mut s, "symbol.new", json!({"name": "Word"}))["id"].as_u64().unwrap());
    let Some(sym) = s.doc().unwrap().doc.symbols.iter().find(|x| x.name == "Word").cloned() else { panic!("the symbol Word") };
    let mut holds = false;
    sym.art.walk(&mut |c| holds |= matches!(c.kind, NodeKind::Text(_)));
    assert!(holds, "Word holds type");
    (s, a)
}

/// Type inside a symbol's art counts as its outlines: an instance in a group gives the area of the
/// glyphs, and a warped instance the area of the glyphs reshaped by the warp.
#[test]
fn type_in_symbol_art_counts_as_its_outlines() {
    let (mut s, a) = word_symbol();
    let unwarped = {
        let d = &s.doc().unwrap().doc;
        let hook = |c: &Node| effects::outline_object(Some(d), c);
        effects::member_leaves(&node(&s, a), Some(&hook)).iter().map(node_area).sum::<f64>()
    };
    assert!(unwarped > 200.0, "the glyphs: {unwarped}");
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    assert!((drawn(&s, id) - (unwarped + 100.0)).abs() < 1.0, "group: {} vs {}", drawn(&s, id), unwarped + 100.0);

    let (mut s, a) = word_symbol();
    run(&mut s, "effect.apply", json!({"effect": "warp.bulge", "params": {"bend": 50}}));
    let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let dot = rect(&mut s, 300.0, 0.0, 10.0, 10.0);
    run(&mut s, "select.set", json!({"ids": [group.0, dot.0]}));
    let id = make(&mut s, "add");
    let d = &s.doc().unwrap().doc;
    let Some(sym) = d.symbols.iter().find(|x| x.name == "Word") else { panic!("the symbol Word") };
    let Some(art) = effects::reshape(Some(d), &node(&s, a), Some(&sym.art)) else { panic!("the warped art") };
    let warped = art_area(&art);
    assert!((warped - unwarped).abs() > 10.0, "the warp changes the area: {warped} vs {unwarped}");
    assert!((drawn(&s, id) - (warped + 100.0)).abs() < 1.0, "warped: {} vs {}", drawn(&s, id), warped + 100.0);
}

/// The Pathfinder panel counts a compound shape of two blends by its members, so the operations
/// that take one shape (Outline, Divide) are available for it.
#[test]
fn a_compound_shape_of_blends_counts_its_members() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let mut blends = vec![];
    for y in [0.0, 50.0] {
        let a = rect(&mut s, 0.0, y, 20.0, 20.0);
        let b = rect(&mut s, 80.0, y, 20.0, 20.0);
        run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
        blends.push(run(&mut s, "object.blend.make", json!({"steps": 1}))["id"].as_u64().unwrap());
    }
    run(&mut s, "select.set", json!({"ids": blends}));
    let id = make(&mut s, "add");
    assert_eq!(crate::cmd::pathops::shape_count(&node(&s, id)), 2);
}

/// Expand compound shape `id` → the area of the path it makes.
fn expanded(s: &mut Session, id: NodeId) -> f64 {
    run(s, "select.set", json!({"ids": [id.0]}));
    let made = run(s, "object.compoundShape.expand", json!({}));
    node_area(&node(s, NodeId(made["ids"][0].as_u64().unwrap())))
}
