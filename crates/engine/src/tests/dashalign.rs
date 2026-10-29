//! Dashes fitted to corners and path ends from `stroke.set` (`alignDashes`, `dashOffset`), and
//! Outline Stroke of fitted dashes.

use kurbo::Shape;
use serde_json::json;
use vectorcraft_doc::Dash;

use crate::tests_outlinestroke::{outline, path_of};
use crate::*;

fn session_with_rect() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let r = s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 100, "height": 50})).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn dash_of(s: &Session, id: NodeId) -> Option<Dash> {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().dash.clone()
}

fn set(s: &mut Session, p: serde_json::Value) {
    s.execute("stroke.set", &p).unwrap();
}

#[test]
fn align_dashes_and_the_offset_edit_the_pattern_in_place() {
    let (mut s, id) = session_with_rect();
    // Without a dash pattern there is nothing to align or offset.
    set(&mut s, json!({"alignDashes": true, "dashOffset": 4}));
    assert_eq!(dash_of(&s, id), None);
    // A new pattern is exact unless asked otherwise.
    set(&mut s, json!({"dash": [12, 6], "dashOffset": 3}));
    assert_eq!(dash_of(&s, id), Some(Dash { pattern: vec![12.0, 6.0], offset: 3.0, align_corners: false }));
    // The flag on its own keeps the pattern and the offset.
    set(&mut s, json!({"alignDashes": true}));
    assert_eq!(dash_of(&s, id), Some(Dash { pattern: vec![12.0, 6.0], offset: 3.0, align_corners: true }));
    // So does a new pattern (as the panel's dash fields send it), and the offset alone.
    set(&mut s, json!({"dash": [4, 2]}));
    assert_eq!(dash_of(&s, id), Some(Dash { pattern: vec![4.0, 2.0], offset: 3.0, align_corners: true }));
    set(&mut s, json!({"dashOffset": 1.5}));
    assert_eq!(dash_of(&s, id), Some(Dash { pattern: vec![4.0, 2.0], offset: 1.5, align_corners: true }));
    // Each edit is one undo step.
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(dash_of(&s, id).unwrap().offset, 3.0);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(dash_of(&s, id).unwrap().pattern, vec![12.0, 6.0]);
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!dash_of(&s, id).unwrap().align_corners);
    // The params doc tells agents about both modes.
    let spec = command_specs().iter().find(|c| c.id == "stroke.set").unwrap();
    assert!(spec.params.contains("alignDashes?: bool") && spec.params.contains("dashOffset? (exact dashes only)"));
}

#[test]
fn fitted_dashes_survive_the_native_format() {
    let (mut s, id) = session_with_rect();
    set(&mut s, json!({"dash": [12, 6], "dashOffset": 2, "alignDashes": true}));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(back.node(id).unwrap().appearance.stroke().unwrap().dash, dash_of(&s, id));
}

#[test]
fn outline_stroke_of_fitted_dashes_wraps_every_corner() {
    let (mut s, id) = session_with_rect();
    set(&mut s, json!({"weight": 2, "dash": [12, 6], "alignDashes": true}));
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    // The fill, then the outlined dashes.
    let ring = path_of(&outline(&mut s).children().unwrap()[1]);
    let bp = ring.to_bezpath();
    // 5 + 2 + 5 + 2 dashes inside the sides and one wrapped round each corner.
    assert_eq!(ring.subpaths.len(), 18);
    let half = 6.0 * 100.0 / 108.0;
    for (x, y) in [(20.0, 20.0), (120.0, 20.0), (120.0, 70.0), (20.0, 70.0)] {
        assert!(bp.contains(kurbo::Point::new(x, y)), "corner ({x}, {y})");
    }
    assert!(bp.contains(kurbo::Point::new(20.0 + half - 0.3, 20.0)) && !bp.contains(kurbo::Point::new(20.0 + half + 0.3, 20.0)));
}
