//! #2143: a ruler guide selected with objects can be the key object Align lines them up on, and
//! Distribute spaces ruler guides selected without objects.

use serde_json::{Value, json};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap())
}

fn bounds(s: &Session, id: NodeId) -> vectorcraft_geom::Rect {
    s.doc().unwrap().doc.bounds_of(&[id], false).unwrap()
}

/// A Selection tool click at (`x`, `y`), with Shift when `shift`.
fn click(s: &mut Session, x: f64, y: f64, shift: bool) {
    let (v, mods) = (ViewInfo::default(), Mods { shift, ..Default::default() });
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), v).unwrap();
    }
}

#[test]
fn a_clicked_guide_is_the_key_and_align_lines_the_objects_up_on_it() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 100.0, 100.0), rect(&mut s, 300.0, 250.0));
    s.execute("guide.add", &json!({"vertical": true, "pos": 500})).unwrap();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    // Click an object, Shift-click the guide, then click the guide again.
    click(&mut s, 110.0, 110.0, false);
    click(&mut s, 500.0, 400.0, true);
    assert_eq!((s.doc().unwrap().selection.objects.clone(), s.doc().unwrap().selection.guides.clone()), (vec![a], vec![0]));
    click(&mut s, 500.0, 400.0, false);
    assert_eq!(s.doc().unwrap().selection.key_guide, Some(0), "the guide is the key");
    assert_eq!(s.execute("document.inspect", &json!({})).unwrap()["keyGuide"], 0);
    // Shift-click a third object: the guide stays the key.
    click(&mut s, 310.0, 260.0, true);
    assert_eq!(s.doc().unwrap().selection.objects, vec![a, b]);
    assert_eq!(s.doc().unwrap().selection.key_guide, Some(0));
    let (ya, yb) = (bounds(&s, a).y0, bounds(&s, b).y0);
    s.execute("object.align", &json!({"horizontal": "center", "vertical": "center"})).unwrap();
    // Both centred on the guide's x; a vertical guide fixes no y, so they keep theirs.
    for (id, y) in [(a, ya), (b, yb)] {
        let r = bounds(&s, id);
        assert!((r.center().x - 500.0).abs() < 1e-6 && (r.y0 - y).abs() < 1e-6, "{id:?}: {r:?}");
    }
    assert_eq!(s.doc().unwrap().doc.guides[0].pos, 500.0, "the guide stays put");
    s.execute("object.align", &json!({"horizontal": "left"})).unwrap();
    assert!((bounds(&s, a).x0 - 500.0).abs() < 1e-6 && (bounds(&s, b).x0 - 500.0).abs() < 1e-6);
    // A click on the key guide again: no key.
    click(&mut s, 500.0, 400.0, false);
    assert_eq!(s.doc().unwrap().selection.key_guide, None);
}

#[test]
fn one_object_aligns_to_a_horizontal_key_guide() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0);
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert_eq!(s.execute("select.key", &json!({"guide": 0})).unwrap(), json!({"key": null, "guide": 0}));
    let x = bounds(&s, a).x0;
    s.execute("object.align", &json!({"vertical": "bottom", "horizontal": "right"})).unwrap();
    let r = bounds(&s, a);
    assert!((r.y1 - 300.0).abs() < 1e-6 && (r.x0 - x).abs() < 1e-6, "{r:?}");
    // `to` still wins.
    s.execute("object.align", &json!({"horizontal": "left", "to": "artboard"})).unwrap();
    assert!(bounds(&s, a).x0.abs() < 1e-6);
}

#[test]
fn select_key_takes_only_a_selected_guide_with_objects() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 100.0, 100.0), rect(&mut s, 300.0, 100.0));
    for pos in [500, 600] {
        s.execute("guide.add", &json!({"vertical": true, "pos": pos})).unwrap();
    }
    let err = |s: &mut Session, p: Value| assert!(s.execute("select.key", &p).is_err(), "{p}");
    s.execute("guide.select", &json!({"indexes": [1]})).unwrap();
    err(&mut s, json!({"guide": 1}));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    err(&mut s, json!({"guide": 1}));
    s.execute("guide.select", &json!({"indexes": [0, 1], "toggle": true})).unwrap();
    for bad in [json!({"guide": 7}), json!({"guide": -1}), json!({"guide": "x"}), json!({"guide": 1, "id": a.0})] {
        err(&mut s, bad);
    }
    s.execute("select.key", &json!({"guide": 1})).unwrap();
    // An object key replaces the guide key, and the other way round.
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    assert_eq!((s.doc().unwrap().selection.key, s.doc().unwrap().selection.key_guide), (Some(b), None));
    s.execute("select.key", &json!({"guide": 1})).unwrap();
    assert_eq!((s.doc().unwrap().selection.key, s.doc().unwrap().selection.key_guide), (None, Some(1)));
    // Deleting a guide before it keeps the key on the same guide.
    s.execute("guide.remove", &json!({"index": 0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.key_guide, Some(0));
    assert_eq!(s.doc().unwrap().doc.guides[0].pos, 600.0);
    // Taking the guide out of the selection, or every object, lets the key go.
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert_eq!(s.doc().unwrap().selection.key_guide, None);
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    s.execute("select.key", &json!({"guide": 0})).unwrap();
    s.execute("select.toggle", &json!({"id": a.0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.key_guide, Some(0), "one object is enough");
    s.execute("select.toggle", &json!({"id": b.0})).unwrap();
    assert_eq!(s.doc().unwrap().selection.key_guide, None, "no object to align");
}

#[test]
fn selected_anchor_points_align_to_a_key_guide() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 400})).unwrap();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    s.execute("select.key", &json!({"guide": 0})).unwrap();
    s.execute("object.align", &json!({"horizontal": "center"})).unwrap();
    let NodeKind::Path { path, .. } = &s.doc().unwrap().doc.node(a).unwrap().kind else { panic!("path") };
    let p = path.subpaths[0].anchors[0].p;
    assert_eq!(p.x, 400.0);
    assert_eq!(path.subpaths[0].anchors[1].p.x, 140.0, "the other points stay");
}

#[test]
fn selected_guides_align_to_a_clicked_key_object() {
    let mut s = session();
    let a = rect(&mut s, 100.0, 100.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 500})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 400})).unwrap();
    s.select_tool("selection", ViewInfo::default()).unwrap();
    // Click the object, Shift-click both guides, then click the object again: one object is
    // enough to be the key while guides are selected.
    click(&mut s, 110.0, 110.0, false);
    click(&mut s, 500.0, 300.0, true);
    click(&mut s, 300.0, 400.0, true);
    assert_eq!(s.doc().unwrap().selection.guides, vec![0, 1]);
    click(&mut s, 110.0, 110.0, false);
    assert_eq!(s.doc().unwrap().selection.key, Some(a), "the object is the key");
    let before = bounds(&s, a);
    s.execute("object.align", &json!({"horizontal": "center", "vertical": "center"})).unwrap();
    let guides = |s: &Session| s.doc().unwrap().doc.guides.iter().map(|g| g.pos).collect::<Vec<_>>();
    assert_eq!(guides(&s), [120.0, 110.0], "the vertical guide on the centre's x, the horizontal one on its y");
    assert_eq!(bounds(&s, a), before, "the key stays put");
    s.execute("object.align", &json!({"horizontal": "right"})).unwrap();
    assert_eq!(guides(&s), [140.0, 110.0], "horizontal alignment moves only the vertical guide");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(guides(&s), [120.0, 110.0], "one undo step");
    // Aligning to the artboard leaves the guides alone.
    s.execute("object.align", &json!({"horizontal": "left", "to": "artboard"})).unwrap();
    assert_eq!(guides(&s), [120.0, 110.0]);
    // Deselecting the guides lets the lone key go.
    s.execute("guide.select", &json!({"indexes": [0, 1], "toggle": true})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().selection.key, None);
}

#[test]
fn guides_align_to_a_key_object_among_others_and_parallel_ones_to_a_key_guide() {
    let mut s = session();
    let (a, b) = (rect(&mut s, 100.0, 100.0), rect(&mut s, 300.0, 250.0));
    for pos in [500, 600] {
        s.execute("guide.add", &json!({"vertical": true, "pos": pos})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0, 1], "toggle": true})).unwrap();
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    s.execute("object.align", &json!({"horizontal": "left"})).unwrap();
    assert_eq!(bounds(&s, a).x0, 300.0);
    assert!(s.doc().unwrap().doc.guides.iter().all(|g| g.pos == 300.0));
    // Aligning to the selection leaves guides alone.
    s.execute("object.align", &json!({"horizontal": "right", "to": "selection"})).unwrap();
    assert!(s.doc().unwrap().doc.guides.iter().all(|g| g.pos == 300.0));
    // A key guide stays put, and the other vertical guide lines up on it.
    s.execute("guide.move", &json!({"index": 1, "pos": 700})).unwrap();
    s.execute("select.key", &json!({"guide": 1})).unwrap();
    s.execute("object.align", &json!({"horizontal": "center"})).unwrap();
    assert_eq!(s.doc().unwrap().doc.guides.iter().map(|g| g.pos).collect::<Vec<_>>(), [700.0, 700.0]);
}

/// Objects 40 pt wide at `xs` (y 100) and a vertical guide at `pos` that is their key.
fn key_guide_across(xs: &[f64], pos: f64) -> (Session, Vec<NodeId>) {
    let mut s = session();
    let ids: Vec<NodeId> = xs.iter().map(|&x| rect(&mut s, x, 100.0)).collect();
    s.execute("guide.add", &json!({"vertical": true, "pos": pos})).unwrap();
    s.execute("select.set", &json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    s.execute("select.key", &json!({"guide": 0})).unwrap();
    (s, ids)
}

fn lefts(s: &Session, ids: &[NodeId]) -> Vec<f64> {
    ids.iter().map(|id| bounds(s, *id).x0).collect()
}

#[test]
fn distribute_with_a_spacing_spaces_the_objects_from_a_key_guide() {
    let (mut s, ids) = key_guide_across(&[100.0, 300.0, 500.0], 350.0);
    // Two objects are before the guide and one after it: the guide is spaced from as a key object
    // with no size would be.
    for (h, want) in [("left", [330.0, 340.0, 360.0]), ("center", [310.0, 320.0, 340.0]), ("right", [290.0, 300.0, 320.0])] {
        s.execute("object.distribute", &json!({"horizontal": h, "spacing": 10})).unwrap();
        assert_eq!(lefts(&s, &ids), want, "{h}");
        assert_eq!(s.doc().unwrap().doc.guides[0].pos, 350.0, "{h}: the guide stays put");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(lefts(&s, &ids), [100.0, 300.0, 500.0], "{h}: one undo step");
    }
    // `to: "key"` is what the Align panel sends; 0 puts every line on the guide.
    s.execute("object.distribute", &json!({"horizontal": "left", "spacing": 0, "to": "key"})).unwrap();
    assert_eq!(lefts(&s, &ids), [350.0; 3]);
    // A guide before or after every object.
    for (pos, want) in [(50.0, [60.0, 70.0, 80.0]), (600.0, [570.0, 580.0, 590.0])] {
        let (mut s, ids) = key_guide_across(&[100.0, 300.0, 500.0], pos);
        s.execute("object.distribute", &json!({"horizontal": "left", "spacing": 10})).unwrap();
        assert_eq!(lefts(&s, &ids), want, "guide at {pos}");
    }
    // Two objects are enough, and a negative spacing reverses the order.
    let (mut s, ids) = key_guide_across(&[100.0, 500.0], 300.0);
    s.execute("object.distribute", &json!({"horizontal": "left", "spacing": -20})).unwrap();
    assert_eq!(lefts(&s, &ids), [320.0, 280.0]);
}

#[test]
fn distribute_from_a_horizontal_key_guide_and_along_a_guide() {
    let mut s = session();
    let ids: Vec<NodeId> = [100.0, 200.0, 400.0].iter().map(|&y| rect(&mut s, 50.0, y)).collect();
    s.execute("guide.add", &json!({"vertical": false, "pos": 300})).unwrap();
    s.execute("select.set", &json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    s.execute("select.key", &json!({"guide": 0})).unwrap();
    let tops = |s: &Session| ids.iter().map(|id| bounds(s, *id).y0).collect::<Vec<_>>();
    s.execute("object.distribute", &json!({"vertical": "bottom", "spacing": 5})).unwrap();
    // Bottoms 120 and 220 are above the guide, 420 below it.
    assert_eq!(tops(&s), [270.0, 275.0, 285.0]);
    // A horizontal guide gives no line across a horizontal distribution: the objects stay where
    // they are, as Align leaves them, and no undo step is added.
    let undo = s.doc().unwrap().history.undo.len();
    let xs: Vec<f64> = ids.iter().map(|id| bounds(&s, *id).x0).collect();
    s.execute("object.distribute", &json!({"horizontal": "center", "spacing": 5})).unwrap();
    assert_eq!(ids.iter().map(|id| bounds(&s, *id).x0).collect::<Vec<_>>(), xs);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn distribute_without_a_spacing_spaces_the_key_guide_too() {
    // Without a spacing, the key guide is spaced with the objects as an item with no size: lines
    // 100, 150, 350 (the guide) and 400 go to 100, 200, 300 and 400.
    let (mut s, ids) = key_guide_across(&[100.0, 150.0, 400.0], 350.0);
    s.execute("object.distribute", &json!({"horizontal": "left"})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![100.0, 200.0, 400.0], vec![300.0]));
    // `to: "selection"` wins over the key guide, so a spacing isn't used either.
    let (mut s, ids) = key_guide_across(&[100.0, 150.0, 400.0], 350.0);
    s.execute("object.distribute", &json!({"horizontal": "left", "spacing": 10, "to": "selection"})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![100.0, 200.0, 400.0], vec![300.0]));
}

#[test]
fn distribute_spacing_spaces_the_objects_from_a_key_guide() {
    // Left edges 100, 300 and 500 of 40 pt objects with a vertical key guide at 350, 10 pt apart:
    // the object before the guide ends 10 pt before it, the one after it starts 10 pt after it.
    let (mut s, ids) = key_guide_across(&[100.0, 300.0, 500.0], 350.0);
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 10})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![250.0, 300.0, 360.0], vec![350.0]));
    // A guide along the axis leaves everything where it is.
    s.execute("object.distributeSpacing", &json!({"axis": "vertical", "spacing": 10})).unwrap();
    assert_eq!(lefts(&s, &ids), [250.0, 300.0, 360.0]);
    assert_eq!(bounds(&s, ids[0]).y0, 100.0);
}

#[test]
fn other_selected_guides_are_spaced_from_a_key_guide() {
    let (mut s, ids) = key_guide_across(&[100.0, 300.0, 500.0], 350.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 600})).unwrap();
    s.execute("guide.add", &json!({"vertical": false, "pos": 50})).unwrap();
    s.execute("guide.select", &json!({"indexes": [1, 2], "toggle": true})).unwrap();
    s.execute("object.distribute", &json!({"horizontal": "left", "spacing": 10})).unwrap();
    // The vertical guide at 600 is the last line, 10 pt after the last object's; the horizontal
    // guide stays.
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![330.0, 340.0, 360.0], vec![350.0, 370.0, 50.0]));
}

/// Guides at `vertical` x positions and `horizontal` y positions, all selected without objects.
fn guides_alone(vertical: &[f64], horizontal: &[f64]) -> Session {
    let mut s = session();
    for (v, pos) in vertical.iter().map(|p| (true, p)).chain(horizontal.iter().map(|p| (false, p))) {
        s.execute("guide.add", &json!({"vertical": v, "pos": pos})).unwrap();
    }
    let all: Vec<usize> = (0..vertical.len() + horizontal.len()).collect();
    s.execute("guide.select", &json!({"indexes": all})).unwrap();
    s
}

/// Selects the objects `ids` and the ruler guides `guides`, and nothing else.
fn select_with_guides(s: &mut Session, ids: &[NodeId], guides: &[usize]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|id| id.0).collect::<Vec<_>>()})).unwrap();
    let now = s.doc().unwrap().selection.guides.clone();
    let flip: Vec<usize> = (0..s.doc().unwrap().doc.guides.len()).filter(|i| now.contains(i) != guides.contains(i)).collect();
    s.execute("guide.select", &json!({"indexes": flip, "toggle": true})).unwrap();
    assert_eq!((s.doc().unwrap().selection.objects.len(), s.doc().unwrap().selection.guides.len()), (ids.len(), guides.len()));
}

fn positions(s: &Session) -> Vec<f64> {
    s.doc().unwrap().doc.guides.iter().map(|g| g.pos).collect()
}

#[test]
fn distribute_spaces_guides_selected_without_objects() {
    let mut s = guides_alone(&[100.0, 150.0, 500.0], &[50.0, 60.0, 400.0]);
    // A guide has no width: the three options of an axis give the same result, and only the
    // guides across the axis move.
    for h in ["left", "center", "right"] {
        s.execute("object.distribute", &json!({"horizontal": h})).unwrap();
        assert_eq!(positions(&s), [100.0, 300.0, 500.0, 50.0, 60.0, 400.0], "{h}");
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(positions(&s), [100.0, 150.0, 500.0, 50.0, 60.0, 400.0], "{h}: one undo step");
    }
    s.execute("object.distribute", &json!({"vertical": "center"})).unwrap();
    assert_eq!(positions(&s), [100.0, 150.0, 500.0, 50.0, 225.0, 400.0]);
    // Align to Artboard: the outermost guides go to its edges (the document is 800 by 600).
    s.execute("object.distribute", &json!({"horizontal": "center", "to": "artboard", "artboard": 0})).unwrap();
    assert_eq!(&positions(&s)[..3], [0.0, 400.0, 800.0]);
    // Distribute Spacing: equal gaps, or `spacing` apart from the first.
    let mut s = guides_alone(&[100.0, 150.0, 500.0], &[]);
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal"})).unwrap();
    assert_eq!(positions(&s), [100.0, 300.0, 500.0]);
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 10})).unwrap();
    assert_eq!(positions(&s), [100.0, 110.0, 120.0]);
    s.execute("object.distributeSpacing", &json!({"axis": "vertical", "spacing": 10})).unwrap();
    assert_eq!(positions(&s), [100.0, 110.0, 120.0], "no horizontal guide to space");
}

#[test]
fn distributing_guides_needs_two_items() {
    // Two guides are the ends already; one guide can't be distributed.
    let mut s = guides_alone(&[100.0, 500.0], &[]);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("object.distribute", &json!({"horizontal": "center"})).unwrap();
    assert_eq!((positions(&s), s.doc().unwrap().history.undo.len()), (vec![100.0, 500.0], undo));
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    assert!(s.execute("object.distribute", &json!({"horizontal": "center"})).is_err());
    // One object and one guide are enough for Distribute Spacing.
    let a = rect(&mut s, 300.0, 100.0);
    select_with_guides(&mut s, &[a], &[0]);
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 10})).unwrap();
    assert_eq!((positions(&s), bounds(&s, a).x0), (vec![100.0, 500.0], 110.0));
}

#[test]
fn guides_selected_with_objects_are_distributed_with_them() {
    // 40 pt objects at 100 and 300, vertical guides at 150 and 400, and a horizontal guide.
    let mut s = guides_alone(&[150.0, 400.0], &[50.0]);
    let ids: Vec<NodeId> = [100.0, 300.0].iter().map(|&x| rect(&mut s, x, 100.0)).collect();
    select_with_guides(&mut s, &ids, &[0, 1, 2]);
    // Lines 100, 150 (a guide), 300 and 400 (a guide): the guide between the objects goes to
    // 200, and the outermost items stay. The horizontal guide stays too.
    s.execute("object.distribute", &json!({"horizontal": "left"})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![100.0, 300.0], vec![200.0, 400.0, 50.0]));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(positions(&s), [150.0, 400.0, 50.0], "one undo step");
    // Distribute Spacing: a guide has no width. From 100 to 360 there are 260 pt, 80 of them
    // objects: three gaps of 60 pt.
    s.execute("guide.move", &json!({"index": 1, "pos": 360})).unwrap();
    select_with_guides(&mut s, &ids, &[0, 1, 2]);
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal"})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![100.0, 260.0], vec![200.0, 360.0, 50.0]));
    // Align to Artboard: the outermost items go to its edges (the document is 800 wide).
    s.execute("object.distribute", &json!({"horizontal": "left", "to": "artboard", "artboard": 0})).unwrap();
    assert_eq!(lefts(&s, &ids)[0], 0.0);
    assert_eq!(positions(&s)[1], 800.0);
}

#[test]
fn selected_guides_are_spaced_from_a_key_object() {
    // Lines 100, 150 (a guide) and 300 (the key object), 10 pt apart outward from the key.
    let mut s = guides_alone(&[150.0], &[]);
    let ids: Vec<NodeId> = [100.0, 300.0].iter().map(|&x| rect(&mut s, x, 100.0)).collect();
    select_with_guides(&mut s, &ids, &[0]);
    s.execute("select.key", &json!({"id": ids[1].0})).unwrap();
    s.execute("object.distribute", &json!({"horizontal": "left", "spacing": 10})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![280.0, 300.0], vec![290.0]));
    // Distribute Spacing: the object ends 10 pt before the guide, the guide 10 pt before the key.
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 10})).unwrap();
    assert_eq!((lefts(&s, &ids), positions(&s)), (vec![240.0, 300.0], vec![290.0]));
}
