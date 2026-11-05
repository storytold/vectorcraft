//! Select by Artboards.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;

use crate::Session;

const C: &str = "addon.select.byArtboards";

/// A document with three 100 × 100 artboards in a row, 50 pt apart: 0 at x 0, 1 at x 150, 2 at x 300.
fn three_boards() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    for x in [150, 300] {
        s.execute("artboard.new", &json!({"x": x, "y": 0, "width": 100, "height": 100})).unwrap();
    }
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

fn pick(s: &mut Session, p: Value) -> Vec<NodeId> {
    s.execute("select.none", &json!({})).unwrap();
    let r = s.execute(C, &p).unwrap();
    let mut ids = s.doc().unwrap().selection.objects.clone();
    assert_eq!(r["count"], json!(ids.len()));
    ids.sort();
    ids
}

#[test]
fn selects_by_overlap_inside_and_outside_several_artboards() {
    let mut s = three_boards();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0); // inside 0
    let ab = rect(&mut s, 90.0, 40.0, 80.0, 20.0); // across 0 and 1
    let b = rect(&mut s, 160.0, 10.0, 20.0, 20.0); // inside 1
    let c = rect(&mut s, 310.0, 10.0, 20.0, 20.0); // inside 2
    let off = rect(&mut s, 0.0, 200.0, 20.0, 20.0); // on no board
    let edge = rect(&mut s, 0.0, 0.0, 100.0, 100.0); // flush with board 0
    let want = |mut v: Vec<NodeId>| {
        v.sort();
        v
    };
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1]})), want(vec![a, ab, b, edge]));
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1], "relation": "overlap"})), want(vec![a, ab, b, edge]));
    // Wholly within one board, edges included; straddling two boards is not inside either.
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1], "relation": "inside"})), want(vec![a, b, edge]));
    assert_eq!(pick(&mut s, json!({"artboards": [0, 1], "relation": "outside"})), want(vec![c, off]));
    assert_eq!(pick(&mut s, json!({"artboards": [2], "relation": "outside"})), want(vec![a, ab, b, off, edge]));
    // One board does what All on Active Artboard does.
    for i in 0..3u64 {
        s.execute("select.allOnArtboard", &json!({"artboard": i})).unwrap();
        let mut on = s.doc().unwrap().selection.objects.clone();
        on.sort();
        assert_eq!(pick(&mut s, json!({"artboards": [i]})), on, "board {i}");
    }
    // A board listed twice counts once.
    assert_eq!(pick(&mut s, json!({"artboards": [2, 2]})), vec![c]);
}

#[test]
fn hidden_locked_and_isolated_art_follow_select_all() {
    let mut s = three_boards();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let hidden = rect(&mut s, 40.0, 10.0, 20.0, 20.0);
    let locked = rect(&mut s, 70.0, 10.0, 20.0, 20.0);
    let far = rect(&mut s, 0.0, 300.0, 20.0, 20.0);
    s.execute("object.setProps", &json!({"ids": [hidden.0], "visible": false})).unwrap();
    s.execute("object.setProps", &json!({"ids": [locked.0], "locked": true})).unwrap();
    assert_eq!(pick(&mut s, json!({"artboards": [0]})), vec![a]);
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "outside"})), vec![far]);
    // Art held by a locked artboard is left out.
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    assert_eq!(pick(&mut s, json!({"artboards": [0]})), vec![]);
    s.execute("artboard.setProps", &json!({"index": 0, "locked": false})).unwrap();
    // In isolation mode only the isolated group's objects are looked at.
    let b = rect(&mut s, 20.0, 50.0, 10.0, 10.0);
    s.execute("select.none", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.isolate", &json!({"id": g})).unwrap();
    let mut both = vec![a, b];
    both.sort();
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "inside"})), both);
    assert_eq!(pick(&mut s, json!({"artboards": [1]})), vec![]);
}

#[test]
fn bad_params_are_errors_and_leave_the_selection() {
    let mut s = three_boards();
    let a = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    for p in [
        json!({}),
        json!({"artboards": []}),
        json!({"artboards": [3]}),
        json!({"artboards": [-1]}),
        json!({"artboards": [0.5]}),
        json!({"artboards": "0"}),
        json!({"artboards": [0], "relation": "near"}),
        json!({"artboards": [0], "relation": 1}),
        json!({"artboards": [0], "relatoin": "inside"}),
    ] {
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        assert!(s.execute(C, &p).is_err(), "{p}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![a], "{p}");
    }
}

#[test]
fn a_symbol_instance_is_measured_by_its_art() {
    // The symbol's art, (100, 60) to (140, 80), drawn 50 pt up and left: (50, 10) to (90, 30), on
    // the 200 × 120 artboard, while the 20 × 20 square around the instance's origin lies off it.
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="200" height="120"><defs><symbol id="Far"><rect x="100" y="60" width="40" height="20" fill="#f00"/></symbol></defs><use href="#Far" x="-50" y="-50"/></svg>"##;
    let mut s = Session::new();
    s.execute("document.open", &json!({"name": "far.svg", "dataBase64": vectorcraft_format::base64_encode(svg.as_bytes())})).unwrap();
    let mut instance = None;
    s.doc().unwrap().doc.walk(|n| {
        if matches!(n.kind, vectorcraft_doc::NodeKind::SymbolInstance { .. }) {
            instance = Some(n.id);
        }
    });
    let instance = instance.unwrap();
    assert_eq!(pick(&mut s, json!({"artboards": [0]})), vec![instance]);
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "inside"})), vec![instance]);
    assert_eq!(pick(&mut s, json!({"artboards": [0], "relation": "outside"})), vec![]);
}
