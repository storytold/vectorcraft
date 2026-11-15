use std::sync::Arc;

use super::*;
use crate::Node;

#[test]
fn basic_ops() {
    let mut s = Selection::default();
    s.add(NodeId(1));
    s.add(NodeId(1));
    s.toggle(NodeId(2));
    assert_eq!(s.objects, vec![NodeId(1), NodeId(2)]);
    s.toggle(NodeId(1));
    assert_eq!(s.objects, vec![NodeId(2)]);
    s.clear();
    assert!(s.is_empty());
}

#[test]
fn remove_all_does_what_removing_each_does() {
    let mut s = Selection::default();
    for i in 1..=6 {
        s.add(NodeId(i));
    }
    for i in [2, 4, 5] {
        s.anchors.insert(NodeId(i), BTreeSet::from([(0, 1)]));
        s.segments.insert(NodeId(i), BTreeSet::from([(0, 0)]));
    }
    s.key = Some(NodeId(4));
    s.target = Some(NodeId(3));
    s.slices = vec![NodeId(9)];
    s.guides = vec![NodeId(10)];
    s.on_path_member = Some(NodeId(11));
    let mut one_by_one = s.clone();
    for id in [NodeId(4), NodeId(1), NodeId(4), NodeId(7)] {
        one_by_one.remove(id);
    }
    let mut all = s.clone();
    all.remove_all([NodeId(4), NodeId(1), NodeId(4), NodeId(7)]);
    assert_eq!(all, one_by_one);
    assert_eq!(all.objects, [NodeId(2), NodeId(3), NodeId(5), NodeId(6)]);
    // Removing nothing leaves the selection as it was.
    let mut none = s.clone();
    none.remove_all([]);
    assert_eq!(none, s);
}

#[test]
fn guides_are_selected_on_their_own() {
    let mut s = Selection::default();
    s.add(NodeId(1));
    let (g0, g2) = (NodeId(10), NodeId(12));
    s.set_guides([g2, g0, g2]);
    assert_eq!((s.objects.clone(), s.guides.clone()), (vec![], vec![g2, g0]));
    assert!(s.has_objects_or_guides() && s.is_empty());
    // Shift-adding an object keeps them; a new selection doesn't.
    s.add(NodeId(1));
    assert_eq!(s.guides, vec![g2, g0]);
    s.set([NodeId(1)]);
    assert!(s.guides.is_empty());
    // Toggled, a guide joins (or leaves) the selected objects.
    s.toggle_guide(g0);
    assert_eq!((s.objects.clone(), s.guides.clone()), (vec![NodeId(1)], vec![g0]));
    s.toggle_guide(g0);
    assert_eq!((s.objects.clone(), s.guides.clone()), (vec![NodeId(1)], vec![]));
    // Pruning drops guides the document no longer has, ids of other objects, and guides that are
    // locked or hidden themselves.
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let kept = d.add_ruler_guide(&crate::Guide::new(true, 10.0));
    let locked = d.add_ruler_guide(&crate::Guide { locked: true, ..crate::Guide::new(true, 20.0) });
    let hidden = d.add_ruler_guide(&crate::Guide { hidden: true, ..crate::Guide::new(true, 30.0) });
    s.set_guides([kept, NodeId(9999), layer, locked, hidden]);
    s.prune(&d);
    assert_eq!(s.guides, vec![kept]);
}

/// A picked object of a dynamic symbol's instance stays picked while it and the groups and layers
/// around it in the symbol's art are visible, unlocked and not a template. An art edit that hides
/// or locks it, or one around it, or makes a layer around it a template, drops the pick.
#[test]
fn a_picked_symbol_part_is_dropped_once_it_cannot_be_edited() {
    let rect = || vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let leaf = Node::path(d.alloc_id(), rect(), crate::Appearance::default_art());
    let group = Node::group(d.alloc_id(), vec![Arc::new(leaf)]);
    let layer = Node::layer(d.alloc_id(), "Inner", crate::LayerColor::Preset(2));
    let root = Node::group(d.alloc_id(), vec![Arc::new(group), Arc::new(layer)]);
    d.symbols.push(crate::Symbol { dynamic: true, name: "Parts".into(), art: Arc::new(root.clone()) });
    let inst = d.alloc_id();
    d.insert(Some(l), 0, Node::new(inst, NodeKind::SymbolInstance { symbol: "Parts".into(), xf: vectorcraft_geom::Affine::IDENTITY })).unwrap();
    // The layer in the art holds a path too.
    let in_layer = Node::path(d.alloc_id(), rect(), crate::Appearance::default_art());
    let mut art = root;
    if let Some(NodeKind::Layer { children, .. }) = art.children_mut().and_then(|c| c.get_mut(1)).map(|c| &mut Arc::make_mut(c).kind) {
        children.push(Arc::new(in_layer));
    }
    d.symbols[0].art = Arc::new(art.clone());
    let picked = |d: &Document, path: &[u32]| {
        let mut s = Selection::default();
        s.set_symbol_part(inst, path.to_vec());
        s.prune(d);
        s.symbol_part.is_some()
    };
    assert!(picked(&d, &[0, 0]) && picked(&d, &[1, 0]), "editable parts stay picked");
    assert!(!picked(&d, &[0]) && !picked(&d, &[1]) && !picked(&d, &[]), "a group or layer does not stay picked");
    let edited = |change: &dyn Fn(&mut Node)| {
        let mut d = d.clone();
        let mut art = art.clone();
        change(&mut art);
        d.symbols[0].art = Arc::new(art);
        d
    };
    let child = |art: &mut Node, path: &[usize]| -> Node {
        let mut n = &*art;
        for &i in path {
            n = &n.children().unwrap()[i];
        }
        n.clone()
    };
    let set = |art: &mut Node, path: &[usize], n: Node| {
        let mut at = art;
        for &i in path {
            at = Arc::make_mut(&mut at.children_mut().unwrap()[i]);
        }
        *at = n;
    };
    let hidden = edited(&|art| {
        let mut n = child(art, &[0, 0]);
        n.visible = false;
        set(art, &[0, 0], n);
    });
    assert!(!picked(&hidden, &[0, 0]), "the part is hidden");
    let locked_group = edited(&|art| {
        let mut n = child(art, &[0]);
        n.locked = true;
        set(art, &[0], n);
    });
    assert!(!picked(&locked_group, &[0, 0]), "the group around it is locked");
    assert!(picked(&locked_group, &[1, 0]), "a part outside the locked group stays");
    let template = edited(&|art| {
        let mut n = child(art, &[1]);
        if let NodeKind::Layer { template, .. } = &mut n.kind {
            *template = true;
        }
        set(art, &[1], n);
    });
    assert!(!picked(&template, &[1, 0]), "the layer around it is a template");
    let hidden_root = edited(&|art| art.visible = false);
    assert!(!picked(&hidden_root, &[0, 0]), "the art is hidden");
}

#[test]
fn guides_show_and_lock_with_their_layer() {
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let g = crate::Guide { layer: Some(layer), ..crate::Guide::new(true, 10.0) };
    assert!(d.guide_shown(&g) && d.guide_editable(&g));
    d.node_mut(layer).unwrap().locked = true;
    assert!(d.guide_shown(&g) && !d.guide_editable(&g), "locked: shown, not editable");
    d.node_mut(layer).unwrap().visible = false;
    assert!(!d.guide_shown(&g) && !d.guide_editable(&g), "hidden");
    // On no layer, or a layer that's gone: always there.
    assert!(d.guide_shown(&crate::Guide::new(false, 5.0)));
    let gone = crate::Guide { layer: Some(NodeId(9999)), ..crate::Guide::new(true, 1.0) };
    assert!(d.guide_shown(&gone) && d.guide_editable(&gone));
    // As a node, a guide shows and locks with its layer and on its own.
    let id = d.add_ruler_guide(&g);
    let node = d.ruler_guide(id).unwrap();
    assert_eq!(d.guides_on(layer), std::slice::from_ref(&node));
    assert!(!d.guide_shown(&node) && !d.guide_editable(&node), "its layer is hidden");
    d.node_mut(layer).unwrap().visible = true;
    d.node_mut(layer).unwrap().locked = false;
    assert!(d.guide_shown(&node) && d.guide_editable(&node));
    d.node_mut(id).unwrap().locked = true;
    let node = d.ruler_guide(id).unwrap();
    assert!(d.guide_shown(&node) && !d.guide_editable(&node), "locked itself");
    d.node_mut(id).unwrap().visible = false;
    assert!(!d.guide_shown(&node), "hidden itself");
    // Saved guides on no layer, or a gone one, go on the top visible, unlocked layer.
    d.saved_guides.push(crate::Guide::new(false, 5.0));
    d.saved_guides.push(gone);
    assert_eq!(d.guides_into_nodes(), 2);
    assert!(d.ruler_guides().iter().all(|g| g.layer == Some(layer)));
    assert_eq!(d.ruler_guides().len(), 3);
    assert_eq!(d.guides_into_nodes(), 0, "once");
    // A guide saved before guides had layers reads as one on no layer.
    let old: crate::Guide = serde_json::from_str(r#"{"vertical": true, "pos": 5.0}"#).unwrap();
    assert_eq!(old.layer, None);
    assert!(!serde_json::to_string(&crate::Guide::new(true, 5.0)).unwrap().contains("layer"), "nothing new written");
}

#[test]
fn selection_changes_clear_the_target() {
    let mut s = Selection::default();
    s.set([NodeId(2), NodeId(3)]);
    s.target = Some(NodeId(1));
    assert_eq!(s.subjects(), &[NodeId(1)]);
    s.add(NodeId(4));
    assert_eq!(s.target, None);
    assert_eq!(s.subjects(), &[NodeId(2), NodeId(3), NodeId(4)]);
    s.target = Some(NodeId(1));
    s.remove(NodeId(2));
    assert_eq!(s.target, None);
    s.target = Some(NodeId(1));
    s.set([NodeId(3)]);
    assert_eq!(s.target, None);
}

#[test]
fn a_chosen_object_on_a_path_needs_its_group_selected_alone() {
    use vectorcraft_geom::{Point, Rect, shapes};
    let mut d = Document::new(200.0, 200.0);
    let l = d.layers[0].id;
    let (line, a, b, g, other) = (d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id(), d.alloc_id());
    let line = Node::path(line, shapes::line(Point::new(0.0, 100.0), Point::new(200.0, 100.0)), Default::default());
    let a = Node::path(a, shapes::rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), Default::default());
    let b = Node::path(b, shapes::rectangle(Rect::new(20.0, 0.0, 30.0, 10.0)), Default::default());
    let rec = crate::objects_on_path::ObjectsOnPath::new(line.id, &[&a, &b], crate::objects_on_path::CENTER, 0.0);
    let (line_id, a_id) = (line.id, a.id);
    let mut group = Node::new(g, NodeKind::Group { children: vec![Arc::new(line), Arc::new(a), Arc::new(b)], clip: false });
    group.extras_mut().objects_on_path = Some(Box::new(rec));
    d.insert(Some(l), 0, group).unwrap();
    d.insert(Some(l), 1, Node::path(other, shapes::rectangle(Rect::new(150.0, 0.0, 160.0, 10.0)), Default::default())).unwrap();
    let mut s = Selection::default();
    s.set_on_path_member(g, a_id);
    s.prune(&d);
    assert_eq!((s.objects.as_slice(), s.on_path_member), (&[g][..], Some(a_id)));
    s.add(other);
    assert_eq!(s.on_path_member, None, "another object joins the selection");
    s.set_on_path_member(g, a_id);
    s.set([g]);
    assert_eq!(s.on_path_member, None, "a new selection");
    // The path is not one of the objects, nor is an object outside the group.
    for m in [line_id, other] {
        s.set_on_path_member(g, m);
        s.prune(&d);
        assert_eq!(s.on_path_member, None, "{m:?}");
    }
    // A locked object is not chosen.
    s.set_on_path_member(g, a_id);
    d.node_mut(a_id).unwrap().locked = true;
    s.prune(&d);
    assert_eq!((s.objects.as_slice(), s.on_path_member), (&[g][..], None), "locked");
    d.node_mut(a_id).unwrap().locked = false;
    // Once the group is a plain group, no object of it is chosen.
    s.set_on_path_member(g, a_id);
    d.node_mut(g).unwrap().extras_mut().objects_on_path = None;
    s.prune(&d);
    assert_eq!((s.objects.as_slice(), s.on_path_member), (&[g][..], None));
}

#[test]
fn a_key_object_needs_two_or_more_selected_objects() {
    let mut s = Selection::default();
    s.set([NodeId(1), NodeId(2)]);
    s.key = Some(NodeId(2));
    s.remove(NodeId(1));
    assert_eq!(s.key, Some(NodeId(2)), "another object left: remove keeps the key");
    s.remove(NodeId(2));
    assert_eq!(s.key, None, "the key object itself left");
    // Pruning drops objects the document no longer has and clears the key once no other
    // selected object lies outside it.
    let mut d = Document::new(100.0, 100.0);
    let layer = d.layers[0].id;
    let mut ids = vec![];
    for _ in 0..4 {
        let id = d.alloc_id();
        ids.push(d.insert(Some(layer), usize::MAX, crate::Node::group(id, vec![])).unwrap());
    }
    s.set(ids.iter().copied());
    s.key = Some(ids[3]);
    d.remove(ids[0]).unwrap();
    s.prune(&d);
    assert_eq!(s.key, Some(ids[3]), "three objects left: the key stays");
    s.toggle(ids[1]);
    s.prune(&d);
    assert_eq!(s.key, Some(ids[3]), "two objects left: the key stays");
    s.toggle(ids[2]);
    s.prune(&d);
    assert_eq!((s.objects.clone(), s.key), (vec![ids[3]], None), "one object left");
    // A group key selected with only its own member is cleared too; another selected object
    // outside the group keeps it.
    let member = d.alloc_id();
    d.insert(Some(ids[3]), usize::MAX, crate::Node::group(member, vec![])).unwrap();
    s.set([member, ids[3]]);
    s.key = Some(ids[3]);
    s.prune(&d);
    assert_eq!(s.key, None, "a group and its own member");
    s.set([member, ids[3], ids[2]]);
    s.key = Some(ids[3]);
    s.prune(&d);
    assert_eq!(s.key, Some(ids[3]), "another object outside the group");
}

fn below(r: &mut u64, n: u64) -> u64 {
    *r ^= *r << 13;
    *r ^= *r >> 7;
    *r ^= *r << 17;
    *r % n.max(1)
}

fn random_group(d: &mut Document, r: &mut u64, depth: u64, ids: &mut Vec<NodeId>) -> Arc<Node> {
    let id = d.alloc_id();
    ids.push(id);
    let count = if depth == 0 { 0 } else { 1 + below(r, 4) };
    let children = (0..count)
        .map(|_| {
            let depth = below(r, depth);
            random_group(d, r, depth, ids)
        })
        .collect();
    let mut group = Node::group(id, children);
    group.locked = below(r, 6) == 0;
    group.visible = below(r, 6) != 0;
    Arc::new(group)
}

fn random_document(r: &mut u64) -> (Document, Vec<NodeId>) {
    let mut d = Document::new(100.0, 100.0);
    let mut ids = vec![];
    for _ in 0..3 {
        let mut layer = Node::layer(d.alloc_id(), "L", crate::LayerColor::Preset(0));
        let children = (0..1 + below(r, 4)).map(|_| random_group(&mut d, r, 3, &mut ids)).collect();
        if let Some(c) = layer.children_mut() {
            *c = children;
        }
        layer.locked = below(r, 6) == 0;
        layer.visible = below(r, 6) != 0;
        d.layers.push(Arc::new(layer));
    }
    (d, ids)
}

fn random_selection(r: &mut u64, ids: &[NodeId]) -> Selection {
    let mut s = Selection::default();
    let missing = (below(r, 2) == 0).then_some(NodeId(u64::MAX));
    s.set(ids.iter().copied().filter(|_| below(r, 2) == 0).chain(missing));
    let k = below(r, s.objects.len() as u64) as usize;
    s.objects.rotate_left(k);
    for &o in &s.objects {
        match below(r, 3) {
            0 => {
                s.anchors.insert(o, BTreeSet::from([(0, 0)]));
            }
            1 => {
                s.anchors.insert(o, BTreeSet::from([(0, 1)]));
                s.segments.insert(o, BTreeSet::from([(0, 0)]));
            }
            _ => {}
        }
    }
    s.key = s.objects.get(below(r, s.objects.len() as u64) as usize).copied();
    s.target = ids.get(below(r, ids.len() as u64) as usize).copied().filter(|_| below(r, 2) == 0);
    s.slices = ids.iter().copied().take(below(r, 3) as usize).collect();
    s.guides = vec![NodeId(1), NodeId(0)];
    s
}

#[test]
fn deselect_uneditable_matches_removing_one_at_a_time() {
    let (mut unchanged, mut partial) = (0, 0);
    for seed in 1..400u64 {
        let mut r = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let (d, mut ids) = random_document(&mut r);
        if seed % 4 == 0 {
            ids.retain(|id| d.is_editable(*id));
        }
        let s = random_selection(&mut r, &ids);
        let mut expected = s.clone();
        for o in s.objects.iter().copied().filter(|o| !d.is_editable(*o)) {
            expected.remove(o);
        }
        let mut actual = s.clone();
        actual.deselect_uneditable(&d);
        assert_eq!(actual, expected, "seed {seed}");
        unchanged += usize::from(actual == s);
        partial += usize::from(!actual.objects.is_empty() && actual.objects.len() < s.objects.len());
    }
    assert!(unchanged > 10 && partial > 100, "{unchanged} {partial}");
}

/// Baseline unoptimized prune implementation that walks the full document via `index_paths`.
fn baseline_prune(s: &mut Selection, doc: &Document) {
    let ids = s.objects.iter().chain(s.anchors.keys()).chain(s.segments.keys()).copied().chain(s.key).chain(s.target);
    let found = doc.index_paths(ids);
    s.objects.retain(|id| found.contains_key(id));
    s.anchors.retain(|id, _| found.contains_key(id));
    s.segments.retain(|id, refs| {
        let Some(path) = found.get(id).and_then(|p| doc.node_at(p)).and_then(|n| n.path_data()) else { return false };
        refs.retain(|&(si, seg)| path.subpaths.get(si).is_some_and(|sp| seg < sp.segment_count()));
        !refs.is_empty() && s.objects.contains(id) && s.anchors.contains_key(id)
    });
    s.guides.retain(|i| doc.guides.get(*i).is_some_and(|g| doc.guide_editable(g)));
    let keep = s.key.and_then(|k| found.get(&k)).and_then(|p| doc.node_at(p)).is_some_and(|n| {
        let mut inside = BTreeSet::new();
        n.walk(&mut |c| {
            inside.insert(c.id);
        });
        s.objects.iter().any(|o| !inside.contains(o)) || (s.objects.contains(&n.id) && !s.guides.is_empty())
    });
    if !keep {
        s.key = None;
    }
    if s.target.is_some_and(|t| !found.contains_key(&t)) {
        s.target = None;
    }
    s.slices.retain(|id| doc.is_slice(*id));
    if s.key.is_some() || s.objects.is_empty() || s.key_guide.is_some_and(|k| !s.guides.contains(&k)) {
        s.key_guide = None;
    }
}

#[test]
fn prune_matches_baseline_for_mixed_objects_anchors_and_guides() {
    for seed in 1..300u64 {
        let mut r = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let (mut d, ids) = random_document(&mut r);
        let layer = d.layers[0].id;
        // Add several guides with mixed layers and editable states.
        d.guides.push(crate::Guide { layer: Some(layer), ..crate::Guide::new(true, 10.0) });
        d.guides.push(crate::Guide { layer: None, ..crate::Guide::new(false, 20.0) });
        d.guides.push(crate::Guide { layer: Some(NodeId(9999)), ..crate::Guide::new(true, 30.0) });

        let mut s = random_selection(&mut r, &ids);
        if below(&mut r, 2) == 0 {
            // Also test key guide when guide is selected
            s.key_guide = s.guides.first().copied();
        }

        // Test across 3 distinct selection cases:
        // 1. Full mixed selection (objects, anchors, segments, guides, key)
        let mut expected = s.clone();
        baseline_prune(&mut expected, &d);
        let mut actual = s.clone();
        actual.prune(&d);
        assert_eq!(actual, expected, "mixed selection mismatch on seed {seed}");

        // 2. Object-only selection (anchors & segments cleared)
        let mut obj_only = s.clone();
        obj_only.anchors.clear();
        obj_only.segments.clear();
        obj_only.key = None;
        obj_only.target = None;
        let mut expected_obj = obj_only.clone();
        baseline_prune(&mut expected_obj, &d);
        let mut actual_obj = obj_only.clone();
        actual_obj.prune(&d);
        assert_eq!(actual_obj, expected_obj, "object-only selection mismatch on seed {seed}");

        // 3. Empty selection (only guides or slices remain)
        let mut empty_sel = obj_only.clone();
        empty_sel.objects.clear();
        let mut expected_empty = empty_sel.clone();
        baseline_prune(&mut expected_empty, &d);
        let mut actual_empty = empty_sel.clone();
        actual_empty.prune(&d);
        assert_eq!(actual_empty, expected_empty, "empty selection mismatch on seed {seed}");
    }
}
