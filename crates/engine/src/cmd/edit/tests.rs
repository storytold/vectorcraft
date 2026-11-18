//! Cut with direct-selected anchors removes the objects selected whole and the paths it empties
//! through one [`Beside`](super::super::pathops::Beside), and leaves the document, the selection,
//! the clipboard and the undo history exactly as removing each of them on its own does.

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use vectorcraft_doc::{Document, NodeId, NodeKind, tree_walks};

use super::*;
use crate::tests_large_edits::{session, square};

/// Cut's removal with every root selected whole or emptied taken out by its own
/// [`Document::remove`], and its compound path removed right after when that left it empty. The
/// tests compare [`cut_direct_selection`] with it.
fn cut_one_at_a_time(s: &mut Session, roots: &[NodeId]) -> Result<Value> {
    let st = s.doc()?;
    let sel = st.selection.clone();
    s.edit("Cut", |d, selection| {
        let gone = selection.guides.clone();
        d.retain_guides(selection, |g| !g.id.is_some_and(|id| gone.contains(&id)));
        let mut survivors = vec![];
        let mut pieces = super::super::pathops::Beside::replacing(d, roots);
        for id in roots {
            let Some(node) = d.node(*id) else { continue };
            let has_direct = sel.anchors.contains_key(id) || sel.segments.contains_key(id);
            if !has_direct || node.path_data().is_none() {
                remove_one(d, *id)?;
                continue;
            }
            let original = super::super::path::deletion_source(d, *id)?;
            let seg_refs = sel.segments.get(id);
            let anchor_refs = sel.anchors.get(id);
            let mut subpaths = Vec::new();
            let owned = std::mem::take(&mut super::super::path::path_mut(d, *id)?.subpaths);
            for (si, sp) in owned.into_iter().enumerate() {
                let cuts = super::super::clipboard::subpath_selected_segments(&sp, si, seg_refs, anchor_refs);
                if cuts.is_empty() {
                    subpaths.push(sp);
                } else if seg_refs.is_some_and(|segments| !segments.is_empty()) {
                    subpaths.extend(super::super::path::without_segments(sp, &cuts));
                } else {
                    let gone: BTreeSet<usize> = anchor_refs.into_iter().flat_map(|refs| super::super::path::subpath_refs(refs, si)).collect();
                    subpaths.extend(super::super::path::without_anchors(sp, &gone));
                }
            }
            subpaths.retain(|sp| sp.anchors.len() >= 2);
            if subpaths.is_empty() {
                remove_one(d, *id)?;
            } else {
                survivors.extend(super::super::path::replace_deleted_pieces(d, &mut pieces, original, subpaths)?);
            }
        }
        pieces.finish(d)?;
        selection.set(survivors);
        selection.anchors.clear();
        selection.segments.clear();
        Ok(())
    })?;
    ok()
}

/// Removes object `id`, and the compound path that held it when it was the compound's last member.
fn remove_one(d: &mut Document, id: NodeId) -> Result<()> {
    let parent = d.parent_of(id).filter(|p| matches!(d.node(*p).map(|n| &n.kind), Some(NodeKind::Compound { .. })));
    d.remove(id)?;
    if let Some(p) = parent
        && d.node(p).is_some_and(|n| n.children().is_some_and(|c| c.is_empty()))
    {
        d.remove(p)?;
    }
    Ok(())
}

/// Cut as `edit.cut` runs it for a direct selection, with [`cut_one_at_a_time`] in place of
/// [`cut_direct_selection`].
fn reference_cut(s: &mut Session) {
    let (direct, roots) = copy_cut_targets(s).unwrap();
    assert!(direct, "the selection is a direct selection");
    copy(s, &json!({})).unwrap();
    cut_one_at_a_time(s, &roots).unwrap();
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

/// Selects `whole` as objects, then anchors `anchors` of each path in `partial`.
fn select_direct(s: &mut Session, whole: &[NodeId], partial: &[(NodeId, Value)]) {
    run(s, "select.set", json!({"ids": whole.iter().map(|id| id.0).collect::<Vec<_>>()}));
    for (id, anchors) in partial {
        run(s, "select.anchors", json!({"id": id.0, "anchors": anchors, "mode": "add"}));
    }
}

/// The ids of every node of the document, depth first, with the depth of each.
fn tree(d: &Document) -> Vec<(usize, NodeId)> {
    fn walk(n: &vectorcraft_doc::Node, depth: usize, out: &mut Vec<(usize, NodeId)>) {
        out.push((depth, n.id));
        for c in n.children().into_iter().flatten() {
            walk(c, depth + 1, out);
        }
    }
    let mut out = vec![];
    for l in &d.layers {
        walk(l, 0, &mut out);
    }
    out
}

/// Whether `a` and `b` hold the same document, selection, clipboard and number of undo and redo
/// steps.
fn assert_same(a: &Session, b: &Session, when: &str) {
    let (x, y) = (a.doc().unwrap(), b.doc().unwrap());
    assert_eq!(tree(&x.doc), tree(&y.doc), "{when}: the tree");
    assert!(*x.doc == *y.doc, "{when}: the document");
    assert_eq!(x.selection, y.selection, "{when}: the selection");
    // Each session's document has its own uid; the clipboard names the one it came from.
    assert_eq!(a.clipboard.source_doc.is_some(), b.clipboard.source_doc.is_some(), "{when}: the clipboard's source");
    let unsourced = |c: &Clipboard| Clipboard { source_doc: None, ..c.clone() };
    assert!(unsourced(&a.clipboard) == unsourced(&b.clipboard), "{when}: the clipboard");
    assert_eq!(x.history.undo.len(), y.history.undo.len(), "{when}: the undo steps");
    assert_eq!(x.history.redo.len(), y.history.redo.len(), "{when}: the redo steps");
}

/// Runs `edit.cut` on one session that `make` builds and [`reference_cut`] on another, and checks
/// that both hold the same after the Cut, after Undo and after Redo. Returns the first session after
/// the Cut.
fn same_as_one_at_a_time(make: impl Fn() -> Session) -> Session {
    let (mut a, mut b) = (make(), make());
    let before = a.doc().unwrap().doc.clone();
    assert_same(&a, &b, "before");
    run(&mut a, "edit.cut", json!({}));
    reference_cut(&mut b);
    assert_same(&a, &b, "Cut");
    let after = a.doc().unwrap().doc.clone();
    run(&mut a, "edit.undo", json!({}));
    run(&mut b, "edit.undo", json!({}));
    assert_same(&a, &b, "Undo");
    assert!(*a.doc().unwrap().doc == *before, "Undo gives back the document");
    run(&mut a, "edit.redo", json!({}));
    run(&mut b, "edit.redo", json!({}));
    assert_same(&a, &b, "Redo");
    assert!(*a.doc().unwrap().doc == *after, "Redo gives back the Cut");
    run(&mut a, "edit.undo", json!({}));
    run(&mut a, "edit.redo", json!({}));
    a
}

/// A new document; [`rect`] adds squares to it.
fn new_doc() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 800, "height": 600}));
    s
}

/// Adds a 10 pt square to the current layer, 20 pt right of the last one.
fn rect(s: &mut Session) -> NodeId {
    let x = 20.0 * s.doc().unwrap().doc.node_count() as f64;
    NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10}))["id"].as_u64().unwrap())
}

/// Makes a compound path of `ids` and returns it with its members in order.
fn compound(s: &mut Session, ids: &[NodeId]) -> (NodeId, Vec<NodeId>) {
    run(s, "select.set", json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()}));
    let c = NodeId(run(s, "object.compoundPath.make", json!({}))["id"].as_u64().unwrap());
    let members = s.doc().unwrap().doc.node(c).unwrap().children().unwrap().iter().map(|m| m.id).collect();
    (c, members)
}

/// Anchors 0 to 2 of a square: Cut leaves it no segment, so it is removed.
fn three_anchors() -> Value {
    json!([[0, 0], [0, 1], [0, 2]])
}

/// Anchors 0 and 1 of a square: Cut keeps its other three sides.
fn two_anchors() -> Value {
    json!([[0, 0], [0, 1]])
}

/// The compound path whose members are cut one whole and one emptied goes with them; the other
/// compound path keeps its one member left, as a compound path.
#[test]
fn cut_removes_a_compound_whose_members_all_go_and_keeps_one_with_one_member_left() {
    let make = || {
        let mut s = new_doc();
        let first = [(); 2].map(|()| rect(&mut s));
        let (gone, gone_members) = compound(&mut s, &first);
        let second = [(); 3].map(|()| rect(&mut s));
        let (kept, kept_members) = compound(&mut s, &second);
        let other = rect(&mut s);
        select_direct(
            &mut s,
            &[gone_members[0], kept_members[0]],
            &[(gone_members[1], three_anchors()), (kept_members[1], three_anchors()), (other, two_anchors())],
        );
        (s, gone, kept, kept_members[2], other)
    };
    let s = same_as_one_at_a_time(|| make().0);
    let (_, gone, kept, left, other) = make();
    let d = &s.doc().unwrap().doc;
    assert!(d.node(gone).is_none(), "the compound path whose members all went is removed");
    let kept = d.node(kept).expect("the compound path with a member left stays");
    assert!(matches!(kept.kind, NodeKind::Compound { .. }));
    assert_eq!(kept.children().unwrap().iter().map(|m| m.id).collect::<Vec<_>>(), [left]);
    assert_eq!(s.doc().unwrap().selection.objects, [other], "the cut square's other sides stay selected");
}

/// Objects inside two groups and on the layer, removed whole, emptied or cut apart between kept
/// siblings, keep the order and ids that removing them one at a time leaves.
#[test]
fn cut_inside_groups_keeps_the_order_and_ids_of_removing_one_at_a_time() {
    let make = || {
        let mut s = new_doc();
        let a = [(); 4].map(|()| rect(&mut s));
        run(&mut s, "select.set", json!({"ids": a.iter().map(|id| id.0).collect::<Vec<_>>()}));
        run(&mut s, "object.group", json!({}));
        let b = [(); 4].map(|()| rect(&mut s));
        run(&mut s, "select.set", json!({"ids": b.iter().map(|id| id.0).collect::<Vec<_>>()}));
        run(&mut s, "object.group", json!({}));
        let c = [(); 4].map(|()| rect(&mut s));
        select_direct(
            &mut s,
            &[a[0], a[2], b[3], c[1]],
            &[(a[1], two_anchors()), (b[0], three_anchors()), (b[2], two_anchors()), (c[3], three_anchors())],
        );
        s
    };
    let s = same_as_one_at_a_time(make);
    assert_eq!(s.doc().unwrap().selection.objects.len(), 2, "the two squares cut apart stay selected");
}

/// Selected ruler guides go with the objects, in the Cut's undo step.
#[test]
fn cut_removes_the_selected_guides_with_the_objects() {
    let make = || {
        let mut s = new_doc();
        let [a, b, c] = [(); 3].map(|()| rect(&mut s));
        run(&mut s, "guide.add", json!({"vertical": true, "pos": 30}));
        run(&mut s, "guide.add", json!({"vertical": false, "pos": 40}));
        select_direct(&mut s, &[a, c], &[(b, two_anchors())]);
        run(&mut s, "guide.select", json!({"indexes": [1], "toggle": true}));
        s
    };
    let s = same_as_one_at_a_time(make);
    assert_eq!(s.doc().unwrap().doc.ruler_guides().len(), 1, "the selected guide is removed");
    assert_eq!(s.clipboard.guides.len(), 1, "the selected guide is on the clipboard");
}

/// Roots that are not separate (one inside another) are removed one at a time, as before.
#[test]
fn cut_of_roots_inside_another_root_matches_removing_one_at_a_time() {
    let make = || {
        let mut s = new_doc();
        let a = [(); 3].map(|()| rect(&mut s));
        run(&mut s, "select.set", json!({"ids": a.iter().map(|id| id.0).collect::<Vec<_>>()}));
        let group = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
        let b = rect(&mut s);
        select_direct(&mut s, &[group, b], &[(a[1], two_anchors())]);
        (s, vec![a[1], group, b])
    };
    let (mut x, roots) = make();
    let (mut y, _) = make();
    copy(&mut x, &json!({})).unwrap();
    copy(&mut y, &json!({})).unwrap();
    cut_direct_selection(&mut x, &roots).unwrap();
    cut_one_at_a_time(&mut y, &roots).unwrap();
    assert_same(&x, &y, "Cut");
}

/// One layer of 10,000 squares with every fifth of the first 10,000 selected whole (2,000) and
/// anchors 0 and 1 of the square after them picked.
fn many() -> Session {
    let mut s = session(10_001, square);
    let ids = crate::tests_large_edits::layer(&s, 0);
    let whole: Vec<NodeId> = ids.iter().take(10_000).step_by(5).copied().collect();
    select_direct(&mut s, &whole, &[(ids[10_000], two_anchors())]);
    s
}

/// The tree walks [`tree_walks`] counts while `f` runs on `s`.
fn walks(s: &mut Session, f: impl FnOnce(&mut Session)) -> u64 {
    let before = tree_walks();
    f(s);
    tree_walks() - before
}

/// Cut of 2,000 objects selected whole among 10,000, with a direct selection, walks the document's
/// tree a few times, where removing them one at a time walks it several times for each object.
#[test]
fn direct_selection_cut_of_2000_of_10000_objects_takes_few_tree_walks() {
    let s = same_as_one_at_a_time(many);
    assert_eq!(crate::tests_large_edits::layer(&s, 0).len(), 8_001);
    let (mut a, mut b) = (many(), many());
    let batched = walks(&mut a, |s| {
        run(s, "edit.cut", json!({}));
    });
    let one_at_a_time = walks(&mut b, reference_cut);
    println!("tree walks: Cut {batched}, one at a time {one_at_a_time}");
    assert!(batched <= 64, "Cut walked the document's tree {batched} times");
    assert!(one_at_a_time >= 4 * batched.max(1), "one at a time walked it {one_at_a_time} times, Cut {batched}");
}

/// This thread's CPU time, where the system reports it (`/proc/thread-self/schedstat` on Linux).
fn cpu_time() -> Option<Duration> {
    let stat = std::fs::read_to_string("/proc/thread-self/schedstat").ok()?;
    stat.split_whitespace().next()?.parse().ok().map(Duration::from_nanos)
}

/// The middle of `v`.
fn median(mut v: Vec<Duration>) -> Duration {
    v.sort_unstable();
    v[v.len() / 2]
}

/// Times Cut of [`many`] and [`reference_cut`] seven times each, in turn, and prints every run's
/// CPU and wall-clock time with the medians. Run it with
/// `cargo test --release -p vectorcraft-engine --lib -- --ignored --nocapture direct_selection_cut_timing`.
#[test]
#[ignore = "timing; prints the numbers"]
fn direct_selection_cut_timing() {
    let mut runs: [(Vec<Duration>, Vec<Duration>); 2] = Default::default();
    for round in 0..7 {
        for (k, (cpu, wall)) in runs.iter_mut().enumerate() {
            let mut s = many();
            let (c0, w0) = (cpu_time(), Instant::now());
            if k == 0 {
                reference_cut(&mut s);
            } else {
                run(&mut s, "edit.cut", json!({}));
            }
            let w = w0.elapsed();
            let c = c0.zip(cpu_time()).map(|(a, b)| b.saturating_sub(a)).unwrap_or(w);
            println!("round {round} {}: cpu {c:?}, wall {w:?}", ["one at a time", "Cut"][k]);
            cpu.push(c);
            wall.push(w);
        }
    }
    for (k, (cpu, wall)) in runs.into_iter().enumerate() {
        println!("median {}: cpu {:?}, wall {:?}", ["one at a time", "Cut"][k], median(cpu), median(wall));
    }
}
