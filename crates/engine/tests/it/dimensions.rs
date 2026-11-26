//! Linked measurements, history, portable artwork and hostile command input.
use serde_json::{Value, json};
use vectorcraft_engine::{
    Session, ViewInfo,
    doc::{Document, NodeId, NodeKind},
    tools::{Mods, PointerEvent, PointerKind, ToolKey},
};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":600,"height":500})).unwrap();
    s
}
fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap()
}
fn edge(s: &mut Session) -> u64 {
    run(s, "shape.line", json!({"x1":100,"y1":100,"x2":172,"y2":100}))["id"].as_u64().unwrap()
}
fn dim(s: &mut Session, node: u64) -> u64 {
    run(
        s,
        "dimension.create",
        json!({"source":{"kind":"segment","edge":{"node":node,"subpath":0,"segment":0}},"options":{"units":"in","precision":2,"scale":"1:10"}}),
    )["id"]
        .as_u64()
        .unwrap()
}
fn text(s: &Session, id: u64) -> String {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap();
    n.children().unwrap().iter().find_map(|n| if let NodeKind::Text(t) = &n.kind { Some(t.plain_text()) } else { None }).unwrap()
}
fn recipe(s: &Session, id: u64) -> vectorcraft_doc::dimension::Dimension {
    (**s.doc().unwrap().doc.node(NodeId(id)).unwrap().extras().dimension.as_ref().unwrap()).clone()
}
fn ev(s: &mut Session, kind: PointerKind, x: f64, y: f64) {
    s.pointer(&PointerEvent::new(kind, x, y), ViewInfo::default()).unwrap();
}

#[test]
fn dimension_length_scale_live_edit_undo_and_roundtrip() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    assert_eq!(text(&s, id), "10.00 in");
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert_eq!(text(&s, id), "20.00 in");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, id), "10.00 in");
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(text(&s, id), "20.00 in");
    let json = serde_json::to_string(&*s.doc().unwrap().doc).unwrap();
    let doc: Document = serde_json::from_str(&json).unwrap();
    assert!(doc.dimensions, "saved with the document");
    let mut reopened = Session::new();
    reopened.add_document(doc, None);
    run(&mut reopened, "select.set", json!({"ids":[line]}));
    run(&mut reopened, "object.scale", json!({"sx":50,"sy":100}));
    assert_eq!(text(&reopened, id), "10.00 in");
    run(&mut reopened, "edit.clear", json!({}));
    assert_eq!(text(&reopened, id), "10.00 in", "deleting the art leaves the dimension");
    assert!(recipe(&reopened, id).source.is_none(), "no longer linked");
    run(&mut reopened, "edit.undo", json!({}));
    assert_eq!(text(&reopened, id), "10.00 in");
    assert!(recipe(&reopened, id).source.is_some(), "undo links it again");
}

/// Documents without dimensions skip the refresh, and lose the flag when the last goes.
#[test]
fn documents_without_dimensions_skip_the_refresh() {
    let mut s = session();
    let line = edge(&mut s);
    assert!(!s.doc().unwrap().doc.dimensions);
    let id = dim(&mut s, line);
    assert!(s.doc().unwrap().doc.dimensions);
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "edit.clear", json!({}));
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.move", json!({"dx":1,"dy":0}));
    assert!(!s.doc().unwrap().doc.dimensions);
}

#[test]
fn apply_to_all_restyles_every_dimension_in_one_undo_step() {
    let mut s = session();
    let line = edge(&mut s);
    let first = dim(&mut s, line);
    let second = run(&mut s, "dimension.create", json!({"a":[0,0],"b":[72,0],"options":{"units":"pt","precision":0}}))["id"].as_u64().unwrap();
    assert_eq!(text(&s, second), "72 pt");
    run(&mut s, "dimension.applyToAll", json!({"id":first}));
    assert_eq!(text(&s, second), "10.00 in", "the first dimension's style, its scale too");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, second), "72 pt");
    assert!(s.execute("dimension.applyToAll", &json!({"id":line})).is_err(), "not a dimension");
}

#[test]
fn each_arrow_style_draws_its_stroke_arrowhead_at_both_ends() {
    use vectorcraft_doc::Arrowhead;
    let mut s = session();
    for (style, head) in
        [("filled", Some(Arrowhead::Triangle)), ("open", Some(Arrowhead::TriangleOpen)), ("tick", Some(Arrowhead::Slash)), ("none", None)]
    {
        let id = run(&mut s, "dimension.create", json!({"a":[0,0],"b":[72,0],"options":{"arrowStyle":style}}))["id"].as_u64().unwrap();
        let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap();
        let heads: Vec<_> = n
            .children()
            .unwrap()
            .iter()
            .filter_map(|n| n.appearance.stroke().map(|s| (s.start_arrow.clone(), s.end_arrow.clone())))
            .filter(|(a, b)| a.is_some() || b.is_some())
            .collect();
        match head {
            Some(head) => assert!(!heads.is_empty() && heads.iter().all(|h| *h == (Some(head.clone()), Some(head.clone()))), "{style}: {heads:?}"),
            None => assert!(heads.is_empty(), "{style}: {heads:?}"),
        }
    }
}

/// `arrowHead` draws any built-in stroke arrowhead at both ends of the dimension line, and the
/// native file keeps it. The saved `arrowStyle` stays one of the four values versions without
/// `arrowHead` read, and a dimension saved without `arrowHead` draws its arrow style. Options
/// without a stroke head are saved without `arrowHead`. A custom arrowhead, an unknown name and an
/// overlong name are refused.
#[test]
fn every_stroke_arrowhead_draws_as_a_dimension_arrowhead() {
    use vectorcraft_doc::{Arrowhead, dimension::ArrowStyle};
    let ends = |s: &Session, id: u64| -> Vec<_> {
        let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap();
        let children = n.children().unwrap();
        let strokes = children.iter().filter_map(|n| n.appearance.stroke().map(|s| (s.start_arrow.clone(), s.end_arrow.clone())));
        strokes.filter(|(a, b)| a.is_some() || b.is_some()).collect()
    };
    let mut s = session();
    let mut made = vec![];
    for head in Arrowhead::ALL {
        let name = serde_json::to_value(&head).unwrap();
        let id = run(&mut s, "dimension.create", json!({"a":[0,0],"b":[72,0],"options":{"arrowHead":name}}))["id"].as_u64().unwrap();
        let drawn = ends(&s, id);
        assert!(!drawn.is_empty() && drawn.iter().all(|h| *h == (Some(head.clone()), Some(head.clone()))), "{name}: {drawn:?}");
        let saved = serde_json::to_value(&recipe(&s, id).options).unwrap();
        assert_eq!((&saved["arrowStyle"], &saved["arrowHead"]), (&json!("filled"), &name));
        made.push((id, head));
    }
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let mut reopened = Session::new();
    reopened.add_document(vectorcraft_format::load(&bytes).unwrap(), None);
    for (id, head) in made {
        assert_eq!(recipe(&reopened, id).options.arrowhead(), Some(head));
    }
    // A dimension saved before `arrowHead` existed.
    let id = run(&mut s, "dimension.create", json!({"a":[0,0],"b":[72,0],"options":{"arrowStyle":"open"}}))["id"].as_u64().unwrap();
    let mut file: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    fn drop_head(v: &mut Value) {
        match v {
            Value::Object(m) => {
                m.remove("arrowHead");
                m.values_mut().for_each(drop_head);
            }
            Value::Array(a) => a.iter_mut().for_each(drop_head),
            _ => {}
        }
    }
    drop_head(&mut file);
    let mut older = Session::new();
    older.add_document(vectorcraft_format::load(&serde_json::to_vec(&file).unwrap()).unwrap(), None);
    let o = recipe(&older, id).options;
    assert_eq!((o.arrow_style, o.arrow_head.clone(), o.arrowhead()), (ArrowStyle::Open, None, Some(Arrowhead::TriangleOpen)));
    // Without a stroke head, the saved and recorded options carry no `arrowHead`, and a null one
    // is still taken.
    let saved = serde_json::to_value(&recipe(&s, id).options).unwrap();
    assert!(saved.get("arrowHead").is_none(), "{saved}");
    assert!(s.execute("dimension.create", &json!({"a":[0,0],"b":[72,0],"options":{"arrowHead":null}})).is_ok());
    let custom = json!({"Custom":{"name":"Dot","path":{"subpaths":[]}}});
    for bad in [json!("Custom"), json!("Arrowhead"), json!("barbed"), json!(3), json!("Barbed".repeat(20)), custom] {
        let p = json!({"a":[0,0],"b":[72,0],"options":{"arrowHead":bad}});
        assert!(s.execute("dimension.create", &p).is_err(), "{bad}");
    }
    assert!(s.execute("dimension.create", &json!({"a":[0,0],"b":[72,0],"options":{"arrowStyle":"Barbed"}})).is_err());
}

#[test]
fn dimension_angles_circles_arcs_and_curves() {
    let mut s = session();
    let a = run(
        &mut s,
        "dimension.create",
        json!({"mode":"angular","a":[100,0],"vertex":[0,0],"b":[0,100],"options":{"precision":1,"scale":"1:10"}}),
    )["id"]
        .as_u64()
        .unwrap();
    assert_eq!(text(&s, a), "90.0°");
    let circle = run(&mut s, "shape.ellipse", json!({"x":100,"y":100,"width":144,"height":144}))["id"].as_u64().unwrap();
    for (mode, want) in [("radius", "1.0 in"), ("diameter", "2.0 in")] {
        let id = run(
            &mut s,
            "dimension.create",
            json!({"mode":mode,"source":{"kind":"circle","node":circle},"options":{"units":"in","precision":1}}),
        )["id"]
            .as_u64()
            .unwrap();
        assert_eq!(text(&s, id), want);
    }
    // One quarter of the circle is a perfect arc.
    let id = run(
        &mut s,
        "dimension.create",
        json!({"mode":"radius","source":{"kind":"arc","edge":{"node":circle,"subpath":0,"segment":0}},"options":{"units":"in","precision":1}}),
    )["id"]
        .as_u64()
        .unwrap();
    assert_eq!(text(&s, id), "1.0 in");
    // A curve that isn't an arc: no radius, and its length is measured end to end.
    let curve = run(&mut s, "path.create", json!({"d":"M 0 0 C 0 100 100 100 100 0"}))["id"].as_u64().unwrap();
    let arc = json!({"mode":"radius","source":{"kind":"arc","edge":{"node":curve,"subpath":0,"segment":0}}});
    assert!(s.execute("dimension.create", &arc).is_err());
    let id = run(
        &mut s,
        "dimension.create",
        json!({"source":{"kind":"segment","edge":{"node":curve,"subpath":0,"segment":0}},"options":{"units":"pt","precision":0}}),
    )["id"]
        .as_u64()
        .unwrap();
    assert_eq!(text(&s, id), "100 pt");
}

#[test]
fn an_angular_dimension_can_take_its_vertex_from_an_anchor() {
    let mut s = session();
    let line = edge(&mut s);
    let request = json!({
        "mode": "angular", "a": [172, 100], "b": [100, 172], "options": {"precision": 1},
        "source": {"kind": "points", "vertex": {"node": line, "subpath": 0, "anchor": 0}}
    });
    let id = run(&mut s, "dimension.create", request.clone())["id"].as_u64().unwrap();
    assert_eq!(text(&s, id), "90.0°");
    run(&mut s, "select.set", json!({"ids": [line]}));
    run(&mut s, "object.move", json!({"dx": 18, "dy": 0}));
    assert!(near(recipe(&s, id).vertex, vectorcraft_geom::Point::new(118.0, 100.0)));

    let mut invalid = request.clone();
    invalid["vertex"] = json!("bad point");
    assert!(s.execute("dimension.create", &invalid).is_err());
    let mut missing = request;
    missing["source"]["vertex"] = Value::Null;
    assert!(s.execute("dimension.create", &missing).is_err());
}

#[test]
fn dimension_rejects_bad_input_without_mutation() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let original = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    for p in [
        json!({}),
        json!({"a":[0,0],"b":[0,0]}),
        json!({"a":[0,0],"b":[1e100,0]}),
        json!({"a":[0,0],"b":[10,0],"options":{"precision":99999}}),
        json!({"a":[0,0],"b":[10,0],"options":{"scale":"0:10"}}),
        json!({"a":[0,0],"b":[10,0],"options":{"scale":"1:NaN"}}),
        json!({"a":[0,0],"b":[10,0],"options":{"units":"furlongs"}}),
        json!({"a":[0,0],"b":[10,0],"options":{"textPosition":"sideways"}}),
        json!({"a":[0,0],"b":[10,0],"mode":"spiral"}),
        json!({"source":{"kind":"segment","edge":{"node":line,"subpath":0,"segment":18446744073709551615u64}}}),
        json!({"mode":"angular","source":{"kind":"segment","edge":{"node":line,"subpath":0,"segment":0}}}),
    ] {
        assert!(s.execute("dimension.create", &p).is_err(), "{p}");
        assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), original);
    }
    assert!(s.execute("dimension.update", &json!({"ids":[id],"options":{"fontSize":-1}})).is_err());
    assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), original);
}

/// Hover a segment and drag outwards: releasing places the dimension; Escape cancels the drag.
#[test]
fn dragging_out_from_a_segment_places_its_dimension() {
    let mut s = session();
    edge(&mut s);
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    let before = s.doc().unwrap().doc.node_count();
    ev(&mut s, PointerKind::Move, 130.0, 100.0);
    ev(&mut s, PointerKind::Down, 130.0, 100.0);
    ev(&mut s, PointerKind::Drag, 130.0, 70.0);
    assert!(s.in_interaction());
    s.tool_key(ToolKey::Escape, Mods::default(), ViewInfo::default()).unwrap();
    assert_eq!(s.doc().unwrap().doc.node_count(), before);
    assert!(!s.in_interaction());
    ev(&mut s, PointerKind::Up, 130.0, 70.0);
    ev(&mut s, PointerKind::Down, 130.0, 100.0);
    ev(&mut s, PointerKind::Drag, 130.0, 80.0);
    ev(&mut s, PointerKind::Drag, 130.0, 60.0);
    ev(&mut s, PointerKind::Up, 130.0, 60.0);
    assert!(!s.in_interaction());
    assert!(s.doc().unwrap().doc.node_count() > before);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(text(&s, id), "72.00 pt");
    assert!((recipe(&s, id).offset.abs() - 40.0).abs() < 1e-6, "placed where it was released");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.node_count(), before);
}

/// Plain clicks pick two points, and a third click places the dimension.
#[test]
fn clicking_two_points_then_placing_measures_between_them() {
    let mut s = session();
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    for (x, y) in [(300.0, 300.0), (372.0, 300.0)] {
        ev(&mut s, PointerKind::Down, x, y);
        ev(&mut s, PointerKind::Up, x, y);
    }
    ev(&mut s, PointerKind::Move, 336.0, 330.0);
    assert!(s.in_interaction());
    ev(&mut s, PointerKind::Down, 336.0, 330.0);
    ev(&mut s, PointerKind::Up, 336.0, 330.0);
    assert!(!s.in_interaction());
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(text(&s, id), "72.00 pt");
}

/// After two clicked points, a pointer beside them (between the perpendiculars at the points)
/// gives the distance between them (aligned), as does clicking again on the second point. Past
/// either point, the quarter that holds the direction straight up or down from it gives their
/// horizontal distance, and the quarter that holds the direction straight left or right gives
/// their vertical distance. The dimension stays linked to the anchors it measures.
#[test]
fn two_points_measure_horizontally_vertically_or_aligned() {
    use vectorcraft_doc::dimension::Orientation;
    for ((x, y), want, orientation) in [
        ((210.0, 120.0), "100.00 pt", Orientation::Horizontal),
        ((90.0, 380.0), "100.00 pt", Orientation::Horizontal),
        ((280.0, 220.0), "100.00 pt", Orientation::Vertical),
        ((40.0, 310.0), "100.00 pt", Orientation::Vertical),
        ((120.0, 160.0), "141.42 pt", Orientation::Aligned),
        ((200.0, 300.0), "141.42 pt", Orientation::Aligned),
        ((200.0, 200.0), "141.42 pt", Orientation::Aligned),
    ] {
        let mut s = session();
        let line = run(&mut s, "shape.line", json!({"x1":100,"y1":300,"x2":200,"y2":200}))["id"].as_u64().unwrap();
        s.select_tool("dimension", ViewInfo::default()).unwrap();
        for (px, py) in [(100.0, 300.0), (200.0, 200.0), (x, y)] {
            ev(&mut s, PointerKind::Move, px, py);
            ev(&mut s, PointerKind::Down, px, py);
            ev(&mut s, PointerKind::Up, px, py);
        }
        let id = s.doc().unwrap().selection.objects[0].0;
        assert_eq!((text(&s, id), recipe(&s, id).orientation), (want.to_string(), orientation), "pointer at ({x}, {y})");
        assert!(recipe(&s, id).source.is_some_and(|src| src.nodes() == vec![NodeId(line), NodeId(line)]));
        if orientation != Orientation::Aligned {
            run(&mut s, "select.set", json!({"ids":[line]}));
            run(&mut s, "object.scale", json!({"sx":200,"sy":200}));
            assert_eq!(text(&s, id), "200.00 pt", "follows the art");
        }
    }
}

/// A third click a little past the second point, still within reach of it, places an aligned
/// dimension, also when Shift moved the second point away from where the pointer was: the click
/// counts as clicking again on the second point.
#[test]
fn clicking_again_by_the_second_point_places_an_aligned_dimension() {
    use vectorcraft_doc::dimension::Orientation;
    let click = |s: &mut Session, x: f64, y: f64, shift: bool| {
        for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
            let e = PointerEvent::new(kind, x, y).with_mods(Mods { shift, ..Mods::default() });
            s.pointer(&e, ViewInfo::default()).unwrap();
        }
    };
    // Free points, the third click 2.2 pt past the second.
    let mut s = session();
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    click(&mut s, 100.0, 300.0, false);
    click(&mut s, 200.0, 200.0, false);
    click(&mut s, 202.0, 201.0, false);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!((text(&s, id), recipe(&s, id).orientation), ("141.42 pt".to_string(), Orientation::Aligned));
    // Shift puts the second point on the 45 degree line, about 21 pt from the pointer.
    let mut s = session();
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    click(&mut s, 100.0, 300.0, false);
    click(&mut s, 260.0, 170.0, true);
    click(&mut s, 262.0, 168.0, false);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!((text(&s, id), recipe(&s, id).orientation), ("206.16 pt".to_string(), Orientation::Aligned));
}

/// Dragged out from a slanted segment, the dimension follows the drag the same way, wherever on
/// the segment the press was: beside the segment it measures its length, and past either end
/// its horizontal or vertical length.
#[test]
fn a_segment_drag_measures_horizontally_vertically_or_aligned() {
    use vectorcraft_doc::dimension::Orientation;
    for (press, (x, y), want, orientation) in [
        ((150.0, 250.0), (210.0, 120.0), "100.00 pt", Orientation::Horizontal),
        ((110.0, 290.0), (210.0, 120.0), "100.00 pt", Orientation::Horizontal),
        ((150.0, 250.0), (280.0, 220.0), "100.00 pt", Orientation::Vertical),
        ((190.0, 210.0), (40.0, 310.0), "100.00 pt", Orientation::Vertical),
        ((150.0, 250.0), (120.0, 160.0), "141.42 pt", Orientation::Aligned),
        ((110.0, 290.0), (200.0, 300.0), "141.42 pt", Orientation::Aligned),
    ] {
        let mut s = session();
        run(&mut s, "shape.line", json!({"x1":100,"y1":300,"x2":200,"y2":200}));
        s.select_tool("dimension", ViewInfo::default()).unwrap();
        ev(&mut s, PointerKind::Move, press.0, press.1);
        ev(&mut s, PointerKind::Down, press.0, press.1);
        ev(&mut s, PointerKind::Drag, (press.0 + x) / 2.0, (press.1 + y) / 2.0);
        ev(&mut s, PointerKind::Drag, x, y);
        ev(&mut s, PointerKind::Up, x, y);
        let id = s.doc().unwrap().selection.objects[0].0;
        assert_eq!((text(&s, id), recipe(&s, id).orientation), (want.to_string(), orientation), "pressed at {press:?}, dragged to ({x}, {y})");
    }
}

/// Two points on one vertical line give an aligned dimension wherever the pointer goes: their
/// vertical distance is the distance between them, and they have no horizontal distance.
#[test]
fn points_on_a_vertical_line_measure_aligned() {
    use vectorcraft_doc::dimension::Orientation;
    for (x, y) in [(60.0, 200.0), (160.0, 80.0), (100.0, 40.0), (40.0, 340.0), (100.0, 300.0)] {
        let mut s = session();
        s.select_tool("dimension", ViewInfo::default()).unwrap();
        for (px, py) in [(100.0, 100.0), (100.0, 300.0), (x, y)] {
            ev(&mut s, PointerKind::Move, px, py);
            ev(&mut s, PointerKind::Down, px, py);
            ev(&mut s, PointerKind::Up, px, py);
        }
        let id = s.doc().unwrap().selection.objects[0].0;
        assert_eq!((text(&s, id), recipe(&s, id).orientation), ("200.00 pt".to_string(), Orientation::Aligned), "pointer at ({x}, {y})");
    }
}

/// The ends of a dimension's generated paths (extension lines first, then the dimension line),
/// rounded.
fn path_ends(s: &Session, id: u64) -> Vec<Vec<(f64, f64)>> {
    let round = |v: f64| (v * 1e6).round() / 1e6;
    let doc = &s.doc().unwrap().doc;
    let art = doc.node(NodeId(id)).unwrap().children().unwrap();
    art.iter().filter_map(|n| n.path_data()).map(|p| p.subpaths[0].anchors.iter().map(|a| (round(a.p.x), round(a.p.y))).collect()).collect()
}

/// A horizontal dimension's line is horizontal and a vertical one's vertical, through the point
/// it was placed at, and each extension line runs from its point to the dimension line. Points
/// given right to left or bottom to top draw the same lines.
#[test]
fn horizontal_and_vertical_dimensions_draw_lines_along_their_axis() {
    let mut s = session();
    let r = run(&mut s, "dimension.create", json!({"a":[100,300],"b":[200,200],"orientation":"horizontal","offset":-150}));
    assert_eq!(r["label"], "100.00 pt");
    // Extension lines: 4 pt off each point, 6 pt past the line at y = 150.
    let horizontal = vec![vec![(100.0, 296.0), (100.0, 144.0)], vec![(200.0, 196.0), (200.0, 144.0)], vec![(100.0, 150.0), (200.0, 150.0)]];
    assert_eq!(path_ends(&s, r["id"].as_u64().unwrap()), horizontal);
    let r = run(&mut s, "dimension.create", json!({"a":[200,200],"b":[100,300],"orientation":"horizontal","offset":-50}));
    let swapped = vec![vec![(200.0, 196.0), (200.0, 144.0)], vec![(100.0, 296.0), (100.0, 144.0)], vec![(200.0, 150.0), (100.0, 150.0)]];
    assert_eq!(path_ends(&s, r["id"].as_u64().unwrap()), swapped, "the line stays on the side the offset puts it");
    // A vertical dimension 50 pt left of the first point: the line at x = 50.
    let r = run(&mut s, "dimension.create", json!({"a":[100,300],"b":[200,200],"orientation":"vertical","offset":50}));
    assert_eq!(r["label"], "100.00 pt");
    let vertical = vec![vec![(96.0, 300.0), (44.0, 300.0)], vec![(196.0, 200.0), (44.0, 200.0)], vec![(50.0, 300.0), (50.0, 200.0)]];
    assert_eq!(path_ends(&s, r["id"].as_u64().unwrap()), vertical);
}

/// A horizontal dimension stays horizontal when it or the art it measures turns: it shows the
/// new horizontal distance, and 0 when the points come one above the other. It is not removed.
#[test]
fn turned_horizontal_dimensions_stay_horizontal() {
    use vectorcraft_doc::dimension::Orientation;
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[100,200],"b":[200,260],"orientation":"horizontal","offset":-50}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.rotate", json!({"angle":90,"origin":[150,230]}));
    assert_eq!((text(&s, id), recipe(&s, id).orientation), ("60.00 pt".to_string(), Orientation::Horizontal));
    // The points are now at y = 180 and 280: the line stays 50 pt above the upper one instead of
    // crossing between them.
    let d = recipe(&s, id);
    let mut ys = [d.a.y.round(), d.b.y.round()];
    ys.sort_by(f64::total_cmp);
    assert_eq!(ys, [180.0, 280.0]);
    assert!(path_ends(&s, id)[2].iter().all(|(_, y)| (y - 130.0).abs() < 1e-6), "{:?}", path_ends(&s, id));
    let line = run(&mut s, "shape.line", json!({"x1":100,"y1":400,"x2":200,"y2":400}))["id"].as_u64().unwrap();
    let anchor = |anchor: u64| json!({"node":line,"subpath":0,"anchor":anchor});
    let src = json!({"kind":"points","a":anchor(0),"b":anchor(1)});
    let id = run(&mut s, "dimension.create", json!({"source":src,"orientation":"horizontal"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.rotate", json!({"angle":90}));
    assert_eq!((text(&s, id), recipe(&s, id).orientation), ("0.00 pt".to_string(), Orientation::Horizontal));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, id), "100.00 pt");
}

/// A dimension of a whole path measures the path's overall width, or its height when vertical,
/// between its outermost points, and follows the path when it turns. Turned 30°, a 100 × 60
/// rectangle is 100 cos 30° + 60 sin 30° wide, more than either side; a turned ellipse is measured
/// at the outermost points of its curves, which are not anchors.
#[test]
fn a_dimension_of_a_whole_path_measures_its_overall_width_and_height() {
    let value = |s: &Session, id: u64| text(s, id).trim_end_matches(" pt").parse::<f64>().unwrap();
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":60}))["id"].as_u64().unwrap();
    let source = json!({"kind":"extent","node":rect});
    let id = run(&mut s, "dimension.create", json!({"source":source,"orientation":"horizontal","offset":-30}))["id"].as_u64().unwrap();
    assert_eq!(text(&s, id), "100.00 pt");
    run(&mut s, "select.set", json!({"ids":[rect]}));
    run(&mut s, "object.rotate", json!({"angle":30}));
    let wide = 100.0 * 30f64.to_radians().cos() + 60.0 * 30f64.to_radians().sin();
    assert!((value(&s, id) - wide).abs() < 0.01, "{} against {wide}", text(&s, id));
    // The rectangle's box: left, top, right, bottom.
    let bounds = |s: &Session| {
        let anchors = s.doc().unwrap().doc.node(NodeId(rect)).unwrap().path_data().unwrap().subpaths[0].anchors.clone();
        let (xs, ys): (Vec<f64>, Vec<f64>) = anchors.iter().map(|a| (a.p.x, a.p.y)).unzip();
        let low = |v: &[f64]| v.iter().copied().fold(f64::INFINITY, f64::min);
        let high = |v: &[f64]| v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        (low(&xs), low(&ys), high(&xs), high(&ys))
    };
    // The line stays 30 pt above the turned rectangle's topmost corner, clear of the art.
    let (left, top, right, _) = bounds(&s);
    let line = recipe(&s, id).line_ends().0.y;
    assert!((line - (top - 30.0)).abs() < 0.01, "line at y = {line}, top at {top}");
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"vertical"}));
    let tall = 100.0 * 30f64.to_radians().sin() + 60.0 * 30f64.to_radians().cos();
    assert!((value(&s, id) - tall).abs() < 0.01, "{} against {tall}", text(&s, id));
    let d = recipe(&s, id);
    let (top, bottom) = (d.a.y.min(d.b.y), d.a.y.max(d.b.y));
    assert!(((bottom - top) - tall).abs() < 0.01, "the points are the topmost and bottommost: {:?} {:?}", d.a, d.b);
    // The line was 30 pt above the path; turned, it is 30 pt beside it.
    let line = d.line_ends().0.x;
    assert!((line - (left - 30.0)).abs() < 0.01 || (line - (right + 30.0)).abs() < 0.01, "line at x = {line}, path from {left} to {right}");
    run(&mut s, "edit.undo", json!({}));
    assert!((value(&s, id) - wide).abs() < 0.01, "{}", text(&s, id));
    // A linear dimension only.
    assert!(s.execute("dimension.create", &json!({"mode":"angular","source":source})).is_err());

    let ellipse = run(&mut s, "shape.ellipse", json!({"x":300,"y":100,"width":100,"height":50}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":ellipse},"orientation":"horizontal"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[ellipse]}));
    run(&mut s, "object.rotate", json!({"angle":45}));
    let anchors = s.doc().unwrap().doc.node(NodeId(ellipse)).unwrap().path_data().unwrap().subpaths[0].anchors.clone();
    let between_anchors = anchors.iter().map(|a| a.p.x).fold(f64::MIN, f64::max) - anchors.iter().map(|a| a.p.x).fold(f64::MAX, f64::min);
    // 2 √(50² cos² 45° + 25² sin² 45°)
    let across = 2.0 * (2500.0f64 * 0.5 + 625.0 * 0.5).sqrt();
    assert!(value(&s, id) > between_anchors + 1.0, "{} against the anchors' {between_anchors}", text(&s, id));
    assert!((value(&s, id) - across).abs() < 0.1, "{} against {across}", text(&s, id));
}

/// A dimension of a whole path measures the segments the path draws: an anchor that starts no
/// segment does not widen it. Its line stays clear of the whole path when only parts other than
/// its measured points move, also when it is aligned or is scaled together with the path, when the
/// path's direction is reversed or the path is reflected onto itself, and when it becomes aligned.
/// A reflection that moves the path puts the line on the reflected side.
#[test]
fn a_dimension_of_a_whole_path_stays_clear_of_the_path_it_draws() {
    let mut s = session();
    let lone = run(&mut s, "path.create", json!({"d":"M100 300 L110 300 M200 400","closed":false}))["id"].as_u64().unwrap();
    assert_eq!(s.doc().unwrap().doc.node(NodeId(lone)).unwrap().path_data().unwrap().subpaths.len(), 2, "the lone anchor is kept");
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":lone},"orientation":"horizontal"}))["id"].as_u64().unwrap();
    assert_eq!(text(&s, id), "10.00 pt");

    // A triangle whose top rises while its leftmost and rightmost points stay: the line stays
    // 30 pt above the top.
    let tri = run(&mut s, "path.create", json!({"d":"M100 200 L150 120 L200 200 Z","closed":true}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":tri},"orientation":"horizontal","offset":-110}))["id"]
        .as_u64()
        .unwrap();
    assert!((recipe(&s, id).line_ends().0.y - 90.0).abs() < 1e-6);
    run(&mut s, "select.set", json!({"ids":[tri]}));
    run(&mut s, "object.scale", json!({"sx":100,"sy":150,"origin":[150,200]}));
    let line = recipe(&s, id).line_ends().0.y;
    assert!((line - 50.0).abs() < 1e-6, "line at y = {line}");
    // The same for an aligned dimension, and for a dimension scaled together with its path.
    for (x, orientation, with_dimension) in [(300.0, "aligned", false), (500.0, "horizontal", true)] {
        let d = format!("M{x} 200 L{} 120 L{} 200 Z", x + 50.0, x + 100.0);
        let tri = run(&mut s, "path.create", json!({"d":d,"closed":true}))["id"].as_u64().unwrap();
        let source = json!({"kind":"extent","node":tri});
        let id = run(&mut s, "dimension.create", json!({"source":source,"orientation":orientation,"offset":-110}))["id"].as_u64().unwrap();
        let ids = if with_dimension { vec![tri, id] } else { vec![tri] };
        run(&mut s, "select.set", json!({"ids":ids}));
        run(&mut s, "object.scale", json!({"sx":100,"sy":150,"origin":[x + 50.0, 200]}));
        let line = recipe(&s, id).line_ends().0.y;
        assert!((line - 50.0).abs() < 1e-6, "{orientation}: line at y = {line}");
    }
    // Reversing the path's direction draws the same outline: the line stays above it.
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":400,"width":100,"height":60}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":rect},"orientation":"horizontal","offset":-30}))["id"]
        .as_u64()
        .unwrap();
    run(&mut s, "path.reverse", json!({"ids":[rect]}));
    let line = recipe(&s, id).line_ends().0.y;
    assert!((line - 370.0).abs() < 1e-6, "line at y = {line}");
    // So does a reflection onto itself whose coordinates do not come out exactly, while a
    // reflection that moves the art puts the line on the reflected side.
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":500.2,"width":100,"height":60}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":rect},"orientation":"horizontal","offset":-30}))["id"]
        .as_u64()
        .unwrap();
    run(&mut s, "select.set", json!({"ids":[rect]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,530.2]}));
    let line = recipe(&s, id).line_ends().0.y;
    assert!((line - 470.2).abs() < 1e-6, "line at y = {line}");
    let tri = run(&mut s, "path.create", json!({"d":"M100 700 L150 620 L200 700 Z","closed":true}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":tri},"orientation":"horizontal","offset":-110}))["id"]
        .as_u64()
        .unwrap();
    run(&mut s, "select.set", json!({"ids":[tri]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,700]}));
    let line = recipe(&s, id).line_ends().0.y;
    assert!((line - 810.0).abs() < 1e-6, "line at y = {line}");

    // Made aligned, the line runs from the leftmost to the rightmost point, with every corner of
    // the rectangle on one side of it and 40 pt from the nearest, as it was 40 pt above the top.
    let rect = run(&mut s, "shape.rectangle", json!({"x":300,"y":100,"width":100,"height":60}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":rect},"orientation":"horizontal","offset":-40}))["id"]
        .as_u64()
        .unwrap();
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"aligned"}));
    let (a, b) = recipe(&s, id).line_ends();
    let side = |x: f64, y: f64| (b - a).cross(vectorcraft_geom::Point::new(x, y) - a) / (b - a).hypot();
    let sides = [side(300.0, 100.0), side(400.0, 100.0), side(400.0, 160.0), side(300.0, 160.0)];
    assert!(sides.iter().all(|v| *v > 1e-6) || sides.iter().all(|v| *v < -1e-6), "{a:?} to {b:?}: {sides:?}");
    let nearest = sides.iter().map(|v| v.abs()).fold(f64::INFINITY, f64::min);
    assert!((nearest - 40.0).abs() < 1e-6, "nearest corner {nearest} pt from the line");

    // A line across the path, 30 pt below the top, made vertical: it goes 30 pt past the path's
    // side rather than onto it.
    let rect = run(&mut s, "shape.rectangle", json!({"x":600,"y":100,"width":100,"height":60}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":rect},"orientation":"horizontal","offset":30}))["id"]
        .as_u64()
        .unwrap();
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"vertical"}));
    let line = recipe(&s, id).line_ends().0.x;
    assert!((line - 570.0).abs() < 1e-6 || (line - 730.0).abs() < 1e-6, "line at x = {line}, path from 600 to 700");
    // When that would put the line farther from its first point than an offset may be, the
    // change is refused rather than leaving the line across the path.
    let far = run(&mut s, "path.create", json!({"d":"M0 0 L950000 -100 L950000 200000 Z","closed":true}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":far},"orientation":"horizontal","offset":100000}))["id"]
        .as_u64()
        .unwrap();
    assert!(s.execute("dimension.update", &json!({"ids":[id],"orientation":"vertical"})).is_err());
    assert_eq!(recipe(&s, id).orientation, vectorcraft_doc::dimension::Orientation::Horizontal);
    // An offset given with the orientation places the line instead.
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"vertical","offset":-30}));
    let line = recipe(&s, id).line_ends().0.x;
    assert!((line - 950030.0).abs() < 1e-6, "line at x = {line}");
}

/// The line of a linked horizontal dimension keeps its gap from the outermost point on its side
/// when the art turns, so it stays clear of the art: after a half turn the line is still 50 pt
/// above the upper point, which is now the other anchor.
#[test]
fn a_linked_horizontal_dimension_keeps_its_gap_from_turned_art() {
    let mut s = session();
    let line = run(&mut s, "shape.line", json!({"x1":100,"y1":300,"x2":200,"y2":200}))["id"].as_u64().unwrap();
    let anchor = |anchor: u64| json!({"node":line,"subpath":0,"anchor":anchor});
    let src = json!({"kind":"points","a":anchor(0),"b":anchor(1)});
    let id = run(&mut s, "dimension.create", json!({"source":src,"orientation":"horizontal","offset":-150}))["id"].as_u64().unwrap();
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 150.0), (200.0, 150.0)]);
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.rotate", json!({"angle":180,"origin":[150,250]}));
    assert_eq!(text(&s, id), "100.00 pt");
    assert_eq!(path_ends(&s, id)[2], vec![(200.0, 150.0), (100.0, 150.0)]);
    // Scaled to twice the size about the lower left point, the upper point is at y = 100.
    run(&mut s, "object.scale", json!({"sx":200,"sy":200,"origin":[100,300]}));
    assert_eq!(text(&s, id), "200.00 pt");
    assert_eq!(path_ends(&s, id)[2], vec![(300.0, 50.0), (100.0, 50.0)]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 150.0), (200.0, 150.0)]);
}

/// An unlinked horizontal dimension reflected on its own has its line where the reflection puts
/// it: a line above the points is below them after a flip top to bottom.
#[test]
fn a_reflected_horizontal_dimension_keeps_its_line_on_the_reflected_side() {
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[100,300],"b":[200,200],"orientation":"horizontal","offset":-150}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,250]}));
    let d = recipe(&s, id);
    assert_eq!((d.a.y.round(), d.b.y.round()), (200.0, 300.0));
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 350.0), (200.0, 350.0)]);
    assert_eq!(text(&s, id), "100.00 pt");
}

/// An unlinked aligned dimension reflected on its own has its line on the reflected side of its
/// points: a line 30 pt above a level pair is 30 pt below it after a flip top to bottom, and a
/// half turn moves it below them as well, by turning it.
#[test]
fn a_reflected_aligned_dimension_keeps_its_line_on_the_reflected_side() {
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[100,200],"b":[200,200],"offset":-30}))["id"].as_u64().unwrap();
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 170.0), (200.0, 170.0)]);
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,200]}));
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 230.0), (200.0, 230.0)]);
    assert_eq!(text(&s, id), "100.00 pt");
    // Reflected left to right, the points swap ends and the line stays below them.
    run(&mut s, "object.reflect", json!({"axis":"vertical","origin":[150,0]}));
    assert_eq!(path_ends(&s, id)[2], vec![(200.0, 230.0), (100.0, 230.0)]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 170.0), (200.0, 170.0)]);
    // A half turn about the points' midpoint puts the line below them by turning it.
    run(&mut s, "object.rotate", json!({"angle":180,"origin":[150,200]}));
    let ends = path_ends(&s, id)[2].iter().map(|(x, y)| (x.round(), y.round())).collect::<Vec<_>>();
    assert_eq!(ends, vec![(200.0, 230.0), (100.0, 230.0)]);
}

/// An unlinked aligned dimension of a diagonal pair reflected about a mirror line through its
/// points has its line on the other side of them, and a scale by a negative factor reverses the
/// side as a reflection does, while a scale by positive factors keeps it.
#[test]
fn an_aligned_dimension_reflected_about_an_angle_or_scaled_by_a_negative_factor_changes_sides() {
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[100,200],"b":[200,100],"offset":-30}))["id"].as_u64().unwrap();
    let k = 30.0 / 2f64.sqrt();
    let near = |got: &[(f64, f64)], want: [(f64, f64); 2]| got.iter().zip(want).all(|(g, w)| (g.0 - w.0).abs() < 1e-6 && (g.1 - w.1).abs() < 1e-6);
    let ends = path_ends(&s, id)[2].clone();
    assert!(near(&ends, [(100.0 - k, 200.0 - k), (200.0 - k, 100.0 - k)]), "{ends:?}");
    run(&mut s, "select.set", json!({"ids":[id]}));
    // The mirror line at 45 degrees through (150, 150) holds both points.
    run(&mut s, "object.reflect", json!({"axis":45,"origin":[150,150]}));
    let ends = path_ends(&s, id)[2].clone();
    assert!(near(&ends, [(100.0 + k, 200.0 + k), (200.0 + k, 100.0 + k)]), "{ends:?}");
    run(&mut s, "edit.undo", json!({}));
    let level = run(&mut s, "dimension.create", json!({"a":[100,200],"b":[200,200],"offset":-30}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[level]}));
    run(&mut s, "object.scale", json!({"sx":-200,"sy":100,"origin":[150,0]}));
    assert_eq!(path_ends(&s, level)[2], vec![(250.0, 170.0), (50.0, 170.0)]);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":200,"origin":[150,200]}));
    assert_eq!(path_ends(&s, level)[2], vec![(50.0, 170.0), (250.0, 170.0)]);
}

/// The line of a linked horizontal dimension below both points stays below them when the art
/// turns a half turn; a line between the points keeps its distance from the first point.
#[test]
fn a_linked_horizontal_dimension_below_or_between_its_points_follows_turned_art() {
    for (offset, want) in [(50.0, vec![(200.0, 350.0), (100.0, 350.0)]), (-50.0, vec![(200.0, 150.0), (100.0, 150.0)])] {
        let mut s = session();
        let line = run(&mut s, "shape.line", json!({"x1":100,"y1":300,"x2":200,"y2":200}))["id"].as_u64().unwrap();
        let anchor = |anchor: u64| json!({"node":line,"subpath":0,"anchor":anchor});
        let src = json!({"kind":"points","a":anchor(0),"b":anchor(1)});
        let id = run(&mut s, "dimension.create", json!({"source":src,"orientation":"horizontal","offset":offset}))["id"].as_u64().unwrap();
        run(&mut s, "select.set", json!({"ids":[line]}));
        run(&mut s, "object.rotate", json!({"angle":180,"origin":[150,250]}));
        assert_eq!(text(&s, id), "100.00 pt");
        assert_eq!(path_ends(&s, id)[2], want, "offset {offset}");
    }
}

/// A dimension recipe without an orientation reads as aligned.
#[test]
fn a_dimension_without_an_orientation_reads_as_aligned() {
    use vectorcraft_doc::dimension::{Dimension, Orientation};
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[0,0],"b":[30,40]}))["id"].as_u64().unwrap();
    let v = serde_json::to_value(recipe(&s, id)).unwrap();
    assert!(v.get("orientation").is_none());
    let d: Dimension = serde_json::from_value(v).unwrap();
    assert_eq!(d.orientation, Orientation::Aligned);
}

/// `orientation` is checked on create and update, and only linear dimensions take one.
#[test]
fn dimension_orientation_is_checked() {
    let mut s = session();
    for (p, want) in [
        (json!({"a":[0,0],"b":[30,40],"orientation":"horizontal"}), Some("30.00 pt")),
        (json!({"a":[0,0],"b":[30,40],"orientation":"vertical"}), Some("40.00 pt")),
        (json!({"a":[0,0],"b":[30,40],"orientation":"aligned"}), Some("50.00 pt")),
        (json!({"a":[0,0],"b":[0,40],"orientation":"horizontal"}), Some("0.00 pt")),
        (json!({"a":[0,0],"b":[30,40],"orientation":"diagonal"}), None),
        (json!({"a":[0,0],"b":[30,40],"orientation":7}), None),
        (json!({"a":[0,0],"b":[30,40],"orientation":null}), None),
        (json!({"mode":"angular","a":[0,0],"b":[30,40],"vertex":[10,0],"orientation":"horizontal"}), None),
    ] {
        let before = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
        match (s.execute("dimension.create", &p), want) {
            (Ok(r), Some(want)) => assert_eq!(r["label"], want, "{p}"),
            (Err(_), None) => assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), before, "{p}"),
            (r, _) => panic!("{p}: {r:?}"),
        }
    }
    // An aligned dimension is written as before, without the field.
    let doc = serde_json::to_string(&*s.doc().unwrap().doc).unwrap();
    assert_eq!(doc.matches("\"orientation\"").count(), 3, "only the horizontal and vertical ones");
    // dimension.update turns an existing linear dimension, and refuses the same bad values.
    let id = run(&mut s, "dimension.create", json!({"a":[0,0],"b":[30,40]}))["id"].as_u64().unwrap();
    let angle = run(&mut s, "dimension.create", json!({"mode":"angular","a":[0,0],"b":[30,40],"vertex":[10,0]}))["id"].as_u64().unwrap();
    for (orientation, want) in [("horizontal", "30.00 pt"), ("vertical", "40.00 pt"), ("aligned", "50.00 pt")] {
        run(&mut s, "dimension.update", json!({"ids":[id],"orientation":orientation}));
        assert_eq!(text(&s, id), want);
    }
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"vertical"}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, id), "50.00 pt", "one undo step");
    let before = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    for p in [
        json!({"ids":[id],"orientation":"diagonal"}),
        json!({"ids":[id],"orientation":7}),
        json!({"ids":[angle],"orientation":"vertical"}),
        // A linear and an angular dimension together: neither changes.
        json!({"ids":[id, angle],"orientation":"vertical"}),
    ] {
        assert!(s.execute("dimension.update", &p).is_err(), "{p}");
        assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), before, "{p}");
    }
    // The orientation it already has, or aligned for an angular one, changes nothing and records
    // no undo step: the undo after it goes back to the vertical dimension of the earlier update.
    for p in [json!({"ids":[id],"orientation":"aligned"}), json!({"ids":[angle],"orientation":"aligned"})] {
        run(&mut s, "dimension.update", p);
        assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), before);
    }
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, id), "40.00 pt");
}

/// A dimension turned with `dimension.update` keeps its line on the side of the points it was
/// on: an aligned line 30 pt above a pair made right to left stays 30 pt above as horizontal,
/// and an aligned line below and right of slanted points goes 30 pt below the lower point.
#[test]
fn a_turned_dimension_keeps_its_line_on_its_side() {
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[200,200],"b":[100,200],"offset":30}))["id"].as_u64().unwrap();
    assert_eq!(path_ends(&s, id)[2], vec![(200.0, 170.0), (100.0, 170.0)]);
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"horizontal"}));
    assert_eq!(path_ends(&s, id)[2], vec![(200.0, 170.0), (100.0, 170.0)]);
    // The middle of the aligned line (about 171, 271) lies between the points' heights.
    let id = run(&mut s, "dimension.create", json!({"a":[100,300],"b":[200,200],"offset":30}))["id"].as_u64().unwrap();
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"horizontal"}));
    assert_eq!(path_ends(&s, id)[2], vec![(100.0, 330.0), (200.0, 330.0)]);
}

/// Dragging out from a corner of two straight segments measures the angle between them.
#[test]
fn dragging_out_from_a_corner_measures_its_angle() {
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":50}));
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    s.set_tool_option_cmd(&json!({"tool":"dimension","values":{"mode":"angular"}})).unwrap();
    ev(&mut s, PointerKind::Move, 100.0, 100.0);
    ev(&mut s, PointerKind::Down, 100.0, 100.0);
    ev(&mut s, PointerKind::Drag, 120.0, 120.0);
    ev(&mut s, PointerKind::Drag, 130.0, 130.0);
    ev(&mut s, PointerKind::Up, 130.0, 130.0);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(text(&s, id), "90.00°");
}

/// Dragging out from a circle gives its radius; dragging inwards, its diameter.
#[test]
fn dragging_from_a_circle_gives_its_radius_or_diameter() {
    let mut s = session();
    run(&mut s, "shape.ellipse", json!({"x":100,"y":100,"width":144,"height":144}));
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    s.set_tool_option_cmd(&json!({"tool":"dimension","values":{"mode":"radial","style":{"units":"in","precision":0}}})).unwrap();
    for ((x0, y0), (x1, y1), want) in [((244.0, 172.0), (280.0, 172.0), "1 in"), ((244.0, 172.0), (200.0, 172.0), "2 in")] {
        ev(&mut s, PointerKind::Move, x0, y0);
        ev(&mut s, PointerKind::Down, x0, y0);
        ev(&mut s, PointerKind::Drag, (x0 + x1) / 2.0, y0);
        ev(&mut s, PointerKind::Drag, x1, y1);
        ev(&mut s, PointerKind::Up, x1, y1);
        let id = s.doc().unwrap().selection.objects[0].0;
        assert_eq!(text(&s, id), want);
    }
}

#[test]
fn dimension_anchor_links_and_native_file_survive_source_moves() {
    let mut s = session();
    let first = edge(&mut s);
    let second = edge(&mut s);
    let id=run(&mut s,"dimension.create",json!({"a":[100,100],"b":[172,100],"source":{"kind":"points","a":{"node":first,"subpath":0,"anchor":0},"b":{"node":second,"subpath":0,"anchor":1},"vertex":null},"options":{"units":"pt","precision":0}}))["id"].as_u64().unwrap();
    assert_eq!(text(&s, id), "72 pt");
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let doc = vectorcraft_format::load(&bytes).unwrap();
    let mut s = Session::new();
    s.add_document(doc, None);
    run(&mut s, "select.set", json!({"ids":[second]}));
    run(&mut s, "object.move", json!({"dx":28,"dy":0}));
    assert_eq!(text(&s, id), "100 pt");
}

/// The anchors of path `id`'s first subpath.
fn anchors(s: &Session, id: u64) -> Vec<vectorcraft_geom::Point> {
    let doc = &s.doc().unwrap().doc;
    doc.node(NodeId(id)).unwrap().path_data().unwrap().subpaths[0].anchors.iter().map(|a| a.p).collect()
}
fn near(p: vectorcraft_geom::Point, q: vectorcraft_geom::Point) -> bool {
    p.distance(q) < 1e-6
}
/// Clicks with the Dimension tool at each point in turn.
fn clicks(s: &mut Session, points: &[(f64, f64)]) -> u64 {
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    for &(x, y) in points {
        ev(s, PointerKind::Move, x, y);
        ev(s, PointerKind::Down, x, y);
        ev(s, PointerKind::Up, x, y);
    }
    s.doc().unwrap().selection.objects[0].0
}

/// A dimension from an anchor to a point on a straight segment follows the art at both ends: the
/// segment end keeps its place along the segment when the art is scaled, moved or reflected.
#[test]
fn a_dimension_to_a_point_on_a_segment_follows_the_art_at_both_ends() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (200.0, 150.0), (120.0, 90.0)]);
    let length = recipe(&s, id).length;
    for (command, p) in [
        ("object.scale", json!({"sx":150,"sy":150})),
        ("object.move", json!({"dx":40,"dy":-30})),
        ("object.rotate", json!({"angle":30})),
        ("object.reflect", json!({"axis":"vertical"})),
    ] {
        run(&mut s, "select.set", json!({"ids":[rect]}));
        run(&mut s, command, p);
        let (d, at) = (recipe(&s, id), anchors(&s, rect));
        assert!(near(d.a, at[0]), "{command}: {:?} {:?}", d.a, at[0]);
        assert!(near(d.b, at[1].midpoint(at[2])), "{command}: {:?} {:?}", d.b, at[1].midpoint(at[2]));
        assert!(d.source.is_some(), "{command}: still linked");
    }
    assert!((recipe(&s, id).length - 1.5 * length).abs() < 1e-6);
}

/// A point off the art is held against the anchors of the art at the other end, and moves with it.
#[test]
fn a_dimension_to_a_point_off_the_art_moves_with_the_art() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(200.0, 200.0), (330.0, 260.0), (260.0, 330.0)]);
    let before = recipe(&s, id);
    run(&mut s, "select.set", json!({"ids":[rect]}));
    run(&mut s, "object.move", json!({"dx":40,"dy":-30}));
    let d = recipe(&s, id);
    let v = vectorcraft_geom::Vec2::new(40.0, -30.0);
    assert!(near(d.a, before.a + v) && near(d.b, before.b + v), "{before:?} → {d:?}");
    assert!((d.length - before.length).abs() < 1e-6 && (d.offset - before.offset).abs() < 1e-6);
    run(&mut s, "object.scale", json!({"sx":200,"sy":200}));
    assert!((recipe(&s, id).length - 2.0 * before.length).abs() < 1e-6);
}

/// When the art it measures is deleted or gets another number of anchors, a dimension stays where
/// it was, no longer linked: later edits of the art do not move it, and undo links it again.
#[test]
fn a_dimension_whose_art_changes_its_anchors_stays_where_it_was() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (200.0, 150.0), (120.0, 90.0)]);
    let before = recipe(&s, id);
    let label = text(&s, id);
    run(&mut s, "path.insertAnchor", json!({"id":rect,"segment":3,"t":0.5}));
    let d = recipe(&s, id);
    assert!(d.source.is_none(), "no longer linked");
    assert_eq!((d.a, d.b, d.offset, text(&s, id)), (before.a, before.b, before.offset, label.clone()));
    run(&mut s, "select.set", json!({"ids":[rect]}));
    run(&mut s, "object.scale", json!({"sx":150,"sy":150}));
    assert_eq!((recipe(&s, id).a, recipe(&s, id).b), (before.a, before.b), "does not follow the art");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(recipe(&s, id).source, before.source, "undo links it again");
    run(&mut s, "select.set", json!({"ids":[rect]}));
    run(&mut s, "edit.clear", json!({}));
    assert!(s.doc().unwrap().doc.node(NodeId(rect)).is_none());
    assert_eq!((recipe(&s, id).source, text(&s, id)), (None, label));
}

/// A linked dimension whose art is reflected has its line on the reflected side: a line above the
/// top edge is below that edge's points after a flip top to bottom.
#[test]
fn a_linked_dimension_of_reflected_art_has_its_line_on_the_reflected_side() {
    use vectorcraft_doc::dimension::Orientation;
    for orientation in ["aligned", "horizontal"] {
        let mut s = session();
        let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
        let at = anchors(&s, rect);
        let top = |p: vectorcraft_geom::Point| (p.y - 100.0).abs() < 1e-9;
        let (i, j) = (at.iter().position(|p| top(*p)).unwrap(), at.iter().rposition(|p| top(*p)).unwrap());
        let anchor = |anchor: usize| json!({"node":rect,"subpath":0,"anchor":anchor});
        let src = json!({"kind":"points","a":anchor(i),"b":anchor(j)});
        // The line 30 pt above the top edge: the offset is along the left normal of a → b when
        // aligned, downward when horizontal.
        let offset = if orientation == "aligned" && at[i].x > at[j].x { 30.0 } else { -30.0 };
        let id = run(&mut s, "dimension.create", json!({"source":src,"orientation":orientation,"offset":offset}))["id"].as_u64().unwrap();
        let line_y = |s: &Session| recipe(s, id).line_ends().0.y;
        assert!((line_y(&s) - 70.0).abs() < 1e-6, "{orientation}: {}", line_y(&s));
        run(&mut s, "select.set", json!({"ids":[rect]}));
        run(&mut s, "object.reflect", json!({"axis":"horizontal"}));
        let d = recipe(&s, id);
        assert!(d.source.is_some());
        assert!(((d.a.y, d.b.y) == (200.0, 200.0)), "{orientation}: {d:?}");
        assert!((line_y(&s) - 230.0).abs() < 1e-6, "{orientation}: line at {}", line_y(&s));
        let want = if orientation == "aligned" { Orientation::Aligned } else { Orientation::Horizontal };
        assert_eq!(d.orientation, want);
        run(&mut s, "edit.undo", json!({}));
        assert!((line_y(&s) - 70.0).abs() < 1e-6);
    }
}

/// Reflected across the line through the two points it measures, an arch keeps those points
/// where they were, and the dimension's line goes to the other side with the arch.
#[test]
fn a_dimension_of_art_reflected_across_its_points_changes_sides() {
    let mut s = session();
    let arch = json!([{"x":100,"y":200,"out":[100,150]},{"x":200,"y":200,"in":[200,150]}]);
    let curve = run(&mut s, "path.create", json!({"anchors":arch,"closed":false}))["id"].as_u64().unwrap();
    let anchor = |anchor: u64| json!({"node":curve,"subpath":0,"anchor":anchor});
    let src = json!({"kind":"points","a":anchor(0),"b":anchor(1)});
    let id = run(&mut s, "dimension.create", json!({"source":src,"offset":-30}))["id"].as_u64().unwrap();
    assert!((recipe(&s, id).line_ends().0.y - 170.0).abs() < 1e-6);
    run(&mut s, "select.set", json!({"ids":[curve]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,200]}));
    let d = recipe(&s, id);
    assert_eq!((d.a, d.b), (vectorcraft_geom::Point::new(100.0, 200.0), vectorcraft_geom::Point::new(200.0, 200.0)));
    assert!((d.line_ends().0.y - 230.0).abs() < 1e-6, "below with the arch: {:?}", d.line_ends());
    // Reflected together with the art, it ends in the same place.
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "select.set", json!({"ids":[curve, id]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,200]}));
    assert!((recipe(&s, id).line_ends().0.y - 230.0).abs() < 1e-6, "{:?}", recipe(&s, id).line_ends());
}

/// A dimension between two paths that are each reflected in their own way in one edit is not
/// mirrored by the map of one of them.
#[test]
fn a_dimension_between_paths_reflected_each_in_its_own_way_keeps_its_offset() {
    let mut s = session();
    let low = json!([{"x":100,"y":200,"out":[100,150]},{"x":200,"y":200,"in":[200,150]}]);
    let high = json!([{"x":300,"y":200,"out":[300,100]},{"x":400,"y":200,"in":[400,100]}]);
    let first = run(&mut s, "path.create", json!({"anchors":low,"closed":false}))["id"].as_u64().unwrap();
    let second = run(&mut s, "path.create", json!({"anchors":high,"closed":false}))["id"].as_u64().unwrap();
    let src = json!({"kind":"points","a":{"node":first,"subpath":0,"anchor":0},"b":{"node":second,"subpath":0,"anchor":1}});
    let id = run(&mut s, "dimension.create", json!({"source":src,"offset":30}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[first, second]}));
    run(&mut s, "object.transformEach", json!({"reflectY":true}));
    let d = recipe(&s, id);
    assert!(d.source.is_some() && (d.a.y - d.b.y).abs() > 1.0, "{d:?}");
    assert!((d.offset - 30.0).abs() < 1e-6, "{d:?}");
}

/// A recipe's free points are checked: weights that do not add up to 1, or an end that is both an
/// anchor and a free point, are refused without changing the document.
#[test]
fn dimension_free_points_are_checked() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let original = serde_json::to_value(&*s.doc().unwrap().doc).unwrap();
    let w = |anchor: u64, weight: f64| json!({"subpath":0,"anchor":anchor,"weight":weight});
    let held = |anchors: Value, across: f64| json!({"kind":"anchors","node":rect,"anchors":anchors,"across":across});
    let on = |segment: u64, t: f64| json!({"kind":"onSegment","edge":{"node":rect,"subpath":0,"segment":segment},"t":t});
    let anchor = json!({"node":rect,"subpath":0,"anchor":0});
    for free in [
        held(json!([w(1, 0.5), w(2, 0.6)]), 0.0),
        held(json!([w(1, 1.0)]), 0.0),
        held(json!([w(1, 1e300), w(2, -1e300)]), 0.0),
        held(json!([w(1, 0.5), w(99, 0.5)]), 0.0),
        held(json!([w(1, 0.5), w(2, f64::MAX)]), 0.0),
        held(json!([w(1, 0.5), w(2, 0.5)]), 1e300),
        held(json!([w(1, 0.5), w(2, 0.25), w(3, 0.25)]), 0.5),
        on(1, 1.5),
        on(1, -0.1),
        on(99, 0.5),
        json!({"kind":"elsewhere","node":rect}),
    ] {
        let p = json!({"source":{"kind":"points","a":anchor,"freeB":free}});
        assert!(s.execute("dimension.create", &p).is_err(), "{p}");
        assert_eq!(serde_json::to_value(&*s.doc().unwrap().doc).unwrap(), original);
    }
    let both = json!({"source":{"kind":"points","a":anchor,"freeA":held(json!([w(1, 0.5), w(2, 0.5)]), 0.0),"b":anchor}});
    assert!(s.execute("dimension.create", &both).is_err());
    let at = anchors(&s, rect);
    for (free, want) in [(held(json!([w(1, 0.5), w(2, 0.5)]), 0.0), at[1].midpoint(at[2])), (on(1, 0.5), at[1].midpoint(at[2]))] {
        let ok = json!({"source":{"kind":"points","a":anchor,"freeB":free}});
        let id = run(&mut s, "dimension.create", ok)["id"].as_u64().unwrap();
        assert!(near(recipe(&s, id).b, want), "{:?}", recipe(&s, id).b);
    }
}

/// A point off a straight two-anchor line is held by its distance along and across the line: it
/// moves, turns and scales with the line.
#[test]
fn a_dimension_to_a_point_off_a_line_follows_the_line() {
    let mut s = session();
    let line = run(&mut s, "shape.line", json!({"x1":100,"y1":100,"x2":200,"y2":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (150.0, 160.0), (60.0, 130.0)]);
    let d = recipe(&s, id);
    assert!(d.source.as_ref().is_some_and(|src| src.nodes() == vec![NodeId(line), NodeId(line)]), "{:?}", d.source);
    run(&mut s, "dimension.update", json!({"ids":[id],"orientation":"aligned"}));
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.rotate", json!({"angle":90,"origin":[100,100]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":200,"origin":[100,100]}));
    run(&mut s, "object.move", json!({"dx":10,"dy":20}));
    let at = anchors(&s, line);
    let (o, u) = (at[0], at[1] - at[0]);
    // 50 along the line and 60 across it before, now in the line's new frame at twice the size.
    let want = o + u * 0.5 + vectorcraft_geom::Vec2::new(-u.y, u.x) * 0.6;
    let d = recipe(&s, id);
    assert!(near(d.a, o) && near(d.b, want), "{d:?} want {want:?}");
    assert!((d.length - 2.0 * (50.0f64.hypot(60.0))).abs() < 1e-6);
}

/// A point on a straight segment stays on the segment when its handles are edited into a curve.
#[test]
fn a_point_on_a_segment_stays_on_it_when_the_segment_curves() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (200.0, 150.0), (120.0, 90.0)]);
    let at = anchors(&s, rect);
    assert!(near(at[1], vectorcraft_geom::Point::new(200.0, 100.0)) && near(at[2], vectorcraft_geom::Point::new(200.0, 200.0)), "{at:?}");
    let a = |p: vectorcraft_geom::Point| json!({"x":p.x,"y":p.y});
    let mut curved = vec![a(at[0]), a(at[1]), a(at[2]), a(at[3])];
    curved[1]["out"] = json!([240, 110]);
    curved[2]["in"] = json!([240, 190]);
    run(&mut s, "path.setAnchors", json!({"id":rect,"subpaths":[{"anchors":curved,"closed":true}]}));
    let d = recipe(&s, id);
    assert!(d.source.is_some(), "still linked");
    assert!(near(d.b, vectorcraft_geom::Point::new(230.0, 150.0)), "on the curve: {:?}", d.b);
}

/// The line of a linked dimension of a curved two-anchor path goes to the reflected side when the
/// path is reflected: its handles tell a reflection from a turn.
#[test]
fn a_linked_dimension_of_a_reflected_curve_has_its_line_on_the_reflected_side() {
    let mut s = session();
    let arch = json!([{"x":100,"y":200,"out":[100,150]},{"x":200,"y":200,"in":[200,150]}]);
    let curve = run(&mut s, "path.create", json!({"anchors":arch,"closed":false}))["id"].as_u64().unwrap();
    let anchor = |anchor: u64| json!({"node":curve,"subpath":0,"anchor":anchor});
    let src = json!({"kind":"points","a":anchor(0),"b":anchor(1)});
    // The line 30 pt below the two points (the left normal of a → b points down).
    let id = run(&mut s, "dimension.create", json!({"source":src,"offset":30}))["id"].as_u64().unwrap();
    assert!((recipe(&s, id).line_ends().0.y - 230.0).abs() < 1e-6);
    run(&mut s, "select.set", json!({"ids":[curve]}));
    run(&mut s, "object.reflect", json!({"axis":"vertical"}));
    let d = recipe(&s, id);
    assert!((d.a.y, d.b.y) == (200.0, 200.0) && d.a.x > d.b.x, "{d:?}");
    assert!((d.line_ends().0.y - 230.0).abs() < 1e-6, "below the points: {:?}", d.line_ends());
    run(&mut s, "object.reflect", json!({"axis":"horizontal"}));
    let d = recipe(&s, id);
    assert!((d.line_ends().0.y - (d.a.y - 30.0)).abs() < 1e-6, "above the points: {d:?}");
}

/// A point off an arch, whose anchors lie on one line, is held against its anchors and handles:
/// it is reflected with the arch, to the same side as the dimension's line.
#[test]
fn a_point_off_an_arch_is_reflected_with_the_arch() {
    let mut s = session();
    let arch = json!([{"x":100,"y":200,"out":[100,150]},{"x":200,"y":200,"in":[200,150]}]);
    let curve = run(&mut s, "path.create", json!({"anchors":arch,"closed":false}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 200.0), (150.0, 120.0), (60.0, 120.0)]);
    let d = recipe(&s, id);
    assert!(d.source.as_ref().is_some_and(|src| src.nodes() == vec![NodeId(curve), NodeId(curve)]), "{:?}", d.source);
    assert!(near(d.b, vectorcraft_geom::Point::new(150.0, 120.0)), "{:?}", d.b);
    run(&mut s, "select.set", json!({"ids":[curve]}));
    run(&mut s, "object.reflect", json!({"axis":"horizontal","origin":[0,200]}));
    let d = recipe(&s, id);
    assert!(near(d.a, vectorcraft_geom::Point::new(100.0, 200.0)), "{:?}", d.a);
    assert!(near(d.b, vectorcraft_geom::Point::new(150.0, 280.0)), "below with the arch: {:?}", d.b);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "select.set", json!({"ids":[curve]}));
    run(&mut s, "object.reflect", json!({"axis":"vertical","origin":[150,0]}));
    let d = recipe(&s, id);
    assert!(near(d.a, vectorcraft_geom::Point::new(200.0, 200.0)), "{:?}", d.a);
    assert!(near(d.b, vectorcraft_geom::Point::new(150.0, 120.0)), "above with the arch: {:?}", d.b);
}

/// A point beside a curve that is almost straight is still held against it.
#[test]
fn a_point_beside_an_almost_straight_curve_follows_it() {
    let mut s = session();
    let flat = json!([{"x":100,"y":200,"out":[100,199.999]},{"x":200,"y":200}]);
    let curve = run(&mut s, "path.create", json!({"anchors":flat,"closed":false}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 200.0), (150.0, 150.0), (60.0, 120.0)]);
    let d = recipe(&s, id);
    assert!(d.source.as_ref().is_some_and(|src| src.nodes() == vec![NodeId(curve), NodeId(curve)]), "{:?}", d.source);
    run(&mut s, "select.set", json!({"ids":[curve]}));
    run(&mut s, "object.move", json!({"dx":40,"dy":-30}));
    let d = recipe(&s, id);
    assert!(near(d.b, vectorcraft_geom::Point::new(190.0, 120.0)), "{:?}", d.b);
}

/// A point off a loop whose two anchors are in one place is held against the loop's handles.
#[test]
fn a_point_off_a_loop_with_its_anchors_in_one_place_follows_it() {
    let mut s = session();
    let tear = json!([{"x":100,"y":200,"out":[60,120]},{"x":100,"y":200,"in":[140,120]}]);
    let curve = run(&mut s, "path.create", json!({"anchors":tear,"closed":false}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 200.0), (100.0, 60.0), (30.0, 130.0)]);
    let d = recipe(&s, id);
    assert!(d.source.as_ref().is_some_and(|src| src.nodes() == vec![NodeId(curve), NodeId(curve)]), "{:?}", d.source);
    run(&mut s, "select.set", json!({"ids":[curve]}));
    run(&mut s, "object.move", json!({"dx":40,"dy":-30}));
    let d = recipe(&s, id);
    assert!(near(d.b, vectorcraft_geom::Point::new(140.0, 30.0)), "{:?}", d.b);
}

/// Beside a straight two-anchor line, a point is moved by a reflection as a half turn would move
/// it: the line's positions alone do not tell the two apart.
#[test]
fn a_point_beside_a_reflected_line_moves_as_a_half_turn_would() {
    let mut s = session();
    let line = run(&mut s, "shape.line", json!({"x1":100,"y1":100,"x2":200,"y2":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (150.0, 160.0), (60.0, 130.0)]);
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.reflect", json!({"axis":"vertical","origin":[150,0]}));
    let d = recipe(&s, id);
    assert!(d.source.is_some());
    assert!(near(d.a, vectorcraft_geom::Point::new(200.0, 100.0)), "{:?}", d.a);
    assert!(near(d.b, vectorcraft_geom::Point::new(150.0, 40.0)), "{:?}", d.b);
}

/// Opening a closed path that a dimension measures leaves the dimension where it was, no longer
/// linked, and undo links it again.
#[test]
fn a_dimension_whose_art_is_opened_stays_where_it_was() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (200.0, 150.0), (120.0, 90.0)]);
    let before = recipe(&s, id);
    let label = text(&s, id);
    let a = |p: vectorcraft_geom::Point| json!({"x":p.x,"y":p.y});
    let open: Vec<Value> = anchors(&s, rect).into_iter().map(a).collect();
    run(&mut s, "path.setAnchors", json!({"id":rect,"subpaths":[{"anchors":open,"closed":false}]}));
    let d = recipe(&s, id);
    assert!(d.source.is_none(), "no longer linked");
    assert_eq!((d.a, d.b, d.offset, text(&s, id)), (before.a, before.b, before.offset, label));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(recipe(&s, id).source, before.source, "undo links it again");
}

/// An angle whose arm becomes a curve, and an arc radius whose arc stops being circular, stay where
/// they were, no longer linked.
#[test]
fn angle_and_arc_dimensions_whose_art_no_longer_fits_stay_where_they_were() {
    let mut s = session();
    let first = edge(&mut s);
    let second = run(&mut s, "shape.line", json!({"x1":100,"y1":100,"x2":100,"y2":172}))["id"].as_u64().unwrap();
    let angle = json!({"mode":"angular","source":{"kind":"angle","first":{"node":first,"subpath":0,"segment":0},"second":{"node":second,"subpath":0,"segment":0}}});
    let id = run(&mut s, "dimension.create", angle)["id"].as_u64().unwrap();
    let label = text(&s, id);
    let curved = json!([{"x":100,"y":100,"out":[140,130]},{"x":100,"y":172}]);
    run(&mut s, "path.setAnchors", json!({"id":second,"subpaths":[{"anchors":curved,"closed":false}]}));
    assert_eq!((recipe(&s, id).source, text(&s, id)), (None, label));
    let circle = run(&mut s, "shape.ellipse", json!({"x":300,"y":100,"width":144,"height":144}))["id"].as_u64().unwrap();
    let arc = json!({"mode":"radius","source":{"kind":"arc","edge":{"node":circle,"subpath":0,"segment":0}},"options":{"units":"in","precision":1}});
    let id = run(&mut s, "dimension.create", arc)["id"].as_u64().unwrap();
    assert_eq!(text(&s, id), "1.0 in");
    run(&mut s, "select.set", json!({"ids":[circle]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert_eq!((recipe(&s, id).source, text(&s, id)), (None, "1.0 in".to_string()));
}

/// Saved and opened again, a dimension to points that are not anchors still follows its art.
#[test]
fn free_points_survive_a_native_file() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(200.0, 150.0), (330.0, 260.0), (260.0, 330.0)]);
    let before = recipe(&s, id);
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let mut reopened = Session::new();
    reopened.add_document(doc, None);
    assert_eq!(recipe(&reopened, id).source, before.source);
    run(&mut reopened, "select.set", json!({"ids":[rect]}));
    run(&mut reopened, "object.move", json!({"dx":40,"dy":-30}));
    let d = recipe(&reopened, id);
    let v = vectorcraft_geom::Vec2::new(40.0, -30.0);
    assert!(near(d.a, before.a + v) && near(d.b, before.b + v), "{before:?} → {d:?}");
}

/// Saved and opened again, a dimension of a whole path still measures the path: turned, the path
/// is wider than before, and so is the dimension.
#[test]
fn a_dimension_of_a_whole_path_survives_a_native_file() {
    let value = |s: &Session, id: u64| text(s, id).trim_end_matches(" pt").parse::<f64>().unwrap();
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":60}))["id"].as_u64().unwrap();
    let id = run(&mut s, "dimension.create", json!({"source":{"kind":"extent","node":rect},"orientation":"horizontal","offset":-30}))["id"]
        .as_u64()
        .unwrap();
    let before = recipe(&s, id);
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let mut reopened = Session::new();
    reopened.add_document(doc, None);
    assert_eq!(recipe(&reopened, id).source, before.source);
    run(&mut reopened, "select.set", json!({"ids":[rect]}));
    run(&mut reopened, "object.rotate", json!({"angle":30}));
    let wide = 100.0 * 30f64.to_radians().cos() + 60.0 * 30f64.to_radians().sin();
    assert!((value(&reopened, id) - wide).abs() < 0.01, "{} against {wide}", text(&reopened, id));

    // A file whose dimension names a node that is not there: the next edit unlinks it.
    let mut v: Value = serde_json::from_slice(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    fn dangle(v: &mut Value) {
        match v {
            Value::Object(m) => {
                if m.get("kind") == Some(&json!("extent")) {
                    m.insert("node".into(), json!(99999));
                }
                m.values_mut().for_each(dangle);
            }
            Value::Array(a) => a.iter_mut().for_each(dangle),
            _ => {}
        }
    }
    dangle(&mut v);
    let mut dangling = Session::new();
    dangling.add_document(vectorcraft_format::load(&serde_json::to_vec(&v).unwrap()).unwrap(), None);
    assert!(recipe(&dangling, id).source.is_some());
    run(&mut dangling, "shape.rectangle", json!({"x":300,"y":300,"width":10,"height":10}));
    assert!(recipe(&dangling, id).source.is_none(), "no longer linked");
}

/// A linked dimension opened from a file with options it cannot be drawn with keeps its art, as a
/// plain group, when the art it measures is deleted.
#[test]
fn a_dimension_that_cannot_be_drawn_again_keeps_its_art() {
    let mut s = session();
    let rect = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":100,"height":100}))["id"].as_u64().unwrap();
    let id = clicks(&mut s, &[(100.0, 100.0), (200.0, 100.0), (150.0, 80.0)]);
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let mut v: Value = serde_json::from_slice(&saved).unwrap();
    fn bad_precision(v: &mut Value) {
        match v {
            Value::Object(m) => {
                if let Some(Value::Object(o)) = m.get_mut("dimension").and_then(|d| d.get_mut("options")) {
                    o.insert("precision".into(), json!(99));
                }
                m.values_mut().for_each(bad_precision);
            }
            Value::Array(a) => a.iter_mut().for_each(bad_precision),
            _ => {}
        }
    }
    bad_precision(&mut v);
    let mut reopened = Session::new();
    reopened.add_document(vectorcraft_format::load(&serde_json::to_vec(&v).unwrap()).unwrap(), None);
    let art = reopened.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap().clone();
    run(&mut reopened, "select.set", json!({"ids":[rect]}));
    run(&mut reopened, "edit.clear", json!({}));
    let n = reopened.doc().unwrap().doc.node(NodeId(id)).cloned().expect("the dimension's art stays");
    assert!(n.extras().dimension.is_none(), "no longer a live dimension");
    assert_eq!(n.children().unwrap(), &art);
}

/// A linked dimension moved on its own keeps its new place when its art changes later; moved with
/// its art, it keeps its distance from it.
#[test]
fn a_moved_linked_dimension_stays_where_it_was_dropped() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let offset = recipe(&s, id).offset;
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.move", json!({"dx":0,"dy":-20}));
    let moved = recipe(&s, id).offset;
    assert!((moved - offset).abs() - 20.0 < 1e-6 && (moved - offset).abs() > 19.0, "{offset} → {moved}");
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert!((recipe(&s, id).offset - moved).abs() < 1e-6, "no jump back");
    run(&mut s, "select.set", json!({"ids":[line, id]}));
    run(&mut s, "object.move", json!({"dx":0,"dy":50}));
    assert!((recipe(&s, id).offset - moved).abs() < 1e-6, "moved together");
}

/// How many nodes a dimension's art holds, how many of them are type, and whether it has a recipe.
fn art(s: &Session, id: u64) -> (usize, usize, bool) {
    let n = s.doc().unwrap().doc.node(NodeId(id)).unwrap();
    let (mut nodes, mut labels) = (0, 0);
    n.walk(&mut |c| {
        nodes += 1;
        labels += usize::from(matches!(c.kind, NodeKind::Text(_)));
    });
    (nodes, labels, n.extras().dimension.is_some())
}
fn bounds(s: &Session, id: u64) -> [f64; 4] {
    let b = s.doc().unwrap().doc.node(NodeId(id)).unwrap().geometric_bounds().unwrap();
    [b.x0, b.y0, b.x1, b.y1]
}
/// The art of `id`, which holds no type and no recipe, follows a move, a scale, undo and redo as
/// plain art does.
fn stays_plain_art(s: &mut Session, id: u64) {
    let plain = art(s, id);
    assert_eq!(plain.1, 0, "no type left: {plain:?}");
    let at = bounds(s, id);
    run(s, "select.set", json!({"ids":[id]}));
    run(s, "object.move", json!({"dx":30,"dy":-20}));
    assert_eq!(art(s, id), plain, "after a move");
    let moved = bounds(s, id);
    for (k, (a, b)) in at.iter().zip(moved).enumerate() {
        let d = if k % 2 == 0 { 30.0 } else { -20.0 };
        assert!((b - a - d).abs() < 1e-6, "{at:?} moved to {moved:?}");
    }
    run(s, "object.scale", json!({"sx":200,"sy":100}));
    assert_eq!(art(s, id), plain, "after a scale");
    let scaled = bounds(s, id);
    assert!(((scaled[2] - scaled[0]) - 2.0 * (at[2] - at[0])).abs() < 1e-6, "{at:?} scaled to {scaled:?}");
    run(s, "edit.undo", json!({}));
    assert_eq!(art(s, id), plain, "after undoing the scale");
    run(s, "edit.redo", json!({}));
    assert_eq!(art(s, id), plain, "after redoing the scale");
    run(s, "edit.undo", json!({}));
    run(s, "edit.undo", json!({}));
    assert_eq!(art(s, id), plain);
    assert_eq!(bounds(s, id), at);
    assert!(!plain.2, "no recipe left");
}

/// Object › Expand turns a dimension into plain art for good: it no longer follows the art it
/// measured, and one undo gives the live dimension back.
#[test]
fn an_expanded_dimension_stays_plain_art() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let other = dim(&mut s, line);
    let live = art(&s, id);
    assert_eq!((live.1, live.2), (1, true));
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.expand", json!({}));
    stays_plain_art(&mut s, id);
    let plain = art(&s, id);
    let at = bounds(&s, id);
    assert_eq!(art(&s, other), live, "the dimension left out of the selection stays live");
    // The art it measured no longer moves it, and deleting that art leaves it.
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert_eq!(text(&s, other), "20.00 in");
    assert_eq!((art(&s, id), bounds(&s, id)), (plain, at));
    run(&mut s, "edit.clear", json!({}));
    assert_eq!((art(&s, id), bounds(&s, id)), (plain, at));
    assert_eq!(text(&s, other), "20.00 in", "the live dimension stays, no longer linked");
    assert!(recipe(&s, other).source.is_none());
    // Saved and opened again, it is still plain art.
    let doc = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let mut reopened = Session::new();
    reopened.add_document(doc, None);
    stays_plain_art(&mut reopened, id);
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(art(&s, id), live, "one undo gives the live dimension back");
    assert_eq!(text(&s, id), "10.00 in");
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert_eq!(text(&s, id), "20.00 in", "linked again");
}

/// Expand's options: any one that changes a dimension's art ends its link, and an Expand that
/// changes nothing leaves it live.
#[test]
fn expand_options_that_change_a_dimension_end_its_link() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let live = art(&s, id);
    run(&mut s, "select.set", json!({"ids":[id]}));
    assert!(s.execute("object.expand", &json!({"object":false,"stroke":false})).is_err(), "no fills to expand");
    assert_eq!(art(&s, id), live);
    assert!(s.doc().unwrap().doc.dimensions);
    // Stroke alone leaves type as it is, so a stroked label on its own does not end the link.
    let label = {
        let children = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap();
        children.iter().find(|c| matches!(c.kind, NodeKind::Text(_))).unwrap().id.0
    };
    run(&mut s, "paint.setStroke", json!({"ids":[label],"color":"#ff0000"}));
    run(&mut s, "select.set", json!({"ids":[label]}));
    assert!(s.execute("object.expand", &json!({"object":false,"fill":false})).is_err(), "no paths to outline");
    assert!(art(&s, id).2, "still live");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(art(&s, id), live);
    run(&mut s, "select.set", json!({"ids":[id]}));
    // Stroke alone: the label stays type, in art that no longer redraws.
    run(&mut s, "object.expand", json!({"object":false,"fill":false}));
    let plain = art(&s, id);
    assert_eq!(plain.1, 1, "{plain:?}");
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.move", json!({"dx":30,"dy":0}));
    assert_eq!(art(&s, id), plain);
    assert!(!plain.2, "no recipe left");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(art(&s, id), live);
    // Object alone: the lines stay stroked paths, in art that no longer redraws.
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.expand", json!({"stroke":false,"fill":false}));
    let plain = art(&s, id);
    assert_eq!(plain.1, 0, "{plain:?}");
    run(&mut s, "object.move", json!({"dx":30,"dy":0}));
    assert_eq!(art(&s, id), plain);
    assert!(!plain.2, "no recipe left");
}

/// An Expand that ends up changing no art fails and leaves the dimension live: Outline Stroke skips
/// guides, here the dimension's lines.
#[test]
fn an_expand_that_changes_no_art_leaves_a_dimension_live() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let lines: Vec<u64> = {
        let children = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap();
        children.iter().filter(|c| matches!(c.kind, NodeKind::Path { .. })).map(|c| c.id.0).collect()
    };
    run(&mut s, "select.set", json!({"ids":lines}));
    run(&mut s, "view.guides.make", json!({}));
    let live = art(&s, id);
    run(&mut s, "select.set", json!({"ids":[id]}));
    assert!(s.execute("object.expand", &json!({"object":false,"fill":false})).is_err());
    assert_eq!(art(&s, id), live);
    assert!(live.2);
    // A stroked path selected with it expands, and the dimension's art stays live.
    let other = run(&mut s, "shape.rectangle", json!({"x":300,"y":300,"width":50,"height":50}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setStroke", json!({"ids":[other],"color":"#000000"}));
    let moved_with = s.doc().unwrap().doc.node_count();
    run(&mut s, "select.set", json!({"ids":[id, other]}));
    run(&mut s, "object.expand", json!({"object":false,"fill":false}));
    assert_ne!(s.doc().unwrap().doc.node_count(), moved_with, "the rectangle's stroke is outlined");
    assert_eq!(art(&s, id), live);
}

/// A dimension held by a selected group without the art it measures expands as a dimension
/// selected on its own does.
#[test]
fn a_dimension_expanded_in_a_group_without_its_art_stays_plain_art() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.group", json!({}));
    run(&mut s, "object.expand", json!({}));
    stays_plain_art(&mut s, id);
}

/// `command` on a dimension selected with the art it measures, or on a group holding both, leaves
/// the dimension's own lines and label as they are and replaces only the art: the dimension stays
/// where it was, no longer linked, and one undo links it again.
fn keeps_its_own_lines_and_label(command: &str) {
    for grouped in [false, true] {
        let context = format!("{command}, grouped: {grouped}");
        let mut s = session();
        let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
        run(&mut s, "paint.setStroke", json!({"ids":[art_id],"color":"#000000"}));
        run(&mut s, "select.set", json!({"ids":[art_id]}));
        run(&mut s, "appearance.addStroke", json!({}));
        let id = dim(&mut s, art_id);
        let live = art(&s, id);
        let label = text(&s, id);
        let at = bounds(&s, id);
        let linked = recipe(&s, id);
        let measured = s.doc().unwrap().doc.node(NodeId(art_id)).unwrap().clone();
        run(&mut s, "select.set", json!({"ids":[art_id, id]}));
        if grouped {
            run(&mut s, "object.group", json!({}));
        }
        let steps = s.doc().unwrap().history.undo.len();
        let result = run(&mut s, command, json!({}));
        if command == "effect.expandAppearance" {
            let mut own = vec![];
            s.doc().unwrap().doc.node(NodeId(id)).unwrap().walk(&mut |c| own.push(json!(c.id.0)));
            assert!(result["ids"].as_array().unwrap().iter().all(|i| !own.contains(i)), "{context}: {result}");
        }
        assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "{context}: one undo step");
        assert_ne!(s.doc().unwrap().doc.node(NodeId(art_id)), Some(&measured), "{context}: the art is replaced");
        assert_eq!(art(&s, id), live, "{context}: its own lines and label");
        assert_eq!(bounds(&s, id), at, "{context}: where it was");
        stays_unlinked(&mut s, id, &label, &context);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(art(&s, id), live, "{context}");
        assert_eq!(recipe(&s, id), linked, "{context}: one undo links it again");
        assert_eq!(s.doc().unwrap().doc.node(NodeId(art_id)), Some(&measured), "{context}");
    }
}

#[test]
fn expand_leaves_a_dimension_selected_with_its_art_as_it_is() {
    keeps_its_own_lines_and_label("object.expand");
}

#[test]
fn expand_appearance_leaves_a_dimension_selected_with_its_art_as_it_is() {
    keeps_its_own_lines_and_label("effect.expandAppearance");
}

#[test]
fn outline_stroke_leaves_a_dimension_selected_with_its_art_as_it_is() {
    keeps_its_own_lines_and_label("object.path.outlineStroke");
}

/// A selected group whose blur Expand Appearance turns into an image takes the dimension it holds,
/// with the art it measures, into that image.
#[test]
fn a_blurred_group_expanded_takes_its_dimension_into_the_image() {
    let mut s = session();
    let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
    let id = dim(&mut s, art_id);
    run(&mut s, "select.set", json!({"ids":[art_id, id]}));
    let group = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[group]}));
    run(&mut s, "effect.apply", json!({"effect":"blur.gaussian"}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(id)).is_none(), "the dimension is part of the image");
    let mut images = 0;
    d.walk(|n| images += usize::from(matches!(n.kind, NodeKind::Image(_))));
    assert_eq!(images, 1);
}

/// Object › Expand keeps a dimension selected with its art in the export assets that list it and
/// bound to its variables.
#[test]
fn expand_keeps_a_dimension_in_its_asset_and_its_variable_binding() {
    let mut s = session();
    let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setStroke", json!({"ids":[art_id],"color":"#000000"}));
    let id = dim(&mut s, art_id);
    let asset = run(&mut s, "assets.add", json!({"ids":[id]}))["assets"][0].as_u64().unwrap();
    run(&mut s, "variable.define", json!({"name":"Shown","kind":"visibility"}));
    run(&mut s, "variable.bind", json!({"variable":"Shown","ids":[id]}));
    run(&mut s, "select.set", json!({"ids":[art_id, id]}));
    run(&mut s, "object.expand", json!({}));
    let d = &s.doc().unwrap().doc;
    assert!(d.node(NodeId(id)).is_some_and(|n| n.extras().dimension.is_some()), "still a dimension");
    let listed = d.assets.iter().find(|a| a.id == asset).map(|a| a.nodes.clone());
    assert_eq!(listed, Some(vec![NodeId(id)]), "in its asset");
    assert!(d.variables.bindings.get(&NodeId(id)).is_some_and(|b| b.contains("Shown")), "bound to its variable");
}

/// A selected group with a geometry effect of its own bakes that effect into everything it holds
/// when its appearance expands, the dimension's lines and label included: the dimension becomes a
/// plain group of that art, as one selected without its art does.
#[test]
fn a_group_effect_expanded_reaches_the_dimension_it_holds() {
    for command in ["effect.expandAppearance", "object.expand"] {
        let mut s = session();
        let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
        let id = dim(&mut s, art_id);
        let live = art(&s, id);
        run(&mut s, "select.set", json!({"ids":[art_id, id]}));
        let group = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
        run(&mut s, "select.set", json!({"ids":[group]}));
        run(&mut s, "effect.apply", json!({"effect":"distort.puckerBloat"}));
        run(&mut s, command, json!({}));
        let d = &s.doc().unwrap().doc;
        let n = d.node(NodeId(id)).unwrap_or_else(|| panic!("{command}: the dimension's group is still there"));
        assert!(n.extras().dimension.is_none(), "{command}: a plain group");
        assert_ne!(art(&s, id), live, "{command}: its art is baked with the group's effect");
    }
}

/// When the dimension selected with its art is all there is to change, the command fails and
/// changes nothing.
#[test]
fn a_dimension_selected_with_art_that_has_nothing_to_expand_changes_nothing() {
    let mut s = session();
    let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[art_id]}));
    run(&mut s, "paint.setStroke", json!({"none": true}));
    // A plain path: Expand would make a live rectangle a path.
    run(&mut s, "object.expand", json!({}));
    let id = dim(&mut s, art_id);
    for grouped in [false, true] {
        run(&mut s, "select.set", json!({"ids":[art_id, id]}));
        if grouped {
            run(&mut s, "object.group", json!({}));
        }
        let before = s.doc().unwrap().doc.clone();
        let steps = s.doc().unwrap().history.undo.len();
        let info = run(&mut s, "object.expand.info", json!({}));
        assert_eq!(info["appearance"], json!(false), "grouped: {grouped}: Object › Expand… is not dimmed for the dimension's label");
        assert_eq!(info["stroke"], json!(false), "grouped: {grouped}: {info}");
        assert!(!vectorcraft_engine::cmd::expand::has_appearance(&s), "grouped: {grouped}: the menus read this");
        for (command, p) in [
            ("object.expand", json!({})),
            ("object.expand", json!({"object":false,"fill":false})),
            ("object.path.outlineStroke", json!({})),
            ("effect.expandAppearance", json!({})),
        ] {
            assert!(s.execute(command, &p).is_err(), "grouped: {grouped}: {command} {p}");
            assert_eq!(s.doc().unwrap().doc.layers, before.layers, "grouped: {grouped}: {command} {p}");
            assert_eq!(s.doc().unwrap().history.undo.len(), steps, "grouped: {grouped}: {command} {p}");
        }
    }
}

/// The dimension `id` is a dimension that measures nothing, still showing `label`, and a move
/// takes it along without redrawing it from other art.
fn stays_unlinked(s: &mut Session, id: u64, label: &str, context: &str) {
    let live = art(s, id);
    assert!(live.2, "{context}: still a dimension");
    assert!(recipe(s, id).source.is_none(), "{context}: no longer linked");
    assert_eq!(text(s, id), label, "{context}");
    let at = bounds(s, id);
    run(s, "select.set", json!({"ids":[id]}));
    run(s, "object.move", json!({"dx":30,"dy":0}));
    assert_eq!(art(s, id), live, "{context}");
    assert_eq!(text(s, id), label, "{context}");
    let moved = bounds(s, id);
    assert!((moved[0] - at[0] - 30.0).abs() < 1e-6 && (moved[1] - at[1]).abs() < 1e-6, "{context}: {at:?} moved to {moved:?}");
    run(s, "edit.undo", json!({}));
}

/// Expand replacing only the art a selected dimension measures leaves the dimension where it was,
/// still a dimension, without a link to the art that went; undo links it again.
#[test]
fn a_dimension_keeps_no_link_to_the_art_expand_replaced() {
    for circle in [false, true] {
        let mut s = session();
        let shape = json!({"x":100,"y":100,"width":72,"height":72});
        let art_id = run(&mut s, if circle { "shape.ellipse" } else { "shape.rectangle" }, shape)["id"].as_u64().unwrap();
        run(&mut s, "paint.setFill", json!({"ids":[art_id],"gradient":{"kind":"linear"}}));
        let id = if circle {
            run(&mut s, "dimension.create", json!({"mode":"radius","source":{"kind":"circle","node":art_id}}))["id"].as_u64().unwrap()
        } else {
            dim(&mut s, art_id)
        };
        let live = art(&s, id);
        let label = text(&s, id);
        run(&mut s, "select.set", json!({"ids":[art_id, id]}));
        run(&mut s, "object.expand", json!({"object":false,"stroke":false}));
        stays_unlinked(&mut s, id, &label, &format!("circle: {circle}"));
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(art(&s, id), live, "circle: {circle}");
        assert!(recipe(&s, id).source.is_some(), "circle: {circle}: one undo links it again");
    }
}

/// Outline Stroke and Expand Appearance replacing only the art a selected dimension measures
/// leave the dimension where it was, still a dimension, no longer linked; undo links it again.
/// Here the dimension's lines are guides, which neither command changes, and the selection holds
/// the whole dimension or one of its lines.
#[test]
fn a_dimension_outlasts_the_art_outline_stroke_or_expand_appearance_replaced() {
    for command in ["object.path.outlineStroke", "effect.expandAppearance"] {
        for whole in [true, false] {
            let mut s = session();
            let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
            run(&mut s, "paint.setStroke", json!({"ids":[art_id],"color":"#000000"}));
            run(&mut s, "select.set", json!({"ids":[art_id]}));
            run(&mut s, "appearance.addStroke", json!({}));
            let id = dim(&mut s, art_id);
            let lines: Vec<u64> = {
                let children = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap();
                children.iter().filter(|c| matches!(c.kind, NodeKind::Path { .. })).map(|c| c.id.0).collect()
            };
            run(&mut s, "select.set", json!({"ids":lines}));
            run(&mut s, "view.guides.make", json!({}));
            let live = art(&s, id);
            let label = text(&s, id);
            let picked = if whole { id } else { lines[0] };
            run(&mut s, "select.set", json!({"ids":[art_id, picked]}));
            run(&mut s, command, json!({}));
            assert!(s.doc().unwrap().doc.node(NodeId(id)).is_some(), "{command} removed the dimension, whole: {whole}");
            stays_unlinked(&mut s, id, &label, &format!("{command}, whole: {whole}"));
            run(&mut s, "edit.undo", json!({}));
            assert_eq!(art(&s, id), live, "{command}, whole: {whole}");
            assert!(recipe(&s, id).source.is_some(), "{command}, whole: {whole}: one undo links it again");
        }
    }
}

/// Object › Expand that outlines only the stroke of the art a selected dimension measures leaves
/// the dimension a dimension, no longer linked, as Outline Stroke does.
#[test]
fn a_dimension_outlasts_the_stroke_expand_outlined() {
    let mut s = session();
    let art_id = run(&mut s, "shape.rectangle", json!({"x":100,"y":100,"width":72,"height":72}))["id"].as_u64().unwrap();
    run(&mut s, "paint.setStroke", json!({"ids":[art_id],"color":"#000000"}));
    let id = dim(&mut s, art_id);
    let lines: Vec<u64> = {
        let children = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap();
        children.iter().filter(|c| matches!(c.kind, NodeKind::Path { .. })).map(|c| c.id.0).collect()
    };
    run(&mut s, "select.set", json!({"ids":lines}));
    run(&mut s, "view.guides.make", json!({}));
    let live = art(&s, id);
    let label = text(&s, id);
    run(&mut s, "select.set", json!({"ids":[art_id, id]}));
    run(&mut s, "object.expand", json!({"object":false,"fill":false}));
    stays_unlinked(&mut s, id, &label, "object.expand stroke");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(art(&s, id), live);
    assert!(recipe(&s, id).source.is_some(), "one undo links it again");
}

/// A part of a dimension expanded on its own ends the link too, so the move does not undo it.
#[test]
fn expanding_a_part_of_a_dimension_ends_its_link() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let part = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap()[0].id;
    run(&mut s, "select.set", json!({"ids":[part]}));
    run(&mut s, "object.expand", json!({}));
    let plain = art(&s, id);
    assert_eq!(plain.1, 1, "{plain:?}");
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.move", json!({"dx":30,"dy":0}));
    assert_eq!(art(&s, id), plain);
    assert!(!plain.2, "no recipe left");
}

/// The instance of a symbol whose art is a dimension expands to art that no longer redraws.
#[test]
fn a_dimension_in_an_expanded_symbol_instance_stays_plain_art() {
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[100,100],"b":[172,100],"options":{"units":"pt","precision":0}}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[id]}));
    let instance = run(&mut s, "symbol.new", json!({"name":"Measure"}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[instance]}));
    run(&mut s, "object.expand", json!({}));
    let root = s.doc().unwrap().selection.objects[0].0;
    let mut recipes = 0;
    s.doc().unwrap().doc.node(NodeId(root)).unwrap().walk(&mut |c| recipes += usize::from(c.extras().dimension.is_some()));
    assert_eq!(recipes, 0);
    stays_plain_art(&mut s, root);
}

/// Create Outlines on a dimension, also one selected with the art it measures, or on its label
/// alone, ends the link in the same undo step.
#[test]
fn outlining_a_dimension_or_its_label_ends_its_link() {
    for target in ["dimension", "label", "with its art"] {
        let mut s = session();
        let line = edge(&mut s);
        let id = dim(&mut s, line);
        let live = art(&s, id);
        let label = {
            let children = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap();
            children.iter().find(|c| matches!(c.kind, NodeKind::Text(_))).unwrap().id
        };
        let ids = match target {
            "label" => vec![label.0],
            "with its art" => vec![line, id],
            _ => vec![id],
        };
        run(&mut s, "select.set", json!({"ids":ids}));
        run(&mut s, "type.createOutlines", json!({}));
        stays_plain_art(&mut s, id);
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(art(&s, id), live, "one undo gives the live dimension back");
        assert_eq!(text(&s, id), "10.00 in");
    }
}

/// Outline Stroke on a dimension ends its link: the label stays type and the art does not redraw.
#[test]
fn outlining_a_dimensions_strokes_ends_its_link() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let live = art(&s, id);
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.path.outlineStroke", json!({}));
    let plain = art(&s, id);
    assert_eq!(plain.1, 1, "{plain:?}");
    run(&mut s, "object.move", json!({"dx":30,"dy":0}));
    assert_eq!(art(&s, id), plain);
    assert!(!plain.2, "no recipe left");
    run(&mut s, "edit.undo", json!({}));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(art(&s, id), live, "one undo gives the live dimension back");
}

/// Expand Appearance, which the menus offer for a dimension in place of Expand, ends its link too.
#[test]
fn expanding_a_dimensions_appearance_ends_its_link() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let live = art(&s, id);
    run(&mut s, "select.set", json!({"ids":[id]}));
    assert_eq!(run(&mut s, "object.expand.info", json!({}))["appearance"], true);
    run(&mut s, "effect.expandAppearance", json!({}));
    stays_plain_art(&mut s, id);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(art(&s, id), live, "one undo gives the live dimension back");
    assert_eq!(text(&s, id), "10.00 in");
}

/// Ungroup takes the group that holds the recipe away, so the parts stay ordinary art.
#[test]
fn an_ungrouped_dimension_stays_ordinary_art() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let (nodes, ..) = art(&s, id);
    run(&mut s, "select.set", json!({"ids":[id]}));
    run(&mut s, "object.ungroup", json!({}));
    assert!(s.doc().unwrap().doc.node(NodeId(id)).is_none());
    let parts = s.doc().unwrap().selection.objects.clone();
    assert_eq!(parts.len(), nodes - 1);
    let boxes = |s: &Session| parts.iter().map(|p| s.doc().unwrap().doc.node(*p).unwrap().geometric_bounds().unwrap()).collect::<Vec<_>>();
    let at = boxes(&s);
    run(&mut s, "object.move", json!({"dx":30,"dy":0}));
    for (a, b) in at.iter().zip(boxes(&s)) {
        assert!((b.x0 - a.x0 - 30.0).abs() < 1e-6 && (b.y0 - a.y0).abs() < 1e-6, "{a:?} moved to {b:?}");
    }
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert!(!s.doc().unwrap().doc.dimensions);
}

#[test]
fn dimension_options_are_persistent_and_invalid_updates_report_errors() {
    let mut s = session();
    s.set_tool_option_cmd(&json!({"tool":"dimension","values":{"style":{"scale":"10:1","units":"mm","precision":3},"mode":"radial"}})).unwrap();
    assert_eq!(s.tool_options_of("dimension")["style"]["scale"], "10:1");
    assert!(s.set_tool_option_cmd(&json!({"tool":"dimension","values":{"style":{"scale":"1:0"}}})).is_err());
    let o: vectorcraft_doc::dimension::DimensionOptions = serde_json::from_value(s.tool_options_of("dimension")["style"].clone()).unwrap();
    assert_eq!(o.label(72.0, false).unwrap(), "2.540 mm");
    let picas: vectorcraft_doc::dimension::DimensionOptions = serde_json::from_value(json!({"units":"pc","precision":1})).unwrap();
    assert_eq!(picas.label(72.0, false).unwrap(), "6.0 pc");
    let prefs = serde_json::to_value(&s.prefs).unwrap();
    let mut fresh = Session::new();
    fresh.prefs = serde_json::from_value(prefs).unwrap();
    assert_eq!(fresh.tool_options_of("dimension")["style"], s.tool_options_of("dimension")["style"]);
}

#[test]
fn dimension_unlinked_geometry_scales_and_updates_its_label() {
    let mut s = session();
    let id = run(&mut s, "dimension.create", json!({"a":[100,100],"b":[172,100],"options":{"units":"in","precision":0}}))["id"].as_u64().unwrap();
    run(&mut s, "object.scale", json!({"sx":200,"sy":200}));
    assert_eq!(text(&s, id), "2 in");
}

#[test]
fn dimension_linked_angle_and_circle_update_and_reject_non_circles() {
    let mut s = session();
    let first = edge(&mut s);
    let second = run(&mut s, "shape.line", json!({"x1":100,"y1":100,"x2":100,"y2":172}))["id"].as_u64().unwrap();
    let id=run(&mut s,"dimension.create",json!({"mode":"angular","source":{"kind":"angle","first":{"node":first,"subpath":0,"segment":0},"second":{"node":second,"subpath":0,"segment":0}},"options":{"precision":1}}))["id"].as_u64().unwrap();
    assert_eq!(text(&s, id), "90.0°");
    run(&mut s, "select.set", json!({"ids":[second]}));
    run(&mut s, "object.rotate", json!({"angle":45,"origin":[100,100]}));
    assert!(["45.0°", "135.0°"].contains(&text(&s, id).as_str()));
    let circle = run(&mut s, "shape.ellipse", json!({"x":100,"y":100,"width":144,"height":144}))["id"].as_u64().unwrap();
    let id = run(
        &mut s,
        "dimension.create",
        json!({"mode":"diameter","source":{"kind":"circle","node":circle},"options":{"units":"in","precision":0}}),
    )["id"]
        .as_u64()
        .unwrap();
    run(&mut s, "select.set", json!({"ids":[circle]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":200}));
    assert_eq!(text(&s, id), "4 in");
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    // An ellipse has no single radius: the dimension stays where it was, no longer linked.
    assert_eq!(text(&s, id), "4 in");
    assert!(recipe(&s, id).source.is_none());
    assert!(s.execute("dimension.create", &json!({"mode":"radius","source":{"kind":"circle","node":circle}})).is_err());
}

#[test]
fn dimension_style_update_keeps_links_and_can_be_undone() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    run(
        &mut s,
        "dimension.update",
        json!({"ids":[id],"offset":-42,"options":{"units":"mm","precision":2,"hideUnits":true,"textPosition":"between","textAlignment":"right","lineColor":[10,20,30]}}),
    );
    assert_eq!(text(&s, id), "25.40");
    let d = recipe(&s, id);
    assert!(d.source.is_some());
    assert_eq!(d.offset, -42.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(text(&s, id), "10.00 in");
}

#[test]
fn dimension_cannot_measure_its_own_generated_art() {
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    let child = s.doc().unwrap().doc.node(NodeId(id)).unwrap().children().unwrap()[0].id;
    assert!(s.execute("dimension.create", &json!({"source":{"kind":"segment","edge":{"node":child,"subpath":0,"segment":0}}})).is_err());
}

#[test]
fn unknown_dimension_options_are_refused_by_commands_but_a_newer_file_still_opens() {
    let mut s = session();
    let line = edge(&mut s);
    let source = json!({"kind":"segment","edge":{"node":line,"subpath":0,"segment":0}});
    let bad = s.execute("dimension.create", &json!({"source":source,"options":{"units":"in","color":"red"}}));
    assert!(bad.unwrap_err().to_string().contains("Unknown dimension option color"));
    assert!(s.set_tool_option_cmd(&json!({"tool":"dimension","values":{"style":{"precison":2}}})).is_err());
    let id = dim(&mut s, line);
    let saved = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let mut v: Value = serde_json::from_slice(&saved).unwrap();
    // A later version's dimension carries fields this one doesn't know.
    fn add_unknown(v: &mut Value) {
        match v {
            Value::Object(m) => {
                if let Some(Value::Object(d)) = m.get_mut("dimension") {
                    d.insert("later".into(), json!(1));
                    if let Some(Value::Object(o)) = d.get_mut("options") {
                        o.insert("later".into(), json!(true));
                    }
                }
                m.values_mut().for_each(add_unknown);
            }
            Value::Array(a) => a.iter_mut().for_each(add_unknown),
            _ => {}
        }
    }
    add_unknown(&mut v);
    let doc = vectorcraft_format::load(&serde_json::to_vec(&v).unwrap()).unwrap();
    assert!(doc.node(NodeId(id)).unwrap().extras().dimension.is_some());
}

/// With Shift down, the dashed line from the first picked point follows the 45-degree step the
/// second click will pick (#2432), not the raw pointer.
#[test]
fn shift_shows_the_constrained_line_while_picking_the_second_point() {
    use vectorcraft_engine::tools::Overlay;
    let mut s = session();
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    ev(&mut s, PointerKind::Down, 300.0, 300.0);
    ev(&mut s, PointerKind::Up, 300.0, 300.0);
    let end = |s: &mut Session, shift: bool| {
        let e = PointerEvent::new(PointerKind::Move, 372.0, 310.0).with_mods(Mods { shift, ..Mods::default() });
        s.pointer(&e, ViewInfo::default()).unwrap();
        s.overlays(ViewInfo::default()).into_iter().find_map(|o| if let Overlay::Line { b, .. } = o { Some(b) } else { None }).unwrap()
    };
    assert_eq!(end(&mut s, false).y, 310.0);
    assert!((end(&mut s, true).y - 300.0).abs() < 1e-9);
}

/// A click near a line, away from its anchors, snaps to the closest point of the line (#2432).
#[test]
fn clicking_near_a_line_snaps_the_measured_point_onto_it() {
    let mut s = session();
    edge(&mut s);
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    for (x, y) in [(130.0, 103.0), (150.0, 97.0), (140.0, 130.0)] {
        ev(&mut s, PointerKind::Down, x, y);
        ev(&mut s, PointerKind::Up, x, y);
    }
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(text(&s, id), "20.00 pt");
}

/// Both picked points snap onto anchors a few pixels away, the rubber band and a ring show the
/// snap before the click, and the dimension stays linked to the anchors (#2516).
#[test]
fn both_points_snap_onto_anchors_with_a_visible_indicator() {
    use vectorcraft_engine::tools::Overlay;
    let mut s = session();
    let line = edge(&mut s);
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    ev(&mut s, PointerKind::Move, 103.0, 102.0);
    let ring = |s: &mut Session| {
        s.overlays(ViewInfo::default())
            .into_iter()
            .any(|o| matches!(o, Overlay::Anchor { p, size, .. } if size > 8.0 && p.x == 100.0 && p.y == 100.0))
    };
    assert!(ring(&mut s), "ring on the anchor before the first click");
    ev(&mut s, PointerKind::Down, 103.0, 102.0);
    ev(&mut s, PointerKind::Up, 103.0, 102.0);
    ev(&mut s, PointerKind::Move, 169.0, 98.0);
    let end = s.overlays(ViewInfo::default()).into_iter().find_map(|o| if let Overlay::Line { b, .. } = o { Some(b) } else { None }).unwrap();
    assert_eq!((end.x, end.y), (172.0, 100.0), "rubber band ends on the anchor");
    ev(&mut s, PointerKind::Down, 169.0, 98.0);
    ev(&mut s, PointerKind::Up, 169.0, 98.0);
    ev(&mut s, PointerKind::Down, 136.0, 130.0);
    ev(&mut s, PointerKind::Up, 136.0, 130.0);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(text(&s, id), "72.00 pt");
    run(&mut s, "select.set", json!({"ids":[line]}));
    run(&mut s, "object.scale", json!({"sx":200,"sy":100}));
    assert_eq!(text(&s, id), "144.00 pt", "linked to both anchors");
}

/// Smart Guides for the Linear picks: hovering away from any line, the first point slides into
/// line with another object's corner and its guides show before the click; the second slides into
/// line with the other corner, with no construction guide through the first point pulling it
/// sideways. The dimension measures between the snapped points.
#[test]
fn linear_picks_show_and_snap_to_smart_guides() {
    use vectorcraft_engine::tools::Overlay;
    let mut s = session();
    run(&mut s, "shape.rectangle", json!({"x":300,"y":200,"width":80,"height":60}));
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", ViewInfo::default()).unwrap();
    let overlays = |s: &mut Session| s.overlays(ViewInfo::default());
    // In line with the rectangle's top edge, 2.5 pt off it: the pick lands on its level.
    ev(&mut s, PointerKind::Move, 250.0, 202.5);
    let o = overlays(&mut s);
    assert!(o.iter().any(|o| matches!(o, Overlay::Anchor { p, .. } if p.x == 250.0 && p.y == 200.0)), "the pick shows on the guide: {o:?}");
    assert!(o.iter().any(|o| matches!(o, Overlay::Line { .. } | Overlay::Label { .. })), "its guides show before the click: {o:?}");
    ev(&mut s, PointerKind::Down, 250.0, 202.5);
    ev(&mut s, PointerKind::Up, 250.0, 202.5);
    // Below the first point, 1.5 pt to its right and 2.5 pt off the bottom edge's level: the
    // rubber band ends on that level, where the pointer is across it.
    ev(&mut s, PointerKind::Move, 251.5, 262.5);
    let end = overlays(&mut s).into_iter().find_map(|o| if let Overlay::Line { b, dashed: true, .. } = o { Some(b) } else { None }).unwrap();
    assert_eq!((end.x, end.y), (251.5, 260.0), "in line with the bottom edge, not on a vertical through the first point");
    ev(&mut s, PointerKind::Down, 251.5, 262.5);
    ev(&mut s, PointerKind::Up, 251.5, 262.5);
    ev(&mut s, PointerKind::Move, 220.0, 230.0);
    assert!(!overlays(&mut s).iter().any(|o| matches!(o, Overlay::Label { .. })), "placing: the guides are gone");
    ev(&mut s, PointerKind::Down, 220.0, 230.0);
    ev(&mut s, PointerKind::Up, 220.0, 230.0);
    let id = s.doc().unwrap().selection.objects[0].0;
    // Aligned from (250, 200) to (251.5, 260): √(1.5² + 60²).
    assert_eq!(text(&s, id), "60.02 pt");
}

/// A shape with a curve at each top corner, for the tests of where Linear picks land.
const CURVED_TOP: &str = "M130 358 C185 358 230 315 230 260 L425 260 C490 260 541 305 541 358 L541 440 L130 440 Z";

/// Where the next Linear click lands (the last marker the overlays draw, and its size) and the
/// labels shown, with the pointer at (`x`, `y`).
fn linear_marker(s: &mut Session, view: ViewInfo, x: f64, y: f64) -> (Option<(vectorcraft_geom::Point, f32)>, Vec<String>) {
    use vectorcraft_engine::tools::Overlay;
    s.pointer(&PointerEvent::new(PointerKind::Move, x, y), view).unwrap();
    let o = s.overlays(view);
    let at = o.iter().filter_map(|o| if let Overlay::Anchor { p, size, .. } = o { Some((*p, *size)) } else { None }).next_back();
    let labels = o.iter().filter_map(|o| if let Overlay::Label { text, .. } = o { Some(text.clone()) } else { None }).collect();
    (at, labels)
}

/// A plain click at (`x`, `y`): the pointer moves there, then presses and releases.
fn linear_click(s: &mut Session, view: ViewInfo, x: f64, y: f64) {
    for kind in [PointerKind::Move, PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, x, y), view).unwrap();
    }
}

/// Linear picks land on the anchors and paths of the art with an "anchor" or "path" label, and on
/// the ends and lines of dimensions already drawn, from as far as on the art's anchors: zoomed in,
/// a dimension started a few pixels from the end of another starts exactly at that end.
#[test]
fn linear_picks_land_on_anchors_paths_and_drawn_dimensions() {
    let (plain, zoomed) = (ViewInfo::default(), ViewInfo { zoom: 8.0, ..ViewInfo::default() });
    let mut s = session();
    run(&mut s, "path.create", json!({"d": CURVED_TOP}));
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", plain).unwrap();
    let pt = vectorcraft_geom::Point::new;
    let hover = |s: &mut Session, view: ViewInfo, x: f64, y: f64| {
        let (at, labels) = linear_marker(s, view, x, y);
        (at.map(|(p, _)| p), labels)
    };
    // Beside the end of the curve: on the anchor, labeled.
    assert_eq!(hover(&mut s, plain, 231.0, 262.0), (Some(pt(230.0, 260.0)), vec!["anchor".to_string()]));
    // Beside the curve: on its closest point, labeled.
    let (at, labels) = hover(&mut s, plain, 200.0, 337.0);
    assert!(at.is_some_and(|q| q.distance(pt(200.0, 337.0)) < 6.0 && q != pt(200.0, 337.0)), "{at:?}");
    assert_eq!(labels, ["path"]);
    // A first dimension from the bottom of the curve to its top, placed above the art.
    linear_click(&mut s, plain, 130.0, 358.0);
    linear_click(&mut s, plain, 230.0, 260.0);
    linear_click(&mut s, plain, 180.0, 207.0);
    let first = s.doc().unwrap().selection.objects[0].0;
    assert_eq!((recipe(&s, first).a, recipe(&s, first).b), (pt(130.0, 358.0), pt(230.0, 260.0)));
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", zoomed).unwrap();
    // Zoomed in, 5.8 pixels from the end of its dimension line: on that end, marked as an anchor.
    assert_eq!(linear_marker(&mut s, zoomed, 230.6, 207.4), (Some((pt(230.0, 207.0), 10.0)), vec!["anchor".to_string()]));
    // 6.4 pixels beside its extension line: on the line.
    let (at, labels) = hover(&mut s, zoomed, 230.8, 230.0);
    assert!(at.is_some_and(|q| (q.x - 230.0).abs() < 1e-6 && (q.y - 230.0).abs() < 1e-6), "{at:?}");
    assert_eq!(labels, ["path"]);
    // The next dimension starts where the first one's line ends.
    linear_click(&mut s, zoomed, 230.6, 207.4);
    linear_click(&mut s, zoomed, 425.0, 260.0);
    linear_click(&mut s, zoomed, 330.0, 207.0);
    let second = s.doc().unwrap().selection.objects[0].0;
    assert_ne!(second, first);
    let d = recipe(&s, second);
    assert!(near(d.a, pt(230.0, 207.0)) && near(d.b, pt(425.0, 260.0)), "{:?} {:?}", d.a, d.b);
}

/// An anchor of the art that a dimension already measures takes a point picked beside it, not the
/// end of the extension line a few points off it. A locked dimension pulls no point, and with
/// Smart Guides and Snap to Point off no dimension pulls a point and no label shows. A press on
/// the end of a dimension's line over a segment of the art that moves a few pixels before the
/// release picks that end, as a press on an anchor does.
#[test]
fn an_anchor_beside_a_drawn_dimension_takes_the_point() {
    let (plain, zoomed) = (ViewInfo::default(), ViewInfo { zoom: 8.0, ..ViewInfo::default() });
    let off = ViewInfo { smart_guides: false, snap_to_point: false, ..zoomed };
    let mut s = session();
    run(&mut s, "path.create", json!({"d": CURVED_TOP}));
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", plain).unwrap();
    let pt = vectorcraft_geom::Point::new;
    linear_click(&mut s, plain, 130.0, 358.0);
    linear_click(&mut s, plain, 230.0, 260.0);
    linear_click(&mut s, plain, 180.0, 207.0);
    let first = s.doc().unwrap().selection.objects[0].0;
    // Locked, the first dimension's line end does not pull a point 5.8 pixels away.
    run(&mut s, "object.lock", json!({}));
    let (at, _) = linear_marker(&mut s, zoomed, 230.6, 207.4);
    assert!(at.is_none_or(|(p, _)| p != pt(230.0, 207.0)), "a locked dimension: {at:?}");
    run(&mut s, "object.unlockAll", json!({}));
    run(&mut s, "select.none", json!({}));
    // 2.5 pixels above the curve's top anchor and 1.5 pixels below the end of the extension line
    // that starts 4 points above it: on the anchor, linked to the art.
    assert_eq!(linear_marker(&mut s, plain, 230.0, 257.5), (Some((pt(230.0, 260.0), 10.0)), vec!["anchor".to_string()]));
    // Smart Guides and Snap to Point off: no pull to the dimension's line end, and no labels.
    let (at, labels) = linear_marker(&mut s, off, 230.6, 207.4);
    assert!(at.is_none_or(|(p, _)| p != pt(230.0, 207.0)) && labels.is_empty(), "{at:?} {labels:?}");
    let (at, labels) = linear_marker(&mut s, off, 230.1, 260.1);
    assert!(at.is_some_and(|(p, _)| p == pt(230.0, 260.0)) && labels.is_empty(), "{at:?} {labels:?}");
    // Snap to Point on and Smart Guides off: the line end takes the point, and no label shows.
    assert_eq!(linear_marker(&mut s, ViewInfo { snap_to_point: true, ..off }, 230.6, 207.4), (Some((pt(230.0, 207.0), 10.0)), vec![]));
    // A second dimension whose line lies on the bottom edge, from x 425 to x 541.
    linear_click(&mut s, plain, 425.0, 260.0);
    linear_click(&mut s, plain, 541.0, 358.0);
    linear_click(&mut s, plain, 480.0, 440.0);
    let second = s.doc().unwrap().selection.objects[0].0;
    assert_ne!(second, first);
    run(&mut s, "select.none", json!({}));
    // A press 1.4 pixels from its line's left end, on the bottom edge, moved 5 pixels: the end is
    // picked, and no dimension of the edge is dragged out.
    for (kind, x) in [(PointerKind::Move, 426.0), (PointerKind::Down, 426.0), (PointerKind::Drag, 431.0), (PointerKind::Up, 431.0)] {
        s.pointer(&PointerEvent::new(kind, x, 441.0), plain).unwrap();
    }
    linear_click(&mut s, plain, 541.0, 440.0);
    linear_click(&mut s, plain, 480.0, 470.0);
    let third = s.doc().unwrap().selection.objects[0].0;
    assert_ne!(third, second);
    let d = recipe(&s, third);
    assert!(near(d.a, pt(425.0, 440.0)) && near(d.b, pt(541.0, 440.0)), "{:?} {:?}", d.a, d.b);
    // A dimension of the left edge whose line lies 4 points off it, where its extension lines
    // start: zoomed in, the end of its line still takes a point 4.7 pixels away.
    run(&mut s, "select.none", json!({}));
    linear_click(&mut s, plain, 130.0, 358.0);
    linear_click(&mut s, plain, 130.0, 440.0);
    linear_click(&mut s, plain, 126.0, 400.0);
    run(&mut s, "select.none", json!({}));
    assert_eq!(linear_marker(&mut s, zoomed, 125.5, 358.3), (Some((pt(126.0, 358.0), 10.0)), vec!["anchor".to_string()]));
}

/// The Smart Guide lines a pointer move at (`x`, `y`) shows, as their ends.
fn guide_lines(s: &mut Session, view: ViewInfo, x: f64, y: f64) -> Vec<(vectorcraft_geom::Point, vectorcraft_geom::Point)> {
    use vectorcraft_engine::tools::Overlay;
    s.pointer(&PointerEvent::new(PointerKind::Move, x, y), view).unwrap();
    s.overlays(view).into_iter().filter_map(|o| if let Overlay::Line { a, b, .. } = o { Some((a, b)) } else { None }).collect()
}

/// An edge with anchors at x 100, 200 and 300, and a dimension from 100 to 200 placed 30 points
/// above it, with the Dimension tool active and nothing selected: the session and the dimension.
fn level_start(view: ViewInfo) -> (Session, u64) {
    let pt = vectorcraft_geom::Point::new;
    let mut s = session();
    run(&mut s, "path.create", json!({"d": "M100 200 L200 200 L300 200 L300 300 L100 300 Z"}));
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", view).unwrap();
    linear_click(&mut s, view, 100.0, 200.0);
    linear_click(&mut s, view, 200.0, 200.0);
    linear_click(&mut s, view, 150.0, 170.0);
    let first = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    run(&mut s, "select.none", json!({}));
    (s, first)
}

/// The next dimension, from 200 to 300, placed with the pointer at `y`: where its line lies.
fn level_next(s: &mut Session, view: ViewInfo, y: f64) -> (vectorcraft_geom::Point, vectorcraft_geom::Point) {
    linear_click(s, view, 200.0, 200.0);
    linear_click(s, view, 300.0, 200.0);
    linear_click(s, view, 250.0, y);
    let id = s.doc().unwrap().selection.objects[0].0;
    recipe(s, id).line_ends()
}

/// A dimension placed next to another, with its line within the Smart Guides snapping tolerance of
/// the level of the other's parallel line, has its line on that level. A level further off, Smart
/// Guides off, a locked dimension or one that is not parallel leaves the line where the pointer
/// puts it.
#[test]
fn a_dimension_line_placed_beside_another_lands_on_its_level() {
    let plain = ViewInfo::default();
    let off = ViewInfo { smart_guides: false, ..plain };
    let pt = vectorcraft_geom::Point::new;
    // 2.5 points below the first line's level, and 3.5 above it: on the level.
    for y in [172.5, 166.5] {
        let (mut s, _) = level_start(plain);
        assert_eq!(level_next(&mut s, plain, y), (pt(200.0, 170.0), pt(300.0, 170.0)), "pointer at y {y}");
    }
    // 10 points off: where the pointer is.
    let (mut s, _) = level_start(plain);
    assert_eq!(level_next(&mut s, plain, 180.0), (pt(200.0, 180.0), pt(300.0, 180.0)));
    // Smart Guides off: where the pointer is.
    let (mut s, _) = level_start(off);
    assert_eq!(level_next(&mut s, off, 172.5), (pt(200.0, 172.5), pt(300.0, 172.5)));
    // A locked dimension does not pull the line.
    let (mut s, first) = level_start(plain);
    run(&mut s, "select.set", json!({"ids":[first]}));
    run(&mut s, "object.lock", json!({}));
    assert_eq!(level_next(&mut s, plain, 172.5), (pt(200.0, 172.5), pt(300.0, 172.5)));
    // A dimension of the right edge has a vertical line, which is not parallel to the first: the
    // pointer 2.5 points right of the first line's left end, within reach across the vertical
    // line, places it where the pointer is.
    let (mut s, _) = level_start(plain);
    linear_click(&mut s, plain, 300.0, 200.0);
    linear_click(&mut s, plain, 300.0, 300.0);
    linear_click(&mut s, plain, 102.5, 172.5);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(recipe(&s, id).line_ends(), (pt(102.5, 200.0), pt(102.5, 300.0)));
}

/// While a dimension is placed with its line on the level of another, one alignment guide runs
/// along the level across both lines. Off the level, after a dropped press and with Smart Guides
/// or Alignment Guides off, no guide shows.
#[test]
fn a_dimension_line_on_another_level_shows_one_guide_across_both() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (mut s, _) = level_start(plain);
    linear_click(&mut s, plain, 200.0, 200.0);
    linear_click(&mut s, plain, 300.0, 200.0);
    let guides = guide_lines(&mut s, plain, 250.0, 172.5);
    assert!(matches!(guides.as_slice(), [(a, b)] if near(*a, pt(100.0, 170.0)) && near(*b, pt(300.0, 170.0))), "{guides:?}");
    // Smart Guides turned off before the next pointer move: the guide goes.
    let off = ViewInfo { smart_guides: false, ..plain };
    assert!(s.overlays(off).iter().all(|o| !matches!(o, vectorcraft_engine::tools::Overlay::Line { .. })), "{:?}", s.overlays(off));
    assert!(guide_lines(&mut s, plain, 250.0, 180.0).is_empty(), "no guide off the level");
    // A press on the level that the app drops cancels the preview, and its guide goes with it.
    assert!(!guide_lines(&mut s, plain, 250.0, 172.5).is_empty());
    s.pointer(&PointerEvent::new(PointerKind::Down, 250.0, 172.5), plain).unwrap();
    s.drop_tool_press(plain).unwrap();
    assert!(s.overlays(plain).iter().all(|o| !matches!(o, vectorcraft_engine::tools::Overlay::Line { .. })), "{:?}", s.overlays(plain));
    // Alignment Guides off: the line still lands on the level, with no guide.
    let (mut s, _) = level_start(plain);
    run(&mut s, "prefs.set", json!({"key": "alignmentGuides", "value": false}));
    linear_click(&mut s, plain, 200.0, 200.0);
    linear_click(&mut s, plain, 300.0, 200.0);
    assert!(guide_lines(&mut s, plain, 250.0, 172.5).is_empty());
    linear_click(&mut s, plain, 250.0, 172.5);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(recipe(&s, id).line_ends(), (pt(200.0, 170.0), pt(300.0, 170.0)));
}

/// A drag with the Dimension tool from a dimension already drawn moves its line as far across as
/// the pointer moved, keeps what it measures and its link to the art, and is one undo step. A
/// click on its line does not move it, and neither does Escape during the drag.
#[test]
fn dragging_a_placed_dimension_with_the_dimension_tool_moves_its_line() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (mut s, first) = level_start(plain);
    let (count, label) = (s.doc().unwrap().doc.node_count(), text(&s, first));
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Move, from), (PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), plain).unwrap();
        }
    };
    // Dragged up and to the side: the line goes up as far as the pointer, and stays over the
    // points it measures.
    drag(&mut s, (150.0, 170.0), (180.0, 130.0));
    assert!(!s.in_interaction());
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
    assert_eq!((recipe(&s, first).a, recipe(&s, first).b), (pt(100.0, 200.0), pt(200.0, 200.0)));
    assert_eq!((s.doc().unwrap().doc.node_count(), text(&s, first)), (count, label));
    // One undo step puts it back.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
    // Still linked: moving the art moves its points, and the line keeps its distance from them.
    let art = s.doc().unwrap().doc.layers[0].children().unwrap()[0].id.0;
    run(&mut s, "select.set", json!({"ids":[art]}));
    run(&mut s, "object.move", json!({"dx":0,"dy":10}));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 140.0), pt(200.0, 140.0)));
    run(&mut s, "object.move", json!({"dx":0,"dy":-10}));
    run(&mut s, "select.none", json!({}));
    run(&mut s, "dimension.update", json!({"ids":[first],"at":[150,170]}));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    // A click on the line picks a point there and leaves the dimension where it is.
    linear_click(&mut s, plain, 150.0, 170.0);
    s.tool_key(ToolKey::Escape, Mods::default(), plain).unwrap();
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    // Escape during the drag leaves it where it was.
    ev(&mut s, PointerKind::Down, 150.0, 170.0);
    ev(&mut s, PointerKind::Drag, 150.0, 120.0);
    assert!(s.in_interaction());
    s.tool_key(ToolKey::Escape, Mods::default(), plain).unwrap();
    ev(&mut s, PointerKind::Up, 150.0, 120.0);
    assert!(!s.in_interaction());
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    // A locked dimension is not moved: the drag from its line drags out nothing either.
    run(&mut s, "select.set", json!({"ids":[first]}));
    run(&mut s, "object.lock", json!({}));
    drag(&mut s, (150.0, 170.0), (150.0, 130.0));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    assert_eq!(s.doc().unwrap().doc.node_count(), count);
}

/// A dimension moved with the Dimension tool lands on the level of a parallel dimension line
/// within the Smart Guides snapping tolerance, and its own line before the drag is not a level.
#[test]
fn a_dimension_moved_beside_another_lands_on_its_level() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (mut s, first) = level_start(plain);
    linear_click(&mut s, plain, 200.0, 200.0);
    linear_click(&mut s, plain, 300.0, 200.0);
    linear_click(&mut s, plain, 250.0, 150.0);
    let second = s.doc().unwrap().selection.objects[0].0;
    run(&mut s, "select.none", json!({}));
    assert_eq!(recipe(&s, second).line_ends(), (pt(200.0, 150.0), pt(300.0, 150.0)));
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), plain).unwrap();
        }
    };
    // 3.5 points off its own level, with no other level there: where the pointer puts it.
    drag(&mut s, (150.0, 170.0), (150.0, 166.5));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 166.5), pt(200.0, 166.5)));
    // 2.5 points below the first line's new level: on it.
    drag(&mut s, (250.0, 150.0), (250.0, 169.0));
    assert_eq!(recipe(&s, second).line_ends(), (pt(200.0, 166.5), pt(300.0, 166.5)));
}

/// `dimension.update` with `at` puts a linear dimension's line, an angular dimension's arc or a
/// radius line through the point, and keeps what each measures; a bad `at` changes nothing.
#[test]
fn dimension_update_at_moves_the_line_through_the_point() {
    let mut s = session();
    let line = edge(&mut s);
    let linear = dim(&mut s, line);
    let label = text(&s, linear);
    run(&mut s, "dimension.update", json!({"ids":[linear],"at":[130,50]}));
    let d = recipe(&s, linear);
    assert!((d.line_ends().0.y - 50.0).abs() < 1e-9 && (d.line_ends().1.y - 50.0).abs() < 1e-9, "{:?}", d.line_ends());
    assert_eq!(text(&s, linear), label);
    let angular = run(&mut s, "dimension.create", json!({"mode":"angular","a":[100,0],"vertex":[0,0],"b":[0,100]}))["id"].as_u64().unwrap();
    run(&mut s, "dimension.update", json!({"ids":[angular],"at":[60,0]}));
    assert!((recipe(&s, angular).offset - 60.0).abs() < 1e-9);
    let circle = run(&mut s, "shape.ellipse", json!({"x":100,"y":100,"width":144,"height":144}))["id"].as_u64().unwrap();
    let radius = run(&mut s, "dimension.create", json!({"mode":"radius","source":{"kind":"circle","node":circle}}))["id"].as_u64().unwrap();
    run(&mut s, "dimension.update", json!({"ids":[radius],"at":[172,400]}));
    let d = recipe(&s, radius);
    assert!(d.b.distance(vectorcraft_geom::Point::new(172.0, 244.0)) < 1e-6, "turned toward the point: {:?}", d.b);
    assert!((d.a.distance(d.b) - 72.0).abs() < 1e-6, "the radius stays");
    let before = recipe(&s, linear);
    for bad in [json!([1]), json!("x"), json!([0, 1e12])] {
        assert!(s.execute("dimension.update", &json!({"ids":[linear],"at":bad})).is_err(), "{bad}");
        assert_eq!(recipe(&s, linear), before);
    }
}

/// A dimension dragged with the Dimension tool follows the part grabbed: a diameter grabbed on its
/// far half turns that half toward the pointer, and an angular arc grabbed away from its middle
/// grows by as much as the pointer moved away from the vertex.
#[test]
fn a_dragged_dimension_follows_the_part_grabbed() {
    let plain = ViewInfo::default();
    let mut s = session();
    let circle = run(&mut s, "shape.ellipse", json!({"x":100,"y":100,"width":144,"height":144}))["id"].as_u64().unwrap();
    let diameter = run(&mut s, "dimension.create", json!({"mode":"diameter","source":{"kind":"circle","node":circle}}))["id"].as_u64().unwrap();
    run(&mut s, "dimension.update", json!({"ids":[diameter],"at":[244,172]}));
    let angular =
        run(&mut s, "dimension.create", json!({"mode":"angular","a":[500,400],"vertex":[400,400],"b":[400,500],"offset":60}))["id"].as_u64().unwrap();
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", plain).unwrap();
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Move, from), (PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), plain).unwrap();
        }
    };
    // Grabbed left of the center and dragged up: the left half goes up, the right half down.
    drag(&mut s, (120.0, 172.0), (120.0, 152.0));
    let d = recipe(&s, diameter);
    assert!(d.b.y > 172.0 + 1.0, "the half not grabbed turns away from the pointer: {:?}", d.b);
    assert!((d.a.distance(d.b) - 72.0).abs() < 1e-6, "the radius stays");
    // Grabbed at the center: it turns toward the pointer.
    drag(&mut s, (172.0, 172.0), (172.0, 100.0));
    assert!(recipe(&s, diameter).b.distance(vectorcraft_geom::Point::new(172.0, 100.0)) < 1e-6, "{:?}", recipe(&s, diameter).b);
    // Grabbed 10° from the arc's start and dragged 20 points straight away from the vertex.
    let (sin, cos) = 10f64.to_radians().sin_cos();
    drag(&mut s, (400.0 + 60.0 * cos, 400.0 + 60.0 * sin), (400.0 + 80.0 * cos, 400.0 + 80.0 * sin));
    assert!((recipe(&s, angular).offset - 80.0).abs() < 1e-6, "the arc's radius: {}", recipe(&s, angular).offset);
}

/// The Dimension tool takes a drag only from a dimension the canvas hits: one painted wider than
/// the reach is taken by its stroke, while one outside isolation mode, around an isolated part of
/// it, clipped away where the press is or drawn elsewhere by a geometry effect stays put. With
/// Real-time Drawing off the dimension being dragged is outlined.
#[test]
fn a_dimension_is_dragged_only_where_the_canvas_hits_it() {
    use vectorcraft_engine::tools::Overlay;
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let at_170 = (pt(100.0, 170.0), pt(200.0, 170.0));
    let drag = |s: &mut Session, from: (f64, f64), to: (f64, f64)| {
        for (kind, (x, y)) in [(PointerKind::Move, from), (PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), plain).unwrap();
        }
    };
    // A 30-point line: a press 13 points from its middle is on its stroke.
    let (mut s, first) = level_start(plain);
    run(&mut s, "dimension.update", json!({"ids":[first],"options":{"lineWeight":30}}));
    drag(&mut s, (115.0, 183.0), (115.0, 143.0));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
    // Outside the isolated group: not moved.
    let (mut s, first) = level_start(plain);
    let rect = run(&mut s, "shape.rectangle", json!({"x":400,"y":50,"width":50,"height":50}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[rect]}));
    let group = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "object.isolate", json!({"id": group}));
    run(&mut s, "select.none", json!({}));
    drag(&mut s, (150.0, 170.0), (150.0, 130.0));
    assert_eq!(recipe(&s, first).line_ends(), at_170);
    // In a clipping mask that leaves its right half out: not taken there, taken on its left half.
    let (mut s, first) = level_start(plain);
    let mask = run(&mut s, "shape.rectangle", json!({"x":90,"y":140,"width":50,"height":90}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[first, mask]}));
    run(&mut s, "object.clippingMask.make", json!({}));
    run(&mut s, "select.none", json!({}));
    let count = s.doc().unwrap().doc.node_count();
    drag(&mut s, (180.0, 170.0), (180.0, 130.0));
    assert_eq!((recipe(&s, first).line_ends(), s.doc().unwrap().doc.node_count()), (at_170, count));
    // That press picked a point, as a click there does: Escape drops it.
    s.tool_key(ToolKey::Escape, Mods::default(), plain).unwrap();
    drag(&mut s, (120.0, 170.0), (120.0, 130.0));
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
    // In a group whose Transform effect draws it 50 points lower: not taken where it is drawn.
    let (mut s, first) = level_start(plain);
    run(&mut s, "select.set", json!({"ids":[first]}));
    let group = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "effect.apply", json!({"effect":"distort.transform","ids":[group],"params":{"moveV":50}}));
    run(&mut s, "select.none", json!({}));
    drag(&mut s, (150.0, 220.0), (150.0, 260.0));
    assert_eq!(recipe(&s, first).line_ends(), at_170);
    // In Outline view the canvas draws no effects: taken where its recipe puts it.
    s.tool_key(ToolKey::Escape, Mods::default(), plain).unwrap();
    let outline = ViewInfo { outline: true, ..plain };
    for (kind, y) in [(PointerKind::Move, 170.0), (PointerKind::Down, 170.0), (PointerKind::Drag, 130.0), (PointerKind::Up, 130.0)] {
        s.pointer(&PointerEvent::new(kind, 150.0, y), outline).unwrap();
    }
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
    // A Transform effect on its parts draws them 50 points lower: not taken where they are drawn.
    let (mut s, first) = level_start(plain);
    let parts: Vec<u64> = s.doc().unwrap().doc.node(NodeId(first)).unwrap().children().unwrap().iter().map(|c| c.id.0).collect();
    run(&mut s, "effect.apply", json!({"effect":"distort.transform","ids":parts,"params":{"moveV":50}}));
    run(&mut s, "select.none", json!({}));
    drag(&mut s, (150.0, 220.0), (150.0, 260.0));
    assert_eq!(recipe(&s, first).line_ends(), at_170);
    // With one of its parts isolated, the dimension around it is outside the editing scope.
    let (mut s, first) = level_start(plain);
    let part = {
        let doc = &s.doc().unwrap().doc;
        let parts = doc.node(NodeId(first)).unwrap().children().unwrap();
        // The dimension line, which the press below lands on.
        let line = parts.iter().find(|c| c.geometric_bounds().is_some_and(|r| r.x0 < 150.0 && r.x1 > 150.0 && r.y0 < 171.0 && r.y1 > 169.0));
        line.unwrap().id.0
    };
    run(&mut s, "object.isolate", json!({"id": part}));
    run(&mut s, "select.none", json!({}));
    drag(&mut s, (150.0, 170.0), (150.0, 130.0));
    assert_eq!(recipe(&s, first).line_ends(), at_170);
    // Real-time Drawing off: the dimension is outlined where the drag puts it, and other selected
    // art, which the drag does not move, is not.
    let (mut s, first) = level_start(plain);
    let rect = run(&mut s, "shape.rectangle", json!({"x":400,"y":400,"width":50,"height":50}))["id"].as_u64().unwrap();
    run(&mut s, "select.set", json!({"ids":[first, rect]}));
    run(&mut s, "prefs.set", json!({"key": "realTimeDrawing", "value": false}));
    for (kind, y) in [(PointerKind::Move, 170.0), (PointerKind::Down, 170.0), (PointerKind::Drag, 110.0)] {
        s.pointer(&PointerEvent::new(kind, 150.0, y), plain).unwrap();
    }
    let boxes: Vec<vectorcraft_geom::Rect> = s
        .overlays(plain)
        .into_iter()
        .filter_map(|o| if let Overlay::Path { path, .. } = o { Some(vectorcraft_geom::kurbo::Shape::bounding_box(&path)) } else { None })
        .collect();
    assert!(boxes.iter().any(|r| r.y0 < 111.0 && r.y1 > 109.0 && r.width() >= 99.0), "the dragged dimension is outlined at its new place");
    assert!(!boxes.iter().any(|r| r.x1 > 399.0), "the rectangle is not outlined: {boxes:?}");
}

/// A geometry effect on a part of a dimension other than the one pressed keeps the Dimension tool
/// from dragging it, and the effect stays on that part.
#[test]
fn a_geometry_effect_on_another_part_keeps_the_dimension_in_place() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (mut s, first) = level_start(plain);
    let extension = {
        let doc = &s.doc().unwrap().doc;
        let parts = doc.node(NodeId(first)).unwrap().children().unwrap();
        // The extension line at x = 100, which the press below on the dimension line misses.
        let line = parts.iter().find(|c| c.geometric_bounds().is_some_and(|r| r.x1 < 101.0 && r.height() > 10.0));
        line.unwrap().id.0
    };
    run(&mut s, "effect.apply", json!({"effect":"distort.transform","ids":[extension],"params":{"moveH":-60}}));
    run(&mut s, "select.none", json!({}));
    for (kind, y) in [(PointerKind::Move, 170.0), (PointerKind::Down, 170.0), (PointerKind::Drag, 130.0), (PointerKind::Up, 130.0)] {
        s.pointer(&PointerEvent::new(kind, 150.0, y), plain).unwrap();
    }
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    let kept = s.doc().unwrap().doc.node(NodeId(extension)).is_some_and(|n| !n.appearance.effects.is_empty());
    assert!(kept, "the extension line keeps its effect");
}

/// In isolation mode, art outside the isolated group does not take a press on a dimension inside
/// it: the drag moves the dimension.
#[test]
fn art_outside_isolation_does_not_take_the_press_on_a_dimension() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (mut s, first) = level_start(plain);
    // A line across the dimension line, outside the group isolated below. The press, a point below
    // the dimension line, is nearer to it than to the dimension line.
    run(&mut s, "path.create", json!({"d": "M150 100 L150 250"}));
    run(&mut s, "select.set", json!({"ids":[first]}));
    let group = run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap();
    run(&mut s, "object.isolate", json!({"id": group}));
    run(&mut s, "select.none", json!({}));
    for (kind, y) in [(PointerKind::Move, 171.0), (PointerKind::Down, 171.0), (PointerKind::Drag, 131.0), (PointerKind::Up, 131.0)] {
        s.pointer(&PointerEvent::new(kind, 150.0, y), plain).unwrap();
    }
    assert_eq!(recipe(&s, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
}

/// A dimension pasted into a document that had none is dragged with the Dimension tool like one
/// drawn there.
#[test]
fn a_dimension_pasted_into_another_document_is_dragged() {
    let plain = ViewInfo::default();
    let (mut s, first) = level_start(plain);
    run(&mut s, "select.set", json!({"ids":[first]}));
    run(&mut s, "edit.copy", json!({}));
    run(&mut s, "file.new", json!({"width": 612, "height": 792}));
    run(&mut s, "edit.paste", json!({}));
    let pasted = s.doc().unwrap().selection.objects[0].0;
    run(&mut s, "select.none", json!({}));
    s.select_tool("dimension", plain).unwrap();
    let (a, b) = recipe(&s, pasted).line_ends();
    let mid = a.midpoint(b);
    for (kind, y) in [(PointerKind::Move, mid.y), (PointerKind::Down, mid.y), (PointerKind::Drag, mid.y - 40.0), (PointerKind::Up, mid.y - 40.0)] {
        s.pointer(&PointerEvent::new(kind, mid.x, y), plain).unwrap();
    }
    let (a2, b2) = recipe(&s, pasted).line_ends();
    assert!((a2.y - (a.y - 40.0)).abs() < 1e-6 && (b2.y - (b.y - 40.0)).abs() < 1e-6, "{:?} → {:?}", (a, b), (a2, b2));
}

/// A file that holds a dimension but was saved without the document's dimension flag (as one saved
/// after a dimension was pasted into a document that had none) opens with its dimension dragged
/// by the Dimension tool.
#[test]
fn a_dimension_in_a_file_saved_without_the_flag_is_dragged() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (s, first) = level_start(plain);
    let mut doc = s.doc().unwrap().doc.as_ref().clone();
    doc.dimensions = false;
    let mut reopened = Session::new();
    reopened.add_document(vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap(), None);
    reopened.select_tool("dimension", plain).unwrap();
    for (kind, y) in [(PointerKind::Move, 170.0), (PointerKind::Down, 170.0), (PointerKind::Drag, 130.0), (PointerKind::Up, 130.0)] {
        reopened.pointer(&PointerEvent::new(kind, 150.0, y), plain).unwrap();
    }
    assert_eq!(recipe(&reopened, first).line_ends(), (pt(100.0, 130.0), pt(200.0, 130.0)));
}

/// A second dimension of the same two points as a dimension already drawn lands on its level too.
/// Off every level, a pointer moved in small steps while it is placed puts the line at each step,
/// with no pull from the preview of the dimension itself.
#[test]
fn a_dimension_of_the_same_points_lines_up_and_the_line_follows_the_pointer() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    let (mut s, first) = level_start(plain);
    linear_click(&mut s, plain, 100.0, 200.0);
    linear_click(&mut s, plain, 200.0, 200.0);
    linear_click(&mut s, plain, 150.0, 172.5);
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_ne!(id, first);
    assert_eq!(recipe(&s, id).line_ends(), (pt(100.0, 170.0), pt(200.0, 170.0)));
    let (mut s, _) = level_start(plain);
    linear_click(&mut s, plain, 200.0, 200.0);
    linear_click(&mut s, plain, 300.0, 200.0);
    for y in [190.0, 189.0, 188.0, 187.0, 186.0] {
        s.pointer(&PointerEvent::new(PointerKind::Move, 250.0, y), plain).unwrap();
    }
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 250.0, 186.0), plain).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0].0;
    assert_eq!(recipe(&s, id).line_ends(), (pt(200.0, 186.0), pt(300.0, 186.0)));
}

/// A command that moves or hides a dimension while another is placed (Real-time Drawing on) does
/// not leave the placed dimension on a level the committed document does not have: committed with
/// its line near the other dimension's line as the document has it then, it is on that level, and
/// otherwise at the pointer.
#[test]
fn a_dimension_placed_after_a_command_lines_up_with_the_committed_document() {
    let plain = ViewInfo::default();
    for (cmd, params, y) in [("dimension.update", json!({"offset": -10.0}), 192.0), ("object.hide", json!({}), 172.0)] {
        let (mut s, first) = level_start(plain);
        run(&mut s, "prefs.set", json!({"key": "realTimeDrawing", "value": true}));
        // The bottom edge dragged out onto the level of the first line.
        for (kind, y) in [(PointerKind::Move, 300.0), (PointerKind::Down, 300.0), (PointerKind::Drag, 240.0), (PointerKind::Drag, 172.0)] {
            s.pointer(&PointerEvent::new(kind, 160.0, y), plain).unwrap();
        }
        let mut params = params;
        params["ids"] = json!([first]);
        run(&mut s, cmd, params);
        if cmd == "dimension.update" {
            assert_eq!(recipe(&s, first).line_ends().0.y, 190.0, "moved before the release");
        }
        // Released without a drag in between: the release places the dimension.
        s.pointer(&PointerEvent::new(PointerKind::Up, 160.0, y), plain).unwrap();
        let placed = s.doc().unwrap().selection.objects[0].0;
        assert_ne!(placed, first, "{cmd}: a new dimension");
        let reference = s.doc().unwrap().doc.node(NodeId(first)).map(|n| (n.visible, recipe(&s, first).line_ends().0.y));
        let expected = match reference {
            Some((true, level)) if (level - y).abs() <= 4.0 => level,
            _ => y,
        };
        assert_eq!(recipe(&s, placed).line_ends().0.y, expected, "{cmd}: the other dimension is {reference:?}");
    }
}

/// A dimension dragged out from a segment, with its line within the Smart Guides snapping
/// tolerance of the level of another dimension's parallel line, lands on that level.
#[test]
fn a_dimension_dragged_out_beside_another_lands_on_its_level() {
    let plain = ViewInfo::default();
    let pt = vectorcraft_geom::Point::new;
    // The bottom edge dragged out to 2 points below the level of the first line, 130 points
    // above the edge: on the level.
    let (mut s, _) = level_start(plain);
    for (kind, y) in
        [(PointerKind::Move, 300.0), (PointerKind::Down, 300.0), (PointerKind::Drag, 240.0), (PointerKind::Drag, 172.0), (PointerKind::Up, 172.0)]
    {
        s.pointer(&PointerEvent::new(kind, 160.0, y), plain).unwrap();
    }
    let id = s.doc().unwrap().selection.objects[0].0;
    let (a, b) = recipe(&s, id).line_ends();
    assert!(near(a, pt(300.0, 170.0)) && near(b, pt(100.0, 170.0)) || near(a, pt(100.0, 170.0)) && near(b, pt(300.0, 170.0)), "{a:?} {b:?}");
}

/// The page content streams of exported PDF `v`, inflated.
fn pdf_content(v: &Value) -> String {
    let bytes = vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap();
    let mut out = String::new();
    let mut rest = &bytes[..];
    while let Some(at) = rest.windows(7).position(|w| w == b"stream\n" || w == b"stream\r") {
        let body = &rest[at + 7..];
        let body = body.strip_prefix(b"\n").unwrap_or(body);
        let end = body.windows(9).position(|w| w == b"endstream").unwrap_or(body.len());
        let mut raw = Vec::new();
        if std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&body[..end]), &mut raw).is_ok() {
            out.push_str(&String::from_utf8_lossy(&raw));
        }
        rest = &body[(end + 9).min(body.len())..];
    }
    out
}

/// #3233: in a CMYK document a dimension's black lines and label print on the black plate only
/// (`0 0 0 1 K`), not as a four-ink rich black; other colours convert to CMYK; RGB documents keep
/// RGB.
#[test]
fn dimension_colours_follow_the_document_colour_model() {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"width":600,"height":500,"colorMode":"cmyk"}));
    let line = edge(&mut s);
    run(&mut s, "object.hide", json!({"ids":[line]}));
    let id = dim(&mut s, line);
    let colours = |s: &Session, id: u64| {
        let mut v = Vec::new();
        s.doc().unwrap().doc.node(NodeId(id)).unwrap().walk(&mut |n| {
            v.extend([n.appearance.fill_paint(), n.appearance.stroke_paint()].iter().filter_map(|p| p.color()));
        });
        v
    };
    let black = colours(&s, id);
    assert!(!black.is_empty() && black.iter().all(|c| *c == vectorcraft_color::Color::cmyk(0.0, 0.0, 0.0, 1.0)), "{black:?}");
    let pdf = pdf_content(&run(&mut s, "document.exportPdf", json!({"preset":"PDF/X-4:2010"})));
    // PDF/X-4 writes CMYK in the output intent's profile as DeviceCMYK: `0 0 0 1 K` strokes, `k` fills.
    let words: Vec<&str> = pdf.split_whitespace().collect();
    let operands = |op: &str| {
        (4..words.len())
            .filter(|&i| words[i] == op && words[i - 4..i].iter().all(|w| w.parse::<f32>().is_ok()))
            .map(|i| words[i - 4..i].join(" "))
            .collect::<Vec<_>>()
    };
    let (strokes, fills) = (operands("K"), operands("k"));
    assert!(!strokes.is_empty() && !fills.is_empty(), "{pdf}");
    assert!(strokes.iter().chain(&fills).all(|v| v == "0 0 0 1"), "K-only colours: {strokes:?} {fills:?}");
    run(&mut s, "dimension.update", json!({"ids":[id],"options":{"lineColor":[255,0,0]}}));
    let red = colours(&s, id);
    assert!(red.iter().any(|c| matches!(c, vectorcraft_color::Color::Cmyk { m, y, .. } if *m > 0.5 && *y > 0.5)), "{red:?}");
    assert!(red.iter().all(|c| matches!(c, vectorcraft_color::Color::Cmyk { .. })), "{red:?}");
    // RGB documents are unchanged.
    let mut s = session();
    let line = edge(&mut s);
    let id = dim(&mut s, line);
    assert!(colours(&s, id).iter().all(|c| *c == vectorcraft_color::Color::rgb8(0, 0, 0)));
    // Document Color Mode redraws dimensions in the new mode: K-only black in CMYK, RGB black back
    // in RGB (not a profile conversion of the old art).
    let k = vectorcraft_color::Color::cmyk(0.0, 0.0, 0.0, 1.0);
    run(&mut s, "file.documentColorMode", json!({"mode":"cmyk"}));
    let after = colours(&s, id);
    assert!(!after.is_empty() && after.iter().all(|c| *c == k), "{after:?}");
    run(&mut s, "file.documentColorMode", json!({"mode":"rgb"}));
    assert!(colours(&s, id).iter().all(|c| *c == vectorcraft_color::Color::rgb8(0, 0, 0)), "{:?}", colours(&s, id));
    // A CMYK file saved with RGB dimension art (before dimensions followed the colour mode) opens
    // with its dimensions redrawn in CMYK.
    let mut old = (*s.doc().unwrap().doc).clone();
    old.color_mode = vectorcraft_doc::ColorMode::Cmyk;
    s.add_document(old, None);
    let opened = colours(&s, id);
    assert!(!opened.is_empty() && opened.iter().all(|c| *c == k), "{opened:?}");
    // Only the label's characters in RGB (their fill is saved apart from the node's): redrawn too.
    let mut old = (*s.doc().unwrap().doc).clone();
    let label = {
        let mut found = None;
        old.node(NodeId(id)).unwrap().walk(&mut |n| {
            if matches!(n.kind, vectorcraft_doc::NodeKind::Text(_)) {
                found = Some(n.id);
            }
        });
        found.expect("a dimension label")
    };
    if let Some(vectorcraft_doc::NodeKind::Text(t)) = old.node_mut(label).map(|n| &mut n.kind) {
        for r in &mut t.runs {
            r.style.fill = vectorcraft_color::Paint::solid(vectorcraft_color::Color::rgb8(0, 0, 0));
        }
    }
    s.add_document(old, None);
    let runs = |s: &Session| match &s.doc().unwrap().doc.node(label).unwrap().kind {
        vectorcraft_doc::NodeKind::Text(t) => t.runs.iter().filter_map(|r| r.style.fill.color()).collect::<Vec<_>>(),
        _ => Vec::new(),
    };
    assert!(!runs(&s).is_empty() && runs(&s).iter().all(|c| *c == k), "{:?}", runs(&s));
}
