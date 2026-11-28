//! Painting a previously unpainted zero-width stroke restores the existing 1 pt default.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_engine::{Session, ViewInfo};
use vectorcraft_tools::{PointerEvent, PointerKind};

fn fixture() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let id = rectangle(&mut s, 10.0);
    (s, id)
}

fn rectangle(s: &mut Session, x: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": 10, "width": 50, "height": 40})).unwrap()["id"].as_u64().unwrap())
}

fn width(s: &Session, id: NodeId) -> f64 {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().unwrap().width
}

#[test]
fn adding_a_stroke_after_sampling_strokeless_art_makes_it_visible_in_one_undo_step() {
    let (mut s, _) = fixture();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 25.0, 25.0), ViewInfo::default()).unwrap();
    let id = rectangle(&mut s, 100.0);
    assert_eq!(width(&s, id), 0.0, "sampled None paint is a normal source of a zero-width stroke");
    let st = s.doc().unwrap();
    let before = st.doc.clone();
    let sel = st.selection.clone();
    let history = st.history.undo.len();
    let geometry = st.doc.node(id).unwrap().geometric_bounds();
    s.execute("paint.setStroke", &json!({"color": "#ee2222"})).unwrap();
    assert_eq!(width(&s, id), 1.0);
    let st = s.doc().unwrap();
    assert_eq!(st.doc.node(id).unwrap().geometric_bounds(), geometry);
    assert_eq!(json!(st.selection), json!(sel));
    assert_eq!(st.history.undo.len(), history + 1);
    let after = st.doc.clone();
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(json!(s.doc().unwrap().doc), json!(before));
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(json!(s.doc().unwrap().doc), json!(after));
}

#[test]
fn adding_stroke_paint_to_the_defaults_gives_the_next_shape_a_visible_stroke() {
    let (mut s, _) = fixture();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("stroke.set", &json!({"weight": 0})).unwrap();
    s.execute("paint.setStroke", &json!({"color": "#ee2222"})).unwrap();
    assert_eq!(s.new_art_stroke().width, 1.0);
    let id = rectangle(&mut s, 100.0);
    assert_eq!(width(&s, id), 1.0);
}

#[test]
fn swapping_fill_into_an_unpainted_zero_width_stroke_makes_it_visible() {
    let (mut s, id) = fixture();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("stroke.set", &json!({"weight": 0})).unwrap();
    let fill = s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint();
    s.execute("paint.swap", &json!({})).unwrap();
    let n = s.doc().unwrap().doc.node(id).unwrap();
    assert!(n.appearance.fill_paint().is_none());
    assert_eq!(n.appearance.stroke_paint(), fill);
    assert_eq!(width(&s, id), 1.0);
    assert_eq!(s.new_art_stroke().width, 1.0);
}

#[test]
fn a_targeted_stroke_is_restored_without_changing_the_other_stroke() {
    let (mut s, id) = fixture();
    s.execute("stroke.set", &json!({"weight": 5})).unwrap();
    s.execute("appearance.addStroke", &json!({})).unwrap();
    s.execute("paint.setStroke", &json!({"item": 1, "none": true})).unwrap();
    s.execute("stroke.set", &json!({"item": 1, "weight": 0})).unwrap();
    s.execute("paint.setStroke", &json!({"item": 1, "color": "#ee2222"})).unwrap();
    let ap = &s.doc().unwrap().doc.node(id).unwrap().appearance;
    assert_eq!(ap.stroke_at(Some(1)).unwrap().width, 1.0);
    assert_eq!(ap.stroke_at(Some(2)).unwrap().width, 5.0);
}

#[test]
fn existing_weights_and_explicitly_zero_painted_strokes_keep_their_widths() {
    for (was_none, weight) in [(true, 5.0), (false, 5.0), (false, 0.0)] {
        let (mut s, id) = fixture();
        if was_none {
            s.execute("paint.setStroke", &json!({"none": true})).unwrap();
        }
        s.execute("stroke.set", &json!({"weight": weight})).unwrap();
        s.execute("paint.setStroke", &json!({"color": "#ee2222"})).unwrap();
        assert_eq!(width(&s, id), weight);
    }
    let (mut s, id) = fixture();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("stroke.set", &json!({"weight": 0})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ffaa00"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    assert_eq!(width(&s, id), 0.0, "fill edits and removing paint do not enable a stroke");
}

#[test]
fn appearance_panel_stroke_colour_restores_an_unpainted_zero_width_stroke() {
    let (mut s, id) = fixture();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    s.execute("stroke.set", &json!({"weight": 0})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 1, "color": "#00ff00"})).unwrap();
    assert_eq!(width(&s, id), 1.0);
    s.execute("appearance.setItem", &json!({"index": 1, "weight": 0})).unwrap();
    s.execute("appearance.setItem", &json!({"index": 1, "color": "#0000ff"})).unwrap();
    assert_eq!(width(&s, id), 0.0, "a painted stroke keeps an explicit zero width");
}

#[test]
fn removing_the_stroke_colour_resets_the_weight_to_zero() {
    let (mut s, id) = fixture();
    s.execute("paint.setStroke", &json!({"color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"weight": 10})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    assert_eq!(width(&s, id), 0.0);
    assert_eq!(s.shown_stroke().unwrap().width, 0.0);
    s.execute("paint.setStroke", &json!({"color": "#ee2222"})).unwrap();
    assert_eq!(width(&s, id), 1.0);
}
