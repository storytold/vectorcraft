//! A stroke weight set on type is a document-space weight, whether the type was enlarged by its
//! font size or by a transform (issue #3241).

use serde_json::json;
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_engine::Session;

const MM: f64 = 72.0 / 25.4;

fn text(s: &mut Session, x: f64) -> NodeId {
    NodeId(s.execute("text.create", &json!({"x": x, "y": 100, "text": "Ab", "size": 30.0 * MM})).unwrap()["id"].as_u64().unwrap())
}

/// The weights the type's character strokes draw at in document space.
fn drawn(s: &Session, id: NodeId) -> Vec<f64> {
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("not type") };
    let k = t.xf.determinant().abs().sqrt();
    t.runs.iter().map(|r| r.style.stroke_width * k).collect()
}

#[test]
fn stroke_weight_on_scaled_type_matches_type_resized_by_size() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    let a = text(&mut s, 10.0);
    let b = text(&mut s, 300.0);
    s.execute("text.setStyle", &json!({"ids": [a.0], "size": 60.0 * MM})).unwrap();
    s.execute("object.scale", &json!({"ids": [b.0], "sx": 200, "sy": 200})).unwrap();
    s.execute("stroke.set", &json!({"ids": [a.0, b.0], "weight": MM, "dash": [MM, 2.0 * MM]})).unwrap();
    for w in drawn(&s, a).into_iter().chain(drawn(&s, b)) {
        assert!((w - MM).abs() < 1e-9, "drawn stroke weight {w}, want {MM}");
    }
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(b).unwrap().kind else { panic!("not type") };
    let dash = t.runs[0].style.stroke_dash.as_ref().unwrap().pattern.iter().map(|d| d * 2.0).collect::<Vec<_>>();
    assert!((dash[0] - MM).abs() < 1e-9 && (dash[1] - 2.0 * MM).abs() < 1e-9, "drawn dash {dash:?}");

    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let shown = s.shown_stroke().unwrap();
    assert!((shown.width - MM).abs() < 1e-9, "Stroke panel shows {}, want {MM}", shown.width);
    assert!((shown.dash.unwrap().pattern[0] - MM).abs() < 1e-9);
}
