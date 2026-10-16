//! Outline Object (`path.outlineObject`, Effect › Path › Outline Object): type that carries it is
//! drawn, exported and expanded as its glyph outlines while it stays editable type.

use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s
}

/// Selected point type "OUTLINE" (48 pt) at (40, 100).
fn text(s: &mut Session) -> NodeId {
    let r = s.execute("text.create", &json!({"x": 40, "y": 100, "text": "OUTLINE", "size": 48})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn apply(s: &mut Session, id: NodeId) {
    s.execute("effect.apply", &json!({"effect": "path.outlineObject", "ids": [id.0], "item": null})).unwrap();
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn count_text(n: &Node) -> usize {
    let mut k = 0;
    n.walk(&mut |m| k += usize::from(matches!(m.kind, NodeKind::Text(_))));
    k
}

#[test]
fn outline_object_is_applied_in_one_undo_step_and_takes_no_parameters() {
    let mut s = session();
    let id = text(&mut s);
    apply(&mut s, id);
    let ids: Vec<String> = node(&s, id).appearance.effects.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["path.outlineObject"]);
    assert_eq!(s.doc().unwrap().history.undo.last().map(|e| e.label.as_str()), Some("Outline Object"));
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(node(&s, id).appearance.effects.is_empty());
    let err = s.execute("effect.apply", &json!({"effect": "path.outlineObject", "ids": [id.0], "params": {"width": 1}})).unwrap_err().to_string();
    assert!(err.contains("`width`") && err.contains("takes no parameters"), "{err}");
    assert!(node(&s, id).appearance.effects.is_empty());
}

#[test]
fn type_with_outline_object_stays_editable_type_and_keeps_its_bounds() {
    let mut s = session();
    let id = text(&mut s);
    let before = [false, true].map(|preview| {
        s.prefs.use_preview_bounds = preview;
        s.transform_bounds(&[id])
    });
    apply(&mut s, id);
    let after = [false, true].map(|preview| {
        s.prefs.use_preview_bounds = preview;
        s.transform_bounds(&[id])
    });
    assert_eq!(after, before);
    // A native save keeps the effect.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let ids: Vec<String> = back.node(id).unwrap().appearance.effects.iter().map(|e| e.id.clone()).collect();
    assert_eq!(ids, ["path.outlineObject"]);
    // The type is still type, drawn as its outlines, and a text edit changes them.
    let n = node(&s, id);
    assert!(matches!(n.kind, NodeKind::Text(_)));
    let drawn = vectorcraft_render::effects::reshape(&n, None).unwrap();
    assert_eq!(count_text(&drawn), 0);
    s.execute("text.setText", &json!({"id": id.0, "text": "OUTLINED TYPE"})).unwrap();
    let longer = vectorcraft_render::effects::reshape(&node(&s, id), None).unwrap();
    assert!(longer.geometric_bounds().unwrap().width() > drawn.geometric_bounds().unwrap().width() + 50.0);
}

#[test]
fn type_with_outline_object_exports_and_expands_as_paths() {
    let mut s = session();
    let id = text(&mut s);
    apply(&mut s, id);
    let doc = s.doc().unwrap().doc.clone();
    let svg = vectorcraft_svg::export(&doc, &Default::default());
    assert!(!svg.contains("<text") && !svg.contains("OUTLINE"), "{svg}");
    // Expand Appearance leaves paths and no type, in one undo step.
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let doc = &s.doc().unwrap().doc;
    let layer = &doc.layers[0];
    assert_eq!(count_text(layer), 0);
    let mut paths = 0;
    layer.walk(&mut |m| paths += usize::from(matches!(m.kind, NodeKind::Path { .. } | NodeKind::Compound { .. })));
    assert!(paths > 0);
}
