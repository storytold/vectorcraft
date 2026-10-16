//! Commands that remove, insert or copy many objects, and an Alt-drag that copies them, each finish
//! within 20 s on one layer of 25,000 to 100,000 objects, and objects in two layers and inside a
//! group keep their order and ids.

use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use vectorcraft_doc::{Appearance, Document, LAYER_COLORS, LayerColor, Node, NodeId, NodeKind};
use vectorcraft_geom::{FillRule, Rect, shapes};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

/// Squares in a row of [`square_at`].
const ROW: usize = 200;

/// Square `k`: 10 pt wide, in rows of 200 squares 20 pt apart.
fn square_at(k: usize) -> Rect {
    let (x, y) = ((k % ROW) as f64 * 20.0, (k / ROW) as f64 * 20.0);
    Rect::new(x, y, x + 10.0, y + 10.0)
}

/// Square `k` ([`square_at`]) as a path with a new id.
pub(crate) fn square(d: &mut Document, k: usize) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(square_at(k)), Appearance::default_art())
}

/// A session whose document holds `make(d, k)` for each `k` below `n` on its one layer, all of it
/// selected.
pub(crate) fn session(n: usize, mut make: impl FnMut(&mut Document, usize) -> Node) -> Session {
    let mut d = Document::new(ROW as f64 * 20.0, (n / ROW + 1) as f64 * 20.0);
    let art: Vec<Arc<Node>> = (0..n).map(|k| Arc::new(make(&mut d, k))).collect();
    if let Some(ch) = Arc::make_mut(&mut d.layers[0]).children_mut() {
        *ch = art;
    }
    let mut s = Session::new();
    s.add_document(d, None);
    s.execute("select.all", &json!({})).unwrap();
    s
}

/// The ids of `n`'s children.
fn ids_of(n: &Node) -> Vec<NodeId> {
    n.children().into_iter().flatten().map(|c| c.id).collect()
}

/// The ids of the objects on layer `i`.
pub(crate) fn layer(s: &Session, i: usize) -> Vec<NodeId> {
    ids_of(&s.doc().unwrap().doc.layers[i])
}

/// The ids of the children of `id`.
fn children(s: &Session, id: NodeId) -> Vec<NodeId> {
    ids_of(s.doc().unwrap().doc.node(id).unwrap())
}

/// `n` ids counting up from `first`.
fn ids_from(first: u64, n: usize) -> Vec<NodeId> {
    (first..).take(n).map(NodeId).collect()
}

/// The ids of `a` and `b` in turn: each id of `a` followed by the id of `b` at the same place.
fn in_turn(a: &[NodeId], b: &[NodeId]) -> Vec<NodeId> {
    a.iter().zip(b).flat_map(|(a, b)| [*a, *b]).collect()
}

/// Runs `f` on `s` on a thread of its own and fails the test when it takes more than 20 s.
pub(crate) fn soon<T: Send + 'static>(mut s: Session, what: &'static str, f: impl FnOnce(&mut Session) -> T + Send + 'static) -> (Session, T) {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = f(&mut s);
        tx.send((s, r)).ok();
    });
    rx.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|_| panic!("{what} did not finish within 20 s"))
}

/// Runs command `cmd` with `params` through [`soon`] and returns its result.
pub(crate) fn run_soon(s: Session, cmd: &'static str, params: Value) -> (Session, Value) {
    let (s, r) = soon(s, cmd, move |s| s.execute(cmd, &params));
    (s, r.unwrap())
}

/// The ids in the `ids` array of a command's result.
fn ids_in(r: &Value) -> Vec<NodeId> {
    r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect()
}

/// Runs Undo and checks that it gives back `before`.
pub(crate) fn undo_gives_back(s: &mut Session, before: &Document) {
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(*s.doc().unwrap().doc == *before, "Undo gives back the document");
}

/// Whether the selected objects are `ids`, in that order.
fn selected(s: &Session, ids: &[NodeId]) -> bool {
    s.doc().unwrap().selection.objects == ids
}

/// The geometric bounds of object `id`.
fn bounds(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

#[test]
fn delete_of_100000_objects_finishes_within_20_s() {
    let s = session(100_000, square);
    let before = s.doc().unwrap().doc.clone();
    let (mut s, _) = run_soon(s, "edit.clear", json!({}));
    assert!(layer(&s, 0).is_empty());
    assert!(selected(&s, &[]));
    undo_gives_back(&mut s, &before);
}

#[test]
fn delete_selection_of_100000_rows_finishes_within_20_s() {
    let s = session(100_000, square);
    let (before, ids) = (s.doc().unwrap().doc.clone(), layer(&s, 0));
    let (mut s, r) = run_soon(s, "layer.delete", json!({"ids": ids}));
    assert_eq!(r["deleted"], json!(100_000));
    assert!(layer(&s, 0).is_empty());
    assert!(selected(&s, &[]));
    undo_gives_back(&mut s, &before);
}

#[test]
fn clean_up_of_100000_unpainted_paths_finishes_within_20_s() {
    let s = session(100_000, |d, k| Node::path(d.alloc_id(), shapes::rectangle(square_at(k)), Appearance::default()));
    let before = s.doc().unwrap().doc.clone();
    let (mut s, r) = run_soon(s, "object.path.cleanUp", json!({}));
    assert_eq!(r["removed"], json!(100_000));
    assert!(layer(&s, 0).is_empty());
    assert!(selected(&s, &[]));
    undo_gives_back(&mut s, &before);
}

#[test]
fn group_of_50000_objects_finishes_within_20_s() {
    let s = session(50_000, square);
    let (before, ids) = (s.doc().unwrap().doc.clone(), layer(&s, 0));
    let (mut s, r) = run_soon(s, "object.group", json!({}));
    let g = NodeId(r["id"].as_u64().unwrap());
    assert_eq!(layer(&s, 0), [g]);
    assert!(children(&s, g) == ids, "the group holds the objects in their order");
    assert!(selected(&s, &[g]));
    undo_gives_back(&mut s, &before);
}

#[test]
fn clipping_mask_of_50000_objects_finishes_within_20_s() {
    let s = session(50_000, square);
    let (before, ids) = (s.doc().unwrap().doc.clone(), layer(&s, 0));
    let (mut s, r) = run_soon(s, "object.clippingMask.make", json!({}));
    let g = NodeId(r["id"].as_u64().unwrap());
    assert_eq!(layer(&s, 0), [g]);
    // The topmost object comes first, as the clipping path, and the others follow in their order.
    let mut want = ids.clone();
    want.rotate_right(1);
    assert!(children(&s, g) == want, "the clip group holds the objects in their order");
    let doc = &s.doc().unwrap().doc;
    assert!(matches!(doc.node(g).unwrap().kind, NodeKind::Group { clip: true, .. }));
    assert!(matches!(doc.node(want[0]).unwrap().kind, NodeKind::Path { clipping: true, .. }));
    assert!(selected(&s, &[g]));
    undo_gives_back(&mut s, &before);
}

#[test]
fn duplicate_of_50000_objects_finishes_within_20_s() {
    let s = session(50_000, square);
    let (before, ids, first) = (s.doc().unwrap().doc.clone(), layer(&s, 0), s.doc().unwrap().doc.peek_next_id());
    let (mut s, r) = run_soon(s, "edit.duplicate", json!({"dx": 5}));
    // Each copy goes directly above its object and takes the next id, in the objects' order.
    let copies = ids_from(first, ids.len());
    assert!(ids_in(&r) == copies);
    assert!(layer(&s, 0) == in_turn(&ids, &copies), "each copy directly above its object");
    assert!(selected(&s, &copies));
    assert_eq!(bounds(&s, copies[0]), square_at(0) + vectorcraft_geom::Vec2::new(5.0, 0.0));
    undo_gives_back(&mut s, &before);
}

#[test]
fn transform_with_copy_of_50000_objects_finishes_within_20_s() {
    let s = session(50_000, square);
    let (before, ids, first) = (s.doc().unwrap().doc.clone(), layer(&s, 0), s.doc().unwrap().doc.peek_next_id());
    let (mut s, r) = run_soon(s, "object.transform", json!({"matrix": [1, 0, 0, 1, 5, 0], "copy": true}));
    let copies = ids_from(first, ids.len());
    assert!(ids_in(&r) == copies);
    assert!(layer(&s, 0) == in_turn(&ids, &copies), "each copy directly above its object");
    assert!(selected(&s, &copies));
    let last = ids.len() - 1;
    assert_eq!(bounds(&s, copies[last]), square_at(last) + vectorcraft_geom::Vec2::new(5.0, 0.0));
    assert_eq!(bounds(&s, ids[last]), square_at(last));
    undo_gives_back(&mut s, &before);
}

#[test]
fn transform_each_with_copy_of_50000_objects_finishes_within_20_s() {
    let s = session(50_000, square);
    let (before, ids, first) = (s.doc().unwrap().doc.clone(), layer(&s, 0), s.doc().unwrap().doc.peek_next_id());
    let (mut s, r) = run_soon(s, "object.transformEach", json!({"moveH": 5, "copy": true}));
    let copies = ids_from(first, ids.len());
    assert!(ids_in(&r) == copies);
    assert!(layer(&s, 0) == in_turn(&ids, &copies), "each copy directly above its object");
    assert!(selected(&s, &copies));
    assert_eq!(bounds(&s, copies[1]), square_at(1) + vectorcraft_geom::Vec2::new(5.0, 0.0));
    undo_gives_back(&mut s, &before);
}

#[test]
fn offset_path_of_50000_objects_finishes_within_20_s() {
    let s = session(50_000, square);
    let (before, ids, first) = (s.doc().unwrap().doc.clone(), layer(&s, 0), s.doc().unwrap().doc.peek_next_id());
    let (mut s, r) = run_soon(s, "object.path.offsetPath", json!({"offset": 1}));
    // Each offset path goes directly above its square and takes the next id, in the squares' order.
    let offsets = ids_from(first, ids.len());
    assert!(ids_in(&r) == offsets);
    assert!(layer(&s, 0) == in_turn(&ids, &offsets), "each offset path directly above its square");
    assert!(selected(&s, &offsets));
    let grown = bounds(&s, offsets[1]);
    assert!((grown.width() - 12.0).abs() < 1e-6 && (grown.x0 - 19.0).abs() < 1e-6, "{grown:?}");
    undo_gives_back(&mut s, &before);
}

#[test]
fn compound_path_of_100000_objects_finishes_within_20_s() {
    let s = session(100_000, square);
    let (before, ids) = (s.doc().unwrap().doc.clone(), layer(&s, 0));
    let (mut s, r) = run_soon(s, "object.compoundPath.make", json!({}));
    let c = NodeId(r["id"].as_u64().unwrap());
    assert_eq!(layer(&s, 0), [c]);
    assert!(children(&s, c) == ids, "the compound path holds the paths in their order");
    assert!(selected(&s, &[c]));
    undo_gives_back(&mut s, &before);
}

#[test]
fn send_to_current_layer_of_100000_objects_finishes_within_20_s() {
    let mut s = session(100_000, square);
    let ids = layer(&s, 0);
    let to = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.all", &json!({})).unwrap();
    s.execute("layer.setCurrent", &json!({"id": to.0})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let (mut s, _) = run_soon(s, "object.arrange.sendToCurrentLayer", json!({}));
    assert!(layer(&s, 0).is_empty());
    assert!(children(&s, to) == ids, "the layer holds the objects in their order");
    assert!(selected(&s, &ids));
    undo_gives_back(&mut s, &before);
}

#[test]
fn row_drag_of_100000_rows_finishes_within_20_s() {
    let mut s = session(100_000, square);
    let ids = layer(&s, 0);
    let to = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let before = s.doc().unwrap().doc.clone();
    let (mut s, r) = run_soon(s, "layer.move", json!({"ids": ids, "target": to}));
    assert!(ids_in(&r) == ids);
    assert!(layer(&s, 0).is_empty());
    assert!(children(&s, to) == ids, "the layer holds the rows in their order");
    assert!(selected(&s, &ids));
    undo_gives_back(&mut s, &before);
}

#[test]
fn merge_selected_of_100000_objects_finishes_within_20_s() {
    let mut s = session(100_000, square);
    let (from, ids) = (s.doc().unwrap().doc.layers[0].id, layer(&s, 0));
    let into = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let before = s.doc().unwrap().doc.clone();
    let (mut s, r) = run_soon(s, "layer.merge", json!({"ids": [from, into]}));
    assert_eq!(r["id"], json!(into));
    assert!(s.doc().unwrap().doc.node(from).is_none(), "the emptied layer goes");
    assert!(children(&s, into) == ids, "the layer holds the objects in their order");
    assert!(selected(&s, &ids));
    undo_gives_back(&mut s, &before);
}

#[test]
fn ungroup_of_25000_groups_finishes_within_20_s() {
    // Each group holds two squares, and ungrouping one adds an object to the layer.
    let s = session(25_000, |d, k| {
        let pair = [square(d, 2 * k), square(d, 2 * k + 1)].map(Arc::new);
        Node::group(d.alloc_id(), pair.to_vec())
    });
    let before = s.doc().unwrap().doc.clone();
    let squares: Vec<NodeId> = layer(&s, 0).into_iter().flat_map(|g| children(&s, g)).collect();
    let (mut s, _) = run_soon(s, "object.ungroup", json!({}));
    assert!(layer(&s, 0) == squares, "each group's objects where the group was");
    assert!(selected(&s, &squares));
    undo_gives_back(&mut s, &before);
}

#[test]
fn compound_path_release_of_25000_compound_paths_finishes_within_20_s() {
    let s = session(25_000, |d, k| {
        let pair = [square(d, 2 * k), square(d, 2 * k + 1)].map(Arc::new);
        Node::new(d.alloc_id(), NodeKind::Compound { children: pair.to_vec(), rule: FillRule::NonZero })
    });
    let before = s.doc().unwrap().doc.clone();
    let paths: Vec<NodeId> = layer(&s, 0).into_iter().flat_map(|c| children(&s, c)).collect();
    let (mut s, _) = run_soon(s, "object.compoundPath.release", json!({}));
    assert!(layer(&s, 0) == paths, "each compound path's paths where it was");
    assert!(selected(&s, &paths));
    undo_gives_back(&mut s, &before);
}

#[test]
fn release_to_layers_of_100000_objects_finishes_within_20_s() {
    let s = session(100_000, square);
    let doc = &s.doc().unwrap().doc;
    let (before, l, ids, first) = (doc.clone(), doc.layers[0].id, layer(&s, 0), doc.peek_next_id());
    let (mut s, r) = run_soon(s, "layer.releaseToLayers", json!({"id": l}));
    // Each object goes into a sublayer of its own, in their order. The sublayers take the next ids,
    // and with Layer 1 counted, sublayer k is named `Layer k + 2` and takes layer color k + 1,
    // wrapping around after the last preset.
    let subs = ids_from(first, ids.len());
    assert!(ids_in(&r) == subs);
    assert!(layer(&s, 0) == subs);
    let doc = &s.doc().unwrap().doc;
    for (k, (sub, id)) in subs.iter().zip(&ids).enumerate() {
        let n = doc.node(*sub).unwrap();
        assert_eq!(ids_of(n), [*id]);
        assert_eq!(n.name.as_deref(), Some(format!("Layer {}", k + 2).as_str()));
        assert!(matches!(n.kind, NodeKind::Layer { color: LayerColor::Preset(c), .. } if usize::from(c) == (k + 1) % LAYER_COLORS.len()));
    }
    assert!(selected(&s, &ids));
    undo_gives_back(&mut s, &before);
}

#[test]
fn outline_stroke_of_25000_objects_finishes_within_20_s() {
    let s = session(25_000, square);
    let (before, first) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().doc.peek_next_id());
    let (mut s, r) = run_soon(s, "object.path.outlineStroke", json!({}));
    // Each square, filled and stroked, becomes a group of its fill and the outline of its stroke,
    // where the square was, and the groups take new ids in the squares' order.
    let groups = ids_in(&r);
    assert_eq!(groups.len(), 25_000);
    assert!(layer(&s, 0) == groups, "each group where its square was");
    assert!(groups[0].0 >= first && groups.windows(2).all(|w| w[0] < w[1]));
    assert!(groups.iter().all(|g| children(&s, *g).len() == 2));
    assert!(selected(&s, &groups));
    undo_gives_back(&mut s, &before);
}

/// With Alt held, a Selection tool press on one of 30,000 selected squares, four moves of 20 pt
/// and the release copy the squares once, as one undo step.
#[test]
fn alt_drag_of_30000_objects_finishes_within_20_s() {
    let s = session(30_000, square);
    let (before, ids, first) = (s.doc().unwrap().doc.clone(), layer(&s, 0), s.doc().unwrap().doc.peek_next_id());
    // The press lands on a square in the middle of the art, away from the handles of the
    // selection's box.
    let grab = square_at(15_100).center();
    let (mut s, r) = soon(s, "the Alt-drag", move |s| {
        let (v, alt) = (ViewInfo::default(), Mods { alt: true, ..Mods::default() });
        s.select_tool("selection", v)?;
        let at = |kind, dx: f64| PointerEvent::new(kind, grab.x + dx, grab.y).with_mods(alt);
        s.pointer(&at(PointerKind::Down, 0.0), v)?;
        for dx in [20.0, 40.0, 60.0, 80.0] {
            s.pointer(&at(PointerKind::Drag, dx), v)?;
        }
        s.pointer(&at(PointerKind::Up, 80.0), v)
    });
    r.unwrap();
    let copies = ids_from(first, ids.len());
    assert!(layer(&s, 0) == in_turn(&ids, &copies), "each copy directly above its square");
    assert!(selected(&s, &copies));
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Copy"));
    // The squares stay; their copies move together.
    let moved = bounds(&s, copies[0]).origin() - square_at(0).origin();
    assert!(moved.x > 0.0, "{moved:?}");
    for (k, (id, copy)) in ids.iter().zip(&copies).enumerate() {
        assert_eq!(bounds(&s, *id), square_at(k));
        assert_eq!(bounds(&s, *copy), square_at(k) + moved);
    }
    undo_gives_back(&mut s, &before);
}

/// The ids under `n`: a child that can hold children is a list of its id and theirs.
fn outline(n: &Node) -> Value {
    let ids = n.children().into_iter().flatten().map(|c| match c.children() {
        Some(_) => {
            let mut v = vec![json!(c.id.0)];
            v.extend(outline(c).as_array().cloned().unwrap_or_default());
            Value::Array(v)
        }
        None => json!(c.id.0),
    });
    Value::Array(ids.collect())
}

/// [`outline`] of each layer.
fn layers(s: &Session) -> Vec<Value> {
    s.doc().unwrap().doc.layers.iter().map(|l| outline(l)).collect()
}

/// A session whose first layer holds a, b, c and a group of d, e and f, and whose second layer
/// holds g and h, with a, c, e, g and h selected. Returns the session and the ids of a to h and of
/// the group.
fn two_layers() -> (Session, [u64; 9]) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let rect = |s: &mut Session| {
        let x = 20.0 * s.doc().unwrap().doc.node_count() as f64;
        s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap()
    };
    let [a, b, c, d, e, f] = [(); 6].map(|()| rect(&mut s));
    s.execute("select.set", &json!({"ids": [d, e, f]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("layer.new", &json!({})).unwrap();
    let [g, h] = [(); 2].map(|()| rect(&mut s));
    s.execute("select.set", &json!({"ids": [a, c, e, g, h]})).unwrap();
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f]]), json!([g, h])]);
    (s, [a, b, c, d, e, f, g, h, group])
}

/// After Group, Delete and Duplicate of objects in two layers and inside a group, the layers hold
/// the objects and the new ones in the order, and with the ids, that each check lists, and Undo
/// gives back the document.
#[test]
fn group_delete_and_duplicate_across_layers_and_inside_a_group_keep_order_and_ids() {
    let (mut s, [a, b, c, d, e, f, g, h, group]) = two_layers();
    let before = s.doc().unwrap().doc.clone();
    let next = before.peek_next_id();

    let r = s.execute("object.group", &json!({})).unwrap();
    assert_eq!(r["id"], json!(next));
    assert_eq!(layers(&s), [json!([b, [group, d, f]]), json!([[next, a, c, e, g, h]])]);
    assert!(selected(&s, &[NodeId(next)]));
    undo_gives_back(&mut s, &before);

    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(layers(&s), [json!([b, [group, d, f]]), json!([])]);
    undo_gives_back(&mut s, &before);

    let r = s.execute("edit.duplicate", &json!({})).unwrap();
    let [a2, c2, e2, g2, h2] = [0, 1, 2, 3, 4].map(|k| next + k);
    assert_eq!(r["ids"], json!([a2, c2, e2, g2, h2]));
    assert_eq!(layers(&s), [json!([a, a2, b, c, c2, [group, d, e, e2, f]]), json!([g, g2, h, h2])]);
    assert!(selected(&s, &ids_from(next, 5)));
    undo_gives_back(&mut s, &before);
}

/// After the Layers panel's Delete Selection, Duplicate Selection, row drags, Collect in New Layer
/// and Merge Selected on rows in two layers and inside a group, the layers hold the rows and the
/// new ones in the order, and with the ids, that each check lists, and Undo gives back the
/// document.
#[test]
fn layers_panel_rows_across_layers_and_inside_a_group_keep_order_and_ids() {
    let (mut s, [a, b, c, d, e, f, g, h, group]) = two_layers();
    let before = s.doc().unwrap().doc.clone();
    let (n, l1, l2) = (before.peek_next_id(), before.layers[0].id.0, before.layers[1].id.0);

    let r = s.execute("layer.delete", &json!({"ids": [a, e, h]})).unwrap();
    assert_eq!(r["deleted"], 3);
    assert_eq!(layers(&s), [json!([b, c, [group, d, f]]), json!([g])]);
    assert!(selected(&s, &[NodeId(c), NodeId(g)]));
    undo_gives_back(&mut s, &before);

    // Each copy goes directly above its row and takes the next id; a layer's copy is renamed.
    let r = s.execute("layer.duplicate", &json!({"ids": [a, e, h]})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 1, n + 2]));
    assert_eq!(layers(&s), [json!([a, n, b, c, [group, d, e, n + 1, f]]), json!([g, h, n + 2])]);
    undo_gives_back(&mut s, &before);
    let r = s.execute("layer.duplicate", &json!({"ids": [l2]})).unwrap();
    assert_eq!(r["ids"], json!([n]));
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f]]), json!([g, h]), json!([n + 1, n + 2])]);
    assert_eq!(s.doc().unwrap().doc.layers[2].name.as_deref(), Some("Layer 2 copy"));
    undo_gives_back(&mut s, &before);

    // Rows are dragged below c, then above d in the group, and copies of rows are dragged above g.
    let r = s.execute("layer.move", &json!({"ids": [a, e, h], "target": c, "place": "below"})).unwrap();
    assert_eq!(r["ids"], json!([a, e, h]));
    assert_eq!(layers(&s), [json!([b, a, e, h, c, [group, d, f]]), json!([g])]);
    undo_gives_back(&mut s, &before);
    s.execute("layer.move", &json!({"ids": [a, g], "target": d, "place": "above"})).unwrap();
    assert_eq!(layers(&s), [json!([b, c, [group, d, a, g, e, f]]), json!([h])]);
    undo_gives_back(&mut s, &before);
    let r = s.execute("layer.move", &json!({"ids": [a, e], "target": g, "place": "above", "copy": true})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 1]));
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f]]), json!([g, n, n + 1, h])]);
    undo_gives_back(&mut s, &before);

    // The new layer goes where the topmost row was when that row is in a layer, and on top of the
    // layer holding the topmost row's group when it is in a group.
    assert_eq!(s.execute("layer.collectInNew", &json!({"ids": [a, e, g]})).unwrap()["id"], json!(n));
    assert_eq!(layers(&s), [json!([b, c, [group, d, f]]), json!([[n, a, e, g], h])]);
    assert_eq!(s.doc().unwrap().doc.node(NodeId(n)).unwrap().name.as_deref(), Some("Layer 3"));
    undo_gives_back(&mut s, &before);
    assert_eq!(s.execute("layer.collectInNew", &json!({"ids": [a, e]})).unwrap()["id"], json!(n));
    assert_eq!(layers(&s), [json!([b, c, [group, d, f], [n, a, e]]), json!([g, h])]);
    undo_gives_back(&mut s, &before);

    // Rows merge into the layer listed last, in the stacking order, and so does a merged layer's
    // art.
    s.execute("layer.merge", &json!({"ids": [e, g, l1]})).unwrap();
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, f], e, g]), json!([h])]);
    undo_gives_back(&mut s, &before);
    s.execute("layer.merge", &json!({"ids": [l1, l2]})).unwrap();
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f], g, h])]);
    undo_gives_back(&mut s, &before);
}

/// Release to Layers (Sequence on a layer and on a group, Build on a layer) and Flatten Artwork
/// leave the layers, sublayers and objects in the order, and with the ids and names, that each
/// check lists, and Undo gives back the document.
#[test]
fn release_to_layers_and_flatten_artwork_keep_order_and_ids() {
    let (mut s, [a, b, c, d, e, f, g, h, group]) = two_layers();
    let l2 = s.doc().unwrap().doc.layers[1].id.0;
    // Layer 2 also holds a sublayer, on top of g and h.
    let sub = s.execute("layer.newSublayer", &json!({"parent": l2})).unwrap()["id"].as_u64().unwrap();
    let before = s.doc().unwrap().doc.clone();
    let (n, l1) = (before.peek_next_id(), before.layers[0].id.0);
    let names = |s: &Session, r: &Value| ids_in(r).iter().map(|id| s.doc().unwrap().doc.node(*id).unwrap().display_name()).collect::<Vec<_>>();

    // A layer's objects go into sublayers of their own, below its sublayers.
    let r = s.execute("layer.releaseToLayers", &json!({"id": l2})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 1]));
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f]]), json!([[n, g], [n + 1, h], [sub]])]);
    assert_eq!(names(&s, &r), ["Layer 4", "Layer 5"]);
    undo_gives_back(&mut s, &before);

    // Build: a sublayer holds copies of the objects below its own and a copy of its own, and the
    // top one holds the objects.
    let r = s.execute("layer.releaseToLayersBuild", &json!({"id": l2})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 2]));
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f]]), json!([[n, n + 1], [n + 2, g, h], [sub]])]);
    assert_eq!(names(&s, &r), ["Layer 4", "Layer 5"]);
    undo_gives_back(&mut s, &before);

    // A group's objects go into sublayers of its layer, where the group was.
    let r = s.execute("layer.releaseToLayers", &json!({"id": group})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 1, n + 2]));
    assert_eq!(layers(&s), [json!([a, b, c, [n, d], [n + 1, e], [n + 2, f]]), json!([g, h, [sub]])]);
    assert_eq!(names(&s, &r), ["Layer 4", "Layer 5", "Layer 6"]);
    undo_gives_back(&mut s, &before);

    // Flatten Artwork into Layer 1 deletes the two hidden layers and keeps the template layer.
    for p in [json!({"top": true, "visible": false}), json!({"top": true, "visible": false}), json!({"top": true, "template": true})] {
        s.execute("layer.new", &p).unwrap();
    }
    let before = s.doc().unwrap().doc.clone();
    assert_eq!(s.execute("layer.flatten", &json!({"id": l1})).unwrap()["discarded"], 2);
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f], g, h, [sub]]), json!([])]);
    undo_gives_back(&mut s, &before);
}

/// Duplicate Artboard and `artboard.move` with `copy` and `moveArt` (the Artboard tool's Alt-drag
/// with Move/Copy Artwork with Artboard on) put a copy of each object on the artboard directly
/// above it, moved with the artboard's copy, and Cut Artboard removes those objects. An object off
/// the artboard stays. The layers then hold the objects in the order, and with the ids, that each
/// check lists, and Undo gives back the document.
#[test]
fn artboard_copies_and_cut_keep_order_and_ids() {
    let (mut s, [a, b, c, d, e, f, g, h, group]) = two_layers();
    let off = s.execute("shape.rectangle", &json!({"x": 900, "y": 10, "width": 10, "height": 10})).unwrap()["id"].as_u64().unwrap();
    s.execute("artboard.new", &json!({})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let n = before.peek_next_id();
    let copies = json!([n, n + 1, n + 2, n + 3, n + 7, n + 8]);
    let copied = [json!([a, n, b, n + 1, c, n + 2, [group, d, e, f], [n + 3, n + 4, n + 5, n + 6]]), json!([g, n + 7, h, n + 8, off])];
    // The copy of h, the last object on the artboard, sits where h sits, moved by `dx`.
    let moved_by = |s: &Session, dx: f64| bounds(s, NodeId(n + 8)) == bounds(s, NodeId(h)) + vectorcraft_geom::Vec2::new(dx, 0.0);

    // Duplicate Artboard places the copy 20 pt right of the last artboard.
    let right = before.artboards.iter().map(|ab| ab.rect.x1).fold(f64::MIN, f64::max);
    assert_eq!(s.execute("artboard.duplicate", &json!({"index": 0, "art": true})).unwrap()["ids"], copies);
    assert_eq!(layers(&s), copied);
    assert!(moved_by(&s, right + 20.0));
    undo_gives_back(&mut s, &before);

    let r = s.execute("artboard.move", &json!({"index": 0, "dx": 900, "dy": 0, "copy": true, "moveArt": true})).unwrap();
    assert_eq!(r["moved"], copies);
    assert_eq!(layers(&s), copied);
    assert!(moved_by(&s, 900.0));
    undo_gives_back(&mut s, &before);

    assert_eq!(s.execute("artboard.cut", &json!({"index": 0, "art": true})).unwrap()["copied"], 6);
    assert_eq!(layers(&s), [json!([]), json!([off])]);
    undo_gives_back(&mut s, &before);
}

/// After Offset Path, Outline Stroke, Unite and Clean Up of objects in two layers and inside a
/// group, the layers hold the objects and the new ones in the order, and with the ids, that each
/// check lists, and Undo gives back the document.
#[test]
fn path_commands_across_layers_and_inside_a_group_keep_order_and_ids() {
    let (mut s, [a, b, c, d, e, f, g, h, group]) = two_layers();
    let before = s.doc().unwrap().doc.clone();
    let n = before.peek_next_id();

    let r = s.execute("object.path.offsetPath", &json!({"offset": 1})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 1, n + 2, n + 3, n + 4]));
    assert_eq!(layers(&s), [json!([a, n, b, c, n + 1, [group, d, e, n + 2, f]]), json!([g, n + 3, h, n + 4])]);
    assert!(selected(&s, &ids_from(n, 5)));
    undo_gives_back(&mut s, &before);

    // Each object becomes a group of its fill and the outline of its stroke, where it was.
    let r = s.execute("object.path.outlineStroke", &json!({})).unwrap();
    let [a2, c2, e2, g2, h2] = <[NodeId; 5]>::try_from(ids_in(&r)).unwrap();
    assert!(n <= a2.0 && a2 < c2 && c2 < e2 && e2 < g2 && g2 < h2);
    assert_eq!(layer(&s, 0), [a2, NodeId(b), c2, NodeId(group)]);
    assert_eq!(children(&s, NodeId(group)), [NodeId(d), e2, NodeId(f)]);
    assert_eq!(layer(&s, 1), [g2, h2]);
    assert!(selected(&s, &[a2, c2, e2, g2, h2]));
    undo_gives_back(&mut s, &before);

    // The result goes where the topmost object, h, was.
    let r = s.execute("object.pathfinder.unite", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([n]));
    assert_eq!(layers(&s)[0], json!([b, [group, d, f]]));
    assert_eq!(layer(&s, 1), [NodeId(n)]);
    assert!(selected(&s, &[NodeId(n)]));
    undo_gives_back(&mut s, &before);

    s.edit("Paint nothing", |d, _| {
        for id in [a, e, h] {
            d.node_mut(NodeId(id)).unwrap().appearance = Appearance::default().into();
        }
        Ok(())
    })
    .unwrap();
    let before = s.doc().unwrap().doc.clone();
    let r = s.execute("object.path.cleanUp", &json!({})).unwrap();
    assert_eq!(r["removed"], json!(3));
    assert_eq!(layers(&s), [json!([b, c, [group, d, f]]), json!([g])]);
    undo_gives_back(&mut s, &before);
}

/// `object.transform` copies ids listed twice, and copies an object listed before a group around it
/// only with the group, as it does with the two selected. Each copy goes directly above its
/// original, and a later copy of the same object goes below an earlier one.
#[test]
fn copies_of_ids_listed_twice_or_inside_another_listed_id_keep_their_order_and_ids() {
    let (mut s, [a, b, c, d, e, f, g, h, group]) = two_layers();
    let before = s.doc().unwrap().doc.clone();
    let n = before.peek_next_id();

    let r = s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 5, 0], "copy": true, "ids": [e, group]})).unwrap();
    assert_eq!(r["ids"], json!([n]));
    let copied = json!([n, n + 1, n + 2, n + 3]);
    assert_eq!(layers(&s), [json!([a, b, c, [group, d, e, f], copied]), json!([g, h])]);
    undo_gives_back(&mut s, &before);

    let r = s.execute("object.transform", &json!({"matrix": [1, 0, 0, 1, 5, 0], "copy": true, "ids": [a, a]})).unwrap();
    assert_eq!(r["ids"], json!([n, n + 1]));
    assert_eq!(layers(&s), [json!([a, n + 1, n, b, c, [group, d, e, f]]), json!([g, h])]);
    undo_gives_back(&mut s, &before);
}
