//! Print Ink Fixes.

use serde_json::{Value, json};
use vectorcraft_color::Color;
use vectorcraft_doc::NodeId;

use crate::Session;

const C: &str = "addon.color.printFixes";

/// A CMYK document with a rectangle per fill, each with overprint as given; nothing selected.
fn doc(fills: &[(Value, bool)]) -> (Session, Vec<NodeId>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200, "colorMode": "cmyk"})).unwrap();
    let ids = fills
        .iter()
        .enumerate()
        .map(|(i, (fill, overprint))| {
            let id = s.execute("shape.rectangle", &json!({"x": 10 + 30 * i, "y": 10, "width": 20, "height": 20})).unwrap()["id"].as_u64().unwrap();
            s.execute("paint.setFill", &json!({ "color": fill })).unwrap();
            s.execute("paint.setStroke", &json!({"none": true})).unwrap();
            s.execute("object.setOverprint", &json!({ "fill": overprint })).unwrap();
            NodeId(id)
        })
        .collect();
    s.execute("select.none", &json!({})).unwrap();
    (s, ids)
}

fn fill(s: &Session, id: NodeId) -> (Color, bool) {
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    let item = n.appearance.items.iter().find(|i| i.is_fill()).unwrap().clone();
    (item.paint().color().unwrap(), item.overprint())
}

fn cmyk(c: f64, m: f64, y: f64, k: f64) -> Value {
    json!({"c": c, "m": m, "y": y, "k": k})
}

fn close(a: Color, b: [f32; 4]) -> bool {
    let Color::Cmyk { c, m, y, k } = a else { return false };
    [c, m, y, k].iter().zip(b).all(|(x, y)| (x - y).abs() < 0.001)
}

#[test]
fn the_ink_limit_scales_heavy_colours_down_to_it() {
    let (mut s, ids) = doc(&[(cmyk(100.0, 100.0, 100.0, 100.0), false), (cmyk(40.0, 30.0, 20.0, 10.0), false), (cmyk(0.0, 0.0, 0.0, 80.0), false)]);
    let undo = s.doc().unwrap().history.undo.len();
    // Nothing selected: the whole document.
    let v = s.execute(C, &json!({"inkLimit": 300})).unwrap();
    assert_eq!((v["inkLimited"].clone(), v["changed"].clone()), (json!(1), json!(1)));
    assert!(close(fill(&s, ids[0]).0, [0.75; 4]), "{:?}", fill(&s, ids[0]));
    assert!(close(fill(&s, ids[1]).0, [0.4, 0.3, 0.2, 0.1]), "under the limit: as it was");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // The selection only.
    s.execute("select.set", &json!({"ids": [ids[1].0]})).unwrap();
    s.execute(C, &json!({"inkLimit": 50})).unwrap();
    assert!(close(fill(&s, ids[1]).0, [0.2, 0.15, 0.1, 0.05]));
    assert!(close(fill(&s, ids[0]).0, [0.75; 4]), "left out");
    // Pure black keeps its ink unless it is included.
    s.execute(C, &json!({"inkLimit": 50, "scope": "document"})).unwrap();
    assert!(close(fill(&s, ids[0]).0, [0.125; 4]));
    assert!(close(fill(&s, ids[2]).0, [0.0, 0.0, 0.0, 0.8]));
    s.execute(C, &json!({"inkLimit": 50, "scope": "document", "excludePureBlack": false})).unwrap();
    assert!(close(fill(&s, ids[2]).0, [0.0, 0.0, 0.0, 0.5]));
}

#[test]
fn spot_and_registration_colours_keep_their_ink() {
    let (mut s, ids) = doc(&[(cmyk(0.0, 0.0, 0.0, 0.0), false), (cmyk(0.0, 0.0, 0.0, 0.0), false)]);
    s.execute("swatch.new", &json!({"name": "Heavy", "color": cmyk(100.0, 100.0, 100.0, 50.0), "spot": true})).unwrap();
    s.execute("select.set", &json!({"ids": [ids[0].0]})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Heavy"})).unwrap();
    s.execute("select.set", &json!({"ids": [ids[1].0]})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "[Registration]"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let before = [fill(&s, ids[0]), fill(&s, ids[1])];
    assert_eq!(s.execute(C, &json!({"inkLimit": 200})).unwrap()["changed"], 0);
    assert_eq!([fill(&s, ids[0]), fill(&s, ids[1])], before);
}

#[test]
fn white_overprint_goes_and_black_overprint_comes_at_the_threshold() {
    let white = cmyk(0.0, 0.0, 0.0, 0.0);
    let (mut s, ids) =
        doc(&[(white.clone(), true), (cmyk(0.0, 0.0, 0.0, 95.0), false), (cmyk(0.0, 0.0, 0.0, 80.0), false), (cmyk(30.0, 0.0, 0.0, 100.0), false)]);
    let undo = s.doc().unwrap().history.undo.len();
    let v = s.execute(C, &json!({"removeWhiteOverprint": true, "overprintBlack": 90})).unwrap();
    assert_eq!((v["whiteOverprintRemoved"].clone(), v["blackOverprinted"].clone()), (json!(1), json!(1)), "{v}");
    let overprints: Vec<bool> = ids.iter().map(|id| fill(&s, *id).1).collect();
    // White stops; 95 % black starts; 80 % is under the threshold; a rich black isn't pure black.
    assert_eq!(overprints, [false, true, false, false]);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(ids.iter().map(|id| fill(&s, *id).1).collect::<Vec<_>>(), [true, false, false, false]);
    // On type only: the rectangles stay.
    let v = s.execute(C, &json!({"overprintBlack": 90, "textOnly": true})).unwrap();
    assert_eq!(v["changed"], 0);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo, "nothing changed, no undo step");
    // Strokes only: the fills stay.
    assert_eq!(s.execute(C, &json!({"overprintBlack": 90, "fill": false})).unwrap()["changed"], 0);
}

#[test]
fn bad_requests_and_rgb_documents_are_refused() {
    let (mut s, ids) = doc(&[(cmyk(100.0, 100.0, 100.0, 100.0), true)]);
    for p in [
        json!({}),
        json!({"inkLimit": 0}),
        json!({"inkLimit": "300"}),
        json!({"inkLimit": 401}),
        json!({"inkLimit": -1}),
        json!({"overprintBlack": 101}),
        json!({"overprintBlack": true}),
        json!({"removeWhiteOverprint": "yes"}),
        json!({"inkLimit": 300, "fill": 1}),
        json!({"inkLimit": 300, "scope": "page"}),
        json!({"inkLimit": 300, "scope": "selection"}),
        json!({"inkLimit": 300, "ids": [999_999]}),
    ] {
        assert!(s.execute(C, &p).is_err(), "{p}");
    }
    assert!(close(fill(&s, ids[0]).0, [1.0; 4]));
    let mut rgb = Session::new();
    rgb.execute("file.new", &json!({})).unwrap();
    rgb.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let e = rgb.execute(C, &json!({"inkLimit": 300})).unwrap_err().to_string();
    assert!(e.contains("CMYK"), "{e}");
}
