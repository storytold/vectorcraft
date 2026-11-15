//! #1636: the key anchor. With Direct Selection, the point a Shift-click adds to two or more
//! selected anchor points is the key anchor; Align moves the other points to it, Distribute with a
//! spacing places them from it, and it stays in place.

use serde_json::json;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn path(s: &mut Session, d: &str) -> NodeId {
    NodeId(s.execute("path.create", &json!({"d": d})).unwrap()["id"].as_u64().unwrap())
}

fn pts(s: &Session, id: NodeId) -> Vec<(f64, f64)> {
    s.doc().unwrap().doc.node(id).unwrap().path_data().unwrap().anchors().map(|(_, _, a)| (a.p.x, a.p.y)).collect()
}

fn key_anchor(s: &Session) -> Option<(NodeId, (usize, usize))> {
    s.doc().unwrap().selection.key_anchor
}

/// A Direct Selection click at (`x`, `y`), with Shift when `shift`.
fn click(s: &mut Session, x: f64, y: f64, shift: bool) {
    let (v, mods) = (ViewInfo::default(), Mods { shift, ..Default::default() });
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, x, y).with_mods(mods), v).unwrap();
    }
}

#[test]
fn a_shift_click_makes_the_key_anchor_and_a_plain_click_narrows_the_selection() {
    let mut s = session();
    let id = path(&mut s, "M100 40 L200 60 L150 160 Z");
    s.select_tool("directSelection", ViewInfo::default()).unwrap();
    click(&mut s, 100.0, 40.0, false);
    assert_eq!(key_anchor(&s), None, "one point: no key anchor");
    click(&mut s, 150.0, 160.0, true);
    assert_eq!(key_anchor(&s), Some((id, (0, 2))), "the point the Shift-click added");
    click(&mut s, 200.0, 60.0, true);
    assert_eq!(key_anchor(&s), Some((id, (0, 1))), "the last point added");
    let inspect = s.execute("document.inspect", &json!({})).unwrap();
    assert_eq!(inspect["keyAnchor"], json!({"id": id.0, "anchor": [0, 1]}));
    // A Shift-click on the key anchor deselects it, and there is no key anchor.
    click(&mut s, 200.0, 60.0, true);
    assert_eq!(key_anchor(&s), None);
    click(&mut s, 200.0, 60.0, true);
    // A plain click on one of the selected points selects that point alone.
    click(&mut s, 100.0, 40.0, false);
    let sel = &s.doc().unwrap().selection;
    assert_eq!(sel.anchors.get(&id).map(|a| a.iter().copied().collect::<Vec<_>>()), Some(vec![(0, 0)]));
    assert_eq!(sel.key_anchor, None);
}

#[test]
fn align_moves_the_other_points_to_the_key_anchor() {
    let mut s = session();
    let id = path(&mut s, "M100 40 L200 60 L150 160 L60 120 Z");
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    assert_eq!(key_anchor(&s), None, "a set selects no key anchor");
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "add"})).unwrap();
    assert_eq!(key_anchor(&s), Some((id, (0, 2))));
    s.execute("object.align", &json!({"horizontal": "left"})).unwrap();
    assert_eq!(pts(&s, id), [(150.0, 40.0), (150.0, 60.0), (150.0, 160.0), (60.0, 120.0)], "to the key anchor's x");
    s.execute("object.align", &json!({"vertical": "bottom"})).unwrap();
    assert_eq!(pts(&s, id), [(150.0, 160.0), (150.0, 160.0), (150.0, 160.0), (60.0, 120.0)], "the key anchor stays");
    let steps = super::history(&s);
    s.execute("object.align", &json!({"vertical": "top"})).unwrap();
    assert_eq!(super::history(&s), steps, "nothing moved: no undo step");
    // `to: "selection"` aligns to the points' bounds instead.
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 3]], "mode": "add"})).unwrap();
    s.execute("object.align", &json!({"horizontal": "left", "to": "selection"})).unwrap();
    assert_eq!(pts(&s, id).iter().map(|p| p.0).collect::<Vec<_>>(), [60.0; 4]);
}

#[test]
fn distribute_with_a_spacing_places_the_points_from_the_key_anchor() {
    let mut s = session();
    let id = path(&mut s, "M10 10 L20 50 L100 30 L60 90");
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 2]], "mode": "toggle"})).unwrap();
    s.execute("object.distributeSpacing", &json!({"axis": "horizontal", "spacing": 20})).unwrap();
    assert_eq!(pts(&s, id), [(60.0, 10.0), (80.0, 50.0), (100.0, 30.0), (60.0, 90.0)], "20 apart, outward from the key anchor");
    s.execute("object.distribute", &json!({"vertical": "center", "spacing": 5})).unwrap();
    assert_eq!(pts(&s, id), [(60.0, 25.0), (80.0, 35.0), (100.0, 30.0), (60.0, 90.0)], "in their order along y, from y 30");
    // Without a spacing, Distribute spaces the points between the first and the last.
    s.execute("object.distribute", &json!({"horizontal": "center"})).unwrap();
    assert_eq!(pts(&s, id)[1].0, 80.0);
}

#[test]
fn the_key_anchor_goes_when_it_can_no_longer_be_one() {
    let mut s = session();
    let a = path(&mut s, "M100 40 L200 60 L150 160 Z");
    let b = path(&mut s, "M300 300 L400 300");
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    s.execute("select.anchors", &json!({"id": b.0, "anchors": [[0, 1]], "mode": "add"})).unwrap();
    assert_eq!(key_anchor(&s), Some((b, (0, 1))), "a point of another path");
    assert!(s.doc().unwrap().selection.has_key());
    let r = s.execute("select.key", &json!({})).unwrap();
    assert_eq!((r["key"].clone(), r.get("keyAnchor")), (json!(null), None), "select.key {{}} cancels it");
    assert_eq!(key_anchor(&s), None);
    let r = s.execute("select.key", &json!({"id": b.0, "anchor": [0, 1]})).unwrap();
    assert_eq!(r["keyAnchor"], json!({"id": b.0, "anchor": [0, 1]}));
    // Deleting the key anchor's path leaves no key anchor.
    s.execute("edit.clear", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(key_anchor(&s), None);
    // A key object replaces a key anchor.
    let c = path(&mut s, "M500 500 L600 500");
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 1]], "mode": "add"})).unwrap();
    assert_eq!(key_anchor(&s), Some((a, (0, 1))), "beside a path selected whole");
    s.execute("select.key", &json!({"id": c.0})).unwrap();
    let sel = &s.doc().unwrap().selection;
    assert_eq!((sel.key, sel.key_anchor), (Some(c), None));
    // Type selected whole has no points to align, so one anchor beside it is no key anchor.
    let t = NodeId(s.execute("text.create", &json!({"x": 40, "y": 400, "size": 12, "text": "Key"})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [t.0]})).unwrap();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 1]], "mode": "add"})).unwrap();
    assert_eq!(key_anchor(&s), None);
}

#[test]
fn select_key_rejects_an_anchor_that_cannot_be_the_key() {
    let mut s = session();
    let a = path(&mut s, "M100 40 L200 60 L150 160 Z");
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "set"})).unwrap();
    let err = |s: &mut Session, p: serde_json::Value| s.execute("select.key", &p).expect_err(&p.to_string()).to_string();
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, 0]})).contains("select another anchor point"), "the only point");
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 9]], "mode": "add"})).unwrap();
    assert_eq!(key_anchor(&s), None, "a point the path does not have");
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, 0]})).contains("select another anchor point"), "the only point on the path");
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, 2]})).contains("is not a selected anchor point"));
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, 9]})).contains("is not a selected anchor point"));
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0]})).contains("expected [subpath, anchor]"));
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, -1]})).contains("non-negative integer"));
    assert!(err(&mut s, json!({"anchor": [0, 1]})).contains("needs `id`"));
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, 1], "guide": 0})).contains("not both"));
    // A selected segment's ends are not selected anchor points.
    s.execute("select.segments", &json!({"id": a.0, "segments": [[0, 0]], "mode": "set"})).unwrap();
    assert!(err(&mut s, json!({"id": a.0, "anchor": [0, 0]})).contains("is not a selected anchor point"));
    assert_eq!(key_anchor(&s), None, "the key stays as it was");
}

#[test]
fn a_rejected_select_key_keeps_the_key_anchor() {
    let mut s = session();
    let a = path(&mut s, "M100 40 L200 60 L150 160 Z");
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0], [0, 9]], "mode": "set"})).unwrap();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 1]], "mode": "add"})).unwrap();
    assert_eq!(key_anchor(&s), Some((a, (0, 1))));
    // [0, 9] is in the selection but not on the path.
    let err = s.execute("select.key", &json!({"id": a.0, "anchor": [0, 9]})).expect_err("a point the path does not have").to_string();
    assert!(err.contains("is not a selected anchor point"), "{err}");
    assert_eq!(key_anchor(&s), Some((a, (0, 1))), "the key stays as it was");
}
