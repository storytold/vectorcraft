//! Align panel menu › Align to Glyph Bounds (#4231): with Point Text or Area Text on, Align and
//! Distribute measure that kind of type by the ink of its glyphs instead of its em box or frame.

use serde_json::{Value, json};
use vectorcraft_doc::{NodeId, NodeKind};
use vectorcraft_geom::Rect;

use crate::*;

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id}: {e}"))
}

fn id(v: Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

/// A document with a 50 pt square at (10, 10), lowercase "ace" (no ascenders or descenders, so
/// its ink sits inside its em box) as point type with its baseline at (100, 300), and as area
/// type in a 200 × 100 pt frame at (100, 350); Align to Glyph Bounds set as `point` and `area`.
fn doc(point: bool, area: bool) -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width": 600, "height": 600}));
    let square = id(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50})));
    let pt = id(run(&mut s, "text.create", json!({"x": 100, "y": 300, "size": 48, "text": "ace"})));
    let ar = id(run(&mut s, "text.create", json!({"x": 100, "y": 350, "size": 48, "area": {"width": 200, "height": 100}, "text": "ace"})));
    run(&mut s, "prefs.set", json!({"values": {"alignGlyphBoundsPoint": point, "alignGlyphBoundsArea": area}}));
    (s, square, pt, ar)
}

/// The em box (point type) or frame (area type): what Align measures by default.
fn em(s: &Session, id: NodeId) -> Rect {
    s.doc().unwrap().doc.node(id).unwrap().geometric_bounds().unwrap()
}

/// The ink of the type's glyphs on the page.
fn ink(s: &Session, id: NodeId) -> Rect {
    let NodeKind::Text(t) = &s.doc().unwrap().doc.node(id).unwrap().kind else { panic!("type") };
    t.xf.transform_rect_bbox(vectorcraft_text::layout(vectorcraft_text::FontDb::global(), t).ink_bounds().unwrap())
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Aligns `ids` to the left and the top of the selection (the square is left and above them).
fn align_top_left(s: &mut Session, ids: &[NodeId]) {
    run(s, "select.set", json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()}));
    run(s, "object.align", json!({"horizontal": "left"}));
    run(s, "object.align", json!({"vertical": "top"}));
}

#[test]
fn glyph_ink_sits_inside_the_em_box_of_lowercase_type() {
    let (s, _, pt, ar) = doc(false, false);
    for t in [pt, ar] {
        let (e, i) = (em(&s, t), ink(&s, t));
        assert!(i.y0 > e.y0 + 1.0 && i.x0 > e.x0, "{t:?}: ink {i:?} inside {e:?}");
    }
}

#[test]
fn point_text_aligns_by_its_glyphs_with_point_text_on() {
    for on in [false, true] {
        let (mut s, square, pt, _) = doc(on, false);
        align_top_left(&mut s, &[square, pt]);
        let r = if on { ink(&s, pt) } else { em(&s, pt) };
        assert!(close(r.x0, 10.0) && close(r.y0, 10.0), "Point Text {on}: {r:?}");
        // The square stays where it is: it is the selection's top-left.
        assert_eq!(em(&s, square), Rect::new(10.0, 10.0, 60.0, 60.0));
    }
}

#[test]
fn area_text_aligns_by_its_glyphs_with_area_text_on() {
    for on in [false, true] {
        let (mut s, square, _, ar) = doc(false, on);
        align_top_left(&mut s, &[square, ar]);
        let r = if on { ink(&s, ar) } else { em(&s, ar) };
        assert!(close(r.x0, 10.0) && close(r.y0, 10.0), "Area Text {on}: {r:?}");
    }
}

/// Point Text leaves area type measured by its frame, and Area Text leaves point type measured
/// by its em box.
#[test]
fn the_two_toggles_are_independent() {
    let (mut s, square, _, ar) = doc(true, false);
    align_top_left(&mut s, &[square, ar]);
    let r = em(&s, ar);
    assert!(close(r.x0, 10.0) && close(r.y0, 10.0), "area type by its frame: {r:?}");
    let (mut s, square, pt, _) = doc(false, true);
    align_top_left(&mut s, &[square, pt]);
    let r = em(&s, pt);
    assert!(close(r.x0, 10.0) && close(r.y0, 10.0), "point type by its em box: {r:?}");
}

/// Distribute measures type by its glyphs too: the point type's ink left edge lands halfway
/// between the two squares' left edges.
#[test]
fn distribute_spaces_point_text_by_its_glyphs() {
    for on in [false, true] {
        let (mut s, square, pt, _) = doc(on, false);
        let far = id(run(&mut s, "shape.rectangle", json!({"x": 410, "y": 10, "width": 50, "height": 50})));
        run(&mut s, "select.set", json!({"ids": [square.0, pt.0, far.0]}));
        run(&mut s, "object.distribute", json!({"horizontal": "left"}));
        let r = if on { ink(&s, pt) } else { em(&s, pt) };
        assert!(close(r.x0, 210.0), "Point Text {on}: {r:?}");
    }
}
