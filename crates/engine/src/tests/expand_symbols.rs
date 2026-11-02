//! Object › Expand on symbol instances: an instance without effects becomes its art, as Break
//! Link to Symbol places it, and so do the instances that art holds, in one undo step.

use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn is_instance(s: &Session, id: NodeId) -> bool {
    matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::SymbolInstance { .. })
}

/// A 20 × 20 square at (50, 50) made into symbol `name`, its instance selected.
fn square_symbol(s: &mut Session, name: &str) -> NodeId {
    let r = rect(s, 50.0, 50.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([r])).unwrap();
    NodeId(s.execute("symbol.new", &json!({"name": name})).unwrap()["id"].as_u64().unwrap())
}

/// The geometric boxes of the paths under `n`, in paint order.
fn path_boxes(n: &Node) -> Vec<Rect> {
    let mut out = vec![];
    n.walk(&mut |c| {
        if let NodeKind::Path { path, .. } = &c.kind {
            out.extend(path.bounds());
        }
    });
    out
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn expand_turns_a_symbol_instance_into_its_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Square");
    s.execute("object.setBounds", &json!({"x": 50, "y": 50, "width": 40, "height": 30, "reference": 0})).unwrap();
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(true));
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let art = d.node(id).expect("the art keeps the instance's id");
    assert!(!matches!(art.kind, NodeKind::SymbolInstance { .. }));
    let boxes = path_boxes(art);
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert!(close(boxes[0], Rect::new(50.0, 50.0, 90.0, 80.0)), "{boxes:?}");
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), vec![id]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(is_instance(&s, id), "one undo step");
}

/// The Fill and Stroke options count the strokes and gradients in the art of the selected
/// instances, nested ones included, so the Expand dialog offers them.
#[test]
fn expand_options_count_the_art_of_instances() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = rect(&mut s, 50.0, 50.0, 20.0, 20.0);
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let blue = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 255));
    doc.node_mut(r).unwrap().appearance = vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, blue, 2.0).into();
    s.select(|_, sel| sel.set([r])).unwrap();
    s.execute("symbol.new", &json!({"name": "Inner"})).unwrap();
    let id = NodeId(s.execute("symbol.new", &json!({"name": "Outer"})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.set([id])).unwrap();
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!((info["object"].clone(), info["stroke"].clone()), (json!(true), json!(true)), "{info}");
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let mut strokes = 0;
    s.doc().unwrap().doc.node(id).unwrap().walk(&mut |c| strokes += usize::from(!c.appearance.stroke_paint().is_none()));
    assert_eq!(strokes, 1, "Stroke off keeps the stroke");
}

/// The Fill and Stroke options read a dynamic instance's art with its own changes: an instance
/// that removes the symbol's only stroke has no stroke to expand.
#[test]
fn expand_options_read_a_dynamic_instances_changes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = rect(&mut s, 50.0, 50.0, 20.0, 20.0);
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let blue = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 255));
    doc.node_mut(r).unwrap().appearance = vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, blue, 2.0).into();
    s.select(|_, sel| sel.set([r])).unwrap();
    let id = NodeId(s.execute("symbol.new", &json!({"name": "Ring", "type": "dynamic"})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["stroke"], json!(true));
    s.execute("symbol.instance.setAppearance", &json!({"id": id.0, "path": [], "stroke": {"none": true}})).unwrap();
    s.select(|_, sel| sel.set([id])).unwrap();
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!(info["stroke"], json!(false), "{info}");
}

/// Past 4,096 symbol arts to read, the Fill and Stroke options are all offered: 4,097 changed
/// instances of a symbol without strokes report a stroke.
#[test]
fn expand_options_are_all_offered_past_the_art_limit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = rect(&mut s, 50.0, 50.0, 20.0, 20.0);
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let blue = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 255));
    doc.node_mut(r).unwrap().appearance = vectorcraft_doc::Appearance::basic(blue, vectorcraft_color::Paint::None, 0.0).into();
    s.select(|_, sel| sel.set([r])).unwrap();
    let id = NodeId(s.execute("symbol.new", &json!({"name": "Tile", "type": "dynamic"})).unwrap()["id"].as_u64().unwrap());
    s.execute("symbol.instance.setAppearance", &json!({"id": id.0, "path": [], "fill": "#ff0000"})).unwrap();
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let mut ids = vec![id];
    for i in 1..4097 {
        let mut copy = doc.node(id).unwrap().clone();
        copy.id = doc.alloc_id();
        ids.push(copy.id);
        let (par, idx, _) = doc.position(id).unwrap();
        doc.insert(par, idx + i, copy).unwrap();
    }
    s.select(|_, sel| sel.set([id])).unwrap();
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["stroke"], json!(false), "one instance");
    s.select(|_, sel| sel.set(ids.clone())).unwrap();
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["stroke"], json!(true), "4,097 instances");
}

/// Object › Expand also expands the instances that a symbol's art holds.
#[test]
fn expand_also_expands_the_instances_in_the_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    square_symbol(&mut s, "Inner");
    let outer = NodeId(s.execute("symbol.new", &json!({"name": "Outer"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let mut kinds = vec![];
    s.doc().unwrap().doc.node(outer).unwrap().walk(&mut |c| kinds.push(matches!(c.kind, NodeKind::SymbolInstance { .. })));
    assert!(!kinds.is_empty() && !kinds.contains(&true), "{kinds:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(is_instance(&s, outer), "one undo step");
}

/// An instance in a group expands too, and the group stays a group.
#[test]
fn expand_expands_an_instance_inside_a_group() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Square");
    let other = rect(&mut s, 150.0, 50.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([id, other])).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(matches!(d.node(group).unwrap().kind, NodeKind::Group { .. }));
    let mut instances = 0;
    d.node(group).unwrap().walk(&mut |c| instances += usize::from(matches!(c.kind, NodeKind::SymbolInstance { .. })));
    assert_eq!(instances, 0);
}

/// An instance of a dynamic symbol keeps its own appearance changes when Object › Expand turns it
/// into its art.
#[test]
fn a_dynamic_instance_keeps_its_changes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ids = [rect(&mut s, 50.0, 50.0, 20.0, 20.0), rect(&mut s, 80.0, 50.0, 20.0, 20.0)];
    s.select(|_, sel| sel.set(ids)).unwrap();
    let id = NodeId(s.execute("symbol.new", &json!({"name": "Pair", "type": "dynamic"})).unwrap()["id"].as_u64().unwrap());
    s.execute("symbol.instance.setAppearance", &json!({"id": id.0, "path": [1], "fill": "#ff0000"})).unwrap();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut fills = vec![];
    d.node(id).unwrap().walk(&mut |c| {
        if matches!(c.kind, NodeKind::Path { .. }) {
            fills.push(c.appearance.fill_paint().color().map(|c| c.to_hex()));
        }
    });
    assert_eq!(fills.len(), 2, "{fills:?}");
    assert_eq!(fills[1].as_deref(), Some("#ff0000"), "{fills:?}");
    assert_ne!(fills[0].as_deref(), Some("#ff0000"), "only the changed object: {fills:?}");
}

/// An instance whose opacity masks hold more than a million objects stays an instance under
/// Object › Expand, though the art outside the masks holds about a thousand. With nothing else
/// to expand, Expand reports it and adds no undo step.
#[test]
fn an_instance_whose_masks_hold_too_many_objects_stays_an_instance() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Masked");
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let square = doc.symbols.iter().find(|s| s.name == "Masked").unwrap().art.clone();
    let mask_art = Node::group(doc.alloc_id(), vec![square.clone(); 1_000]);
    let mut masked = (*square).clone();
    masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(mask_art, true)));
    let masked = std::sync::Arc::new(masked);
    let big = Node::group(doc.alloc_id(), vec![masked; 1_000]);
    assert!(big.count() < 1_000_000 && big.count_with_masks() > 1_000_000);
    doc.symbols.iter_mut().find(|s| s.name == "Masked").unwrap().art = std::sync::Arc::new(big);
    let steps = s.doc().unwrap().history.undo.len();
    assert!(s.execute("object.expand", &json!({"fill": false, "stroke": false})).is_err(), "nothing to expand");
    assert!(is_instance(&s, id), "Object › Expand leaves it an instance");
    assert_eq!(s.doc().unwrap().history.undo.len(), steps, "no undo step");
}

/// Three instances whose art together holds 999,999 objects all expand: each instance's art takes
/// its place, so the selection then holds 999,999 objects, under the limit.
#[test]
fn instances_whose_art_reaches_the_limit_exactly_expand() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Big");
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let square = doc.symbols.iter().find(|s| s.name == "Big").unwrap().art.clone();
    let big = Node::group(doc.alloc_id(), vec![square; 333_332]);
    assert_eq!(big.count_with_masks(), 333_333);
    doc.symbols.iter_mut().find(|s| s.name == "Big").unwrap().art = std::sync::Arc::new(big);
    let mut ids = vec![id];
    for i in 1..3 {
        let mut copy = doc.node(id).unwrap().clone();
        copy.id = doc.alloc_id();
        ids.push(copy.id);
        let (par, idx, _) = doc.position(id).unwrap();
        doc.insert(par, idx + i, copy).unwrap();
    }
    s.select(|_, sel| sel.set(ids.clone())).unwrap();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    assert!(ids.iter().all(|i| !is_instance(&s, *i)), "all three expand");
}

/// The opacity-mask art already in the selection counts against the limit: an instance whose art
/// would take the selection past it stays an instance.
#[test]
fn mask_art_in_the_selection_counts_against_the_limit() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Big");
    let other = rect(&mut s, 150.0, 50.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([id, other])).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let square = doc.symbols.iter().find(|s| s.name == "Big").unwrap().art.clone();
    let big = Node::group(doc.alloc_id(), vec![square.clone(); 500_000]);
    doc.symbols.iter_mut().find(|s| s.name == "Big").unwrap().art = std::sync::Arc::new(big);
    let mask_art = Node::group(doc.alloc_id(), vec![square; 600_000]);
    doc.node_mut(other).unwrap().extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(mask_art, true)));
    assert!(doc.node(group).unwrap().count() + 500_001 < 1_000_000);
    s.select(|_, sel| sel.set([group])).unwrap();
    let _ = s.execute("object.expand", &json!({"fill": false, "stroke": false}));
    assert!(is_instance(&s, id), "Object › Expand leaves it an instance");
}

/// An instance with effects is left to Expand Appearance; the instances in the art that baking
/// its effects leaves are expanded too.
#[test]
fn the_instances_left_by_baking_an_instances_effects_are_expanded() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    square_symbol(&mut s, "Inner");
    let outer = NodeId(s.execute("symbol.new", &json!({"name": "Outer"})).unwrap()["id"].as_u64().unwrap());
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let fx = vectorcraft_render::effects::new_effect("stylize.dropShadow", &json!({"x": 4, "y": 0})).unwrap();
    doc.node_mut(outer).unwrap().appearance.effects.push(fx);
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut instances = 0;
    for id in &s.doc().unwrap().selection.objects {
        d.node(*id).unwrap().walk(&mut |c| instances += usize::from(matches!(c.kind, NodeKind::SymbolInstance { .. })));
    }
    assert_eq!(instances, 0);
}

/// The effects in the art of an instance that baking another instance's effects leaves are
/// baked too, in the same command.
#[test]
fn effects_in_the_art_of_instances_left_by_baking_are_baked() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = rect(&mut s, 50.0, 50.0, 20.0, 20.0);
    let shadow = || vectorcraft_render::effects::new_effect("stylize.dropShadow", &json!({"x": 4, "y": 0})).unwrap();
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(r).unwrap().appearance.effects.push(shadow());
    s.select(|_, sel| sel.set([r])).unwrap();
    s.execute("symbol.new", &json!({"name": "Inner"})).unwrap();
    let outer = NodeId(s.execute("symbol.new", &json!({"name": "Outer"})).unwrap()["id"].as_u64().unwrap());
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(outer).unwrap().appearance.effects.push(shadow());
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (mut instances, mut effects) = (0, 0);
    for id in &s.doc().unwrap().selection.objects {
        d.node(*id).unwrap().walk(&mut |c| {
            instances += usize::from(matches!(c.kind, NodeKind::SymbolInstance { .. }));
            effects += c.appearance.effects.len();
        });
    }
    assert_eq!((instances, effects), (0, 0));
}

/// A blend between two instances, the second turned 90°, expands into the steps the blend draws:
/// the step halfway is the square turned 45°, and then every step is plain art.
#[test]
fn a_blend_of_instances_expands_into_the_steps_it_draws() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = square_symbol(&mut s, "Dot");
    let b = NodeId(s.execute("symbol.place", &json!({"name": "Dot", "x": 210, "y": 60})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.set([b])).unwrap();
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    let g = NodeId(s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.set([g])).unwrap();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (mut instances, mut boxes) = (0, vec![]);
    for id in &s.doc().unwrap().selection.objects {
        let n = d.node(*id).unwrap();
        n.walk(&mut |c| instances += usize::from(matches!(c.kind, NodeKind::SymbolInstance { .. })));
        boxes.extend(path_boxes(n));
    }
    assert_eq!(instances, 0);
    let turned = 20.0 * std::f64::consts::SQRT_2;
    assert!(boxes.iter().any(|r| (r.width() - turned).abs() < 1e-6), "the step halfway is turned 45°: {boxes:?}");
}

/// A linked opacity mask of a 10 × 10 square at (50, 50).
fn mask(s: &mut Session) -> Box<vectorcraft_doc::OpacityMask> {
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let art = Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(50.0, 50.0, 60.0, 60.0)), Default::default());
    Box::new(vectorcraft_doc::OpacityMask::new(art, true))
}

/// The instance's own opacity mask goes on its expanded art, and on a group around the art when
/// the symbol's art has a mask of its own.
#[test]
fn expand_keeps_the_instances_opacity_mask() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Square");
    let m = mask(&mut s);
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().extras_mut().mask = Some(m);
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let art = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(!matches!(art.kind, NodeKind::SymbolInstance { .. }));
    assert!(art.extras().mask.is_some(), "the art keeps the mask");

    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Masked");
    let (own, m) = (mask(&mut s), mask(&mut s));
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let sym = doc.symbols.iter_mut().find(|s| s.name == "Masked").unwrap();
    std::sync::Arc::make_mut(&mut sym.art).extras_mut().mask = Some(own);
    doc.node_mut(id).unwrap().extras_mut().mask = Some(m);
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let art = s.doc().unwrap().doc.node(id).unwrap().clone();
    let NodeKind::Group { children, .. } = &art.kind else { panic!("a group around the art: {}", art.kind_label()) };
    assert!(art.extras().mask.is_some(), "the group holds the instance's mask");
    assert!(children.len() == 1 && children[0].extras().mask.is_some(), "the art keeps its own mask");
}

/// Break Link to Symbol keeps the instance's opacity mask on the art too.
#[test]
fn break_link_keeps_the_instances_opacity_mask() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = square_symbol(&mut s, "Square");
    let m = mask(&mut s);
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(id).unwrap().extras_mut().mask = Some(m);
    let art = NodeId(s.execute("symbol.breakLink", &json!({"ids": [id.0]})).unwrap()["ids"][0].as_u64().unwrap());
    assert!(s.doc().unwrap().doc.node(art).unwrap().extras().mask.is_some(), "the art keeps the mask");
}

/// A hidden instance and a locked one inside a selected group expand into art that stays hidden
/// and locked.
#[test]
fn hidden_and_locked_instances_expand_into_hidden_and_locked_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let hidden = square_symbol(&mut s, "Square");
    let locked = NodeId(s.execute("symbol.place", &json!({"name": "Square", "x": 200, "y": 60})).unwrap()["id"].as_u64().unwrap());
    let other = rect(&mut s, 150.0, 150.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([hidden, locked, other])).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    doc.node_mut(hidden).unwrap().visible = false;
    doc.node_mut(locked).unwrap().locked = true;
    s.select(|_, sel| sel.set([group])).unwrap();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    assert!(!is_instance(&s, hidden) && !is_instance(&s, locked), "both expand");
    assert!(!d.node(hidden).unwrap().visible, "the hidden instance's art stays hidden");
    assert!(d.node(locked).unwrap().locked, "the locked instance's art stays locked");
}

/// An instance in perspective stays an instance under Object › Expand, beside an instance that
/// expands.
#[test]
fn an_instance_in_perspective_stays_an_instance() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 1000, "height": 800})).unwrap();
    s.execute("perspective.grid.preset", &json!({"kind": 2})).unwrap();
    let flat = square_symbol(&mut s, "Square");
    let projected = NodeId(s.execute("symbol.place", &json!({"name": "Square", "x": 500, "y": 350})).unwrap()["id"].as_u64().unwrap());
    s.execute("perspective.attach", &json!({"ids": [projected.0], "plane": "right"})).unwrap();
    assert!(s.doc().unwrap().doc.node(projected).unwrap().projection().is_some(), "the instance is in perspective");
    s.select(|_, sel| sel.set([flat, projected])).unwrap();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    assert!(!is_instance(&s, flat), "the flat instance expands");
    assert!(is_instance(&s, projected), "the instance in perspective stays an instance");
}

/// A blend between instances of a symbol whose art is a repeat expands completely in one
/// command: the repeats that expanding the steps' instances leaves are expanded too.
#[test]
fn repeats_in_the_art_of_a_blends_instances_are_expanded() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let square = rect(&mut s, 50.0, 50.0, 10.0, 10.0);
    let repeat = NodeId(s.execute("object.repeat.grid", &json!({"ids": [square.0]})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.set([repeat])).unwrap();
    let a = NodeId(s.execute("symbol.new", &json!({"name": "Grid"})).unwrap()["id"].as_u64().unwrap());
    let b = NodeId(s.execute("symbol.place", &json!({"name": "Grid", "x": 300, "y": 200})).unwrap()["id"].as_u64().unwrap());
    let g = NodeId(s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap()["id"].as_u64().unwrap());
    s.select(|_, sel| sel.set([g])).unwrap();
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let mut live = 0;
    for id in &s.doc().unwrap().selection.objects {
        d.node(*id).unwrap().walk(&mut |c| {
            live += usize::from(matches!(c.kind, NodeKind::SymbolInstance { .. } | NodeKind::Blend { .. } | NodeKind::Repeat(_)));
        });
    }
    assert_eq!(live, 0, "no instance, blend or repeat is left");
}
