//! The batched lookups ([`Document::index_paths`], `node_at`, `node_mut_at`, `ancestry_at`) agree
//! with the one-id ones on nested trees with missing and duplicated ids, and the edits of many nodes
//! ([`Document::remove_many`], `wrap`, `insert_beside`) leave the tree that `remove`, `insert` and
//! `move_node` leave one node at a time.
use crate::*;
use vectorcraft_geom::shapes;

/// Layers with paths and groups nested `depth` deep; every 7th id is reused (a damaged file), so
/// first-in-tree-order matters.
fn tree(seed: u64) -> (Document, Vec<NodeId>) {
    let mut d = Document::new(612.0, 792.0);
    let mut r = seed | 1;
    let mut next = |n: u64| {
        r ^= r << 13;
        r ^= r >> 7;
        r ^= r << 17;
        r % n
    };
    let mut ids = vec![];
    let leaf = |d: &mut Document, ids: &mut Vec<NodeId>| {
        let id = if ids.len() % 7 == 6 { ids[ids.len() / 2] } else { d.alloc_id() };
        ids.push(id);
        Arc::new(Node::path(id, shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)), Appearance::default_art()))
    };
    for l in 0..3 {
        let lid = d.alloc_id();
        let mut layer = Node::layer(lid, &format!("L{l}"), LayerColor::Preset(0));
        ids.push(lid);
        for _ in 0..(5 + next(10)) {
            let child = if next(3) == 0 {
                let mut inner: Vec<Arc<Node>> = (0..(1 + next(4))).map(|_| leaf(&mut d, &mut ids)).collect();
                for _ in 0..next(3) {
                    let gid = d.alloc_id();
                    ids.push(gid);
                    inner = vec![Arc::new(Node::group(gid, inner))];
                }
                let gid = d.alloc_id();
                ids.push(gid);
                Arc::new(Node::group(gid, inner))
            } else {
                leaf(&mut d, &mut ids)
            };
            if let Some(ch) = layer.children_mut() {
                ch.push(child);
            }
        }
        d.layers.push(Arc::new(layer));
    }
    ids.push(NodeId(999_999)); // not in the tree
    (d, ids)
}

#[test]
fn batched_lookups_match_one_at_a_time() {
    for seed in 1..40 {
        let (d, ids) = tree(seed);
        let paths = d.index_paths(ids.iter().copied());
        for id in &ids {
            assert_eq!(paths.get(id).cloned(), d.index_path(*id), "seed {seed} id {id:?}");
            let p = paths.get(id);
            assert_eq!(p.and_then(|p| d.node_at(p)).map(|n| n as *const Node), d.node(*id).map(|n| n as *const Node), "seed {seed} node {id:?}");
            assert_eq!(p.and_then(|p| d.ancestry_at(p)), d.ancestry(*id), "seed {seed} ancestry {id:?}");
        }
        // A subset, and nothing.
        let some: Vec<NodeId> = ids.iter().copied().step_by(3).collect();
        let sub = d.index_paths(some.iter().copied());
        assert!(some.iter().all(|id| sub.get(id).cloned() == d.index_path(*id)));
        assert!(d.index_paths(std::iter::empty()).is_empty());
        // Mutable access reaches the same node as node_mut, copy-on-write.
        let mut a = d.clone();
        let mut b = d.clone();
        for id in ids.iter().step_by(5) {
            if let Some(n) = a.node_mut(*id) {
                n.opacity = 0.25;
            }
            if let Some(n) = d.index_path(*id).and_then(|p| b.node_mut_at(&p)) {
                n.opacity = 0.25;
            }
        }
        assert_eq!(serde_json::to_string(&a).ok(), serde_json::to_string(&b).ok(), "seed {seed}");
    }
}

/// What a walk of the tree gives for every id, checked against the cached lookups.
fn agree(d: &Document, ids: &[NodeId], what: &str) {
    agree_except(d, ids, None, what);
}

/// [`agree`], except for `dup`, an id given a second node after the cache was built: lookups of it
/// may return either of its nodes (see `id_index`), but always one with that id.
fn agree_except(d: &Document, ids: &[NodeId], dup: Option<NodeId>, what: &str) {
    if let Some(dup) = dup {
        assert_eq!(d.node(dup).map(|n| n.id), Some(dup), "{what}: node {dup:?} has its id");
    }
    let ids: Vec<NodeId> = ids.iter().copied().filter(|id| Some(*id) != dup).collect();
    let ids = &ids[..];
    for id in ids {
        let walked = d.walk_index_path(*id);
        assert_eq!(d.index_path(*id), walked, "{what}: index_path {id:?}");
        let by_walk = walked.as_deref().and_then(|p| d.node_at(p)).map(|n| n as *const Node);
        assert_eq!(d.node(*id).map(|n| n as *const Node), by_walk, "{what}: node {id:?}");
    }
    let batched = d.index_paths(ids.iter().copied());
    for id in ids {
        assert_eq!(batched.get(id).cloned(), d.walk_index_path(*id), "{what}: index_paths {id:?}");
    }
}

/// The id cache never answers differently from a walk, however the tree is changed behind its
/// back (through the public fields), including in a clone that shares the cache.
#[test]
fn cached_lookups_follow_any_edit_of_the_tree() {
    for seed in 1..30 {
        let (mut d, ids) = tree(seed);
        agree(&d, &ids, "fresh");
        let snapshot = d.clone(); // shares the cache
        // Insert at the front of the first layer: every path in it shifts.
        let extra = d.alloc_id();
        let leaf = Arc::new(Node::path(extra, shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art()));
        if let Some(ch) = Arc::make_mut(&mut d.layers[0]).children_mut() {
            ch.insert(0, leaf.clone());
        }
        let mut all = ids.clone();
        all.push(extra);
        agree(&d, &all, "inserted at the front");
        // Remove a child, swap two, and move one into another layer.
        if let Some(ch) = Arc::make_mut(&mut d.layers[1]).children_mut()
            && ch.len() > 3
        {
            ch.remove(1);
            ch.swap(0, 2);
        }
        let moved = Arc::make_mut(&mut d.layers[2]).children_mut().and_then(|ch| (!ch.is_empty()).then(|| ch.remove(0)));
        if let Some(m) = moved
            && let Some(ch) = Arc::make_mut(&mut d.layers[0]).children_mut()
        {
            ch.push(m);
        }
        agree(&d, &all, "removed, swapped and moved");
        // A new duplicate of an existing id, earlier in tree order than the original.
        let dup = ids[ids.len() / 3];
        let copy = Arc::new(Node::path(dup, shapes::rectangle(Rect::new(0.0, 0.0, 2.0, 2.0)), Appearance::default_art()));
        if let Some(ch) = Arc::make_mut(&mut d.layers[0]).children_mut() {
            ch.insert(0, copy);
        }
        agree_except(&d, &all, Some(dup), "a duplicate inserted earlier");
        // Delete a whole layer, then look everything up again, missing ids included.
        d.layers.remove(1);
        agree_except(&d, &all, Some(dup), "a layer deleted");
        // A rebuilt cache (as after loading a file) knows the duplicate and walks for it.
        let mut fresh = d.clone();
        fresh.layers = d.layers.clone();
        fresh.index = Default::default();
        agree(&fresh, &all, "a damaged tree, cached afresh");
        // The clone made before all this still answers for its own tree.
        agree(&snapshot, &ids, "the earlier clone");
    }
}

/// Nodes of `d` picked and ordered by `seed`. None lies inside another, and each of them, and each
/// node it is in, appears in the tree once.
fn picks(d: &Document, seed: u64) -> Vec<NodeId> {
    let mut count = std::collections::HashMap::<NodeId, usize>::new();
    d.walk(|n| *count.entry(n.id).or_default() += 1);
    let once = |id: &NodeId| count.get(id) == Some(&1);
    let mut found: Vec<(Vec<usize>, NodeId)> = count
        .keys()
        .filter(|id| !(id.0 + seed).is_multiple_of(3))
        .filter_map(|id| d.index_path(*id).map(|p| (p, *id)))
        .filter(|(p, _)| d.ancestry_at(p).is_some_and(|a| a.iter().all(once)))
        .collect();
    found.sort();
    let mut kept: Vec<(Vec<usize>, NodeId)> = vec![];
    for (p, id) in found {
        if !kept.iter().any(|(q, _)| p.starts_with(q)) {
            kept.push((p, id));
        }
    }
    let mut out: Vec<NodeId> = kept.into_iter().map(|(_, id)| id).collect();
    out.sort_by_key(|id| id.0.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ seed);
    out
}

/// `a` and `b` hold the same tree, and their lookups agree with each other and with a walk.
fn same(a: &Document, b: &Document, ids: &[NodeId], what: &str) {
    assert_eq!(serde_json::to_string(a).ok(), serde_json::to_string(b).ok(), "{what}");
    for id in ids {
        assert_eq!(a.index_path(*id), b.index_path(*id), "{what}: {id:?}");
    }
    agree(b, ids, what);
}

/// A 1 pt square with a new id.
fn square(d: &mut Document) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(Rect::new(0.0, 0.0, 1.0, 1.0)), Appearance::default_art())
}

#[test]
fn remove_many_matches_removing_one_at_a_time() {
    for seed in 1..40 {
        let (d, ids) = tree(seed);
        let picked = picks(&d, seed);
        let (mut a, mut b) = (d.clone(), d.clone());
        let one: Vec<Arc<Node>> = picked.iter().map(|id| a.remove(*id).unwrap()).collect();
        let many = b.remove_many(&picked).unwrap();
        assert_eq!(many.len(), one.len(), "seed {seed}");
        assert!(one.iter().zip(&many).all(|(x, y)| Arc::ptr_eq(x, y)), "seed {seed}: the same nodes, in the order of the list");
        same(&a, &b, &ids, &format!("seed {seed}"));
    }
}

#[test]
fn insert_beside_matches_inserting_one_at_a_time() {
    for seed in 1..40 {
        for replace in [false, true] {
            let (mut d, mut ids) = tree(seed);
            let picked = picks(&d, seed);
            // None to three new nodes for each anchor.
            let items: Vec<(NodeId, Vec<Node>)> =
                picked.iter().enumerate().map(|(k, id)| (*id, (0..k % 4).map(|_| square(&mut d)).collect())).collect();
            ids.extend(items.iter().flat_map(|(_, new)| new.iter().map(|n| n.id)));
            let (mut a, mut b) = (d.clone(), d.clone());
            for (id, new) in items.clone() {
                let (parent, at, _) = a.position(id).unwrap();
                let at = if replace {
                    a.remove(id).unwrap();
                    at
                } else {
                    at + 1
                };
                for (k, n) in new.into_iter().enumerate() {
                    a.insert(parent, at + k, n).unwrap();
                }
            }
            b.insert_beside(items, replace).unwrap();
            same(&a, &b, &ids, &format!("seed {seed}, replace {replace}"));
        }
    }
}

#[test]
fn wrap_matches_inserting_a_group_and_moving_the_nodes_into_it_one_at_a_time() {
    for seed in 1..40 {
        let (mut d, mut ids) = tree(seed);
        let picked = picks(&d, seed);
        let Some(&last) = picked.last() else { continue };
        let group = d.alloc_id();
        ids.push(group);
        let (mut a, mut b) = (d.clone(), d.clone());
        let (parent, at, _) = a.position(last).unwrap();
        a.insert(parent, at + 1, Node::group(group, vec![])).unwrap();
        for id in &picked {
            a.move_node(*id, Some(group), usize::MAX).unwrap();
        }
        assert_eq!(b.wrap(&picked, |nodes| Node::group(group, nodes)), Ok(group), "seed {seed}");
        same(&a, &b, &ids, &format!("seed {seed}"));
    }
}

/// An id not in the tree, listed twice, or inside another listed node fails with `NoNode`, and the
/// document stays as it was.
#[test]
fn edits_of_many_refuse_missing_repeated_and_nested_ids() {
    let mut d = Document::new(612.0, 792.0);
    let layer = d.layers[0].id;
    let (x, y, z) = (square(&mut d), square(&mut d), square(&mut d));
    let (inner, other) = (y.id, z.id);
    let group = d.alloc_id();
    d.insert(Some(layer), 0, Node::group(group, vec![Arc::new(x), Arc::new(y)])).unwrap();
    d.insert(Some(layer), 1, z).unwrap();
    let saved = serde_json::to_string(&d).ok();
    let missing = NodeId(999_999);
    for (list, bad) in [(vec![other, missing], missing), (vec![other, other], other), (vec![group, inner], inner), (vec![inner, group], inner)] {
        let what = format!("{list:?}");
        let mut b = d.clone();
        assert_eq!(b.remove_many(&list).err(), Some(DocError::NoNode(bad)), "{what}");
        assert_eq!(b.wrap(&list, |nodes| Node::group(NodeId(1_000_000), nodes)).err(), Some(DocError::NoNode(bad)), "{what}");
        for replace in [false, true] {
            let items = list.iter().map(|id| (*id, vec![])).collect();
            assert_eq!(b.insert_beside(items, replace).err(), Some(DocError::NoNode(bad)), "{what}");
        }
        assert_eq!(serde_json::to_string(&b).ok(), saved, "{what}: the document stays as it was");
    }
}
