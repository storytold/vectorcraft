//! Clear deletes its exact targets, including compound members (#786).

use std::collections::BTreeSet;
use std::time::{Duration, Instant};

use serde_json::json;
use vectorcraft_doc::{NodeId, Selection};
use vectorcraft_engine::{EngineError, Session};

fn compound() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut rectangle =
        |x| NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let a = rectangle(10);
    let b = rectangle(50);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let c = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    (s, a, b, c)
}

/// The two right-hand corners of a square are indexes 1 and 2. Delete must remove those corners
/// and leave the left side, not a diagonal from shifting indexes as the path opens.
#[test]
fn clear_deletes_every_selected_anchor_of_a_closed_path() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let points = |s: &Session| {
        s.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect::<Vec<_>>()
    };
    let before = points(&s);
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1], [0, 2]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    let sp = &s.active().unwrap().doc.node(id).unwrap().path_data().unwrap().subpaths[0];
    assert!(!sp.closed);
    assert_eq!(points(&s), vec![before[3], before[0]]);
    assert_eq!(s.active().unwrap().selection.objects, vec![id]);
    assert!(s.active().unwrap().selection.anchors.is_empty());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(id).is_none(), "a second Delete removes the remainder");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(points(&s), before);
    // One corner still opens the square into the other three, in order.
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 1]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(points(&s), vec![before[2], before[3], before[0]]);
}

/// A marquee that covers one path completely and only some anchors of another stores the first as
/// a whole object. Delete removes that object and the selected anchors of the other.
#[test]
fn clear_deletes_fully_selected_objects_along_with_partial_anchors() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let mut rectangle =
        |x| NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let whole = rectangle(10);
    let partial = rectangle(80);
    let refs = |s: &Session, id| {
        s.active().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(si, ai, _)| vec![si, ai]).collect::<Vec<_>>()
    };
    let all = refs(&s, whole);
    let one = refs(&s, partial)[0].clone();
    let before = refs(&s, partial).len();
    s.execute("select.anchorsMany", &json!({"items": [{"id": whole.0, "anchors": all}, {"id": partial.0, "anchors": [one]}]})).unwrap();
    assert!(s.active().unwrap().selection.contains(whole) && s.active().unwrap().selection.partial(whole).is_none());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(whole).is_none());
    assert_eq!(refs(&s, partial).len(), before - 1);
    assert_eq!(s.active().unwrap().selection.objects, vec![partial]);
    assert!(s.active().unwrap().selection.anchors.is_empty());
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(partial).is_none(), "a second Delete removes what the first left");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(whole).is_some());
    assert_eq!(refs(&s, partial).len(), before);
}

#[test]
fn clear_selected_compound_member_preserves_its_sibling_and_is_undoable() {
    let (mut s, a, b, c) = compound();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    assert!(s.active().unwrap().selection.is_empty());
    assert_eq!(s.execute("edit.undo", &json!({})).unwrap()["undone"], "Clear");
    assert_eq!(s.active().unwrap().doc.parent_of(a), Some(c));
    assert_eq!(s.active().unwrap().selection.objects, vec![a]);
    s.execute("edit.redo", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_explicit_ids_works_without_a_selection_and_overrides_selected_anchors() {
    let (mut s, a, b, c) = compound();
    s.execute("select.none", &json!({})).unwrap();
    let info = s.commands().into_iter().find(|info| info.id == "edit.clear").unwrap();
    assert!(!info.enabled, "the menu stays disabled with no selection");
    s.execute("edit.clear", &json!({"ids": [a.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("select.anchors", &json!({"id": b.0, "anchors": [[0, 0]]})).unwrap();
    let sibling = s.active().unwrap().doc.node(b).unwrap().clone();
    s.execute("edit.clear", &json!({"ids": [a.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.node(b).unwrap(), &sibling);
}

#[test]
fn clear_explicit_targets_are_validated_atomically() {
    let (mut s, a, _, _) = compound();
    s.execute("select.none", &json!({})).unwrap();
    for ids in [json!([a.0, 999]), json!([a.0, -1]), json!([a.0, "bad"]), json!([a.0, 2.5]), json!(null), json!(a.0)] {
        let doc = s.active().unwrap().doc.clone();
        let journal = s.journal.clone();
        assert!(matches!(s.execute("edit.clear", &json!({"ids": ids})), Err(EngineError::BadParams { .. })), "{ids}");
        assert_eq!(s.active().unwrap().doc, doc);
        assert_eq!(s.journal, journal);
    }
    assert!(s.execute("edit.clear", &json!({})).is_err());
    assert!(Session::new().execute("edit.clear", &json!({"ids": [a.0]})).is_err());
}

#[test]
fn clear_compound_and_member_together_deletes_the_compound() {
    for explicit in [false, true] {
        let (mut s, a, b, c) = compound();
        s.execute("select.set", &json!({"ids": [a.0, c.0]})).unwrap();
        s.execute("edit.clear", &if explicit { json!({"ids": [a.0, c.0, c.0]}) } else { json!({}) }).unwrap();
        for id in [a, b, c] {
            assert!(s.active().unwrap().doc.node(id).is_none());
        }
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc.parent_of(a), Some(c));
        assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    }
}

#[test]
fn cut_compound_member_keeps_copy_and_delete_targets_consistent() {
    let (mut s, a, b, c) = compound();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("edit.cut", &json!({})).unwrap();
    // Cut takes the member it copies, and the other member stays in the compound.
    assert_eq!(s.clipboard.nodes.len(), 1);
    assert_eq!(s.clipboard.nodes[0].id, a);
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
    s.execute("edit.pasteInPlace", &json!({})).unwrap();
    let pasted = s.active().unwrap().selection.objects[0];
    assert!(s.active().unwrap().doc.node(pasted).unwrap().path_data().is_some(), "an ordinary path is pasted");
    s.execute("edit.undo", &json!({})).unwrap();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.parent_of(a), Some(c));
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_direct_selected_anchors_keeps_the_rest_of_the_path() {
    let (mut s, a, b, c) = compound();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().path_data().unwrap().anchor_count(), 3);
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_guides_and_empty_explicit_targets_keep_their_separate_behavior() {
    let (mut s, a, _, _) = compound();
    s.execute("guide.add", &json!({"vertical": true, "pos": 10})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    let history = s.active().unwrap().history.undo.len();
    let selection = s.active().unwrap().selection.clone();
    s.execute("edit.clear", &json!({"ids": []})).unwrap();
    assert_eq!(s.active().unwrap().history.undo.len(), history);
    assert_eq!(s.active().unwrap().selection, selection);
    assert_eq!(s.active().unwrap().doc.ruler_guides().len(), 1);
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.ruler_guides().is_empty());
    assert!(s.active().unwrap().doc.node(a).is_some());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.ruler_guides().len(), 1);
}

#[test]
fn clear_keeps_layers_but_deletes_an_explicitly_targeted_child() {
    let (mut s, a, b, c) = compound();
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("select.none", &json!({})).unwrap();
    s.execute("edit.clear", &json!({"ids": [layer.0, a.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(layer).is_some());
    assert!(s.active().unwrap().doc.node(a).is_none());
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn cut_direct_selected_anchors_retains_anchor_deletion() {
    let (mut s, a, b, c) = compound();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]]})).unwrap();
    s.execute("edit.cut", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().path_data().unwrap().anchor_count(), 3);
    assert_eq!(s.active().unwrap().doc.parent_of(b), Some(c));
}

#[test]
fn clear_explicit_ids_preserves_unrelated_selection() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r1 = NodeId(s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    let r2 = NodeId(s.execute("shape.rectangle", &json!({"x": 50, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [r1.0, r2.0]})).unwrap();
    assert_eq!(s.active().unwrap().selection.objects, vec![r1, r2]);

    // Clearing r1 explicitly should remove r1 from doc and from selection, keeping r2 selected.
    s.execute("edit.clear", &json!({"ids": [r1.0]})).unwrap();
    assert!(s.active().unwrap().doc.node(r1).is_none());
    assert!(s.active().unwrap().doc.node(r2).is_some());
    assert_eq!(s.active().unwrap().selection.objects, vec![r2]);

    // Parameterless clear clears the rest of the selection.
    s.execute("edit.clear", &json!({})).unwrap();
    assert!(s.active().unwrap().doc.node(r2).is_none());
    assert!(s.active().unwrap().selection.is_empty());
}

fn rectangles(s: &mut Session, n: i64) -> Vec<NodeId> {
    (0..n)
        .map(|i| {
            NodeId(s.execute("shape.rectangle", &json!({"x": 10 + 40 * i, "y": 10, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap())
        })
        .collect()
}

/// Clear with `ids` leaves the selection that removing each id on its own leaves: the other objects
/// in their order, their anchor and segment picks, a key object or key anchor that is not removed,
/// and the selected guides. Undo brings back the document and the selection, and Redo the result.
#[test]
fn clear_explicit_ids_leaves_what_removing_each_id_leaves() {
    for key_anchor in [false, true] {
        for removed in [[1, 4, 1, 6], [0, 2, 2, 6]] {
            let mut s = Session::new();
            s.execute("file.new", &json!({})).unwrap();
            let r = rectangles(&mut s, 7);
            s.execute("guide.add", &json!({"vertical": true, "pos": 10})).unwrap();
            let guide = s.active().unwrap().doc.ruler_guides()[0].id.unwrap();
            let sel = &mut s.active_mut().unwrap().selection;
            // The seventh rectangle is not selected: Clear removes it from the document only.
            sel.set(r[..6].iter().copied());
            sel.guides = vec![guide];
            for (id, anchor) in [(r[1], (0, 1)), (r[2], (0, 2)), (r[4], (0, 0)), (r[5], (0, 2))] {
                sel.anchors.insert(id, BTreeSet::from([anchor, (0, 1)]));
            }
            for id in [r[2], r[5]] {
                sel.segments.insert(id, BTreeSet::from([(0, 1)]));
            }
            if key_anchor {
                sel.key_anchor = Some((r[4], (0, 0)));
            } else {
                sel.key = Some(r[4]);
            }
            let before = sel.clone();
            // The selection is one that `prune` keeps as it is, so Undo can restore it unchanged.
            let mut pruned = before.clone();
            pruned.prune(&s.active().unwrap().doc);
            assert_eq!(pruned, before);
            let doc_before = s.active().unwrap().doc.clone();
            let ids: Vec<NodeId> = removed.iter().map(|i| r[*i]).collect();
            s.execute("edit.clear", &json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
            let st = s.active().unwrap();
            let mut expected = before.clone();
            for id in &ids {
                expected.remove(*id);
            }
            expected.prune(&st.doc);
            assert_eq!(st.selection, expected, "key anchor {key_anchor}, removed {removed:?}");
            assert_eq!(st.selection.guides, vec![guide]);
            assert!(ids.iter().all(|id| st.doc.node(*id).is_none()));
            let doc_after = st.doc.clone();
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, doc_before);
            assert_eq!(s.active().unwrap().selection, before);
            s.execute("edit.redo", &json!({})).unwrap();
            assert_eq!(s.active().unwrap().doc, doc_after);
            assert_eq!(s.active().unwrap().selection, expected);
        }
    }
}

/// Cut of many selected objects puts each of them on the clipboard in paint order, removes them and
/// the selected guide, and leaves nothing selected. One Undo brings all of it back.
#[test]
fn cut_of_many_objects_takes_them_all_out_of_the_selection() {
    let mut s = Session::new();
    s.execute("file.new", &json!({})).unwrap();
    let r = rectangles(&mut s, 40);
    s.execute("guide.add", &json!({"vertical": true, "pos": 10})).unwrap();
    let guide = s.active().unwrap().doc.ruler_guides()[0].id.unwrap();
    let ids: Vec<u64> = r.iter().rev().map(|id| id.0).collect();
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    let sel = &mut s.active_mut().unwrap().selection;
    sel.guides = vec![guide];
    sel.key = Some(r[7]);
    let before = sel.clone();
    let doc_before = s.active().unwrap().doc.clone();
    s.execute("edit.cut", &json!({})).unwrap();
    assert_eq!(s.clipboard.nodes.iter().map(|n| n.id).collect::<Vec<_>>(), r);
    let st = s.active().unwrap();
    assert!(r.iter().all(|id| st.doc.node(*id).is_none()));
    assert!(st.doc.ruler_guides().is_empty());
    assert_eq!(st.selection, Selection::default());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc, doc_before);
    assert_eq!(s.active().unwrap().selection, before);
}

/// Timings of taking the cut objects out of a selection of 2,000, 20,000 and 50,000 objects: one
/// [`Selection::remove`] per id, as Clear with `ids` did before, against one
/// [`Selection::remove_all`], for all of the objects and for every other one. The two run in turn
/// in one process, 7 times each, and the two results are compared field by field. It also times
/// Cut of every object, with Undo between the runs. Ignored by default; it prints its timings and
/// sets no bound on them:
/// `cargo test --release -p vectorcraft-engine --test it clear_selection_perf -- --ignored --nocapture`.
#[test]
#[ignore = "a benchmark: run with --ignored --nocapture"]
fn clear_selection_perf() {
    fn ms(d: &[Duration]) -> String {
        d.iter().map(|d| format!("{:.3}", d.as_secs_f64() * 1000.0)).collect::<Vec<_>>().join(" ")
    }
    fn median(d: &mut [Duration]) -> f64 {
        d.sort();
        d[d.len() / 2].as_secs_f64() * 1000.0
    }
    for n in [2_000_i64, 20_000, 50_000] {
        let mut b = vectorcraft_testkit::fixtures::DocBuilder::new(4000.0, 4000.0);
        let r: Vec<NodeId> = (0..n)
            .map(|i| {
                let (x, y) = ((i % 400) as f64 * 10.0, (i / 400) as f64 * 10.0);
                b.rect(vectorcraft_geom::Rect::new(x, y, x + 8.0, y + 8.0), vectorcraft_color::Color::rgb(0.2, 0.4, 0.6), |_| {})
            })
            .collect();
        let doc = b.build();
        let mut sel = Selection::default();
        sel.set(r.iter().copied());
        for id in r.iter().step_by(10) {
            sel.anchors.insert(*id, BTreeSet::from([(0, 0)]));
        }
        sel.key = Some(r[r.len() / 2]);
        for (what, ids) in [("all", r.clone()), ("every other", r.iter().copied().step_by(2).collect::<Vec<_>>())] {
            let (mut each, mut all) = (Vec::new(), Vec::new());
            for _ in 0..7 {
                let mut one_by_one = sel.clone();
                let start = Instant::now();
                for id in &ids {
                    one_by_one.remove(*id);
                }
                one_by_one.prune(&doc);
                each.push(start.elapsed());
                let mut at_once = sel.clone();
                let start = Instant::now();
                at_once.remove_all(ids.iter().copied());
                at_once.prune(&doc);
                all.push(start.elapsed());
                assert_eq!(at_once, one_by_one);
            }
            eprintln!("clear_selection_perf: {n} selected, {what} ({} ids), remove each ms: {}", ids.len(), ms(&each));
            eprintln!("clear_selection_perf: {n} selected, {what} ({} ids), remove_all ms:  {}", ids.len(), ms(&all));
            eprintln!(
                "clear_selection_perf: {n} selected, {what}: median remove each {:.3} ms, remove_all {:.3} ms",
                median(&mut each),
                median(&mut all)
            );
        }
        let mut s = Session::new();
        let data = vectorcraft_format::base64_encode(&vectorcraft_format::save(&doc, false));
        s.execute("document.open", &json!({"name": "perf.vectorcraft", "dataBase64": data})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
        let mut cut = Vec::new();
        for _ in 0..7 {
            let start = Instant::now();
            s.execute("edit.cut", &json!({})).unwrap();
            cut.push(start.elapsed());
            assert!(s.active().unwrap().selection.objects.is_empty());
            s.execute("edit.undo", &json!({})).unwrap();
            assert_eq!(s.active().unwrap().selection.objects.len(), r.len());
        }
        eprintln!("clear_selection_perf: {n} selected, edit.cut ms: {}", ms(&cut));
        eprintln!("clear_selection_perf: {n} selected, edit.cut median {:.3} ms", median(&mut cut));
    }
}
