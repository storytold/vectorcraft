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
fn guides_are_selected_on_their_own() {
    let mut s = Selection::default();
    s.add(NodeId(1));
    s.set_guides([2, 0, 2]);
    assert_eq!((s.objects.clone(), s.guides.clone()), (vec![], vec![2, 0]));
    assert!(s.has_objects_or_guides() && s.is_empty());
    // Shift-adding an object keeps them; a new selection doesn't.
    s.add(NodeId(1));
    assert_eq!(s.guides, vec![2, 0]);
    s.set([NodeId(1)]);
    assert!(s.guides.is_empty());
    // Toggled, a guide joins (or leaves) the selected objects.
    s.toggle_guide(0);
    assert_eq!((s.objects.clone(), s.guides.clone()), (vec![NodeId(1)], vec![0]));
    s.toggle_guide(0);
    assert_eq!((s.objects.clone(), s.guides.clone()), (vec![NodeId(1)], vec![]));
    // Pruning drops guides the document no longer has.
    let mut d = Document::new(100.0, 100.0);
    d.guides.push(crate::Guide::new(true, 10.0));
    s.set_guides([0, 1]);
    s.prune(&d);
    assert_eq!(s.guides, vec![0]);
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
    d.guides.push(g);
    assert_eq!(d.guides_on(layer).count(), 1);
    // Guides on no layer, or a gone one, are adopted by the top visible, unlocked layer.
    d.node_mut(layer).unwrap().visible = true;
    d.node_mut(layer).unwrap().locked = false;
    d.guides.push(crate::Guide::new(false, 5.0));
    d.guides.push(gone);
    assert_eq!(d.adopt_guides(), 2);
    assert!(d.guides.iter().all(|g| g.layer == Some(layer)));
    assert_eq!(d.adopt_guides(), 0, "once");
    // A guide saved before guides had layers reads as one on no layer.
    let old: crate::Guide = serde_json::from_str(r#"{"vertical": true, "pos": 5.0}"#).unwrap();
    assert_eq!(old.layer, None);
    assert!(!serde_json::to_string(&crate::Guide::new(true, 5.0)).unwrap().contains("layer"), "nothing new written");
}

#[test]
fn guide_finite_span_and_bounds() {
    let d = Document::new(300.0, 200.0);
    let mut g = crate::Guide::new(true, 100.0);
    // Canvas guide without explicit span defaults to 16,383 pt centered on artboard.
    assert_eq!(d.guide_span(&g), None);
    let (s0, s1) = d.guide_effective_span(&g);
    assert_eq!(s1 - s0, crate::MAX_GUIDE_LEN);
    assert_eq!((s0 + s1) / 2.0, 100.0); // centered at artboard center y = 100
    assert_eq!(d.guide_rect(&g).height(), crate::MAX_GUIDE_LEN);
    assert!(d.guide_passes(&g, vectorcraft_geom::Point::new(100.0, 50.0), 3.0));

    // Finite span
    g.span = Some((20.0, 80.0));
    assert_eq!(d.guide_span(&g), Some((20.0, 80.0)));
    assert_eq!(d.guide_effective_span(&g), (20.0, 80.0));
    let r = d.guide_rect(&g);
    assert_eq!(r.x0, 100.0);
    assert_eq!(r.height(), 60.0);

    // Hit testing passes within span, fails outside
    assert!(d.guide_passes(&g, vectorcraft_geom::Point::new(100.0, 50.0), 3.0));
    assert!(!d.guide_passes(&g, vectorcraft_geom::Point::new(100.0, 95.0), 3.0));
    assert!(!d.guide_passes(&g, vectorcraft_geom::Point::new(100.0, 5.0), 3.0));

    // Moving moves both pos and span
    let moved = g.moved(vectorcraft_geom::Vec2::new(15.0, 10.0));
    assert_eq!(moved.pos, 115.0);
    assert_eq!(moved.span, Some((30.0, 90.0)));

    // Serde roundtrip
    let json = serde_json::to_string(&g).unwrap();
    assert!(json.contains(r#""span":[20.0,80.0]"#));
    let read: crate::Guide = serde_json::from_str(&json).unwrap();
    assert_eq!(read.span, Some((20.0, 80.0)));
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
    s.guides = vec![1, 0];
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
