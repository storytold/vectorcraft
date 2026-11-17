//! The selection that Select ▸ Object ▸ Direction Handles builds.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use vectorcraft_doc::{NodeId, Selection, SymbolPart};

use super::select_anchors;

type Items = Vec<(NodeId, BTreeSet<(usize, usize)>)>;

/// Selects `items` one path at a time: a clear, then an add and an anchor insert per path.
fn one_by_one(sel: &mut Selection, items: Items) {
    sel.clear();
    for (id, refs) in items {
        sel.add(id);
        sel.anchors.insert(id, refs);
    }
}

/// A selection with every field set, as one left by earlier selecting.
fn busy() -> Selection {
    let mut sel = Selection::default();
    sel.set([NodeId(90), NodeId(91)]);
    sel.anchors.insert(NodeId(90), BTreeSet::from([(0, 0)]));
    sel.segments.insert(NodeId(91), BTreeSet::from([(0, 1)]));
    sel.key = Some(NodeId(91));
    sel.target = Some(NodeId(92));
    sel.slices = vec![NodeId(93)];
    sel.guides = vec![NodeId(94)];
    sel.key_guide = Some(NodeId(94));
    sel.key_anchor = Some((NodeId(90), (0, 0)));
    sel.symbol_part = Some(SymbolPart { instance: NodeId(95), path: vec![0, 1] });
    sel.on_path_member = Some(NodeId(96));
    sel.new_art = Arc::new(HashSet::from([NodeId(97)]));
    sel
}

fn refs(n: usize) -> BTreeSet<(usize, usize)> {
    (0..n).map(|i| (0, i)).collect()
}

#[test]
fn select_anchors_gives_the_selection_one_by_one_selecting_gives() {
    let cases: Vec<Items> = vec![
        vec![],
        vec![(NodeId(1), refs(4))],
        (1..=500).map(|i| (NodeId(i), refs(4))).collect(),
        // A path listed again keeps its first place and its last anchors.
        vec![(NodeId(3), refs(1)), (NodeId(1), refs(4)), (NodeId(3), refs(2)), (NodeId(2), BTreeSet::new()), (NodeId(1), refs(3))],
    ];
    for items in cases {
        for start in [Selection::default(), busy()] {
            let (mut want, mut got) = (start.clone(), start);
            one_by_one(&mut want, items.clone());
            select_anchors(&mut got, items.clone());
            assert_eq!(got, want, "{items:?}");
        }
    }
    let mut got = busy();
    select_anchors(&mut got, vec![(NodeId(3), refs(1)), (NodeId(1), refs(4)), (NodeId(3), refs(2))]);
    assert_eq!(got.objects, vec![NodeId(3), NodeId(1)]);
    assert_eq!(got.anchors[&NodeId(3)], refs(2));
    assert!(got.segments.is_empty() && got.slices.is_empty() && got.guides.is_empty());
    assert_eq!((got.key, got.target, got.key_guide, got.key_anchor, got.on_path_member), (None, None, None, None, None));
    assert!(got.symbol_part.is_none());
    assert_eq!(*got.new_art, HashSet::from([NodeId(97)]), "selecting leaves the new-art list alone");
}
