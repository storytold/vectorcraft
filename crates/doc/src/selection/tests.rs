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

/// `expand_to` selects a member of a selected group in the group's place, beside the group's other
/// objects at each level down to it, and leaves a selection it cannot open alone.
#[test]
fn expand_to_selects_a_member_in_place_of_its_selected_group() {
    let rect = || vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let [x, g, e, h, b, c, dd] = [(); 7].map(|_| d.alloc_id());
    let path = |id| Arc::new(Node::path(id, rect(), crate::Appearance::default_art()));
    let inner = Node::group(h, vec![path(b), path(c)]);
    let group = Node::group(g, vec![path(e), Arc::new(inner), path(dd)]);
    d.insert(Some(l), 0, Node::path(x, rect(), crate::Appearance::default_art())).unwrap();
    d.insert(Some(l), 1, group).unwrap();
    let mut s = Selection::default();
    s.set([x, g]);
    s.key = Some(g);
    assert!(s.expand_to(&d, b));
    assert_eq!(s.objects, [x, e, dd, b, c], "the group's objects beside the inner group, then the inner group's");
    assert_eq!(s.key, None, "the group leaves the selection with its key");
    assert!(!s.expand_to(&d, b), "a member selected itself with nothing selected around it");
    assert!(!s.expand_to(&d, x), "an object inside nothing selected");
    // A member selected beside the group around it keeps its place.
    let mut with_member = Selection::default();
    with_member.set([b, g]);
    assert!(with_member.expand_to(&d, b));
    assert_eq!(with_member.objects, [b, e, dd, c]);
    // An inner group selected beside the group leaves the selection too.
    let mut both = Selection::default();
    both.set([h, g]);
    assert!(both.expand_to(&d, b));
    assert_eq!(both.objects, [e, dd, b, c]);
    // A group with anchors recorded for it is left as it is.
    let mut partly = Selection::default();
    partly.set([g]);
    partly.anchors.insert(g, BTreeSet::from([(0, 0)]));
    let before = partly.clone();
    assert!(!partly.expand_to(&d, b));
    assert_eq!(partly, before);
    // A Shaper Group opens to its faces and never to its original shapes.
    let [shaper, sources, o1, o2, face] = [(); 5].map(|_| d.alloc_id());
    let mut sources_group = Node::group(sources, vec![path(o1), path(o2)]);
    sources_group.name = Some(crate::shaper::SOURCES.into());
    let mut shaper_group = Node::group(shaper, vec![Arc::new(sources_group), path(face)]);
    shaper_group.extras_mut().shaper = Some(Box::default());
    d.insert(Some(l), 2, shaper_group).unwrap();
    let mut shaped = Selection::default();
    shaped.set([shaper]);
    let before = shaped.clone();
    assert!(!shaped.expand_to(&d, o1), "an original shape");
    assert_eq!(shaped, before);
    assert!(shaped.expand_to(&d, face));
    assert_eq!(shaped.objects, [face]);
    // Hidden and locked objects beside the member stay out of the selection.
    let [k, shown, hidden, locked, last] = [(); 5].map(|_| d.alloc_id());
    let mut hidden_path = Node::path(hidden, rect(), crate::Appearance::default_art());
    hidden_path.visible = false;
    let mut locked_path = Node::path(locked, rect(), crate::Appearance::default_art());
    locked_path.locked = true;
    let kept = Node::group(k, vec![path(shown), Arc::new(hidden_path), Arc::new(locked_path), path(last)]);
    d.insert(Some(l), 3, kept).unwrap();
    let mut with_locked = Selection::default();
    with_locked.set([k]);
    assert!(with_locked.expand_to(&d, last));
    assert_eq!(with_locked.objects, [shown, last]);
    // A locked or hidden member itself does not open the group.
    for target in [locked, hidden] {
        let mut s = Selection::default();
        s.set([k]);
        let before = s.clone();
        assert!(!s.expand_to(&d, target), "{target:?}");
        assert_eq!(s, before);
    }
}

/// A selection with every field set: objects 1 to 6 with 3 listed twice, anchors, segments, a key,
/// a key anchor, a target, slices, guides, a key guide, new art, a symbol part and an on-path member.
fn seeded() -> Selection {
    let mut s = Selection { objects: [1, 2, 3, 4, 5, 3, 6].map(NodeId).to_vec(), ..Selection::default() };
    for i in [2, 3, 4, 5] {
        s.anchors.insert(NodeId(i), BTreeSet::from([(0, 1)]));
        s.segments.insert(NodeId(i), BTreeSet::from([(0, 0)]));
    }
    s.key = Some(NodeId(4));
    s.key_anchor = Some((NodeId(5), (0, 1)));
    s.target = Some(NodeId(3));
    s.slices = vec![NodeId(9)];
    s.guides = vec![NodeId(10)];
    s.key_guide = Some(NodeId(10));
    s.new_art = Arc::new(HashSet::from([NodeId(2)]));
    s.symbol_part = Some(SymbolPart { instance: NodeId(1), path: vec![0] });
    s.on_path_member = Some(NodeId(11));
    s
}

/// Id lists with repeats, ids not selected, removals followed by additions, and the empty list.
fn id_streams() -> Vec<Vec<NodeId>> {
    let mut streams: Vec<Vec<NodeId>> = [
        vec![],
        vec![4],
        vec![7],
        vec![3],
        vec![4, 1, 4, 7],
        vec![7, 8, 7, 7, 3, 3, 5],
        vec![1, 2, 3, 4, 5, 6],
        vec![6, 5, 4, 3, 2, 1, 1, 2, 3],
        vec![8, 2, 8, 2, 9, 9, 9],
    ]
    .into_iter()
    .map(|ids| ids.into_iter().map(NodeId).collect())
    .collect();
    // Deterministic pseudo-random lists over ids 1 to 12, half of them selected to start with.
    let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
    for len in [5, 17, 40, 120] {
        let ids = (0..len)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                NodeId(x % 12 + 1)
            })
            .collect();
        streams.push(ids);
    }
    streams
}

#[test]
fn add_all_does_what_adding_each_does() {
    for ids in id_streams() {
        let mut one_by_one = seeded();
        for id in &ids {
            one_by_one.add(*id);
        }
        let mut all = seeded();
        all.add_all(ids.iter().copied());
        assert_eq!(all, one_by_one, "add_all({ids:?})");
    }
}

#[test]
fn toggle_all_does_what_toggling_each_does() {
    for ids in id_streams() {
        let mut one_by_one = seeded();
        for id in &ids {
            one_by_one.toggle(*id);
        }
        let mut all = seeded();
        all.toggle_all(ids.iter().copied());
        assert_eq!(all, one_by_one, "toggle_all({ids:?})");
    }
    // An id toggled twice is selected again, at the end, without the anchors it had.
    let mut s = seeded();
    s.toggle_all([NodeId(2), NodeId(2)]);
    assert_eq!(s.objects, [1, 3, 4, 5, 3, 6, 2].map(NodeId));
    assert!(!s.anchors.contains_key(&NodeId(2)) && !s.segments.contains_key(&NodeId(2)));
    // A removed key and key anchor stay cleared.
    let mut s = seeded();
    s.toggle_all([NodeId(4), NodeId(5), NodeId(4), NodeId(5)]);
    assert_eq!((s.key, s.key_anchor), (None, None));
}

#[test]
fn set_slices_and_set_guides_keep_the_first_of_each_id() {
    let mut s = seeded();
    s.set_slices([3, 1, 3, 2, 1].map(NodeId));
    assert_eq!(s.slices, [3, 1, 2].map(NodeId));
    assert!(s.objects.is_empty() && s.guides.is_empty());
    s.set_guides([5, 5, 4, 6, 4].map(NodeId));
    assert_eq!(s.guides, [5, 4, 6].map(NodeId));
    assert!(s.slices.is_empty());
}

/// Shift-clicking a target circle does what removing or adding each of its art one by one does,
/// for a layer with art, an empty layer and an object.
#[test]
fn toggle_target_does_what_toggling_its_art_one_by_one_does() {
    let rect = || vectorcraft_geom::shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 0.0, 10.0, 10.0));
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    let art: Vec<NodeId> = (0..4).map(|_| d.alloc_id()).collect();
    for id in &art {
        d.insert(Some(l), usize::MAX, Node::path(*id, rect(), crate::Appearance::default_art())).unwrap();
    }
    let empty = d.alloc_id();
    d.insert(None, usize::MAX, Node::layer(empty, "Empty", crate::LayerColor::Preset(2))).unwrap();
    let one_by_one = |s: &Selection, id: NodeId| {
        let mut s = s.clone();
        let ids = match d.node(id) {
            Some(n) if n.is_layer() => n.layer_art(true),
            _ => vec![id],
        };
        if ids.iter().all(|i| s.contains(*i)) {
            ids.iter().for_each(|i| s.remove(*i));
        } else {
            ids.into_iter().for_each(|i| s.add(i));
        }
        s.target = None;
        s
    };
    let mut starts = vec![Selection::default()];
    for objects in [vec![art[1]], vec![art[3], art[0], art[1], art[2]], vec![art[2], art[2], NodeId(99)]] {
        let mut s = seeded();
        s.objects = objects;
        s.key = s.objects.first().copied();
        s.anchors.insert(art[1], BTreeSet::from([(0, 0)]));
        starts.push(s);
    }
    for start in &starts {
        for id in [l, empty, art[2], NodeId(99)] {
            let mut s = start.clone();
            s.toggle_target(&d, id);
            assert_eq!(s, one_by_one(start, id), "toggle_target({id:?}) on {:?}", start.objects);
        }
    }
}

#[test]
fn remove_all_does_what_removing_each_does_to_a_key_anchor_and_symbol_part() {
    for kept in [false, true] {
        let mut s = Selection::default();
        for i in 1..=4 {
            s.add(NodeId(i));
        }
        s.anchors.insert(NodeId(2), BTreeSet::from([(0, 0), (0, 1)]));
        s.anchors.insert(NodeId(3), BTreeSet::from([(0, 2)]));
        s.key_anchor = Some((NodeId(2), (0, 0)));
        s.symbol_part = Some(SymbolPart { instance: NodeId(1), path: vec![0] });
        s.guides = vec![NodeId(10)];
        s.key_guide = Some(NodeId(10));
        let ids = if kept { [NodeId(3), NodeId(8), NodeId(3)] } else { [NodeId(2), NodeId(8), NodeId(2)] };
        let mut one_by_one = s.clone();
        for id in ids {
            one_by_one.remove(id);
        }
        let mut all = s.clone();
        all.remove_all(ids);
        assert_eq!(all, one_by_one);
        assert_eq!(all.key_anchor.is_some(), kept);
        assert_eq!((all.symbol_part.clone(), all.guides.clone(), all.key_guide), (None, vec![NodeId(10)], Some(NodeId(10))));
    }
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
