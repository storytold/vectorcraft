//! Symbol Options › 9-slice guides: a scaled instance stays an instance and draws its corners at
//! their size; Break Link to Symbol and Object › Expand give the art as it draws, in one step.

use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-6)
}

/// A 100 × 100 frame with a 10 × 10 square in its top-left corner and one in its center, made
/// into symbol "Panel" with 9-slice guides on, its instance selected at its natural size.
fn panel_symbol() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let ids = [rect(&mut s, 50.0, 50.0, 100.0, 100.0), rect(&mut s, 50.0, 50.0, 10.0, 10.0), rect(&mut s, 95.0, 95.0, 10.0, 10.0)];
    s.select(|_, sel| sel.set(ids)).unwrap();
    let r = s.execute("symbol.new", &json!({"name": "Panel", "nineSlice": true})).unwrap();
    assert_eq!(r["nineSlice"], json!([0.25, 0.25, 0.25, 0.25]));
    (s, NodeId(r["id"].as_u64().unwrap()))
}

/// [`panel_symbol`] with its instance resized to 200 × 150 at (50, 50).
fn panel() -> (Session, NodeId) {
    let (mut s, id) = panel_symbol();
    s.execute("object.setBounds", &json!({"x": 50, "y": 50, "width": 200, "height": 150, "reference": 0})).unwrap();
    (s, id)
}

/// The stroke weights of the paths under `n`, in paint order.
fn stroke_widths(n: &Node) -> Vec<f64> {
    let mut out = vec![];
    n.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Path { .. }) {
            out.push(c.appearance.stroke_width());
        }
    });
    out
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

/// The art as a 200 × 150 instance draws it: the frame, the corner square at 10 × 10 and the
/// center square at 30 × 20.
fn assert_sliced(boxes: &[Rect]) {
    let want = [Rect::new(50.0, 50.0, 250.0, 200.0), Rect::new(50.0, 50.0, 60.0, 60.0), Rect::new(135.0, 115.0, 165.0, 135.0)];
    assert_eq!(boxes.len(), want.len(), "{boxes:?}");
    for (b, w) in boxes.iter().zip(want) {
        assert!(close(*b, w), "{b:?} vs {w:?} in {boxes:?}");
    }
}

#[test]
fn a_resized_instance_stays_linked_and_break_link_gives_the_sliced_art() {
    let (mut s, id) = panel();
    let d = &s.doc().unwrap().doc;
    let inst = d.node(id).unwrap().clone();
    assert!(matches!(inst.kind, NodeKind::SymbolInstance { .. }), "still an instance");
    assert!(close(inst.geometric_bounds_with(&d.symbol_boxes()).unwrap(), Rect::new(50.0, 50.0, 250.0, 200.0)));
    let r = s.execute("symbol.breakLink", &json!({})).unwrap();
    let art = NodeId(r["ids"][0].as_u64().unwrap());
    let d = &s.doc().unwrap().doc;
    assert_sliced(&path_boxes(d.node(art).unwrap()));
    // The pieces are editable paths: the frame, cut at the guides, is a plain path, and the
    // squares, each inside one cell, stay live rectangles.
    let mut live = vec![];
    d.node(art).unwrap().walk(&mut |c| match &c.kind {
        NodeKind::SymbolInstance { .. } => panic!("no instance is left"),
        NodeKind::Path { live: l, .. } => live.push(l.is_some()),
        _ => {}
    });
    assert_eq!(live, [false, true, true]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::SymbolInstance { .. }), "one undo step");
}

#[test]
fn expand_turns_a_symbol_instance_into_its_sliced_art() {
    let (mut s, id) = panel();
    assert_eq!(s.execute("object.expand.info", &json!({})).unwrap()["object"], json!(true));
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let d = &s.doc().unwrap().doc;
    let art = d.node(id).expect("the art keeps the instance's id");
    assert!(!matches!(art.kind, NodeKind::SymbolInstance { .. }));
    assert_sliced(&path_boxes(art));
    assert_eq!(s.doc().unwrap().selection.objects.to_vec(), vec![id]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::SymbolInstance { .. }), "one undo step");
}

/// Object › Expand also expands the instances that a symbol's art holds.
#[test]
fn expand_also_expands_the_instances_in_the_art() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = rect(&mut s, 50.0, 50.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([r])).unwrap();
    s.execute("symbol.new", &json!({"name": "Inner"})).unwrap();
    let outer = NodeId(s.execute("symbol.new", &json!({"name": "Outer"})).unwrap()["id"].as_u64().unwrap());
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    let mut kinds = vec![];
    s.doc().unwrap().doc.node(outer).unwrap().walk(&mut |c| kinds.push(matches!(c.kind, NodeKind::SymbolInstance { .. })));
    assert!(!kinds.is_empty() && !kinds.contains(&true), "{kinds:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(matches!(s.doc().unwrap().doc.node(outer).unwrap().kind, NodeKind::SymbolInstance { .. }), "one undo step");
}

/// Scaling the instance with Scale Strokes & Effects on or off leaves the art's strokes at their
/// weight in the symbol: the corners and the cut paths draw them unchanged.
#[test]
fn the_art_keeps_its_stroke_weights_with_scale_strokes_on_or_off() {
    for strokes in [true, false] {
        let (mut s, id) = panel_symbol();
        let d = &s.doc().unwrap().doc;
        let want = stroke_widths(&d.symbols.iter().find(|s| s.name == "Panel").unwrap().art);
        assert_eq!(want.len(), 3);
        s.execute("prefs.set", &json!({"key": "scaleStrokes", "value": strokes})).unwrap();
        s.execute("object.scale", &json!({"sx": 200, "sy": 150})).unwrap();
        let d = &s.doc().unwrap().doc;
        let NodeKind::SymbolInstance { symbol, xf } = &d.node(id).unwrap().kind else { panic!("still an instance") };
        assert!(d.symbol_slice_map(symbol, *xf).is_some(), "scaled through its guides");
        let r = s.execute("symbol.breakLink", &json!({})).unwrap();
        let art = s.doc().unwrap().doc.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
        assert_eq!(stroke_widths(art), want, "scaleStrokes {strokes}");
    }
}

/// An instance whose art, cut at the guides, would make more than a million objects stays an
/// instance under Object › Expand, and Break Link to Symbol reports an error without an edit.
#[test]
fn an_instance_that_slices_into_too_many_objects_stays_an_instance() {
    let (mut s, id) = panel();
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let art = doc.symbols.iter().find(|s| s.name == "Panel").unwrap().art.clone();
    let frame = art.geometric_bounds().unwrap();
    let gradient = vectorcraft_color::Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
    let mut kids = art.children().unwrap().to_vec();
    for _ in 0..36_000 {
        let look = vectorcraft_doc::Appearance::basic(gradient.clone(), vectorcraft_color::Paint::None, 0.0);
        kids.push(std::sync::Arc::new(Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(frame), look)));
    }
    let big = Node::group(doc.alloc_id(), kids);
    doc.symbols.iter_mut().find(|s| s.name == "Panel").unwrap().art = std::sync::Arc::new(big);
    let is_instance = |s: &Session| matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::SymbolInstance { .. });
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    assert!(is_instance(&s), "Object › Expand leaves it an instance");
    assert!(s.execute("symbol.breakLink", &json!({})).is_err());
    assert!(is_instance(&s), "Break Link leaves it an instance");
}

/// Break Link to Symbol on two instances, each of which would make fewer than a million objects
/// but which together would make more, reports an error without an edit.
#[test]
fn break_link_counts_the_pieces_of_every_selected_instance_together() {
    let (mut s, id) = panel();
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let art = doc.symbols.iter().find(|s| s.name == "Panel").unwrap().art.clone();
    let frame = art.geometric_bounds().unwrap();
    let gradient = vectorcraft_color::Paint::Gradient(Box::new(vectorcraft_color::GradientPaint::new(vectorcraft_color::Gradient::default())));
    let mut kids = art.children().unwrap().to_vec();
    for _ in 0..20_000 {
        let look = vectorcraft_doc::Appearance::basic(gradient.clone(), vectorcraft_color::Paint::None, 0.0);
        kids.push(std::sync::Arc::new(Node::path(doc.alloc_id(), vectorcraft_geom::shapes::rectangle(frame), look)));
    }
    let big = std::sync::Arc::new(Node::group(doc.alloc_id(), kids));
    doc.symbols.iter_mut().find(|s| s.name == "Panel").unwrap().art = big.clone();
    let mut second = doc.node(id).unwrap().clone();
    second.id = doc.alloc_id();
    let id2 = second.id;
    let (par, idx, _) = doc.position(id).unwrap();
    doc.insert(par, idx + 1, second).unwrap();
    let NodeKind::SymbolInstance { symbol, xf } = &doc.node(id).unwrap().kind else { panic!("an instance") };
    let each = doc.symbol_slice_map(symbol, *xf).unwrap().most_placed(&big, doc);
    assert!(each < 1_000_000 && 2 * each > 1_000_000, "{each}");
    s.select(|_, sel| sel.set([id, id2])).unwrap();
    assert!(s.execute("symbol.breakLink", &json!({})).is_err());
    for i in [id, id2] {
        assert!(matches!(s.doc().unwrap().doc.node(i).unwrap().kind, NodeKind::SymbolInstance { .. }), "no edit");
    }
}

/// An instance of a symbol without guides whose opacity masks hold more than a million objects
/// stays an instance under Object › Expand, and Break Link to Symbol reports an error without an
/// edit, though the art outside the masks holds about a thousand.
#[test]
fn an_instance_whose_masks_hold_too_many_objects_stays_an_instance() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let r = rect(&mut s, 50.0, 50.0, 20.0, 20.0);
    s.select(|_, sel| sel.set([r])).unwrap();
    let id = NodeId(s.execute("symbol.new", &json!({"name": "Masked"})).unwrap()["id"].as_u64().unwrap());
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    assert!(doc.symbol_nine_slice("Masked").is_none(), "no guides");
    let square = doc.symbols.iter().find(|s| s.name == "Masked").unwrap().art.clone();
    let mask_art = Node::group(doc.alloc_id(), vec![square.clone(); 1_000]);
    let mut masked = (*square).clone();
    masked.extras_mut().mask = Some(Box::new(vectorcraft_doc::OpacityMask::new(mask_art, true)));
    let masked = std::sync::Arc::new(masked);
    let big = Node::group(doc.alloc_id(), vec![masked; 1_000]);
    assert!(big.count() < 1_000_000 && big.count_with_masks() > 1_000_000);
    doc.symbols.iter_mut().find(|s| s.name == "Masked").unwrap().art = std::sync::Arc::new(big);
    let is_instance = |s: &Session| matches!(s.doc().unwrap().doc.node(id).unwrap().kind, NodeKind::SymbolInstance { .. });
    s.execute("object.expand", &json!({"fill": false, "stroke": false})).unwrap();
    assert!(is_instance(&s), "Object › Expand leaves it an instance");
    assert!(s.execute("symbol.breakLink", &json!({})).is_err());
    assert!(is_instance(&s), "Break Link leaves it an instance");
}

/// A scaled instance of a dynamic symbol keeps its own appearance changes when Break Link to
/// Symbol and Object › Expand place its art through the guides.
#[test]
fn a_dynamic_instance_keeps_its_changes_through_the_guides() {
    for cmd in ["symbol.breakLink", "object.expand"] {
        let (mut s, id) = panel_symbol();
        s.execute("symbol.options", &json!({"name": "Panel", "type": "dynamic"})).unwrap();
        s.execute("symbol.instance.setAppearance", &json!({"id": id.0, "path": [2], "fill": "#ff0000"})).unwrap();
        s.execute("object.setBounds", &json!({"x": 50, "y": 50, "width": 200, "height": 150, "reference": 0})).unwrap();
        let params = if cmd == "object.expand" { json!({"fill": false, "stroke": false}) } else { json!({}) };
        let r = s.execute(cmd, &params).unwrap();
        let art = if cmd == "object.expand" { id } else { NodeId(r["ids"][0].as_u64().unwrap()) };
        let d = &s.doc().unwrap().doc;
        let art = d.node(art).unwrap();
        assert_sliced(&path_boxes(art));
        let mut fills = vec![];
        art.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Path { .. }) {
                fills.push(c.appearance.fill_paint().color().map(|c| c.to_hex()));
            }
        });
        assert_eq!(fills.last().cloned().flatten().as_deref(), Some("#ff0000"), "{cmd}: {fills:?}");
        assert_ne!(fills[0].as_deref(), Some("#ff0000"), "{cmd}: only the changed object");
    }
}
