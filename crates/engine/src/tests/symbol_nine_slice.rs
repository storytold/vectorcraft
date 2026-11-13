//! Symbol Options › 9-slice guides: a scaled instance stays an instance and draws its corners at
//! their size; Break Link to Symbol gives the art as it draws, in one step.

use serde_json::json;
use vectorcraft_doc::{EditedNineSlice, NineSliceGuide, Node, NodeKind};
use vectorcraft_geom::{Point, Rect};
use vectorcraft_tools::{PointerEvent, PointerKind};

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

/// Break Link to Symbol on an instance whose art, cut at the guides, would make more than a
/// million objects reports an error without an edit.
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

/// Break Link to Symbol on an instance of a symbol without guides whose opacity masks hold more
/// than a million objects reports an error without an edit, though the art outside the masks
/// holds about a thousand.
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
    assert!(s.execute("symbol.breakLink", &json!({})).is_err());
    assert!(is_instance(&s), "Break Link leaves it an instance");
}

/// A scaled instance of a dynamic symbol keeps its own appearance changes when Break Link to
/// Symbol places its art through the guides.
#[test]
fn a_dynamic_instance_keeps_its_changes_through_the_guides() {
    let (mut s, id) = panel_symbol();
    s.execute("symbol.options", &json!({"name": "Panel", "type": "dynamic"})).unwrap();
    s.execute("symbol.instance.setAppearance", &json!({"id": id.0, "path": [2], "fill": "#ff0000"})).unwrap();
    s.execute("object.setBounds", &json!({"x": 50, "y": 50, "width": 200, "height": 150, "reference": 0})).unwrap();
    let r = s.execute("symbol.breakLink", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let art = d.node(NodeId(r["ids"][0].as_u64().unwrap())).unwrap();
    assert_sliced(&path_boxes(art));
    let mut fills = vec![];
    art.walk(&mut |c| {
        if matches!(c.kind, NodeKind::Path { .. }) {
            fills.push(c.appearance.fill_paint().color().map(|c| c.to_hex()));
        }
    });
    assert_eq!(fills.last().cloned().flatten().as_deref(), Some("#ff0000"), "{fills:?}");
    assert_ne!(fills[0].as_deref(), Some("#ff0000"), "only the changed object");
}

/// The boxes of the paths a symbol instance `id` draws, placed through its symbol's guides.
fn placed_boxes(s: &Session, id: NodeId) -> Vec<Rect> {
    let d = &s.doc().unwrap().doc;
    let NodeKind::SymbolInstance { symbol, xf } = &d.node(id).unwrap().kind else { panic!("still an instance") };
    let art = d.symbols.iter().find(|s| &s.name == symbol).unwrap().art.clone();
    path_boxes(&d.place_symbol_art(symbol, &art, *xf).unwrap())
}

/// Dragging the left guide on the canvas while editing the symbol changes how a scaled instance
/// draws: with the guide moved from 25% to 5%, the corner square of the 200 × 150 instance reaches
/// past the left cell into the stretched center band, so it draws about 17.14 pt wide instead of 10.
#[test]
fn a_guide_dragged_on_the_canvas_changes_how_a_scaled_instance_draws() {
    let (mut s, id) = panel();
    assert_sliced(&placed_boxes(&s, id));
    s.execute("symbol.edit", &json!({"name": "Panel"})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let mid = |e: &EditedNineSlice, g| {
        let (a, b) = e.line(g);
        Point::new((a.x + b.x) / 2.0, (a.y + b.y) / 2.0)
    };
    let e = s.doc().unwrap().doc.edited_nine_slice().unwrap();
    // The left guide sits at 25% of the art's width and the right one at 75%: 5% lies 0.4 of
    // that span to the left of the left guide.
    let (from, right) = (mid(&e, NineSliceGuide::Left), mid(&e, NineSliceGuide::Right));
    let to = from + (from - right) * 0.4;
    s.pointer(&PointerEvent::new(PointerKind::Down, from.x + 1.0, from.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to.x, to.y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to.x, to.y), v).unwrap();
    s.execute("symbol.exitEditing", &json!({})).unwrap();
    let boxes = placed_boxes(&s, id);
    assert_eq!(boxes.len(), 3, "{boxes:?}");
    // 5 pt of the square in the left cell, then 5 pt of the 70 pt center band stretched to 170 pt.
    let want = 5.0 + 5.0 * 170.0 / 70.0;
    assert!(close(boxes[1], Rect::new(50.0, 50.0, 50.0 + want, 60.0)), "{boxes:?}");
    let left = s.doc().unwrap().doc.symbol_nine_slice("Panel").unwrap().left;
    assert!((left - 0.05).abs() < 1e-9, "{left}");
}

/// The 200 × 150 [`panel`] instance with its left guide 25 % outside the art: the left column
/// holds no art, so the corner square is in the center column, which runs 0..75 in the art and
/// 0..175 in the instance, and the center square moves with it. The rows are as before.
fn assert_left_outside(boxes: &[Rect]) {
    let k = 175.0 / 75.0;
    let want = [
        Rect::new(50.0, 50.0, 250.0, 200.0),
        Rect::new(50.0, 50.0, 50.0 + 10.0 * k, 60.0),
        Rect::new(50.0 + 45.0 * k, 115.0, 50.0 + 55.0 * k, 135.0),
    ];
    assert_eq!(boxes.len(), want.len(), "{boxes:?}");
    for (b, w) in boxes.iter().zip(want) {
        assert!(close(*b, w), "{b:?} vs {w:?} in {boxes:?}");
    }
}

/// `symbol.options` takes guides outside the art, within -16..16 and in order on each axis, as
/// one undo step; a scaled instance then draws by the cells its art falls in, and Break Link to
/// Symbol gives that art. Values out of range or out of order are refused without an edit.
#[test]
fn guides_outside_the_art_are_set_by_command_and_scale_by_the_cells_that_hold_art() {
    let (mut s, id) = panel();
    s.execute("symbol.options", &json!({"name": "Panel", "nineSliceGuides": [-0.25, 0.25, 0.25, 0.25]})).unwrap();
    assert_left_outside(&placed_boxes(&s, id));
    let list = s.execute("symbol.list", &json!({})).unwrap();
    assert_eq!(list["symbols"][0]["nineSlice"], json!([-0.25, 0.25, 0.25, 0.25]));
    for bad in [json!([-16.5, 0.25, 0.25, 0.25]), json!([0.25, 0.25, 0.25, 17]), json!([1.2, 0.25, -0.2, 0.25]), json!([0.25, 0.6, 0.25, 0.6])] {
        let e = s.execute("symbol.options", &json!({"name": "Panel", "nineSliceGuides": bad})).unwrap_err().to_string();
        assert!(e.contains("each -16..16"), "{e}");
    }
    assert_eq!(s.doc().unwrap().doc.symbol_nine_slice("Panel").unwrap().left, -0.25, "no edit");
    s.execute("symbol.options", &json!({"name": "Panel", "nineSliceGuides": [1.25, 0.25, -0.5, 0.25]})).unwrap();
    // Both vertical guides past the art's right edge: the art keeps its width from the left.
    let boxes = placed_boxes(&s, id);
    assert!(close(boxes[0], Rect::new(50.0, 50.0, 150.0, 200.0)) && close(boxes[1], Rect::new(50.0, 50.0, 60.0, 60.0)), "{boxes:?}");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_left_outside(&placed_boxes(&s, id));
    let r = s.execute("symbol.breakLink", &json!({})).unwrap();
    let art = NodeId(r["ids"][0].as_u64().unwrap());
    assert_left_outside(&path_boxes(s.doc().unwrap().doc.node(art).unwrap()));
}

/// Dragging the left guide past the art's left edge on the canvas puts it outside the art, as one
/// undo step, and the scaled instance draws by the cells that hold art.
#[test]
fn a_guide_dragged_past_the_arts_edge_lies_outside_it() {
    let (mut s, id) = panel();
    s.execute("symbol.edit", &json!({"name": "Panel"})).unwrap();
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    let e = s.doc().unwrap().doc.edited_nine_slice().unwrap();
    let ((a, b), (l, r)) = (e.line(NineSliceGuide::Left), (e.line(NineSliceGuide::Left).0.x, e.line(NineSliceGuide::Right).0.x));
    let y = (a.y + b.y) / 2.0;
    // The guides lie a quarter in from each edge, so the art's left edge lies half the guides'
    // distance to the left of the left guide, and 25 % outside it lies as far again.
    let to = l - (r - l);
    s.pointer(&PointerEvent::new(PointerKind::Down, l + 1.0, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, to, y), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, to, y), v).unwrap();
    let g = s.doc().unwrap().doc.symbol_nine_slice("Panel").unwrap();
    assert!((g.left + 0.25).abs() < 1e-9, "{g:?}");
    let e = s.doc().unwrap().doc.edited_nine_slice().unwrap();
    assert!((e.line(NineSliceGuide::Left).0.x - to).abs() < 1e-9, "the guide is drawn where it was dropped");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc.symbol_nine_slice("Panel").unwrap().left, 0.25, "one undo step");
    s.execute("edit.redo", &json!({})).unwrap();
    s.execute("symbol.exitEditing", &json!({})).unwrap();
    assert_left_outside(&placed_boxes(&s, id));
}

/// A native file keeps guides outside the art exactly, and an instance opened from it draws as
/// before saving; a file whose saved guides are out of range or out of order opens with 9-slice
/// off for that symbol.
#[test]
fn guides_outside_the_art_survive_saving_and_reopening_a_native_file() {
    let (mut s, id) = panel();
    let guides = [-0.123456789, 1.5, 0.3, -2.75];
    s.execute("symbol.options", &json!({"name": "Panel", "nineSliceGuides": guides})).unwrap();
    s.execute("symbol.options", &json!({"name": "Panel", "nineSliceGuides": [-0.25, 0.25, 0.25, 0.25]})).unwrap();
    s.execute("symbol.options", &json!({"name": "Panel", "nineSliceGuides": guides})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    let g = back.symbol_nine_slice("Panel").unwrap();
    assert_eq!([g.left, g.top, g.right, g.bottom], guides);
    let placed = |d: &vectorcraft_doc::Document| {
        let NodeKind::SymbolInstance { symbol, xf } = &d.node(id).unwrap().kind else { panic!("an instance") };
        let art = d.symbols.iter().find(|s| &s.name == symbol).unwrap().art.clone();
        path_boxes(&d.place_symbol_art(symbol, &art, *xf).unwrap())
    };
    assert_eq!(placed(&back), placed(&doc));
    for bad in [json!([-20.0, 0.25, 0.25, 0.25]), json!([0.8, 0.25, 0.5, 0.25]), json!([0.25, 0.25, 0.25]), json!("x")] {
        let mut d = (*doc).clone();
        d.unknown.insert(vectorcraft_doc::SYMBOL_NINE_SLICE.into(), json!({ "Panel": bad }));
        let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
        assert!(back.symbol_nine_slice("Panel").is_none(), "{bad}");
        let NodeKind::SymbolInstance { symbol, xf } = &back.node(id).unwrap().kind else { panic!("an instance") };
        assert!(back.symbol_slice_map(symbol, *xf).is_none(), "{bad}: scaled as a whole");
    }
}
