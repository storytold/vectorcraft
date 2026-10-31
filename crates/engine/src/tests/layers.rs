//! The Layers panel's commands: rows at every depth (layers, sublayers, groups, objects) are
//! clickable, highlighted and targeted; their art is selected through the selection column; rows
//! move, copy and reparent with validation; and the panel menu's operations (merge, flatten,
//! release to layers, reverse, template, others/all) are single undo steps.

use serde_json::{Value, json};
use vectorcraft_doc::{LayerColor, Node, NodeId, NodeKind};

use crate::*;

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

/// Send to Current Layer fails with the error that new art gets (#1002) when the current layer is
/// hidden or locked, or is inside a hidden or locked layer: the objects stay where they were and stay
/// selected, and no undo step is recorded. Once the layer is shown and unlocked again, the command
/// moves them.
#[test]
fn send_to_current_layer_refuses_a_hidden_or_locked_current_layer() {
    for prop in ["visible", "locked"] {
        let mut s = session();
        let layer = s.doc().unwrap().doc.layers[0].id;
        let a = rect(&mut s, 0.0);
        let other = id(&run(&mut s, "layer.new", json!({})));
        let sub = id(&run(&mut s, "layer.newSublayer", json!({})));
        assert_eq!(parent(&s, sub), Some(other), "the sublayer is in the new layer");
        run(&mut s, "layer.setProps", json!({"ids": [other.0], prop: prop == "locked"}));
        run(&mut s, "select.set", json!({"ids": [a.0]}));
        for current in [other, sub] {
            // A click on the layer's row makes that layer current and keeps the selection.
            run(&mut s, "layer.setCurrent", json!({"id": current.0}));
            let (before, undo) = (s.doc().unwrap().doc.clone(), undos(&s));
            let e = s.execute("object.arrange.sendToCurrentLayer", &json!({})).unwrap_err();
            assert_eq!(e.to_string(), "the target layer is locked or hidden", "{prop} {current:?}");
            assert_eq!(s.doc().unwrap().doc.layers, before.layers, "{prop} {current:?}: nothing moved");
            assert_eq!(parent(&s, a), Some(layer), "{prop} {current:?}");
            assert_eq!(undos(&s), undo, "{prop} {current:?}: no undo step");
            assert_eq!(s.doc().unwrap().selection.objects, vec![a], "{prop} {current:?}");
        }
        run(&mut s, "layer.setProps", json!({"ids": [other.0], prop: prop != "locked"}));
        run(&mut s, "object.arrange.sendToCurrentLayer", json!({}));
        assert_eq!(parent(&s, a), Some(sub), "{prop}");
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

/// Preferences › Units › Identify Objects By › XML ID (#394): `layer.setProps` takes a valid XML
/// id and stores the object name; Object Name (the default) keeps free text. The Layers label is
/// the XML id when that preference is on.
#[test]
fn identify_objects_by_xml_id_names_layers() {
    let mut s = session();
    let a = rect(&mut s, 0.0);
    run(&mut s, "layer.setProps", json!({"id": a.0, "name": "My Rect #2"}));
    assert_eq!(node(&s, a).display_name(), "My Rect #2");
    assert_eq!(node(&s, a).identify_label(true), "My_Rect__x23_2");
    run(&mut s, "prefs.set", json!({"key": "identifyObjectsBy", "value": "xmlId"}));
    assert!(s.execute("layer.setProps", &json!({"id": a.0, "name": "My Rect"})).is_err(), "spaces are refused");
    run(&mut s, "layer.setProps", json!({"id": a.0, "name": "My_Rect__x23_2"}));
    assert_eq!(node(&s, a).display_name(), "My Rect #2", "the XML id is stored as the object name");
    run(&mut s, "prefs.set", json!({"key": "identifyObjectsBy", "value": "objectName"}));
    run(&mut s, "layer.setProps", json!({"id": a.0, "name": "My Rect"}));
    assert_eq!(node(&s, a).display_name(), "My Rect");
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
    // A 32-bit build read color 2^32 + 4 as preset 4.
    assert!(s.execute("layer.setProps", &json!({"id": layer.0, "color": (1u64 << 32) + 4})).is_err());
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

/// `below` puts a new layer directly below the current layer, at its level, and `bottom` puts a
/// new sublayer under the parent's contents, each in one undo step.
#[test]
fn new_layers_go_below_the_current_one_and_sublayers_under_the_contents() {
    let mut s = session();
    let (layer, sub, a, g, _, c) = tree(&mut s);
    run(&mut s, "layer.setCurrent", json!({"id": sub.0}));
    let before = undos(&s);
    let n = id(&run(&mut s, "layer.new", json!({"below": true})));
    assert_eq!(children(&s, layer), vec![c, n, sub], "a sublayer directly below the current sublayer");
    assert_eq!(undos(&s), before + 1);
    assert_eq!(s.doc().unwrap().highlighted_rows(), vec![n]);
    assert_eq!(s.doc().unwrap().current_layer(), Some(n));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(children(&s, layer), vec![c, sub]);
    // At the top level: below the current layer, above the layers under it.
    let top = id(&run(&mut s, "layer.new", json!({"top": true, "name": "Top"})));
    let middle = id(&run(&mut s, "layer.new", json!({"below": true, "name": "Middle", "locked": true})));
    let order = s.doc().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    assert_eq!(order, vec![layer, middle, top]);
    assert!(node(&s, middle).locked, "options as without `below`");
    run(&mut s, "layer.setCurrent", json!({"id": layer.0}));
    let bottom = id(&run(&mut s, "layer.new", json!({"below": true})));
    assert_eq!(s.doc().unwrap().doc.layers[0].id, bottom, "below the bottom layer");
    // Sublayers: on top of the parent's contents, or under them with `bottom`.
    let before = undos(&s);
    let under = id(&run(&mut s, "layer.newSublayer", json!({"parent": sub.0, "bottom": true})));
    assert_eq!(children(&s, sub), vec![under, a, g]);
    assert_eq!(undos(&s), before + 1);
    assert_eq!(s.doc().unwrap().highlighted_rows(), vec![under]);
    let over = id(&run(&mut s, "layer.newSublayer", json!({"parent": sub.0, "bottom": false})));
    assert_eq!(children(&s, sub), vec![under, a, g, over]);
    // Bad values change nothing.
    let count = s.doc().unwrap().doc.node_count();
    for (cmd, p) in [
        ("layer.new", json!({"below": true, "top": true})),
        ("layer.new", json!({"below": "yes"})),
        ("layer.newSublayer", json!({"parent": sub.0, "bottom": 1})),
    ] {
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
    }
    assert_eq!(s.doc().unwrap().doc.node_count(), count);
    // In a clipping layer the clipping path stays at the bottom.
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let (back, front) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
    run(&mut s, "layer.clippingMask.toggle", json!({"id": layer.0}));
    assert_eq!(children(&s, layer), vec![front, back], "the top object clips");
    let under = id(&run(&mut s, "layer.newSublayer", json!({"parent": layer.0, "bottom": true})));
    assert_eq!(children(&s, layer), vec![front, under, back]);
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

/// The Layers panel's commands refuse a switch given as text or as another value that is not true
/// or false, and leave the document, the selection, the highlighted rows and the undo history as
/// they were; true, false and null still work.
#[test]
fn layer_commands_refuse_a_switch_given_as_text() {
    let fixture = || {
        let mut s = session();
        let (layer, sub, a, g, _, c) = tree(&mut s);
        run(&mut s, "layer.setCurrent", json!({"id": sub.0}));
        (s, layer, sub, a, g, c)
    };
    let (_, layer, sub, a, g, _) = fixture();
    let mut unexpected = vec![];
    for (cmd, p, key) in [
        ("layer.new", json!({"top": "true"}), "top"),
        ("layer.new", json!({"top": 1, "name": "Top"}), "top"),
        ("layer.newSublayer", json!({"top": "false"}), "top"),
        ("layer.selectAll", json!({"id": layer.0, "add": "true"}), "add"),
        ("layer.target", json!({"id": layer.0, "add": 1}), "add"),
        ("layer.move", json!({"ids": [a.0], "target": layer.0, "copy": "true"}), "copy"),
        ("layer.move", json!({"ids": [a.0], "target": g.0, "copy": {}}), "copy"),
        ("layer.template", json!({"ids": [layer.0], "on": "false"}), "on"),
        ("layer.pasteRemembersLayers", json!({"on": "false"}), "on"),
        ("layer.pasteRemembersLayers", json!({"on": 0}), "on"),
        ("layer.releaseToLayers", json!({"id": sub.0, "build": [true]}), "build"),
    ] {
        // Each call on a fresh copy of the same tree, so one that goes through does not change the next.
        let (mut s, l, sl, o, gr, _) = fixture();
        assert_eq!((l, sl, o, gr), (layer, sub, a, g), "the same ids");
        let before = layer_target_snapshot(&s);
        let r = s.execute(cmd, &p);
        let want = format!("`{key}` must be true or false");
        if !matches!(&r, Err(EngineError::BadParams { cmd: got, msg }) if got == cmd && *msg == want) {
            unexpected.push(format!("{cmd} {p}: {r:?}"));
        } else if layer_target_snapshot(&s) != before {
            unexpected.push(format!("{cmd} {p}: refused, but the document, selection, rows or history changed"));
        }
    }
    assert!(unexpected.is_empty(), "not refused: {}", unexpected.join("; "));
    let (mut s, layer, sub, a, g, c) = fixture();
    // A null reads as the key left out.
    let n = id(&run(&mut s, "layer.new", json!({"top": null})));
    assert_eq!(parent(&s, n), Some(layer), "beside the current sublayer");
    run(&mut s, "layer.selectAll", json!({"id": sub.0, "add": null}));
    run(&mut s, "layer.selectAll", json!({"id": c.0, "add": true}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, g, c]);
    run(&mut s, "layer.move", json!({"ids": [a.0], "target": g.0, "copy": false}));
    assert_eq!(parent(&s, a), Some(g));
    assert_eq!(run(&mut s, "layer.template", json!({"ids": [layer.0], "on": null}))["on"], true);
    assert_eq!(run(&mut s, "layer.template", json!({"ids": [layer.0], "on": false}))["on"], false);
    assert_eq!(run(&mut s, "layer.pasteRemembersLayers", json!({"on": null}))["on"], true, "null toggles");
    assert_eq!(ids(&run(&mut s, "layer.releaseToLayers", json!({"id": g.0, "build": null}))).len(), 2);
}

/// `node.move` into a hidden or locked layer deselects the object it moves, here the key object.
/// Undo puts the object back and selects it again as the key object.
#[test]
fn node_move_into_a_hidden_or_locked_layer_deselects_the_object() {
    for prop in ["visible", "locked"] {
        let mut s = session();
        let layer = s.doc().unwrap().doc.layers[0].id;
        let (a, b) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
        let other = id(&run(&mut s, "layer.new", json!({})));
        run(&mut s, "layer.setProps", json!({"ids": [other.0], prop: prop == "locked"}));
        run(&mut s, "select.set", json!({"ids": [a.0, b.0]}));
        run(&mut s, "select.key", json!({"id": b.0}));
        run(&mut s, "node.move", json!({"id": b.0, "parent": other.0, "index": 0}));
        assert_eq!(parent(&s, b), Some(other), "{prop}");
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![a], None), "{prop}: b, the key object, is deselected");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(parent(&s, b), Some(layer), "{prop}");
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![a, b], Some(b)), "{prop}: undo selects b again as the key object");
    }
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

/// The selected art in rows dragged into a hidden layer or group is deselected, the key object
/// included. Undo puts the rows back and selects the art again.
#[test]
fn rows_dragged_into_a_hidden_layer_or_group_deselect_their_art() {
    let mut s = session();
    let (layer, sub, a, g, b, c) = tree(&mut s);
    let hidden = id(&run(&mut s, "layer.new", json!({"visible": false})));
    run(&mut s, "select.set", json!({"ids": [a.0, b.0, c.0]}));
    run(&mut s, "select.key", json!({"id": a.0}));
    // The sublayer, holding a and the group of b, goes into the hidden layer.
    run(&mut s, "layer.move", json!({"ids": [sub.0], "target": hidden.0}));
    assert_eq!(parent(&s, sub), Some(hidden));
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![c], None), "a, the key object, and b are hidden");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(parent(&s, sub), Some(layer));
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![a, b, c], Some(a)), "undo selects them again");
    // c goes into the group once the group is hidden.
    run(&mut s, "object.setProps", json!({"ids": [g.0], "visible": false}));
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, c], "b is in the hidden group");
    run(&mut s, "layer.move", json!({"ids": [c.0], "target": g.0, "place": "inside"}));
    assert_eq!(parent(&s, c), Some(g));
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![a], None), "c is hidden, and a single selected object has no key object");
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
    // A layer and its sublayer: the sublayer merges into its layer, never the other way round.
    run(&mut s, "layer.merge", json!({"ids": [layer.0, sub.0]}));
    assert!(s.doc().unwrap().doc.node(sub).is_none());
    assert_eq!(children(&s, layer), vec![c, a, g]);
    run(&mut s, "edit.undo", json!({}));
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

/// Merge Selected with every row highlighted (a layer, its sublayer and another layer, in panel
/// order) merges the layers into the last top-level one instead of refusing a sublayer target (#2383).
#[test]
fn merge_selected_with_all_layer_rows_highlighted_merges_into_the_last_top_level_layer() {
    for reversed in [false, true] {
        let mut s = session();
        let (layer, sub, a, _, _, c) = tree(&mut s);
        let l2 = id(&run(&mut s, "layer.new", json!({"top": true})));
        let d = rect(&mut s, 500.0);
        // Panel order, top first: Layer 2, Layer 1, its sublayer.
        let mut rows = vec![l2.0, layer.0, sub.0];
        if reversed {
            rows.reverse();
        }
        run(&mut s, "layer.highlight", json!({"ids": rows}));
        let into = id(&run(&mut s, "layer.merge", json!({})));
        let st = s.doc().unwrap();
        assert_eq!(st.doc.layers.len(), 1, "reversed: {reversed}");
        assert_eq!(into, st.doc.layers[0].id);
        for n in [a, c, d] {
            assert!(st.doc.layer_of(n) == Some(into), "{n:?} reversed: {reversed}");
        }
        assert!(st.doc.node(sub).is_some() || st.doc.node(layer).is_some() || st.doc.node(l2).is_some());
    }
}

/// Collect in New Layer puts the new layer beside the topmost row when that row is directly in a
/// layer, and on top of the layer holding its group when it is in a group. When that row is in a
/// hidden or locked layer, the selected art it collects from another layer is deselected, the key
/// object included, and undo selects it again.
#[test]
fn collect_in_new_layer_inside_a_hidden_or_locked_layer_deselects_the_art_it_moves() {
    for prop in ["visible", "locked"] {
        let mut s = session();
        let (x, z) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
        let top = id(&run(&mut s, "layer.new", json!({})));
        let y = rect(&mut s, 200.0);
        run(&mut s, "layer.setProps", json!({"ids": [top.0], prop: prop == "locked"}));
        run(&mut s, "select.set", json!({"ids": [x.0, z.0]}));
        run(&mut s, "select.key", json!({"id": x.0}));
        let col = id(&run(&mut s, "layer.collectInNew", json!({"ids": [x.0, y.0]})));
        assert_eq!((parent(&s, col), children(&s, col)), (Some(top), vec![x, y]), "{prop}");
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![z], None), "{prop}: x, the key object, is deselected");
        run(&mut s, "edit.undo", json!({}));
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![x, z], Some(x)), "{prop}: undo selects x again as the key object");
    }
}

/// Merge Selected into a hidden or locked layer deselects the selected art it moves there, the key
/// object included, and undo selects it again.
#[test]
fn merge_selected_into_a_hidden_or_locked_layer_deselects_the_art_it_moves() {
    for prop in ["visible", "locked"] {
        let mut s = session();
        let layer = s.doc().unwrap().doc.layers[0].id;
        let (x, z) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
        let into = id(&run(&mut s, "layer.new", json!({})));
        run(&mut s, "layer.setProps", json!({"ids": [into.0], prop: prop == "locked"}));
        run(&mut s, "select.set", json!({"ids": [x.0, z.0]}));
        run(&mut s, "select.key", json!({"id": x.0}));
        run(&mut s, "layer.merge", json!({"ids": [x.0, into.0]}));
        assert_eq!((parent(&s, x), parent(&s, z)), (Some(into), Some(layer)), "{prop}");
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![z], None), "{prop}: x, the key object, is deselected");
        run(&mut s, "edit.undo", json!({}));
        let sel = s.doc().unwrap().selection.clone();
        assert_eq!((sel.objects, sel.key), (vec![x, z], Some(x)), "{prop}: undo selects x again as the key object");
    }
}

/// Flatten Artwork into a locked layer deselects the selected art it moves there, the key object
/// included, and undo selects it again.
#[test]
fn flatten_artwork_into_a_locked_layer_deselects_the_art_it_moves() {
    let mut s = session();
    let layer = s.doc().unwrap().doc.layers[0].id;
    let (x, z) = (rect(&mut s, 0.0), rect(&mut s, 100.0));
    let into = id(&run(&mut s, "layer.new", json!({"locked": true})));
    run(&mut s, "select.set", json!({"ids": [x.0, z.0]}));
    run(&mut s, "select.key", json!({"id": x.0}));
    run(&mut s, "layer.flatten", json!({"id": into.0}));
    assert_eq!((parent(&s, x), parent(&s, z)), (Some(into), Some(into)));
    assert!(s.doc().unwrap().doc.node(layer).is_none(), "Layer 1 was merged into the locked layer");
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![], None), "x and z are locked");
    run(&mut s, "edit.undo", json!({}));
    let sel = s.doc().unwrap().selection.clone();
    assert_eq!((sel.objects, sel.key), (vec![x, z], Some(x)), "undo selects them again with the key object");
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

/// #1158: Alt-click on a hidden (locked) layer's eye (lock) isolates it: it shows (unlocks) as the
/// others hide (lock), in one undo step.
#[test]
fn hide_and_lock_others_show_and_unlock_the_layer_kept() {
    let mut s = session();
    let l1 = s.doc().unwrap().doc.layers[0].id;
    let l2 = id(&run(&mut s, "layer.new", json!({})));
    let flags = |s: &Session, f: fn(&Node) -> bool| [l1, l2].map(|l| f(&node(s, l)));
    run(&mut s, "layer.setProps", json!({"ids": [l2.0], "visible": false, "locked": true}));
    assert_eq!(run(&mut s, "layer.hideOthers", json!({"ids": [l2.0]}))["count"], 2);
    assert_eq!(flags(&s, |n| n.visible), [false, true]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(flags(&s, |n| n.visible), [true, false], "one undo step");
    run(&mut s, "layer.lockOthers", json!({"ids": [l2.0]}));
    assert_eq!(flags(&s, |n| n.locked), [true, false]);
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

/// #785: explicit malformed layer targets must never become the current-layer default.
#[test]
fn layer_delete_rejects_invalid_explicit_targets() {
    let mut unexpected = vec![];
    for params in [json!({"id": "1"}), json!({"ids": [1, "2"]}), json!({"id": "$1.id"})] {
        let mut s = session();
        run(&mut s, "layer.new", json!({}));
        run(&mut s, "layer.new", json!({}));
        let before = layer_target_snapshot(&s);
        let result = s.execute("layer.delete", &params);
        if result.is_ok() {
            let names: Vec<_> = s.doc().unwrap().doc.layers.iter().map(|l| l.display_name()).collect();
            unexpected.push(format!("{params}: {result:?}; remaining layers {names:?}"));
        } else {
            assert_eq!(layer_target_snapshot(&s), before, "{params}");
        }
    }
    assert!(unexpected.is_empty(), "malformed requests succeeded: {}", unexpected.join("; "));
}

fn layer_target_snapshot(s: &Session) -> Value {
    let st = s.doc().unwrap();
    let history =
        |entries: &[HistoryEntry]| entries.iter().map(|e| json!({"label": e.label, "doc": e.doc, "selection": e.selection})).collect::<Vec<_>>();
    json!({"doc": st.doc, "selection": st.selection, "revision": st.revision, "activeLayer": st.active_layer,
        "rows": st.layer_rows, "undo": history(&st.history.undo), "redo": history(&st.history.redo)})
}

fn layer_target_fixture() -> (Session, Vec<NodeId>) {
    let mut s = session();
    let first = s.doc().unwrap().doc.layers[0].id;
    rect(&mut s, 0.0);
    let second = id(&run(&mut s, "layer.new", json!({})));
    rect(&mut s, 40.0);
    let third = id(&run(&mut s, "layer.new", json!({})));
    rect(&mut s, 80.0);
    run(&mut s, "layer.highlight", json!({"ids": [first.0, second.0]}));
    (s, vec![first, second, third])
}

fn malformed_layer_targets(first: NodeId, second: NodeId) -> Vec<Value> {
    let mut params = vec![];
    for bad in [json!("1"), json!("$1.id"), json!(1.0), json!(1.5), json!(-1), Value::Null, json!(true), json!({}), json!([])] {
        params.push(json!({"id": bad}));
        params.push(json!({"ids": [first.0, bad]}));
    }
    for bad in [Value::Null, json!("1"), json!(1), json!(false), json!({})] {
        params.push(json!({"ids": bad, "id": first.0}));
    }
    for missing in [0, u64::MAX] {
        params.push(json!({"id": missing}));
        params.push(json!({"ids": [first.0, second.0, missing]}));
    }
    params
}

fn assert_layer_target_error(s: &mut Session, command: &str, params: Value) {
    let before = layer_target_snapshot(s);
    let error = s.execute(command, &params).unwrap_err();
    assert!(error.to_string().contains(command), "{command} {params}: {error}");
    assert_eq!(layer_target_snapshot(s), before, "{command} {params}: rejected targets must leave all state unchanged");
}

/// #785: shared row commands reject malformed or unknown targets before any partial edits.
#[test]
fn layer_target_validation_keeps_document_selection_revision_and_history_unchanged() {
    let (_, rows) = layer_target_fixture();
    for command in [
        "layer.delete",
        "layer.duplicate",
        "layer.setProps",
        "layer.template",
        "layer.merge",
        "layer.reverse",
        "layer.hideOthers",
        "layer.outlineOthers",
        "layer.lockOthers",
    ] {
        for mut params in malformed_layer_targets(rows[0], rows[1]) {
            let (mut s, _) = layer_target_fixture();
            if command == "layer.setProps" {
                params["name"] = json!("Changed");
            }
            assert_layer_target_error(&mut s, command, params);
        }
    }
}

/// These commands have their own selectors, not the shared row-default parser.
#[test]
fn layer_collect_and_flatten_validate_their_own_explicit_targets() {
    let (_, rows) = layer_target_fixture();
    for params in malformed_layer_targets(rows[0], rows[1]) {
        let command = if params.get("ids").is_some() { "layer.collectInNew" } else { "layer.flatten" };
        let (mut s, _) = layer_target_fixture();
        assert_layer_target_error(&mut s, command, params);
    }
}

/// Truncation limits work, not validation: a malformed or unknown tail cannot be ignored.
#[test]
fn layer_target_arrays_are_fully_validated_before_the_row_limit() {
    // The existing panel row limit is 100,000; exercise the public command beyond it.
    const ROW_LIMIT: usize = 100_000;
    for tail in [json!("$1.id"), json!(u64::MAX)] {
        let (mut s, rows) = layer_target_fixture();
        let mut values = vec![json!(rows[0].0); ROW_LIMIT + 1];
        *values.last_mut().unwrap() = tail;
        assert_layer_target_error(&mut s, "layer.delete", json!({"ids": values}));
    }
    let (mut s, rows) = layer_target_fixture();
    let mut values = vec![json!(rows[0].0); ROW_LIMIT + 1];
    *values.last_mut().unwrap() = json!(rows[1].0);
    run(&mut s, "layer.setProps", json!({"ids": values, "name": "Within limit"}));
    assert_eq!(node(&s, rows[0]).name.as_deref(), Some("Within limit"));
    assert_ne!(node(&s, rows[1]).name.as_deref(), Some("Within limit"), "the valid tail remains beyond the work cap");
}

#[test]
fn layer_target_defaults_and_valid_explicit_precedence_are_preserved() {
    let (mut s, rows) = layer_target_fixture();
    run(&mut s, "layer.setProps", json!({"name": "Highlighted"}));
    assert_eq!(node(&s, rows[0]).name.as_deref(), Some("Highlighted"));
    assert_eq!(node(&s, rows[1]).name.as_deref(), Some("Highlighted"));
    run(&mut s, "layer.setProps", json!({"id": rows[2].0, "name": "Single"}));
    assert_eq!(node(&s, rows[2]).name.as_deref(), Some("Single"));
    let before = undos(&s);
    run(&mut s, "layer.setProps", json!({"ids": [rows[1].0, rows[0].0, rows[1].0], "id": "$1.id", "name": "Explicit"}));
    assert_eq!(node(&s, rows[0]).name.as_deref(), Some("Explicit"));
    assert_eq!(node(&s, rows[1]).name.as_deref(), Some("Explicit"));
    assert_eq!(node(&s, rows[2]).name.as_deref(), Some("Single"));
    assert_eq!(undos(&s), before + 1, "duplicates remain one edit");
    assert_layer_target_error(&mut s, "layer.setProps", json!({"ids": [], "id": rows[2].0, "name": "Must not fall back"}));
    run(&mut s, "layer.setCurrent", json!({"id": rows[2].0}));
    run(&mut s, "layer.highlight", json!({"ids": []}));
    run(&mut s, "layer.setProps", json!({"name": "Current"}));
    assert_eq!(node(&s, rows[2]).name.as_deref(), Some("Current"));
    let before = undos(&s);
    run(&mut s, "layer.delete", json!({}));
    assert!(s.doc().unwrap().doc.node(rows[2]).is_none());
    assert_eq!(undos(&s), before + 1);
}

#[test]
fn layer_collect_and_flatten_keep_valid_and_omitted_target_behavior() {
    let (mut s, rows) = layer_target_fixture();
    let r = run(&mut s, "layer.flatten", json!({"id": rows[2].0}));
    assert_eq!(r["id"], json!(rows[2].0));
    assert_eq!(s.doc().unwrap().doc.layers.len(), 1);
    let (mut s, _) = layer_target_fixture();
    let selected = s.doc().unwrap().selection.objects.clone();
    s.doc_mut().unwrap().layer_rows.clear();
    let r = run(&mut s, "layer.collectInNew", json!({}));
    let collected = id(&r);
    assert!(selected.iter().all(|id| parent(&s, *id) == Some(collected)));
}
