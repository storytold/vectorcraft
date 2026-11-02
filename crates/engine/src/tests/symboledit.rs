//! Symbol editing mode (`symbol.edit`, `symbol.exitEditing`).

use serde_json::{Value, json};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().expect("an id"))
}

/// A 400 × 300 document with the 80 × 40 symbol "Box" made from a rectangle at (100, 50), its
/// instance there and a second instance centered on (300, 200).
fn two_instances() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 50, "width": 80, "height": 40}));
    let first = id(&run(&mut s, "symbol.new", json!({"name": "Box"})));
    let second = id(&run(&mut s, "symbol.place", json!({"name": "Box", "x": 300, "y": 200})));
    (s, first, second)
}

/// The bounds of `id` as the document measures them: an instance's by its symbol's art.
fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.bounds_of(&[id], false).expect("bounds")
}

fn close(a: Rect, b: Rect) -> bool {
    [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().all(|(x, y)| (x - y).abs() < 1e-6)
}

fn is_instance(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().doc.node(id).is_some_and(|n| matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Box"))
}

/// The objects on the editing layer.
fn layer_art(s: &Session) -> Vec<NodeId> {
    let st = s.doc().unwrap();
    let layer = st.doc.symbol_edit.as_ref().expect("editing a symbol").layer;
    st.doc.node(layer).and_then(|l| l.children()).map(|c| c.iter().map(|n| n.id).collect()).unwrap_or_default()
}

#[test]
fn edit_symbol_edits_the_art_where_the_instance_shows_it_and_every_instance_follows() {
    let (mut s, first, second) = two_instances();
    run(&mut s, "select.set", json!({"ids": [first.0]}));
    let r = run(&mut s, "symbol.edit", json!({}));
    assert_eq!((r["name"].as_str(), r["id"].as_u64()), (Some("Box"), Some(first.0)));
    let layer = NodeId(r["layer"].as_u64().unwrap());
    let st = s.doc().unwrap();
    assert_eq!(st.isolation, Some(layer));
    assert_eq!(st.active_layer, Some(layer));
    // The instance stays linked: editing the symbol doesn't expand it.
    assert!(is_instance(&s, first) && is_instance(&s, second));
    let art = layer_art(&s);
    assert_eq!(art.len(), 1);
    assert!(close(bounds(&s, art[0]), Rect::new(100.0, 50.0, 180.0, 90.0)), "{:?}", bounds(&s, art[0]));
    assert_eq!(s.doc().unwrap().selection.objects, art);
    // An edit of the art reaches both instances at once, each at its own place.
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    assert!(close(bounds(&s, first), Rect::new(110.0, 50.0, 190.0, 90.0)), "{:?}", bounds(&s, first));
    assert!(close(bounds(&s, second), Rect::new(270.0, 180.0, 350.0, 220.0)), "{:?}", bounds(&s, second));
    // New art goes into the symbol.
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 10, "height": 10}));
    assert_eq!(layer_art(&s).len(), 2);
    assert!(close(bounds(&s, second), Rect::new(260.0, 180.0, 350.0, 240.0)), "{:?}", bounds(&s, second));
    let r = run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(r["name"], "Box");
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
    assert!(st.doc.node(layer).is_none(), "the editing layer is gone");
    assert_eq!(st.selection.objects, vec![first]);
    assert!(is_instance(&s, first));
    assert!(close(bounds(&s, second), Rect::new(260.0, 180.0, 350.0, 240.0)), "{:?}", bounds(&s, second));
}

#[test]
fn an_instance_transform_places_the_art_and_edits_go_back_through_it() {
    let (mut s, first, second) = two_instances();
    // The second instance twice as large: a 20 pt move there is a 10 pt move of the symbol's art.
    run(&mut s, "select.set", json!({"ids": [second.0]}));
    run(&mut s, "object.scale", json!({"sx": 200, "sy": 200, "origin": [300, 200]}));
    let big = bounds(&s, second);
    assert!(close(big, Rect::new(220.0, 160.0, 380.0, 240.0)), "{big:?}");
    run(&mut s, "symbol.edit", json!({"id": second.0}));
    let art = layer_art(&s);
    assert!(close(bounds(&s, art[0]), big), "{:?}", bounds(&s, art[0]));
    run(&mut s, "object.move", json!({"dx": 20, "dy": 0}));
    assert!(close(bounds(&s, second), Rect::new(240.0, 160.0, 400.0, 240.0)), "{:?}", bounds(&s, second));
    assert!(close(bounds(&s, first), Rect::new(110.0, 50.0, 190.0, 90.0)), "{:?}", bounds(&s, first));
}

#[test]
fn the_symbols_panel_edits_a_symbol_without_an_instance_at_the_artboard_center() {
    let (mut s, first, second) = two_instances();
    run(&mut s, "edit.clear", json!({"ids": [first.0, second.0]}));
    run(&mut s, "select.none", json!({}));
    let r = run(&mut s, "symbol.edit", json!({"name": "Box"}));
    assert!(r.get("id").is_none());
    let art = layer_art(&s);
    assert!(close(bounds(&s, art[0]), Rect::new(160.0, 130.0, 240.0, 170.0)), "{:?}", bounds(&s, art[0]));
    run(&mut s, "object.move", json!({"dx": 0, "dy": 5}));
    run(&mut s, "symbol.exitEditing", json!({}));
    // No instance was added, and a new one shows the edited art.
    let mut instances = 0;
    s.doc().unwrap().doc.walk(|n| instances += usize::from(matches!(n.kind, NodeKind::SymbolInstance { .. })));
    assert_eq!(instances, 0);
    let placed = id(&run(&mut s, "symbol.place", json!({"name": "Box", "x": 200, "y": 150})));
    assert!(close(bounds(&s, placed), Rect::new(160.0, 135.0, 240.0, 175.0)), "{:?}", bounds(&s, placed));
    assert!(s.execute("symbol.edit", &json!({"name": "Nope"})).is_err());
}

#[test]
fn the_symbols_panel_edits_a_symbol_at_the_center_of_the_active_artboard_or_the_view() {
    let (mut s, _, _) = two_instances();
    run(&mut s, "artboard.new", json!({"x": 500, "y": 0, "width": 200, "height": 100}));
    run(&mut s, "select.none", json!({}));
    run(&mut s, "symbol.edit", json!({"name": "Box", "artboard": 1}));
    let art = layer_art(&s);
    assert!(close(bounds(&s, art[0]), Rect::new(560.0, 30.0, 640.0, 70.0)), "{:?}", bounds(&s, art[0]));
    run(&mut s, "symbol.exitEditing", json!({}));
    // An artboard the document doesn't have: the first one.
    run(&mut s, "symbol.edit", json!({"name": "Box", "artboard": 7, "viewCenter": [1000, 1000]}));
    let art = layer_art(&s);
    assert!(close(bounds(&s, art[0]), Rect::new(160.0, 130.0, 240.0, 170.0)), "{:?}", bounds(&s, art[0]));
    run(&mut s, "symbol.exitEditing", json!({}));
    // Without artboards, the view's center.
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).artboards.clear();
    run(&mut s, "symbol.edit", json!({"name": "Box", "viewCenter": [1000, 1000]}));
    let art = layer_art(&s);
    assert!(close(bounds(&s, art[0]), Rect::new(960.0, 980.0, 1040.0, 1020.0)), "{:?}", bounds(&s, art[0]));
}

#[test]
fn leaving_without_an_edit_leaves_the_document_and_its_history_as_they_were() {
    let (mut s, first, _) = two_instances();
    let steps = s.doc().unwrap().history.undo.len();
    let art = s.doc().unwrap().doc.symbols[0].art.clone();
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    run(&mut s, "object.exitIsolation", json!({}));
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), steps);
    assert!(std::sync::Arc::ptr_eq(&st.doc.symbols[0].art, &art));
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
    // Outside the mode, leaving it does nothing.
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    // A step undone before editing can still be redone after leaving without an edit.
    run(&mut s, "shape.rectangle", json!({"x": 300, "y": 20, "width": 10, "height": 10}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    assert!(s.doc().unwrap().history.redo.is_empty());
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(s.doc().unwrap().history.redo.len(), 1);
    run(&mut s, "edit.redo", json!({}));
    // Leaving after an edit that was undone adds a step, keeping the redo step that holds the
    // editing layer apart from the document outside the mode.
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "edit.undo", json!({}));
    let steps = s.doc().unwrap().history.undo.len();
    run(&mut s, "symbol.exitEditing", json!({}));
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), steps + 1);
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
}

#[test]
fn leaving_without_an_edit_keeps_the_current_symbol_picked_while_editing() {
    let (mut s, first, _) = two_instances();
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
    run(&mut s, "symbol.new", json!({"name": "Small"}));
    run(&mut s, "symbol.setCurrent", json!({"name": "Box"}));
    let steps = s.doc().unwrap().history.undo.len();
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    // A click in the Symbols panel while editing: not an undo step.
    run(&mut s, "symbol.setCurrent", json!({"name": "Small"}));
    run(&mut s, "symbol.exitEditing", json!({}));
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), steps);
    assert_eq!(st.doc.unknown.get("currentSymbol").and_then(Value::as_str), Some("Small"));
}

#[test]
fn leaving_right_after_a_batch_that_only_entered_takes_its_step_back() {
    let (mut s, first, _) = two_instances();
    let steps = s.doc().unwrap().history.undo.len();
    run(&mut s, "command.batch", json!({"commands": [{"command": "symbol.edit", "params": {"id": first.0}}]}));
    assert!(s.doc().unwrap().doc.symbol_edit.is_some());
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    run(&mut s, "symbol.exitEditing", json!({}));
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), steps);
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
}

#[test]
fn leaving_right_after_a_batch_that_edited_and_entered_keeps_its_edits() {
    let (mut s, first, second) = two_instances();
    let steps = s.doc().unwrap().history.undo.len();
    // A batch named like the entering step that moves an instance before entering: leaving keeps
    // the move and adds a step of its own.
    let moved = Rect::new(310.0, 180.0, 390.0, 220.0);
    let commands = json!([
        {"command": "select.set", "params": {"ids": [second.0]}},
        {"command": "object.move", "params": {"dx": 50, "dy": 0}},
        {"command": "symbol.edit", "params": {"id": first.0}},
    ]);
    run(&mut s, "command.batch", json!({"label": "Edit Symbol", "commands": commands}));
    assert!(close(bounds(&s, second), moved), "{:?}", bounds(&s, second));
    run(&mut s, "symbol.exitEditing", json!({}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
    assert_eq!(st.history.undo.len(), steps + 2);
    assert!(close(bounds(&s, second), moved), "{:?}", bounds(&s, second));
    // Undo takes back the exit, then the batch's move and entry together.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.symbol_edit.is_some());
    run(&mut s, "edit.undo", json!({}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
    assert!(close(bounds(&s, second), Rect::new(260.0, 180.0, 340.0, 220.0)), "{:?}", bounds(&s, second));
}

#[test]
fn redefining_the_symbol_being_edited_leaves_symbol_editing_mode_in_the_same_step() {
    let (mut s, first, second) = two_instances();
    let other = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 200, "width": 30, "height": 30})));
    let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": first.0}))["layer"].as_u64().unwrap());
    // The art on the editing layer can't redefine the symbol.
    let e = s.execute("symbol.update", &json!({"name": "Box"})).unwrap_err().to_string();
    assert!(e.contains("can't hold an instance of itself"), "{e}");
    let steps = s.doc().unwrap().history.undo.len();
    run(&mut s, "symbol.update", json!({"name": "Box", "ids": [other.0]}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none() && st.doc.node(layer).is_none());
    assert_eq!(st.history.undo.len(), steps + 1);
    // Every instance shows the new 30 × 30 art, and a later edit can't bring the old art back.
    assert!(close(bounds(&s, second), Rect::new(285.0, 185.0, 315.0, 215.0)), "{:?}", bounds(&s, second));
    run(&mut s, "shape.rectangle", json!({"x": 300, "y": 10, "width": 10, "height": 10}));
    assert!(close(bounds(&s, second), Rect::new(285.0, 185.0, 315.0, 215.0)), "{:?}", bounds(&s, second));
    // Undo goes back into the edit as it was.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    let st = s.doc().unwrap();
    assert_eq!(st.isolation, Some(layer));
    assert!(st.doc.symbol_edit.is_some());
}

#[test]
fn leaving_goes_back_to_the_isolation_before_and_undo_keeps_a_group_isolated_inside() {
    let (mut s, first, _) = two_instances();
    // The instance in an isolated group.
    let other = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 200, "width": 20, "height": 20})));
    run(&mut s, "select.set", json!({"ids": [first.0, other.0]}));
    let group = id(&run(&mut s, "object.group", json!({})));
    run(&mut s, "object.isolate", json!({"id": group.0}));
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(group));
    // A group isolated inside the symbol's art stays isolated through an undo.
    let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": first.0}))["layer"].as_u64().unwrap());
    run(&mut s, "shape.rectangle", json!({"x": 100, "y": 100, "width": 10, "height": 10}));
    let art: Vec<u64> = layer_art(&s).iter().map(|i| i.0).collect();
    run(&mut s, "select.set", json!({ "ids": art }));
    let inner = id(&run(&mut s, "object.group", json!({})));
    run(&mut s, "object.isolate", json!({"id": inner.0}));
    run(&mut s, "select.all", json!({}));
    run(&mut s, "object.move", json!({"dx": 5, "dy": 0}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(inner));
    run(&mut s, "object.exitIsolation", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(layer));
    run(&mut s, "object.exitIsolation", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(group));
    // Deleting the editing layer goes back there too.
    let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": first.0}))["layer"].as_u64().unwrap());
    run(&mut s, "layer.delete", json!({"ids": [layer.0]}));
    assert_eq!(s.doc().unwrap().isolation, Some(group));
}

#[test]
fn leaving_after_a_save_keeps_the_document_saved() {
    let (mut s, first, _) = two_instances();
    for edit in [false, true] {
        run(&mut s, "symbol.edit", json!({"id": first.0}));
        if edit {
            run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
        }
        // The save leaves the editing layer out of the file.
        s.doc_mut().unwrap().mark_saved();
        run(&mut s, "symbol.exitEditing", json!({}));
        let st = s.doc().unwrap();
        assert!(!st.is_dirty(), "edit: {edit}");
        assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none(), "edit: {edit}");
    }
}

#[test]
fn deleting_the_editing_layer_leaves_symbol_editing_mode() {
    let (mut s, first, _) = two_instances();
    let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": first.0}))["layer"].as_u64().unwrap());
    run(&mut s, "layer.delete", json!({"ids": [layer.0]}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.doc.node(layer).is_none());
    assert_eq!(st.isolation, None);
    assert!(st.active_layer.is_some_and(|l| st.doc.node(l).is_some()));
    assert!(is_instance(&s, first));
}

#[test]
fn undo_and_redo_restore_the_art_every_instance_shows_and_the_mode() {
    let (mut s, first, second) = two_instances();
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    let layer = s.doc().unwrap().doc.symbol_edit.as_ref().unwrap().layer;
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "symbol.exitEditing", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(layer), "back in symbol editing mode");
    run(&mut s, "edit.undo", json!({}));
    assert!(close(bounds(&s, second), Rect::new(260.0, 180.0, 340.0, 220.0)), "{:?}", bounds(&s, second));
    assert_eq!(s.doc().unwrap().isolation, Some(layer));
    run(&mut s, "edit.undo", json!({}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none());
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "edit.redo", json!({}));
    assert!(close(bounds(&s, second), Rect::new(270.0, 180.0, 350.0, 220.0)), "{:?}", bounds(&s, second));
    assert_eq!(s.doc().unwrap().isolation, Some(layer));
}

#[test]
fn instances_of_one_symbol_only_and_no_instance_of_itself() {
    let (mut s, first, _) = two_instances();
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
    let other = id(&run(&mut s, "symbol.new", json!({"name": "Small"})));
    run(&mut s, "select.set", json!({"ids": [first.0, other.0]}));
    let e = s.execute("symbol.edit", &json!({})).unwrap_err().to_string();
    assert!(e.contains("select instances of one symbol"), "{e}");
    let e = s.execute("symbol.edit", &json!({"ids": [first.0, other.0]})).unwrap_err().to_string();
    assert!(e.contains("select instances of one symbol"), "{e}");
    assert!(s.execute("symbol.edit", &json!({"ids": [first.0, 999_999]})).is_err());
    assert!(s.doc().unwrap().doc.symbol_edit.is_none());
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    let e = s.execute("symbol.place", &json!({"name": "Box"})).unwrap_err().to_string();
    assert!(e.contains("can't hold an instance of itself"), "{e}");
    assert_eq!(layer_art(&s).len(), 1);
    // An instance of another symbol may go in.
    run(&mut s, "symbol.place", json!({"name": "Small"}));
    assert_eq!(layer_art(&s).len(), 2);
}

#[test]
fn other_editing_modes_and_symbol_editing_do_not_overlap() {
    let (mut s, first, second) = two_instances();
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    assert!(s.execute("object.pattern.make", &json!({})).is_err());
    assert!(s.execute("transparency.editOpacityMask", &json!({})).is_err());
    // Editing another symbol from the panel ends this edit first.
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20}));
    run(&mut s, "symbol.exitEditing", json!({}));
    run(&mut s, "select.set", json!({"ids": [second.0]}));
    run(&mut s, "symbol.edit", json!({}));
    run(&mut s, "symbol.edit", json!({"name": "Box", "x": 50, "y": 50}));
    assert_eq!(s.doc().unwrap().doc.symbol_edit.as_ref().unwrap().instance, None);
    assert_eq!(s.doc().unwrap().doc.layers.iter().filter(|l| l.name.as_deref() == Some("Box")).count(), 1);
}

/// Editing another symbol at a place off the canvas fails and keeps the current edit open.
#[test]
fn a_switch_to_another_symbol_that_fails_keeps_the_current_edit() {
    let (mut s, first, _) = two_instances();
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 200, "width": 20, "height": 20}));
    run(&mut s, "symbol.new", json!({"name": "Dot"}));
    let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": first.0}))["layer"].as_u64().unwrap());
    let e = s.execute("symbol.edit", &json!({"name": "Dot", "x": 1e300})).unwrap_err().to_string();
    assert!(e.contains("exceed the canvas"), "{e}");
    let st = s.doc().unwrap();
    assert_eq!(st.doc.symbol_edit.as_ref().map(|e| (e.symbol.as_str(), e.layer)), Some(("Box", layer)));
    assert_eq!((st.isolation, st.active_layer), (Some(layer), Some(layer)));
    assert!(st.doc.node(layer).is_some());
}

/// While a symbol is edited, art outside the editing layer is not isolated, so leaving isolation
/// mode leaves symbol editing mode.
#[test]
fn art_outside_the_editing_layer_is_not_isolated_while_a_symbol_is_edited() {
    let (mut s, first, _) = two_instances();
    let other = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 200, "width": 20, "height": 20})));
    run(&mut s, "select.set", json!({"ids": [first.0]}));
    let layer = NodeId(run(&mut s, "symbol.edit", json!({"id": first.0}))["layer"].as_u64().unwrap());
    let e = s.execute("object.isolate", &json!({"id": other.0})).unwrap_err().to_string();
    assert!(e.contains("outside the isolated group"), "{e}");
    assert_eq!(s.doc().unwrap().isolation, Some(layer));
    // Isolation that names art outside the layer (a stale state) still leaves the mode.
    s.doc_mut().unwrap().isolation = Some(other);
    run(&mut s, "object.exitIsolation", json!({}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.doc.node(layer).is_none());
    assert_eq!(st.isolation, None);
    assert_eq!(st.selection.objects, vec![first]);
}

#[test]
fn renaming_deleting_and_saving_while_a_symbol_is_edited() {
    let (mut s, first, _) = two_instances();
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    let layer = s.doc().unwrap().doc.symbol_edit.as_ref().unwrap().layer;
    // Saving leaves the editing layer out; the symbol already holds the edit.
    let saved = s.doc().unwrap().doc.without_edit_modes().into_owned();
    assert!(saved.symbol_edit.is_none() && saved.node(layer).is_none());
    assert!(saved.symbols[0].art.geometric_bounds().is_some());
    run(&mut s, "symbol.options", json!({"name": "Box", "newName": "Crate"}));
    let st = s.doc().unwrap();
    assert_eq!(st.doc.symbol_edit.as_ref().unwrap().symbol, "Crate");
    assert_eq!(st.doc.node(layer).and_then(|l| l.name.clone()).as_deref(), Some("Crate"));
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    // Deleting the symbol being edited leaves the mode in the same step.
    run(&mut s, "symbol.delete", json!({"name": "Crate"}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none() && st.doc.node(layer).is_none());
    assert!(st.doc.symbols.is_empty());
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(layer));
}

/// Deleting several symbols, the one being edited among them, leaves the mode in the same step.
#[test]
fn deleting_several_symbols_with_the_edited_one_leaves_the_mode() {
    let (mut s, first, _) = two_instances();
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 200, "width": 20, "height": 20}));
    run(&mut s, "symbol.new", json!({"name": "Dot"}));
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    let layer = s.doc().unwrap().doc.symbol_edit.as_ref().unwrap().layer;
    run(&mut s, "symbol.delete", json!({"names": ["Dot", "Box"]}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.isolation.is_none() && st.doc.node(layer).is_none());
    assert!(st.doc.symbols.is_empty());
}

/// Editing a dynamic symbol keeps each instance's appearance changes to the objects its art still
/// has, and drops the changes to an object the edit deleted.
#[test]
fn editing_a_dynamic_symbol_keeps_the_instances_changes_to_the_objects_still_there() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})));
    let inst = id(&run(&mut s, "symbol.new", json!({"name": "Pair", "ids": [a.0, b.0], "type": "dynamic"})));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [0], "fill": "#00ff00"}));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [1], "fill": "#ff0000"}));
    // Each change by the path it names and the color it paints.
    let changes = |s: &Session| -> Vec<(Vec<u32>, String)> {
        let n = s.doc().unwrap().doc.node(inst).unwrap().clone();
        n.extras().symbol_overrides.iter().map(|o| (o.path.clone(), o.fill.as_ref().and_then(|p| p.color()).unwrap().to_hex())).collect()
    };
    let other = id(&run(&mut s, "symbol.place", json!({"name": "Pair", "x": 200, "y": 200})));
    run(&mut s, "symbol.edit", json!({"id": other.0}));
    let art = layer_art(&s);
    assert_eq!(art.len(), 2);
    // Deleting the first object: the change to the second follows it to the first place.
    run(&mut s, "select.set", json!({"ids": [art[0].0]}));
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(changes(&s), vec![(vec![0], "#ff0000".to_string())], "the change to the deleted object goes");
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(changes(&s), vec![(vec![0], "#ff0000".to_string())]);
    assert!(s.doc().unwrap().doc.node(inst).is_some_and(|n| matches!(n.kind, NodeKind::SymbolInstance { .. })));
}

/// An instance of the edited dynamic symbol inside another symbol's art keeps its appearance
/// changes on the same objects when the edit reorders them, and drops the change to an object the
/// edit deleted.
#[test]
fn an_instance_inside_another_symbols_art_follows_an_edit_of_its_dynamic_symbol() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})));
    let inst = id(&run(&mut s, "symbol.new", json!({"name": "Pair", "ids": [a.0, b.0], "type": "dynamic"})));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [0], "fill": "#00ff00"}));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [1], "fill": "#ff0000"}));
    run(&mut s, "symbol.new", json!({"name": "Outer", "ids": [inst.0]}));
    // The changes of the instance of "Pair" in the art of "Outer", by path and color.
    let nested = |s: &Session| -> Vec<(Vec<u32>, String)> {
        let d = &s.doc().unwrap().doc;
        let outer = d.symbols.iter().find(|x| x.name == "Outer").expect("Outer");
        let mut out = vec![];
        outer.art.walk(&mut |n| {
            if matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Pair") {
                out.extend(n.extras().symbol_overrides.iter().map(|o| (o.path.clone(), o.fill.as_ref().and_then(|p| p.color()).unwrap().to_hex())));
            }
        });
        out
    };
    let start = vec![(vec![0], "#00ff00".to_string()), (vec![1], "#ff0000".to_string())];
    assert_eq!(nested(&s), start);
    let other = id(&run(&mut s, "symbol.place", json!({"name": "Pair", "x": 200, "y": 200})));
    run(&mut s, "symbol.edit", json!({"id": other.0}));
    let art = layer_art(&s);
    assert_eq!(art.len(), 2);
    // Sending the second object to the back: each change follows its object.
    run(&mut s, "select.set", json!({"ids": [art[1].0]}));
    run(&mut s, "object.arrange.sendToBack", json!({}));
    assert_eq!(layer_art(&s), vec![art[1], art[0]]);
    assert_eq!(nested(&s), vec![(vec![1], "#00ff00".to_string()), (vec![0], "#ff0000".to_string())]);
    // Deleting the first object: the change to it goes.
    run(&mut s, "select.set", json!({"ids": [art[0].0]}));
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(nested(&s), vec![(vec![0], "#ff0000".to_string())]);
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(nested(&s), vec![(vec![0], "#ff0000".to_string())]);
}

/// Instances of the edited dynamic symbol in a pattern tile, a graph design and an art brush keep
/// their appearance changes on the same objects when the edit reorders them, and drop the change
/// to an object the edit deleted.
#[test]
fn instances_in_pattern_tiles_graph_designs_and_brushes_follow_an_edit_of_their_dynamic_symbol() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})));
    let inst = id(&run(&mut s, "symbol.new", json!({"name": "Pair", "ids": [a.0, b.0], "type": "dynamic"})));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [0], "fill": "#00ff00"}));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [1], "fill": "#ff0000"}));
    run(&mut s, "object.pattern.make", json!({"ids": [inst.0], "name": "Tile", "edit": false}));
    run(&mut s, "select.set", json!({"ids": [inst.0]}));
    run(&mut s, "graph.design", json!({"new": "Pairs"}));
    run(&mut s, "brush.new", json!({"type": "art", "name": "Pair Band", "ids": [inst.0]}));
    // A stored entry that does not parse, and a field the brush definition does not read, are kept.
    let junk = json!({"name": "Unreadable", "kind": 7});
    let doc = std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc);
    let Some(Value::Array(entries)) = doc.unknown.get_mut(vectorcraft_brush::DOC_KEY) else { panic!("no stored brushes") };
    entries.iter_mut().filter(|e| e["name"] == "Pair Band").for_each(|e| e["note"] = json!("kept"));
    entries.push(junk.clone());
    // The changes of each instance of "Pair" in `n`, by path and color.
    fn changes(n: &Node, out: &mut Vec<(Vec<u32>, String)>) {
        n.walk(&mut |n| {
            if matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Pair") {
                out.extend(n.extras().symbol_overrides.iter().map(|o| (o.path.clone(), o.fill.as_ref().and_then(|p| p.color()).unwrap().to_hex())));
            }
        });
    }
    // The changes in the tile, the graph design and the brush.
    let held = |s: &Session| -> [Vec<(Vec<u32>, String)>; 3] {
        let d = &s.doc().unwrap().doc;
        let mut out: [Vec<(Vec<u32>, String)>; 3] = Default::default();
        d.patterns.iter().filter(|p| p.name == "Tile").flat_map(|p| &p.art).for_each(|n| changes(n, &mut out[0]));
        d.graph_designs.iter().for_each(|g| changes(&g.art, &mut out[1]));
        let brush = vectorcraft_brush::find(d, "Pair Band").expect("the brush");
        brush.art().into_iter().for_each(|n| changes(n, &mut out[2]));
        out
    };
    let start = vec![(vec![0], "#00ff00".to_string()), (vec![1], "#ff0000".to_string())];
    assert_eq!(held(&s), [start.clone(), start.clone(), start]);
    let other = id(&run(&mut s, "symbol.place", json!({"name": "Pair", "x": 200, "y": 200})));
    run(&mut s, "symbol.edit", json!({"id": other.0}));
    let art = layer_art(&s);
    assert_eq!(art.len(), 2);
    // Sending the second object to the back: each change follows its object.
    run(&mut s, "select.set", json!({"ids": [art[1].0]}));
    run(&mut s, "object.arrange.sendToBack", json!({}));
    let swapped = vec![(vec![1], "#00ff00".to_string()), (vec![0], "#ff0000".to_string())];
    assert_eq!(held(&s), [swapped.clone(), swapped.clone(), swapped]);
    // Deleting the first object: the change to it goes.
    run(&mut s, "select.set", json!({"ids": [art[0].0]}));
    run(&mut s, "edit.clear", json!({}));
    let left = vec![(vec![0], "#ff0000".to_string())];
    assert_eq!(held(&s), [left.clone(), left.clone(), left.clone()]);
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(held(&s), [left.clone(), left.clone(), left]);
    let stored = &s.doc().unwrap().doc.unknown[vectorcraft_brush::DOC_KEY];
    assert!(stored.as_array().is_some_and(|a| a.contains(&junk)), "{stored}");
    assert!(stored.as_array().is_some_and(|a| a.iter().any(|e| e["name"] == "Pair Band" && e["note"] == "kept")), "{stored}");
}

/// An instance of a dynamic symbol that is the opacity mask of an object on a layer keeps its
/// appearance changes on the same objects when the edit reorders them, and drops the change to an
/// object the edit deleted.
#[test]
fn an_instance_in_an_opacity_mask_follows_an_edit_of_its_dynamic_symbol() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 40, "height": 40})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 60, "y": 10, "width": 40, "height": 40})));
    let inst = id(&run(&mut s, "symbol.new", json!({"name": "Pair", "ids": [a.0, b.0], "type": "dynamic"})));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [0], "fill": "#00ff00"}));
    run(&mut s, "symbol.instance.setAppearance", json!({"id": inst.0, "path": [1], "fill": "#ff0000"}));
    let masked = id(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 0, "width": 120, "height": 60})));
    run(&mut s, "select.set", json!({"ids": [inst.0]}));
    run(&mut s, "object.arrange.bringToFront", json!({}));
    run(&mut s, "transparency.makeOpacityMask", json!({"ids": [masked.0, inst.0]}));
    // The changes of the instance of "Pair" in the masked object's mask, by path and color.
    let held = |s: &Session| -> Vec<(Vec<u32>, String)> {
        let d = &s.doc().unwrap().doc;
        let mask = d.node(masked).and_then(|n| n.extras().mask.clone()).expect("the opacity mask");
        let mut out = vec![];
        vectorcraft_doc::swatches::walk_with_masks(&mask.art, &mut |n| {
            if matches!(&n.kind, NodeKind::SymbolInstance { symbol, .. } if symbol == "Pair") {
                out.extend(n.extras().symbol_overrides.iter().map(|o| (o.path.clone(), o.fill.as_ref().and_then(|p| p.color()).unwrap().to_hex())));
            }
        });
        out
    };
    assert_eq!(held(&s), vec![(vec![0], "#00ff00".to_string()), (vec![1], "#ff0000".to_string())]);
    let other = id(&run(&mut s, "symbol.place", json!({"name": "Pair", "x": 200, "y": 200})));
    run(&mut s, "symbol.edit", json!({"id": other.0}));
    let art = layer_art(&s);
    assert_eq!(art.len(), 2);
    // Sending the second object to the back: each change follows its object.
    run(&mut s, "select.set", json!({"ids": [art[1].0]}));
    run(&mut s, "object.arrange.sendToBack", json!({}));
    assert_eq!(held(&s), vec![(vec![1], "#00ff00".to_string()), (vec![0], "#ff0000".to_string())]);
    // Deleting the first object: the change to it goes.
    run(&mut s, "select.set", json!({"ids": [art[0].0]}));
    run(&mut s, "edit.clear", json!({}));
    assert_eq!(held(&s), vec![(vec![0], "#ff0000".to_string())]);
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(held(&s), vec![(vec![0], "#ff0000".to_string())]);
}

#[test]
fn undo_back_into_an_earlier_edit_leaves_it_to_its_own_isolation() {
    let (mut s, first, second) = two_instances();
    let other = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 200, "width": 20, "height": 20})));
    run(&mut s, "select.set", json!({"ids": [second.0, other.0]}));
    let group = id(&run(&mut s, "object.group", json!({})));
    // The first edit is entered with nothing isolated.
    run(&mut s, "symbol.edit", json!({"id": first.0}));
    run(&mut s, "object.move", json!({"dx": 10, "dy": 0}));
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(s.doc().unwrap().isolation, None);
    // The second edit is entered with the group isolated.
    run(&mut s, "object.isolate", json!({"id": group.0}));
    run(&mut s, "symbol.edit", json!({"id": second.0}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().isolation, Some(group));
    // Back in the first edit, leaving it goes back to what it was entered from.
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.symbol_edit.is_some());
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(s.doc().unwrap().isolation, None);
}

#[test]
fn undoing_entry_with_a_group_isolated_inside_the_art_goes_back_to_before() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 400, "height": 300}));
    let a = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 20, "height": 20})));
    let b = id(&run(&mut s, "shape.rectangle", json!({"x": 40, "y": 10, "width": 20, "height": 20})));
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let inner = id(&run(&mut s, "object.group", json!({})));
    let c = id(&run(&mut s, "shape.rectangle", json!({"x": 10, "y": 40, "width": 20, "height": 20})));
    run(&mut s, "select.set", json!({"ids": [inner.0, c.0]}));
    let instance = id(&run(&mut s, "symbol.new", json!({"name": "Pair"})));
    let before = s.doc().unwrap().active_layer;
    assert!(before.is_some());
    run(&mut s, "symbol.edit", json!({"id": instance.0}));
    let layer = s.doc().unwrap().doc.symbol_edit.as_ref().unwrap().layer;
    let group = {
        let st = s.doc().unwrap();
        layer_art(&s).into_iter().find(|i| st.doc.node(*i).is_some_and(|n| matches!(n.kind, NodeKind::Group { .. }))).expect("a group")
    };
    run(&mut s, "object.isolate", json!({"id": group.0}));
    assert_eq!(s.doc().unwrap().isolation, Some(group));
    run(&mut s, "edit.undo", json!({}));
    let st = s.doc().unwrap();
    assert!(st.doc.symbol_edit.is_none() && st.doc.node(layer).is_none());
    assert_eq!(st.isolation, None);
    assert_eq!(st.active_layer, before);
}

/// The text wrap of the area type in `n`.
fn wrap_in(n: &Node) -> Vec<vectorcraft_doc::WrapShape> {
    let mut wrap = None;
    n.walk(&mut |n| {
        if let NodeKind::Text(t) = &n.kind {
            wrap = Some(t.wrap.clone());
        }
    });
    wrap.expect("area type")
}

#[test]
fn moving_a_wrap_object_in_the_edit_gives_the_symbol_the_new_text_wrap() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 600, "height": 600}));
    let text = "word ".repeat(200);
    let t = id(&run(&mut s, "text.create", json!({"x": 0, "y": 0, "text": text, "size": 12, "area": {"width": 400, "height": 400}})));
    let r = id(&run(&mut s, "shape.rectangle", json!({"x": 0, "y": 50, "width": 200, "height": 100})));
    run(&mut s, "select.set", json!({"ids": [r.0]}));
    run(&mut s, "object.textWrap.make", json!({}));
    run(&mut s, "select.set", json!({"ids": [t.0, r.0]}));
    let instance = id(&run(&mut s, "symbol.new", json!({"name": "Wrapped"})));
    let symbol_wrap = |s: &Session| wrap_in(&s.doc().unwrap().doc.symbols.iter().find(|x| x.name == "Wrapped").expect("the symbol").art);
    let entered = symbol_wrap(&s);
    assert!(!entered.is_empty());
    run(&mut s, "symbol.edit", json!({"id": instance.0}));
    let (layer, rect) = {
        let st = s.doc().unwrap();
        let layer = st.doc.symbol_edit.as_ref().unwrap().layer;
        let rect = layer_art(&s).into_iter().find(|i| st.doc.node(*i).is_some_and(|n| n.extras().wrap.is_some())).expect("the wrap object");
        (layer, rect)
    };
    run(&mut s, "select.set", json!({"ids": [rect.0]}));
    run(&mut s, "object.move", json!({"dx": 150, "dy": 0}));
    let on_layer = wrap_in(s.doc().unwrap().doc.node(layer).unwrap());
    assert_ne!(on_layer, entered, "the editing layer's text wraps around the moved object");
    assert_eq!(symbol_wrap(&s), on_layer);
    // Leaving keeps it.
    run(&mut s, "symbol.exitEditing", json!({}));
    assert_eq!(symbol_wrap(&s), on_layer);
}
