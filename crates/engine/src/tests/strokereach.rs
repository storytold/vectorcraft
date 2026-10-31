//! Visual bounds and clicks take in the whole stroke: arrowheads, outside alignment, width
//! profiles (through the Selection tool and the commands that use visual bounds).

use serde_json::json;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 400})).unwrap();
    s
}

fn id_of(v: &serde_json::Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// Click with the Selection tool at (x, y) on a 100% view: what is selected afterwards.
fn click(s: &mut Session, x: f64, y: f64) -> Vec<NodeId> {
    let v = ViewInfo::default();
    s.select_tool("selection", v).unwrap();
    for k in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(k, x, y), v).unwrap();
    }
    s.active().unwrap().selection.objects.clone()
}

/// The artboard after Fit to Selected Art (which fits the selection's visual bounds).
fn fitted(s: &mut Session) -> vectorcraft_geom::Rect {
    s.execute("artboard.fitToSelection", &json!({})).unwrap();
    s.doc().unwrap().doc.artboards[0].rect
}

#[test]
fn a_300_percent_arrowhead_grows_the_visual_bounds() {
    let mut s = session();
    s.execute("shape.line", &json!({"x1": 100, "y1": 200, "x2": 300, "y2": 200})).unwrap();
    s.execute("stroke.set", &json!({"weight": 2, "join": "round", "endArrow": "Triangle"})).unwrap();
    let plain = fitted(&mut s);
    s.execute("stroke.setAdvanced", &json!({"arrowScale": [100, 300]})).unwrap();
    let big = fitted(&mut s);
    // The 24 pt head reaches past the end and 12 pt to each side of the line.
    assert!(big.x1 > plain.x1 + 15.0 && big.y1 >= 212.0 && big.y0 <= 188.0, "{plain:?} → {big:?}");
    assert_eq!(big.x0, plain.x0, "the start has no head");
}

#[test]
fn a_click_outside_an_outside_stroke_selects_it() {
    let mut s = session();
    let r = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 200})).unwrap());
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("stroke.set", &json!({"weight": 20, "align": "outside"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    // 0.8 × the weight outside the path: past where a centred stroke (plus the tolerance) reaches.
    assert_eq!(click(&mut s, 84.0, 200.0), [r]);
    s.execute("select.none", &json!({})).unwrap();
    assert!(click(&mut s, 116.0, 200.0).is_empty(), "nothing paints inside");
    s.execute("stroke.set", &json!({"ids": [r.0], "align": "center"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert!(click(&mut s, 84.0, 200.0).is_empty(), "a centred stroke doesn't reach");
}

#[test]
fn a_click_at_the_lens_maximum_selects_its_stroke() {
    let mut s = session();
    let l = id_of(&s.execute("shape.line", &json!({"x1": 100, "y1": 200, "x2": 300, "y2": 200})).unwrap());
    s.execute("stroke.set", &json!({"weight": 20, "profile": "lens"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    assert_eq!(click(&mut s, 200.0, 209.0), [l], "the widest point");
    s.execute("select.none", &json!({})).unwrap();
    assert!(click(&mut s, 110.0, 209.0).is_empty(), "the thin end");
    // Fitted bounds take the profile in too.
    s.execute("select.set", &json!({"ids": [l.0]})).unwrap();
    assert_eq!(fitted(&mut s).y1, 210.0);
}
