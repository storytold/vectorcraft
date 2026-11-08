//! Tests for Pathfinder panel › Trap (`object.pathfinder.trap`).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

const YELLOW: Color = Color::Cmyk { c: 0.0, m: 0.0, y: 1.0, k: 0.0 };
const BLUE: Color = Color::Cmyk { c: 1.0, m: 0.6, y: 0.0, k: 0.0 };

/// A yellow rectangle (0..50) abutting a blue one (50..100), both 40 pt tall, both selected.
fn abutting() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let mut rect = |x: f64, c: Color| {
        let id = NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 0, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap());
        s.edit("fill", |d, _| {
            if let Some(n) = d.node_mut(id) {
                n.appearance.set_fill(Paint::solid(c));
            }
            Ok(())
        })
        .unwrap();
        id
    };
    let (a, b) = (rect(0.0, YELLOW), rect(50.0, BLUE));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    (s, a, b)
}

fn ids(v: &Value) -> Vec<NodeId> {
    v["ids"].as_array().unwrap().iter().map(|x| NodeId(x.as_u64().unwrap())).collect()
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 0.02
}

/// The traps of the trap group the command made.
fn traps(s: &Session, v: &Value) -> Vec<Node> {
    let g = node(s, ids(v)[0]);
    assert!(matches!(g.kind, NodeKind::Group { .. }));
    g.children().unwrap().iter().map(|c| (**c).clone()).collect()
}

#[test]
fn trap_spreads_the_lighter_colour_into_the_darker_along_the_shared_edge() {
    let (mut s, a, b) = abutting();
    let before = s.doc().unwrap().doc.clone();
    let undo_len = s.doc().unwrap().history.undo.len();
    let r = s.execute("object.pathfinder.trap", &json!({"thickness": 2, "tintReduction": 100})).unwrap();
    let t = traps(&s, &r);
    assert_eq!(t.len(), 1);
    let bb = t[0].geometric_bounds().unwrap();
    assert!(near(bb.x0, 50.0) && near(bb.x1, 52.0) && near(bb.y0, 0.0) && near(bb.y1, 40.0), "2 pt into the blue: {bb:?}");
    let fill = t[0].appearance.fill().unwrap();
    assert_eq!(fill.paint.color(), Some(YELLOW), "filled with the lighter colour");
    assert!(fill.overprint, "set to overprint");
    assert!(t[0].appearance.stroke_paint().is_none());
    // The originals stay; the traps sit directly above the topmost one and are selected.
    let st = s.doc().unwrap();
    assert_eq!(node(&s, a), *before.node(a).unwrap());
    assert_eq!(node(&s, b), *before.node(b).unwrap());
    let (parent, ib, _) = st.doc.position(b).unwrap();
    let (gparent, ig, _) = st.doc.position(ids(&r)[0]).unwrap();
    assert_eq!((gparent, ig), (parent, ib + 1));
    assert_eq!(st.selection.objects, ids(&r));
    // One undo step.
    assert_eq!(st.history.undo.len(), undo_len + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().doc, before);
}

#[test]
fn height_width_and_tint_reduction_shape_and_tint_the_trap() {
    let (mut s, _, _) = abutting();
    let r = s.execute("object.pathfinder.trap", &json!({"thickness": "1 pt", "tintReduction": 25})).unwrap();
    let t = traps(&s, &r);
    let [_, _, y, _] = t[0].appearance.fill_paint().color().unwrap().to_cmyk();
    assert!((y - 0.25).abs() < 1e-4, "yellow at 25%: {y}");
    let bb = t[0].geometric_bounds().unwrap();
    assert!(near(bb.x1, 51.0), "{bb:?}");
}

#[test]
fn reverse_traps_spread_the_darker_colour_into_the_lighter() {
    let (mut s, _, _) = abutting();
    let r = s.execute("object.pathfinder.trap", &json!({"thickness": 2, "tintReduction": 100, "reverseTraps": true})).unwrap();
    let t = traps(&s, &r);
    let bb = t[0].geometric_bounds().unwrap();
    assert!(near(bb.x0, 48.0) && near(bb.x1, 50.0), "2 pt into the yellow: {bb:?}");
    assert_eq!(t[0].appearance.fill_paint().color(), Some(BLUE));
}

#[test]
fn no_adjoining_edges_is_a_clear_error_that_changes_nothing() {
    let (mut s, a, b) = abutting();
    s.edit("apart", |d, _| {
        if let Some(n) = d.node_mut(b) {
            n.kind = NodeKind::Path {
                path: vectorcraft_geom::shapes::rectangle(Rect::new(200.0, 0.0, 250.0, 40.0)),
                rule: vectorcraft_geom::FillRule::NonZero,
                live: None,
                clipping: false,
                guide: false,
            };
        }
        Ok(())
    })
    .unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let before = s.doc().unwrap().doc.clone();
    let undo_len = s.doc().unwrap().history.undo.len();
    let e = s.execute("object.pathfinder.trap", &json!({})).unwrap_err().to_string();
    assert!(e.contains("no edges of different colours meet"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo_len);
    // One object alone has nothing to trap with.
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let e = s.execute("object.pathfinder.trap", &json!({})).unwrap_err().to_string();
    assert!(e.contains("two or more objects"), "{e}");
    // Given as ids, the objects are selected for the command, and its failure puts the
    // selection back.
    s.execute("select.none", &json!({})).unwrap();
    let e = s.execute("object.pathfinder.trap", &json!({"ids": [a.0, b.0]})).unwrap_err().to_string();
    assert!(e.contains("no edges of different colours meet"), "{e}");
    assert!(s.doc().unwrap().selection.objects.is_empty(), "{:?}", s.doc().unwrap().selection.objects);
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert!(s.execute("object.pathfinder.trap", &json!({"ids": [a.0, b.0]})).is_err());
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().doc, before);
}

#[test]
fn out_of_range_and_unreadable_numbers_are_refused_or_defaulted() {
    let (mut s, _, _) = abutting();
    for p in [json!({"thickness": 1e300}), json!({"thickness": 0}), json!({"heightWidth": 0}), json!({"tintReduction": 101})] {
        assert!(s.execute("object.pathfinder.trap", &p).is_err(), "{p}");
    }
    // A thickness that isn't a number falls back to the default.
    let r = s.execute("object.pathfinder.trap", &json!({"thickness": "NaN", "tintReduction": 100})).unwrap();
    let bb = traps(&s, &r)[0].geometric_bounds().unwrap();
    assert!(near(bb.x1, 50.25), "{bb:?}");
}

#[test]
fn ids_name_the_objects_to_trap() {
    let (mut s, a, b) = abutting();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let r = s.execute("object.pathfinder.trap", &json!({"ids": [a.0, b.0]})).unwrap();
    assert_eq!(traps(&s, &r).len(), 1);
}
