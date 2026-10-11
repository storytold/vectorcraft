//! The Layers panel's commands: rows at every depth (layers, sublayers, groups, objects) are
//! clickable, highlighted and targeted; their art is selected through the selection column; rows
//! move, copy and reparent with validation; and the panel menu's operations (merge, flatten,
//! release to layers, reverse, template, others/all) are single undo steps.

use serde_json::{Value, json};
use vectorcraft_doc::{LayerColor, Node, NodeId, NodeKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn id(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

fn rect(s: &mut Session, x: f64) -> NodeId {
    id(&s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 20, "height": 20})).unwrap())
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn parent(s: &Session, id: NodeId) -> Option<NodeId> {
    s.doc().unwrap().doc.parent_of(id)
}

fn children(s: &Session, id: NodeId) -> Vec<NodeId> {
    node(s, id).children().unwrap().iter().map(|c| c.id).collect()
}

fn undos(s: &Session) -> usize {
    s.doc().unwrap().history.undo.len()
}

/// Layer 1 holding a sublayer `sub` with a rectangle `a` and a group `g` of rectangle `b`, plus a
/// rectangle `c` on Layer 1 itself → (layer, sub, a, g, b, c). Nothing selected.
fn tree(s: &mut Session) -> (NodeId, NodeId, NodeId, NodeId, NodeId, NodeId) {
    let layer = s.doc().unwrap().doc.layers[0].id;
    let c = rect(s, 300.0);
    let sub = id(&run(s, "layer.newSublayer", json!({"name": "Sub"})));
    let a = rect(s, 0.0);
    let b = rect(s, 100.0);
    run(s, "select.set", json!({"ids": [b.0]}));
    let g = id(&run(s, "object.group", json!({})));
    run(s, "select.none", json!({}));
    (layer, sub, a, g, b, c)
}

#[test]
fn art_in_a_sublayer_is_selected_as_objects_not_the_sublayer() {
    let mut s = session();
    let (_, sub, a, g, _, c) = tree(&mut s);
    assert_eq!(parent(&s, a), Some(sub), "new art went into the current sublayer");
    let n = run(&mut s, "select.all", json!({}))["count"].as_u64().unwrap();
    assert_eq!(n, 3);
    let sel = s.doc().unwrap().selection.objects.clone();
    assert!(sel.contains(&a) && sel.contains(&g) && sel.contains(&c) && !sel.contains(&sub), "{sel:?}");
    // The Selection tool's click takes the object in the sublayer.
    let h = vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, vectorcraft_geom::Point::new(10.0, 10.0), Default::default()).unwrap();
    assert_eq!(h.top_object(None), a);
    // Deleting it works (layers were skipped by Clear before).
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    run(&mut s, "edit.clear", json!({}));
    assert!(s.doc().unwrap().doc.node(a).is_none());
}

/// #1002: with no layer shown and unlocked, new art is refused (the document unchanged) rather
/// than put on a hidden or locked layer; another layer that takes art still gets it.
#[test]
fn new_art_is_refused_when_no_layer_is_shown_and_unlocked() {
    for prop in ["visible", "locked"] {
        let mut s = session();
        let layer = s.doc().unwrap().doc.layers[0].id;
        let copied = rect(&mut s, 0.0);
        run(&mut s, "select.set", json!({"ids": [copied.0]}));
        run(&mut s, "edit.copy", json!({}));
        run(&mut s, "select.none", json!({}));
        run(&mut s, "layer.setProps", json!({"ids": [layer.0], prop: prop == "locked"}));
        let before = s.doc().unwrap().doc.clone();
        for (cmd, p) in [
            ("shape.ellipse", json!({"x": 10, "y": 10, "width": 20, "height": 20})),
            ("shape.polarGrid", json!({"x": 10, "y": 10, "width": 20, "height": 20})),
            ("text.create", json!({"x": 10, "y": 40, "text": "A"})),
            ("edit.paste", json!({})),
        ] {
            let e = s.execute(cmd, &p).unwrap_err();
            assert_eq!(e.to_string(), "the target layer is locked or hidden", "{prop} {cmd}");
        }
        assert_eq!(s.doc().unwrap().doc.layers, before.layers, "{prop}: nothing was added");
        let other = id(&run(&mut s, "layer.new", json!({})));
        run(&mut s, "layer.setCurrent", json!({"id": layer.0}));
        let n = rect(&mut s, 50.0);
        assert_eq!(parent(&s, n), Some(other), "{prop}");
    }
}

#[test]
fn clicking_rows_at_any_depth_highlights_them_and_sets_the_current_layer() {
    let mut s = session();
    let (layer, sub, a, g, b, c) = tree(&mut s);
    let before = undos(&s);
    for (row, current) in [(layer, layer), (sub, sub), (g, sub), (b, sub), (a, sub), (c, layer)] {
        let r = run(&mut s, "layer.setCurrent", json!({"id": row.0}));
        assert_eq!(NodeId(r["layer"].as_u64().unwrap()), current);
        let st = s.doc().unwrap();
        assert_eq!((st.highlighted_rows(), st.current_layer()), (vec![row], Some(current)));
    }
    assert_eq!(undos(&s), before, "clicking rows is not an undo step");
    // New art goes into the current sublayer.
    run(&mut s, "layer.setCurrent", json!({"id": a.0}));
    let n = rect(&mut s, 50.0);
    assert_eq!(parent(&s, n), Some(sub));
    assert!(s.execute("layer.setCurrent", &json!({"id": 9999})).is_err());
    // Shift/Ctrl: several rows.
    run(&mut s, "layer.highlight", json!({"ids": [a.0, g.0], "mode": "set"}));
    run(&mut s, "layer.highlight", json!({"ids": [c.0], "mode": "add"}));
    run(&mut s, "layer.highlight", json!({"ids": [a.0], "mode": "toggle"}));
    let st = s.doc().unwrap();
    assert_eq!((st.highlighted_rows(), st.current_layer()), (vec![g, c], Some(layer)));
    assert!(s.execute("layer.highlight", &json!({"ids": [a.0], "mode": "nope"})).is_err());
    assert_eq!(document_rows(&s), vec![g.0, c.0], "document.inspect lists the rows");
}

fn document_rows(s: &Session) -> Vec<u64> {
    crate::inspect::document(s)["layerRows"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap()).collect()
}

#[test]
fn selecting_art_makes_its_layer_current() {
    let mut s = session();
    let (layer, sub, a, _, _, c) = tree(&mut s);
    run(&mut s, "select.set", json!({"ids": [c.0]}));
    assert_eq!(s.doc().unwrap().current_layer(), Some(layer));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    assert_eq!(s.doc().unwrap().current_layer(), Some(sub));
    let n = rect(&mut s, 200.0);
    assert_eq!(parent(&s, n), Some(sub));
    // A locked sublayer takes no new art: it goes to the top layer.
    run(&mut s, "select.none", json!({}));
    run(&mut s, "layer.setProps", json!({"id": sub.0, "locked": true}));
    run(&mut s, "layer.setCurrent", json!({"id": sub.0}));
    let n = rect(&mut s, 200.0);
    assert_eq!(parent(&s, n), Some(layer));
}

#[test]
fn the_selection_column_selects_a_rows_art_and_shift_adds_or_removes_it() {
    let mut s = session();
    let (layer, sub, a, g, _, c) = tree(&mut s);
    run(&mut s, "layer.setCurrent", json!({"id": sub.0}));
    let sel = |s: &Session| s.doc().unwrap().selection.objects.clone();
    run(&mut s, "layer.selectAll", json!({"id": layer.0}));
    assert_eq!(sel(&s), vec![c, a, g], "the layer's art, its sublayer's too");
    run(&mut s, "layer.selectAll", json!({"id": sub.0}));
    assert_eq!(sel(&s), vec![a, g]);
    run(&mut s, "layer.selectAll", json!({"id": c.0, "add": true}));
    assert_eq!(sel(&s), vec![a, g, c]);
    run(&mut s, "layer.selectAll", json!({"id": sub.0, "add": true}));
    assert_eq!(sel(&s), vec![c], "all selected already: removed");
    assert_eq!(s.doc().unwrap().highlighted_rows(), vec![sub], "the highlighted row stays");
    // Locked rows select nothing.
    run(&mut s, "layer.setProps", json!({"id": sub.0, "locked": true}));
    run(&mut s, "layer.selectAll", json!({"id": a.0}));
    assert!(sel(&s).is_empty());
    // Targeting a layer selects its sublayers' art too.
    run(&mut s, "layer.setProps", json!({"id": sub.0, "locked": false}));
    assert_eq!(ids_sel(&run(&mut s, "layer.target", json!({"id": layer.0}))), vec![c, a, g]);
}

fn ids_sel(v: &Value) -> Vec<NodeId> {
    v["selected"].as_array().unwrap().iter().map(|i| NodeId(i.as_u64().unwrap())).collect()
}

#[test]
fn visibility_and_lock_per_row_are_undoable() {
    let mut s = session();
    let (_, sub, a, g, b, _) = tree(&mut s);
    run(&mut s, "select.set", json!({"ids": [a.0, g.0]}));
    let before = undos(&s);
    run(&mut s, "layer.setProps", json!({"ids": [b.0, sub.0], "visible": false}));
    assert!(!node(&s, b).visible && !node(&s, sub).visible);
    assert!(s.doc().unwrap().selection.is_empty(), "hidden art is deselected");
    assert_eq!(undos(&s), before + 1, "one step for several rows");
    run(&mut s, "edit.undo", json!({}));
    assert!(node(&s, b).visible && node(&s, sub).visible);
    run(&mut s, "layer.setProps", json!({"id": a.0, "locked": true, "name": "Box"}));
    assert!(node(&s, a).locked);
    assert_eq!(node(&s, a).display_name(), "Box");
    run(&mut s, "layer.setProps", json!({"id": a.0, "name": ""}));
    assert_eq!(node(&s, a).display_name(), "<Rectangle>", "an empty name goes back to the generated one");
    run(&mut s, "layer.setProps", json!({"id": sub.0, "name": ""}));
    assert_eq!(node(&s, sub).display_name(), "Sub", "a layer keeps its name");
}

#[test]
fn object_set_props_deselects_the_objects_it_hides_or_locks() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
    for (prop, value) in [("visible", false), ("locked", true)] {
        run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
        run(&mut s, "select.key", json!({"id": b.0}));
        let mut p = json!({"ids": [b.0]});
        p[prop] = json!(value);
        run(&mut s, "object.setProps", p);
        assert!(!s.doc().unwrap().doc.is_editable(b));
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![a], None), "{prop}: {value} deselects the key object");
        run(&mut s, "edit.undo", json!({}));
        assert!(s.doc().unwrap().doc.is_editable(b));
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![a, b], Some(b)), "undoing {prop}: {value} selects it again as the key");
    }
}

#[test]
fn object_set_props_deselects_the_objects_inside_a_group_or_layer_it_hides_or_locks() {
    let mut s = session();
    let (_, sub, a, g, b, c) = tree(&mut s);
    run(&mut s, "select.set", json!({"ids": [a.0, b.0, c.0]}));
    run(&mut s, "object.setProps", json!({"ids": [g.0], "locked": true}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, c], "b is in the locked group");
    run(&mut s, "select.key", json!({"id": a.0}));
    assert_eq!(s.doc().unwrap().selection.key, Some(a));
    run(&mut s, "object.setProps", json!({"ids": [sub.0], "visible": false}));
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![c], None), "a, the key object, is in the hidden sublayer");
}

#[test]
fn object_set_props_that_hides_or_locks_nothing_keeps_the_selection() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let x = rect(&mut s, 0.0);
    run(&mut s, "select.set", json!({"ids": [x.0]}));
    run(&mut s, "object.lock", json!({}));
    run(&mut s, "layer.setProps", json!({"id": layer.0, "locked": true}));
    run(&mut s, "object.unlockAll", json!({}));
    let st = s.doc().unwrap();
    assert!(st.selection.objects == vec![x] && !st.doc.is_editable(x), "Unlock All selects x, and its layer still locks it");
    for p in [json!({"opacity": 50}), json!({"visible": true, "locked": false})] {
        run(&mut s, "object.setProps", p.clone());
        assert_eq!(s.doc().unwrap().selection.objects, vec![x], "{p} keeps x selected");
    }
    assert_eq!(node(&s, x).opacity, 0.5);
}

#[test]
fn object_set_props_that_hides_or_locks_deselects_objects_hidden_or_locked_before() {
    let mut s = session();
    let (x, y, z) = (rect(&mut s, 0.0), rect(&mut s, 100.0), rect(&mut s, 200.0));
    run(&mut s, "object.setProps", json!({"ids": [x.0], "visible": false}));
    run(&mut s, "select.set", json!({"ids": [x.0, y.0, z.0]}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![x, y, z], "select.set selects the hidden x");
    run(&mut s, "object.setProps", json!({"ids": [y.0], "locked": true}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![z], "locking y deselects x, hidden before the call, too");
}

#[test]
fn object_set_props_that_changes_nothing_deselects_what_it_leaves_hidden_or_locked() {
    let mut s = session();
    let (x, y) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
    for (prop, value) in [("visible", false), ("locked", true)] {
        let mut p = json!({"ids": [x.0]});
        p[prop] = json!(value);
        run(&mut s, "object.setProps", p.clone());
        run(&mut s, "select.set", json!({"ids": [x.0, y.0]}));
        run(&mut s, "select.key", json!({"id": x.0}));
        let before = undos(&s);
        run(&mut s, "object.setProps", p);
        assert_eq!(undos(&s), before, "{prop}: {value} again records no undo step");
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![y], None), "{prop}: {value} again deselects x, the key object");
        run(&mut s, "edit.undo", json!({}));
        assert!(s.doc().unwrap().doc.is_editable(x));
    }
}

#[test]
fn layer_options_validate_and_template_locks_and_dims() {
    let mut s = session();
    let (layer, ..) = tree(&mut s);
    for bad in [json!({"color": 27}), json!({"color": "#zz"}), json!({"color": [1]}), json!({"locked": "yes"}), json!({"dimImages": "half"})] {
        let mut p = bad.clone();
        p["id"] = json!(layer.0);
        assert!(s.execute("layer.setProps", &p).is_err(), "{bad}");
    }
    let color = |s: &Session| match node(s, layer).kind {
        NodeKind::Layer { color, .. } => color,
        _ => panic!("not a layer"),
    };
    run(&mut s, "layer.setProps", json!({"id": layer.0, "color": "red"}));
    assert_eq!(color(&s), LayerColor::Preset(1));
    run(&mut s, "layer.setProps", json!({"id": layer.0, "color": "#123456"}));
    assert_eq!(color(&s), LayerColor::Custom([0x12, 0x34, 0x56]));
    run(&mut s, "layer.setProps", json!({"id": layer.0, "color": 4}));
    assert_eq!(color(&s), LayerColor::Preset(4));
    run(&mut s, "layer.template", json!({"ids": [layer.0]}));
    let n = node(&s, layer);
    assert!(n.is_template() && n.locked);
    assert!(matches!(n.kind, NodeKind::Layer { dim_images: Some(50), .. }));
    run(&mut s, "layer.template", json!({"ids": [layer.0]}));
    let n = node(&s, layer);
    assert!(!n.is_template() && !n.locked);
    assert!(matches!(n.kind, NodeKind::Layer { dim_images: None, .. }));
    run(&mut s, "layer.setProps", json!({"id": layer.0, "preview": false, "dimImages": 30, "printable": false}));
    assert!(matches!(node(&s, layer).kind, NodeKind::Layer { preview: false, dim_images: Some(30), printable: false, .. }));
    let summary = &crate::inspect::document(&s)["layers"][0];
    assert_eq!((summary["preview"].clone(), summary["dimImages"].clone()), (json!(false), json!(30)));
}

#[test]
fn new_layers_go_above_the_current_one_at_its_level() {
    let mut s = session();
    let (layer, sub, _, g, ..) = tree(&mut s);
    run(&mut s, "layer.setCurrent", json!({"id": sub.0}));
    let n = id(&run(&mut s, "layer.new", json!({})));
    assert_eq!(parent(&s, n), Some(layer), "a sublayer beside the current sublayer");
    assert_eq!(children(&s, layer).iter().position(|c| *c == n), children(&s, layer).iter().position(|c| *c == sub).map(|i| i + 1));
    assert_eq!(s.doc().unwrap().highlighted_rows(), vec![n]);
    let top = id(&run(&mut s, "layer.new", json!({"top": true, "name": "Top", "color": "green", "locked": true})));
    let st = s.doc().unwrap();
    assert_eq!(st.doc.layers.last().unwrap().id, top);
    assert!(st.doc.layers.last().unwrap().locked);
    // Sublayers only go into layers.
    assert!(s.execute("layer.newSublayer", &json!({"parent": g.0})).is_err());
    let inner = id(&run(&mut s, "layer.newSublayer", json!({"parent": sub.0})));
    assert_eq!(parent(&s, inner), Some(sub));
    // Names count sublayers: no two layers share one.
    let mut names = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if n.is_layer() {
            names.push(n.display_name());
        }
    });
    let unique: std::collections::HashSet<&String> = names.iter().collect();
    assert_eq!(unique.len(), names.len(), "{names:?}");
}

#[test]
fn rows_move_above_below_and_into_layers_and_groups() {
    let mut s = session();
    let (layer, sub, a, g, b, c) = tree(&mut s);
    let before = undos(&s);
    // Into the group: on top of its contents.
    run(&mut s, "layer.move", json!({"ids": [a.0], "target": g.0, "place": "inside"}));
    assert_eq!(children(&s, g), vec![b, a]);
    // Below b, in the group.
    run(&mut s, "layer.move", json!({"ids": [a.0], "target": b.0, "place": "below"}));
    assert_eq!(children(&s, g), vec![a, b]);
    // Above the sublayer, out into Layer 1.
    run(&mut s, "layer.move", json!({"ids": [a.0], "target": sub.0, "place": "above"}));
    assert_eq!(parent(&s, a), Some(layer));
    assert_eq!(undos(&s), before + 3);
    // Out of the layer into a new top-level layer: an object beside a top-level layer goes in it.
    let l2 = id(&run(&mut s, "layer.new", json!({"top": true})));
    run(&mut s, "layer.move", json!({"ids": [c.0], "target": l2.0, "place": "above"}));
    assert_eq!(parent(&s, c), Some(l2));
    // A sublayer becomes a top-level layer, and back.
    run(&mut s, "layer.move", json!({"ids": [sub.0], "target": l2.0, "place": "above"}));
    assert_eq!(parent(&s, sub), None);
    assert_eq!(s.doc().unwrap().doc.layers.last().unwrap().id, sub);
    run(&mut s, "layer.move", json!({"ids": [sub.0], "target": layer.0}));
    assert_eq!(parent(&s, sub), Some(layer));
    // Several rows keep their stacking order.
    run(&mut s, "layer.move", json!({"ids": [g.0, a.0], "target": l2.0}));
    assert_eq!(children(&s, l2), vec![c, a, g]);
    // Undo walks it back.
    for _ in 0..4 {
        run(&mut s, "edit.undo", json!({}));
    }
    assert_eq!(parent(&s, sub), Some(layer));
}

#[test]
fn moves_are_validated() {
    let mut s = session();
    let (layer, sub, a, g, b, _) = tree(&mut s);
    let doc_before = s.doc().unwrap().doc.clone();
    let bad = [
        json!({"ids": [sub.0], "target": g.0, "place": "inside"}),
        json!({"ids": [layer.0], "target": a.0, "place": "inside"}),
        json!({"ids": [layer.0], "target": sub.0, "place": "above"}),
        json!({"ids": [sub.0], "target": sub.0}),
        json!({"ids": [g.0], "target": b.0, "place": "inside"}),
        json!({"ids": [a.0], "target": b.0, "place": "inside"}),
        json!({"ids": [a.0], "target": 9999}),
        json!({"ids": [9999], "target": a.0}),
        json!({"ids": [a.0], "target": g.0, "place": "sideways"}),
        json!({"ids": [], "target": g.0}),
        json!({"target": g.0}),
    ];
    for p in bad {
        assert!(s.execute("layer.move", &p).is_err(), "{p}");
    }
    // A locked group takes nothing.
    run(&mut s, "layer.setProps", json!({"id": g.0, "locked": true}));
    assert!(s.execute("layer.move", &json!({"ids": [a.0], "target": g.0})).is_err());
    run(&mut s, "edit.undo", json!({}));
    assert!(std::sync::Arc::ptr_eq(&s.doc().unwrap().doc, &doc_before), "nothing changed");
    // node.move: layers only at the top level or in layers.
    assert!(s.execute("node.move", &json!({"id": sub.0, "parent": g.0, "index": 0})).is_err());
    assert!(s.execute("node.move", &json!({"id": a.0, "index": 0})).is_err());
    assert!(s.execute("node.move", &json!({"id": layer.0, "parent": sub.0, "index": 0})).is_err());
    run(&mut s, "node.move", json!({"id": sub.0, "index": 5}));
    assert_eq!(parent(&s, sub), None);
}

#[test]
fn alt_drag_copies_rows_and_the_selected_art_moves_between_layers() {
    let mut s = session();
    let (layer, sub, a, g, _, c) = tree(&mut s);
    let count = s.doc().unwrap().doc.node_count();
    let copies = ids(&run(&mut s, "layer.move", json!({"ids": [sub.0], "target": layer.0, "place": "above", "copy": true})));
    assert_eq!(copies.len(), 1);
    assert_eq!(s.doc().unwrap().doc.node_count(), count + 4, "the sublayer and its art (a, the group, b), copied");
    assert_eq!(parent(&s, copies[0]), None, "a copy of the sublayer at the top level");
    // Dragging the selected-art square: the selection moves into another layer.
    run(&mut s, "select.set", json!({"ids": [a.0, g.0]}));
    run(&mut s, "layer.move", json!({"ids": [a.0, g.0], "target": layer.0}));
    assert_eq!(children(&s, layer), vec![c, sub, a, g]);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, g], "it stays selected");
}

#[test]
fn delete_and_duplicate_rows() {
    let mut s = session();
    let (layer, sub, a, g, _, c) = tree(&mut s);
    run(&mut s, "layer.highlight", json!({"ids": [a.0, g.0]}));
    let copies = ids(&run(&mut s, "layer.duplicate", json!({})));
    assert_eq!(copies.len(), 2);
    assert_eq!(children(&s, sub).len(), 4);
    assert_eq!(s.doc().unwrap().highlighted_rows(), copies);
    run(&mut s, "layer.duplicate", json!({"id": sub.0}));
    assert!(children(&s, layer).iter().any(|l| node(&s, *l).display_name() == "Sub copy"));
    run(&mut s, "layer.highlight", json!({"ids": copies.iter().map(|c| c.0).collect::<Vec<_>>()}));
    assert_eq!(run(&mut s, "layer.delete", json!({}))["deleted"], 2);
    assert_eq!(children(&s, sub), vec![a, g]);
    run(&mut s, "layer.delete", json!({"ids": [c.0, a.0]}));
    assert!(s.doc().unwrap().doc.node(c).is_none() && s.doc().unwrap().doc.node(a).is_none());
    run(&mut s, "edit.undo", json!({}));
    assert!(s.doc().unwrap().doc.node(c).is_some());
    assert!(s.execute("layer.delete", &json!({"ids": [layer.0]})).is_err(), "the last layer stays");
    run(&mut s, "layer.setCurrent", json!({"id": sub.0}));
    run(&mut s, "layer.delete", json!({}));
    assert!(s.doc().unwrap().doc.node(sub).is_none());
    assert_eq!(s.doc().unwrap().current_layer(), Some(layer), "the current layer moves on");
}

#[test]
fn collect_merge_flatten_and_reverse() {
    let mut s = session();
    let (layer, sub, a, g, _, c) = tree(&mut s);
    // Collect two rows of the sublayer in a new sublayer of it.
    run(&mut s, "layer.highlight", json!({"ids": [a.0, g.0]}));
    let col = id(&run(&mut s, "layer.collectInNew", json!({})));
    assert_eq!((parent(&s, col), children(&s, col)), (Some(sub), vec![a, g]));
    // Reverse their order.
    run(&mut s, "layer.reverse", json!({"ids": [a.0, g.0]}));
    assert_eq!(children(&s, col), vec![g, a]);
    assert!(s.execute("layer.reverse", &json!({"ids": [a.0]})).is_err());
    run(&mut s, "edit.undo", json!({}));
    // Merge the sublayer into Layer 1 (listed last).
    run(&mut s, "layer.merge", json!({"ids": [col.0, layer.0]}));
    assert!(s.doc().unwrap().doc.node(col).is_none());
    assert_eq!(children(&s, layer), vec![c, sub, a, g]);
    assert!(s.execute("layer.merge", &json!({"ids": [layer.0, sub.0]})).is_err(), "not into its own sublayer");
    assert!(s.execute("layer.merge", &json!({"ids": [layer.0]})).is_err());
    // Flatten: other visible layers merge in, hidden ones go, templates stay.
    let l2 = id(&run(&mut s, "layer.new", json!({"top": true})));
    let d = rect(&mut s, 0.0);
    let l3 = id(&run(&mut s, "layer.new", json!({"top": true, "visible": false})));
    let l4 = id(&run(&mut s, "layer.new", json!({"top": true, "template": true})));
    let before = undos(&s);
    let r = run(&mut s, "layer.flatten", json!({"id": layer.0}));
    assert_eq!(r["discarded"], 1);
    let st = s.doc().unwrap();
    assert_eq!(st.doc.layers.iter().map(|l| l.id).collect::<Vec<_>>(), vec![layer, l4]);
    assert!(st.doc.node(l2).is_none() && st.doc.node(l3).is_none());
    assert_eq!(parent(&s, d), Some(layer));
    assert_eq!(undos(&s), before + 1);
}

#[test]
fn release_to_layers_sequence_and_build() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let (a, b, c) = (rect(&mut s, 0.0), rect(&mut s, 50.0), rect(&mut s, 100.0));
    let new = ids(&run(&mut s, "layer.releaseToLayers", json!({"id": layer.0})));
    assert_eq!(new.len(), 3);
    assert_eq!(children(&s, layer), new);
    assert_eq!(new.iter().map(|l| children(&s, *l)).collect::<Vec<_>>(), vec![vec![a], vec![b], vec![c]]);
    run(&mut s, "edit.undo", json!({}));
    let new = ids(&run(&mut s, "layer.releaseToLayers", json!({"id": layer.0, "build": true})));
    let counts: Vec<usize> = new.iter().map(|l| children(&s, *l).len()).collect();
    assert_eq!(counts, vec![1, 2, 3], "the layers add up");
    assert_eq!(children(&s, new[2]), vec![a, b, c], "the top one keeps the originals");
    run(&mut s, "edit.undo", json!({}));
    // A group's objects go into sublayers where the group was.
    run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
    let g = id(&run(&mut s, "object.group", json!({})));
    let new = ids(&run(&mut s, "layer.releaseToLayersBuild", json!({"id": g.0})));
    assert_eq!(new.len(), 2);
    assert!(s.doc().unwrap().doc.node(g).is_none());
    assert_eq!(children(&s, layer), vec![new[0], new[1], c]);
    assert!(s.execute("layer.releaseToLayers", &json!({"id": c.0})).is_err());
}

#[test]
fn hide_lock_and_outline_others_and_their_all_counterparts() {
    let mut s = session();
    let l1 = s.doc().unwrap().doc.layers[0].id;
    let l2 = id(&run(&mut s, "layer.new", json!({})));
    let l3 = id(&run(&mut s, "layer.new", json!({})));
    run(&mut s, "layer.setCurrent", json!({"id": l2.0}));
    let flags = |s: &Session, f: fn(&Node) -> bool| [l1, l2, l3].map(|l| f(&node(s, l)));
    assert_eq!(run(&mut s, "layer.hideOthers", json!({}))["count"], 2);
    assert_eq!(flags(&s, |n| n.visible), [false, true, false]);
    run(&mut s, "layer.showAll", json!({}));
    assert_eq!(flags(&s, |n| n.visible), [true; 3]);
    run(&mut s, "layer.lockOthers", json!({}));
    assert_eq!(flags(&s, |n| n.locked), [true, false, true]);
    run(&mut s, "layer.unlockAll", json!({}));
    assert_eq!(flags(&s, |n| n.locked), [false; 3]);
    run(&mut s, "layer.outlineOthers", json!({}));
    let preview = |n: &Node| matches!(n.kind, NodeKind::Layer { preview: true, .. });
    assert_eq!(flags(&s, preview), [false, true, false]);
    run(&mut s, "layer.previewAll", json!({}));
    assert_eq!(flags(&s, preview), [true; 3]);
    assert_eq!(run(&mut s, "layer.previewAll", json!({}))["count"], 0, "nothing to do: no undo step");
}

#[test]
fn locate_object_highlights_its_row() {
    let mut s = session();
    let (layer, sub, _, g, b, _) = tree(&mut s);
    run(&mut s, "select.set", json!({"ids": [g.0]}));
    let r = run(&mut s, "layer.locate", json!({}));
    assert_eq!(r["ancestry"], json!([layer.0, sub.0, g.0]));
    assert_eq!(s.doc().unwrap().highlighted_rows(), vec![g]);
    assert_eq!(run(&mut s, "layer.locate", json!({"id": b.0}))["id"], b.0);
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("layer.locate", &json!({})).is_err());
}

#[test]
fn the_clipping_mask_button_acts_on_the_highlighted_group() {
    let mut s = session();
    let (_, _, a, g, _, _) = tree(&mut s);
    run(&mut s, "layer.move", json!({"ids": [a.0], "target": g.0}));
    run(&mut s, "layer.setCurrent", json!({"id": g.0}));
    assert_eq!(run(&mut s, "layer.clippingMask.toggle", json!({}))["clip"], true);
    assert!(node(&s, g).clips());
}
