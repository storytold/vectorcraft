//! #2143: a ruler guide selected with objects can be the key object Align lines them up on.

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
