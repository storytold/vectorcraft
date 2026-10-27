//! Stroke geometry from the Stroke panel's commands (`cmd/stroke.rs`): arrowhead alignment and
//! the width-profile presets.

use serde_json::json;
use vectorcraft_doc::{ArrowAlign, StrokeLayer, WidthProfile};

use crate::*;

fn session_with_line() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("shape.line", &json!({"x1": 10, "y1": 100, "x2": 150, "y2": 100})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn stroke(s: &Session, id: NodeId) -> StrokeLayer {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().clone()
}

#[test]
fn arrow_align_is_set_per_stroke_and_validated() {
    let (mut s, id) = session_with_line();
    assert_eq!(stroke(&s, id).arrow_align, ArrowAlign::Extend);
    s.execute("stroke.set", &json!({"endArrow": "TriangleOpen", "arrowAlign": "tip"})).unwrap();
    let st = stroke(&s, id);
    assert_eq!((st.end_arrow, st.arrow_align), (Some(vectorcraft_doc::Arrowhead::TriangleOpen), ArrowAlign::Tip));
    s.execute("stroke.set", &json!({"arrowAlign": "extend", "ids": [id.0]})).unwrap();
    assert_eq!(stroke(&s, id).arrow_align, ArrowAlign::Extend);
    assert!(s.execute("stroke.set", &json!({"arrowAlign": "middle"})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(stroke(&s, id).arrow_align, ArrowAlign::Tip);
}

#[test]
fn profiles_come_from_the_preset_catalogue() {
    let (mut s, id) = session_with_line();
    for p in WidthProfile::PRESETS {
        s.execute("stroke.set", &json!({"profile": p.id})).unwrap();
        let st = stroke(&s, id);
        assert_eq!(WidthProfile::id_of(st.profile.as_ref()), p.id);
    }
    assert!(stroke(&s, id).profile.is_some());
    s.execute("stroke.set", &json!({"profile": "uniform"})).unwrap();
    assert!(stroke(&s, id).profile.is_none(), "uniform is the plain stroke");
    assert!(s.execute("stroke.set", &json!({"profile": "zigzag"})).is_err());
    // The params doc (what agents read) lists every preset and the arrow alignment.
    let spec = command_specs().iter().find(|c| c.id == "stroke.set").unwrap();
    for p in WidthProfile::PRESETS {
        assert!(spec.params.contains(&format!("\"{}\"", p.id)), "{}", p.id);
    }
    assert!(spec.params.contains("arrowAlign"));
    // …and every arrowhead name `startArrow`/`endArrow` accept.
    for a in vectorcraft_doc::Arrowhead::ALL {
        let name = serde_json::to_value(a).unwrap();
        assert!(spec.params.contains(name.as_str().unwrap()), "{name}");
        s.execute("stroke.set", &json!({"endArrow": name})).unwrap();
        assert_eq!(stroke(&s, id).end_arrow, Some(a));
    }
}

#[test]
fn arrow_align_and_profiles_survive_the_native_format() {
    let (mut s, id) = session_with_line();
    s.execute(
        "stroke.set",
        &json!({"startArrow": "CircleOpen", "endArrow": "Arrow", "arrowAlign": "tip", "profile": "lens", "dash": [0, 6], "cap": "round"}),
    )
    .unwrap();
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(back.node(id).unwrap().appearance.stroke().unwrap(), &stroke(&s, id));
    // A stroke with the default alignment writes nothing new, so older builds read it unchanged.
    s.execute("stroke.set", &json!({"arrowAlign": "extend"})).unwrap();
    let text = String::from_utf8(vectorcraft_format::save(&s.doc().unwrap().doc, true)).unwrap();
    assert!(!text.contains("arrow_align"));
}
