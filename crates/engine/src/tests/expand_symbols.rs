//! Object › Expand on symbol instances: a selected instance without effects becomes its art, as
//! Break Link to Symbol places it, in one undo step. The instances that art holds stay instances.

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

/// A 20 × 20 square at (50, 50) with a 2 pt blue stroke and no fill, selected.
fn stroked_square(s: &mut Session) -> NodeId {
    let r = rect(s, 50.0, 50.0, 20.0, 20.0);
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let blue = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 255));
    doc.node_mut(r).unwrap().appearance = vectorcraft_doc::Appearance::basic(vectorcraft_color::Paint::None, blue, 2.0).into();
    s.select(|_, sel| sel.set([r])).unwrap();
    r
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

/// Expand goes one level deep: the instance that a symbol's art holds stays an instance.
#[test]
fn the_instances_in_the_art_stay_instances() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let inner = square_symbol(&mut s, "Inner");
    let other = rect(&mut s, 150.0, 50.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([inner, other])).unwrap();
    let outer = NodeId(s.execute("symbol.new", &json!({"name": "Outer"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let art = s.doc().unwrap().doc.node(outer).unwrap();
    assert!(matches!(art.kind, NodeKind::Group { .. }), "the selected instance expands: {}", art.kind_label());
    let mut instances = vec![];
    art.walk(&mut |c| {
        if let NodeKind::SymbolInstance { symbol, .. } = &c.kind {
            instances.push(symbol.clone());
        }
    });
    assert_eq!(instances, vec!["Inner".to_string()], "the instance in its art stays an instance");
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
    assert!(!is_instance(&s, id));
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

/// The Stroke option counts the strokes in the art of the selected instances, so the Expand
/// dialog offers it, and Stroke outlines them in the same command.
#[test]
fn expand_options_count_the_art_of_instances() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    stroked_square(&mut s);
    s.execute("symbol.new", &json!({"name": "Ring"})).unwrap();
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!((info["object"].clone(), info["stroke"].clone()), (json!(true), json!(true)), "{info}");
    s.execute("object.expand", &json!({"fill": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let (mut paths, mut strokes) = (0, 0);
    for n in s.doc().unwrap().selection.objects.iter().filter_map(|i| d.node(*i)) {
        n.walk(&mut |c| {
            paths += usize::from(matches!(c.kind, NodeKind::Path { .. }));
            strokes += usize::from(!c.appearance.stroke_paint().is_none());
        });
    }
    assert!(paths > 0, "the selection holds the outlined art");
    assert_eq!(strokes, 0, "the stroke is outlined");
}

/// The Fill and Stroke options read a dynamic instance's art with its own changes: an instance
/// that removes the symbol's only stroke has no stroke to expand.
#[test]
fn expand_options_read_a_dynamic_instances_changes() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    stroked_square(&mut s);
    let id = NodeId(s.execute("symbol.new", &json!({"name": "Ring", "type": "dynamic"})).unwrap()["id"].as_u64().unwrap());
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["stroke"], json!(true));
    s.execute("symbol.instance.setAppearance", &json!({"id": id.0, "path": [], "stroke": {"none": true}})).unwrap();
    s.select(|_, sel| sel.set([id])).unwrap();
    let info = s.execute("object.expand.info", &json!({})).unwrap();
    assert_eq!(info["stroke"], json!(false), "{info}");
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

/// An instance with an opacity mask expands into art that keeps the mask, as Break Link gives it.
#[test]
fn an_instance_with_an_opacity_mask_keeps_it_on_its_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let masked = square_symbol(&mut s, "Square");
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let art = Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(Rect::new(50.0, 50.0, 60.0, 60.0)), Default::default());
    let mask = Box::new(vectorcraft_doc::OpacityMask::new(art, true));
    doc.node_mut(masked).unwrap().extras_mut().mask = Some(mask.clone());
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    assert!(!is_instance(&s, masked), "the instance expands");
    assert_eq!(s.doc().unwrap().doc.node(masked).unwrap().extras().mask, Some(mask), "the art keeps the mask");
}

/// An instance with an effect on its fill stays an instance under Object › Expand and keeps the
/// effect, beside an instance that expands.
#[test]
fn an_instance_with_an_effect_on_its_fill_stays_an_instance() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let plain = square_symbol(&mut s, "Square");
    let filled = NodeId(s.execute("symbol.place", &json!({"name": "Square", "x": 200, "y": 60})).unwrap()["id"].as_u64().unwrap());
    let red = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(255, 0, 0));
    let mut look = vectorcraft_doc::Appearance::basic(red, vectorcraft_color::Paint::None, 0.0);
    look.items[0].effects_mut().push(vectorcraft_render::effects::new_effect("stylize.dropShadow", &json!({"x": 4, "y": 0})).unwrap());
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).node_mut(filled).unwrap().appearance = look.into();
    s.select(|_, sel| sel.set([plain, filled])).unwrap();
    let _ = s.execute("object.expand", &json!({"fill": false, "stroke": false}));
    assert!(!is_instance(&s, plain), "the plain instance expands");
    assert!(is_instance(&s, filled), "the instance with an effect on its fill stays an instance");
    let n = s.doc().unwrap().doc.node(filled).unwrap().clone();
    assert_eq!(n.appearance.items[0].effects().len(), 1, "it keeps the effect");
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

/// A blend between two instances, the second turned 90°, still expands into the steps it draws:
/// Expand leaves the instances in a blend alone, and the blend's steps stay instances.
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
    let (mut blends, mut instances) = (0, 0);
    for id in &s.doc().unwrap().selection.objects {
        d.node(*id).unwrap().walk(&mut |c| {
            blends += usize::from(matches!(c.kind, NodeKind::Blend { .. }));
            instances += usize::from(matches!(c.kind, NodeKind::SymbolInstance { .. }));
        });
    }
    assert_eq!((blends, instances), (0, 3), "the steps stay instances");
}
