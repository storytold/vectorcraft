//! A blend's spine and keys on the canvas (M8.19): spine points moved, added and deleted (the
//! straight spine becomes a path on the first edit), keys picked and edited live, Release keeping
//! the spine as a path.

use serde_json::{Value, json};
use vectorcraft_color::Paint;
use vectorcraft_doc::live::BlendSpec;
use vectorcraft_doc::{Node, NodeKind};
use vectorcraft_geom::{Point, Rect};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn spec(s: &Session, id: NodeId) -> BlendSpec {
    match node(s, id).kind {
        NodeKind::Blend { spec, .. } => *spec,
        _ => panic!("not a blend"),
    }
}

fn center(n: &Node) -> Point {
    n.geometric_bounds().unwrap().center()
}

/// Two 10 pt squares centred on (5, 5) and (105, 5), blended with one step.
fn blend(s: &mut Session) -> (NodeId, NodeId, NodeId) {
    let a = id_of(&s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 10, "height": 10})).unwrap());
    let b = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 0, "width": 10, "height": 10})).unwrap());
    let g = id_of(&s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 1})).unwrap());
    (a, b, g)
}

fn steps(s: &Session, g: NodeId) -> Vec<Node> {
    vectorcraft_doc::live::expand_live(&node(s, g))
}

#[test]
fn a_spine_point_added_and_moved_bends_the_blend() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    assert!(spec(&s, g).spine.is_none(), "straight until edited");
    let r = s.execute("object.blend.spine.addAnchor", &json!({"x": 55, "y": 8})).unwrap();
    assert_eq!(r["anchor"], json!(1));
    let sp = spec(&s, g);
    assert_eq!(sp.key_anchors, vec![0, 2], "the keys keep their points");
    assert!(sp.spine.unwrap().subpaths[0].anchors[1].p.distance(Point::new(55.0, 5.0)) < 1e-9, "on the spine, nearest the click");
    s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 1, "x": 55, "y": 105})).unwrap();
    let st = steps(&s, g);
    assert!(center(&st[1]).distance(Point::new(55.0, 105.0)) < 1e-6, "the step follows the spine: {:?}", center(&st[1]));
    assert_eq!(center(&st[0]), Point::new(5.0, 5.0), "the keys stay put");
    // One undo step each.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(center(&steps(&s, g)[1]).distance(Point::new(55.0, 5.0)) < 1e-6);
}

/// #2544: an Anchor Point drag on a blend spine must create two symmetric
/// handles, not bend the adjacent segment and leave one handle behind.
#[test]
fn spine_anchor_conversion_preserves_keys_and_undo() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    let keys_before = spec(&s, g).key_anchors;
    let point_before = spec(&s, g).spine.unwrap().subpaths[0].anchors[1].p;
    assert!(point_before.distance(Point::new(55.0, 5.0)) < 1e-9);
    s.execute("object.blend.spine.convertAnchor", &json!({"id": g.0, "anchor": 1, "to": "smooth", "x": 55, "y": 35})).unwrap();
    let sp = spec(&s, g);
    assert_eq!(sp.key_anchors, keys_before);
    let middle = sp.spine.as_ref().unwrap().subpaths[0].anchors[1];
    assert_eq!(middle.kind, vectorcraft_geom::AnchorKind::Smooth);
    assert_eq!(middle.p, point_before);
    assert_eq!(middle.h_in, point_before - (Point::new(55.0, 35.0) - point_before));
    assert_eq!(middle.h_out, Point::new(55.0, 35.0));
    assert_eq!(center(&node(&s, a)), Point::new(5.0, 5.0));
    assert_eq!(center(&node(&s, b)), Point::new(105.0, 5.0));

    s.execute("edit.undo", &json!({})).unwrap();
    let before = spec(&s, g);
    assert_eq!(before.spine.unwrap().subpaths[0].anchors[1].h_out, point_before);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(spec(&s, g).spine.unwrap().subpaths[0].anchors[1], middle);

    s.execute("object.blend.spine.convertAnchor", &json!({"id": g.0, "anchor": 1, "to": "corner"})).unwrap();
    let corner = spec(&s, g).spine.unwrap().subpaths[0].anchors[1];
    assert!(!corner.has_in() && !corner.has_out());
    assert_eq!(corner.kind, vectorcraft_geom::AnchorKind::Corner);
}

#[test]
fn spine_conversion_rejects_invalid_arguments_without_mutation() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    let before = spec(&s, g);
    for params in [
        json!({"id": g.0, "anchor": 5, "to": "smooth", "x": 20, "y": 20}),
        json!({"id": g.0, "anchor": 0, "to": "unknown"}),
        json!({"id": g.0, "anchor": 0, "to": "smooth", "x": 10}),
        json!({"id": g.0, "anchor": 0, "to": "smooth", "x": f64::MAX, "y": 1}),
    ] {
        assert!(s.execute("object.blend.spine.convertAnchor", &params).is_err(), "{params}");
        assert_eq!(spec(&s, g), before);
    }
}

#[test]
fn moving_a_key_point_moves_the_key_and_the_other_way_round() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 0, "x": 5, "y": 105})).unwrap();
    assert_eq!(center(&node(&s, a)), Point::new(5.0, 105.0));
    // A handle drag curves the spine without moving the key.
    s.execute("object.blend.spine.moveAnchor", &json!({"anchor": 0, "handle": "out", "x": 5, "y": 300})).unwrap();
    assert_eq!(center(&node(&s, a)), Point::new(5.0, 105.0));
    assert!(center(&steps(&s, g)[1]).y > 100.0, "the step bows out");
    // Editing a key on the canvas (it is selectable inside the blend) drags its point along.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.move", &json!({"dx": 0, "dy": 50})).unwrap();
    let n = node(&s, g);
    let NodeKind::Blend { children, spec } = &n.kind else { panic!() };
    let (path, _) = vectorcraft_doc::live::blend_spine(children, spec).unwrap();
    assert_eq!(path.subpaths[0].anchors[1].p, Point::new(105.0, 55.0));
    assert!(s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 9, "x": 0, "y": 0})).is_err());
    assert!(s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 0, "x": f64::MAX, "y": 0})).is_err());
}

/// Moving a spine point refuses an `independent`, and reshaping a spine segment a `perpendicular`,
/// given as text or as another value that is not true or false, and the blend stays as it was; null
/// is read as the key left out.
#[test]
fn spine_edits_refuse_a_boolean_given_as_text() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.convertAnchor", &json!({"id": g.0, "anchor": 1, "to": "smooth", "x": 75, "y": 5})).unwrap();
    let before = node(&s, g);
    for (cmd, key, p) in [
        ("object.blend.spine.moveAnchor", "independent", json!({"id": g.0, "anchor": 1, "handle": "out", "x": 75, "y": 40, "independent": "true"})),
        ("object.blend.spine.moveAnchor", "independent", json!({"id": g.0, "anchor": 1, "x": 55, "y": 60, "independent": 1})),
        ("object.blend.spine.reshapeSegment", "perpendicular", json!({"id": g.0, "segment": 0, "dx": 10, "dy": 40, "perpendicular": "true"})),
        ("object.blend.spine.reshapeSegment", "perpendicular", json!({"id": g.0, "segment": 0, "dy": 40, "perpendicular": [false]})),
    ] {
        let r = s.execute(cmd, &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == format!("`{key}` must be true or false")), "{cmd} {p}: {r:?}");
        assert_eq!(node(&s, g), before, "{cmd} {p}");
    }
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "handle": "out", "x": 75, "y": 40, "independent": null})).unwrap();
    let a = spec(&s, g).spine.unwrap().subpaths[0].anchors[1];
    assert_eq!((a.h_out, a.kind), (Point::new(75.0, 40.0), vectorcraft_geom::AnchorKind::Smooth), "the point stays smooth");
    s.execute("object.blend.spine.reshapeSegment", &json!({"id": g.0, "segment": 0, "dy": 40, "perpendicular": null})).unwrap();
}

#[test]
fn only_points_without_a_key_can_be_deleted() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    assert!(s.execute("object.blend.spine.removeAnchor", &json!({"id": g.0, "anchor": 0})).is_err());
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 30, "y": 5})).unwrap();
    s.execute("object.blend.spine.removeAnchor", &json!({"id": g.0, "anchor": 1})).unwrap();
    let sp = spec(&s, g);
    assert_eq!((sp.spine.unwrap().subpaths[0].anchors.len(), sp.key_anchors), (2, vec![0, 1]));
}

/// An `anchor` or `segment` that is not a whole number is an error that leaves the blend as it
/// was; a whole number written as 1.0 still names its point.
#[test]
fn spine_commands_refuse_an_index_that_is_not_a_whole_number() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    let before = node(&s, g);
    for (cmd, key, p) in [
        ("object.blend.spine.moveAnchor", "anchor", json!({"id": g.0, "anchor": 1.5, "x": 55, "y": 60})),
        ("object.blend.spine.moveAnchor", "anchor", json!({"id": g.0, "anchor": 0.5, "handle": "out", "x": 5, "y": 300})),
        ("object.blend.spine.reshapeSegment", "segment", json!({"id": g.0, "segment": 0.5, "dy": 40})),
        ("object.blend.spine.removeAnchor", "anchor", json!({"id": g.0, "anchor": 1.5})),
    ] {
        let r = s.execute(cmd, &p);
        assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == format!("`{key}` must be a whole number")), "{cmd} {p}: {r:?}");
        assert_eq!(node(&s, g), before, "{cmd} {p}");
    }
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1.0, "x": 55, "y": 60})).unwrap();
    assert_eq!(spec(&s, g).spine.unwrap().subpaths[0].anchors[1].p, Point::new(55.0, 60.0));
}

/// Reshape Spine Segment refuses a `t`, `dx` or `dy` given as text or as another value that is not
/// a number, instead of bending the spine by 0 or at the segment's middle, and the blend stays as
/// it was; null is read as the key left out.
#[test]
fn spine_reshape_segment_refuses_a_number_given_as_text() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    let before = node(&s, g);
    for key in ["t", "dx", "dy"] {
        for v in [json!("10"), json!([10]), json!(true)] {
            let mut p = json!({"id": g.0, "segment": 0, "t": 0.5, "dx": 0, "dy": 40});
            p[key] = v.clone();
            let r = s.execute("object.blend.spine.reshapeSegment", &p);
            assert!(matches!(&r, Err(EngineError::BadParams { msg, .. }) if *msg == format!("`{key}` must be a number, not {v}")), "{p}: {r:?}");
            assert_eq!(node(&s, g), before, "{p}");
        }
    }
    s.execute("object.blend.spine.reshapeSegment", &json!({"id": g.0, "segment": 0, "t": null, "dx": null, "dy": 40})).unwrap();
    assert!(center(&steps(&s, g)[1]).y > 20.0, "the step follows the bent spine");
}

#[test]
fn info_lists_the_spine() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    let info = s.execute("object.blend.info", &json!({})).unwrap();
    assert_eq!(info["spine"]["explicit"], json!(false));
    assert_eq!(info["spine"]["keyAnchors"], json!([0, 1]));
    assert_eq!(info["spine"]["anchors"][1]["x"], json!(105.0));
    let _ = g;
}

#[test]
fn release_keeps_the_spine_as_a_path() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "x": 55, "y": 60})).unwrap();
    let r = s.execute("object.blend.release", &json!({})).unwrap();
    assert_eq!(r["ids"], json!([a.0, b.0]));
    let spine = NodeId(r["spines"][0].as_u64().unwrap());
    let n = node(&s, spine);
    assert!(n.appearance.fill_paint().is_none() && n.appearance.stroke_paint() == Paint::None, "paints nothing");
    let pts: Vec<Point> = n.path_data().unwrap().anchors().map(|(_, _, a)| a.p).collect();
    assert_eq!(pts, vec![Point::new(5.0, 5.0), Point::new(55.0, 60.0), Point::new(105.0, 5.0)]);
    let d = &s.doc().unwrap().doc;
    let layer = d.layers[0].children().unwrap().iter().map(|c| c.id).collect::<Vec<_>>();
    assert_eq!(layer, vec![spine, a, b], "the spine below the keys");
}

#[test]
fn reverse_spine_and_front_to_back_keep_an_edited_spine() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "x": 55, "y": 60})).unwrap();
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    s.execute("object.blend.reverseSpine", &json!({})).unwrap();
    assert_eq!((center(&node(&s, a)), center(&node(&s, b))), (Point::new(105.0, 5.0), Point::new(5.0, 5.0)), "the keys swap ends");
    assert!(center(&steps(&s, g)[1]).distance(Point::new(55.0, 60.0)) < 1e-6, "the spine keeps its shape");
    s.execute("object.blend.reverseFrontToBack", &json!({})).unwrap();
    assert_eq!(center(&node(&s, a)), Point::new(105.0, 5.0), "stacking changes, places don't");
    assert!(center(&steps(&s, g)[1]).distance(Point::new(55.0, 60.0)) < 1e-6);
}

#[test]
fn a_blend_with_an_edited_spine_takes_more_keys_along_it() {
    let mut s = session();
    let (a, b, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    s.execute("object.blend.spine.moveAnchor", &json!({"id": g.0, "anchor": 1, "x": 55, "y": 60})).unwrap();
    let c = id_of(&s.execute("shape.rectangle", &json!({"x": 200, "y": 0, "width": 10, "height": 10})).unwrap());
    let g2 = id_of(&s.execute("object.blend.make", &json!({"ids": [g.0, c.0]})).unwrap());
    let sp = spec(&s, g2);
    assert_eq!(sp.key_anchors, vec![0, 2, 3]);
    let pts: Vec<Point> = sp.spine.unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect();
    assert_eq!(pts[1], Point::new(55.0, 60.0), "the bend is kept");
    assert_eq!(pts[3], Point::new(205.0, 5.0));
    let _ = (a, b);
}

#[test]
fn direct_selection_picks_keys_and_edits_update_the_blend_live() {
    let mut s = session();
    let (a, _, g) = blend(&mut s);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": "#000000"})).ok();
    let opt = vectorcraft_doc::hit::HitOptions::default();
    let h = vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, Point::new(3.0, 3.0), opt).unwrap();
    assert_eq!((h.leaf, h.top_object(None)), (a, g), "the key is hit; the Selection tool still takes the blend");
    let h = vectorcraft_doc::hit::hit_test(&s.doc().unwrap().doc, Point::new(55.0, 5.0), opt).unwrap();
    assert_eq!(h.leaf, g, "a step hits as the blend");
    let mut r = vectorcraft_render::Renderer::new();
    let before = r.render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 120.0, 20.0), 1.0, true).pixel(55, 5);
    s.execute("paint.setFill", &json!({"ids": [a.0], "color": "#ff0000"})).unwrap();
    let after = r.render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 120.0, 20.0), 1.0, true).pixel(55, 5);
    assert_ne!(before, after, "the step redraws with the key's new colour");
}

#[test]
fn key_anchors_round_trip() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    s.execute("object.blend.spine.addAnchor", &json!({"id": g.0, "x": 55, "y": 5})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(&back.layers, &d.layers);
    let old: BlendSpec = serde_json::from_value(json!({"spine": {"subpaths": []}})).unwrap();
    assert!(old.key_anchors.is_empty());
}

/// #1111: a straight spine's segment bends into a curve, its middle moving where it's dragged, and
/// the steps follow it; junk segments and deltas are refused.
#[test]
fn a_straight_spine_segment_bends_into_a_curve() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    s.execute("object.blend.spine.reshapeSegment", &json!({"segment": 0, "t": 0.5, "dx": 0, "dy": 40})).unwrap();
    let spine = spec(&s, g).spine.unwrap();
    let seg = spine.subpaths[0].segment(0);
    use vectorcraft_geom::kurbo::ParamCurve;
    assert!(seg.eval(0.5).distance(Point::new(55.0, 45.0)) < 1e-9, "{seg:?}");
    assert!(center(&steps(&s, g)[1]).y > 20.0, "the step follows the curve");
    for bad in [json!({"segment": 1, "dy": 10}), json!({"segment": 0, "dx": f64::MAX, "dy": f64::MAX}), json!({"segment": -1})] {
        assert!(s.execute("object.blend.spine.reshapeSegment", &bad).is_err(), "{bad}");
    }
}

/// With `perpendicular` (Shift) the spine segment's handles stand at right angles to it, at one
/// length: its middle goes straight down, wherever the drag goes sideways.
#[test]
fn a_spine_segment_bends_with_perpendicular_handles() {
    let mut s = session();
    let (_, _, g) = blend(&mut s);
    s.execute("object.blend.spine.reshapeSegment", &json!({"segment": 0, "t": 0.5, "dx": 10, "dy": 40, "perpendicular": true})).unwrap();
    let sp = spec(&s, g).spine.unwrap().subpaths[0].clone();
    let (a, b) = (sp.anchors[0], sp.anchors[1]);
    let (ha, hb) = (a.h_out - a.p, b.h_in - b.p);
    assert!(ha.x.abs() < 1e-9 && hb.x.abs() < 1e-9 && (ha.y - hb.y).abs() < 1e-9, "{sp:?}");
    use vectorcraft_geom::kurbo::ParamCurve;
    assert!(sp.segment(0).eval(0.5).distance(Point::new(55.0, 45.0)) < 1e-9, "{sp:?}");
}
