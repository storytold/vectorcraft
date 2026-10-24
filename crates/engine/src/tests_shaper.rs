use super::*;
use serde_json::json;
use vectorcraft_doc::shaper::{FILL_PREFIX, SOURCES, STROKE_PREFIX};
use vectorcraft_doc::{LiveShape, Node};
use vectorcraft_geom::{Point, Shape as _};
use vectorcraft_tools::{Mods, PointerEvent, PointerKind};

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    s
}
fn rectangle(s: &mut Session, x: f64, fill: &str) -> NodeId {
    s.execute("paint.setFill", &json!({"color": fill, "ids": []})).unwrap();
    let id = s.execute("shape.rectangle", &json!({"x":x,"y":0,"width":100,"height":100})).unwrap()["id"].as_u64().unwrap();
    NodeId(id)
}
fn scribble(s: &mut Session, pts: &[[f64; 2]]) -> NodeId {
    NodeId(s.execute("shaper.scribble", &json!({"points":pts})).unwrap()["id"].as_u64().unwrap())
}
fn node(s: &Session, id: NodeId) -> &Node {
    s.doc().unwrap().doc.node(id).unwrap()
}
fn fill(s: &Session, id: NodeId, x: f64, y: f64) -> Option<String> {
    node(s, id)
        .children()
        .unwrap()
        .iter()
        .filter(|n| n.name.as_deref() != Some(SOURCES))
        .rev()
        .find(|n| {
            n.name.as_deref().is_some_and(|n| n.starts_with(FILL_PREFIX))
                && n.path_data().is_some_and(|p| p.to_bezpath().winding(Point::new(x, y)) != 0)
        })
        .map(|n| n.appearance.fill_paint().label())
}
fn stroke(s: &Session, id: NodeId, x: f64, y: f64) -> bool {
    node(s, id).children().unwrap().iter().filter(|n| n.name.as_deref() != Some(SOURCES)).any(|n| {
        n.name.as_deref().is_some_and(|n| n.starts_with(STROKE_PREFIX))
            && !n.appearance.stroke_paint().is_none()
            && n.path_data().and_then(|p| p.nearest(Point::new(x, y))).is_some_and(|(_, _, _, _, d)| d < 0.1)
    })
}

/// An invalid explicit Shaper target must never silently change which shapes get edited.
#[test]
fn shaper_scribble_rejects_malformed_explicit_targets() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    let untouched = s.doc().unwrap().doc.layers.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for ids in [json!([a.0, "bad"]), json!([a.0, 999999]), json!({"id": a.0}), json!("bad")] {
        let err = s.execute("shaper.scribble", &json!({"points": [[25, 20], [35, 30]], "ids": ids}));
        assert!(err.is_err(), "malformed targets were accepted");
        assert_eq!(s.doc().unwrap().doc.layers, untouched);
        assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    }
    s.execute("shaper.scribble", &json!({"points": [[25, 20], [35, 30]], "ids": [a.0]})).unwrap();
}

#[test]
fn shaper_merges_with_origin_color_keeps_live_originals_and_undoes_once() {
    for reverse in [false, true] {
        let mut s = session();
        let a = rectangle(&mut s, 0.0, "#ff0000");
        let b = rectangle(&mut s, 50.0, "#0000ff");
        let originals = [node(&s, a).clone(), node(&s, b).clone()];
        let undo = s.doc().unwrap().history.undo.len();
        let mut pts = vec![[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0], [75.0, 80.0], [125.0, 85.0]];
        if reverse {
            pts.reverse();
        }
        let g = scribble(&mut s, &pts);
        let color = if reverse { "#0000ff" } else { "#ff0000" };
        for x in [25.0, 75.0, 125.0] {
            assert_eq!(fill(&s, g, x, 50.0).as_deref(), Some(color));
        }
        for n in &originals {
            assert_eq!(node(&s, n.id), n);
        }
        assert_eq!(node(&s, g).children().unwrap()[0].name.as_deref(), Some(SOURCES));
        assert!(!stroke(&s, g, 50.0, 50.0));
        assert!(stroke(&s, g, 0.0, 50.0));
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
        s.execute("edit.undo", &json!({})).unwrap();
        assert!(s.doc().unwrap().doc.node(g).is_none());
        for n in &originals {
            assert_eq!(node(&s, n.id), n);
        }
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(fill(&s, g, 75.0, 50.0).as_deref(), Some(color));
    }
}

#[test]
fn shaper_interior_punch_removes_the_whole_visible_front_face() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    let b = rectangle(&mut s, 50.0, "#0000ff");
    let originals = [node(&s, a).clone(), node(&s, b).clone()];
    let g = scribble(&mut s, &[[65.0, 20.0], [85.0, 40.0], [65.0, 60.0], [85.0, 80.0]]);
    assert_eq!(fill(&s, g, 75.0, 50.0), None);
    assert_eq!(fill(&s, g, 25.0, 50.0).as_deref(), Some("#ff0000"));
    assert_eq!(fill(&s, g, 125.0, 50.0), None);
    assert!(stroke(&s, g, 50.0, 50.0) && stroke(&s, g, 150.0, 50.0));
    for n in &originals {
        assert_eq!(node(&s, n.id), n);
    }
}

#[test]
fn shaper_scribble_merges_a_face_narrower_than_its_sampling_step() {
    let mut s = session();
    rectangle(&mut s, 0.0, "#0000ff");
    s.execute("paint.setFill", &json!({"color": "#ff0000", "ids": []})).unwrap();
    s.execute("shape.rectangle", &json!({"x": 49.8, "y": 10, "width": 0.4, "height": 80})).unwrap();
    // Every crossing of the 0.4 pt strip falls between two of the 2 pt samples.
    let g = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [75.0, 65.0]]);
    assert_eq!(fill(&s, g, 50.0, 50.0).as_deref(), Some("#0000ff"), "the strip merges into the face around it");
    assert_eq!(fill(&s, g, 25.0, 50.0).as_deref(), Some("#0000ff"));
}

#[test]
fn shaper_exiting_scribble_erases_only_touched_fill_and_boundary_strokes() {
    let mut s = session();
    rectangle(&mut s, 0.0, "#ff0000");
    rectangle(&mut s, 50.0, "#0000ff");
    let g = scribble(&mut s, &[[110.0, 20.0], [140.0, 35.0], [110.0, 50.0], [140.0, 65.0], [200.0, 70.0]]);
    assert_eq!(fill(&s, g, 125.0, 50.0), None);
    assert!(!stroke(&s, g, 150.0, 50.0));
    assert_eq!(fill(&s, g, 25.0, 50.0).as_deref(), Some("#ff0000"));
    assert_eq!(fill(&s, g, 75.0, 50.0), None);
    assert!(stroke(&s, g, 0.0, 50.0));
}

#[test]
fn shaper_erases_only_protruding_stroke_pieces_keeps_fill_and_original_line() {
    let mut s = session();
    s.execute("paint.setFill", &json!({"color":"#ffff00"})).unwrap();
    let circle = NodeId(s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap()["id"].as_u64().unwrap());
    let line = NodeId(s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap()["id"].as_u64().unwrap());
    let original_line = node(&s, line).clone();
    let g = scribble(&mut s, &[[60.0, 140.0], [70.0, 160.0], [80.0, 140.0], [90.0, 160.0]]);
    assert!(!stroke(&s, g, 75.0, 150.0));
    assert!(stroke(&s, g, 150.0, 150.0));
    assert!(stroke(&s, g, 225.0, 150.0));
    assert_eq!(fill(&s, g, 150.0, 130.0).as_deref(), Some("#ffff00"));
    assert_eq!(node(&s, line), &original_line);
    assert!(matches!(node(&s, circle).live_shape(), Some(LiveShape::Ellipse { .. })));
    let g = scribble(&mut s, &[[210.0, 140.0], [220.0, 160.0], [230.0, 140.0], [240.0, 160.0]]);
    assert!(!stroke(&s, g, 225.0, 150.0));
    assert!(stroke(&s, g, 150.0, 150.0));
    assert_eq!(node(&s, line), &original_line);
}

#[test]
fn shaper_scribble_with_a_small_tolerance_erases_a_stroke_it_crosses_between_samples() {
    for (tolerance, from, to) in [(0.1, 140.25, 160.25), (0.01, 50.0123, 250.0123)] {
        let mut s = session();
        s.execute("paint.setFill", &json!({"color":"#ffff00"})).unwrap();
        s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
        s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap();
        // No sample along the stroke lands within `tolerance` of the line at y = 150.
        let v = s.execute("shaper.scribble", &json!({"points": [[60.0, from], [60.0, to]], "tolerance": tolerance})).unwrap();
        assert_eq!(v["strokes"], json!(1), "tolerance {tolerance}: {v}");
        let g = NodeId(v["id"].as_u64().unwrap());
        assert!(!stroke(&s, g, 75.0, 150.0));
        assert!(stroke(&s, g, 150.0, 150.0));
    }
}

/// Export Selection crops a Shaper group to the art it shows, not to its transparent source art.
#[test]
fn shaper_export_selection_crops_to_the_shown_art() {
    let mut s = session();
    s.execute("paint.setFill", &json!({"color":"#ffff00"})).unwrap();
    s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
    s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap();
    scribble(&mut s, &[[60.0, 140.0], [70.0, 160.0], [80.0, 140.0], [90.0, 160.0]]);
    let g = scribble(&mut s, &[[210.0, 140.0], [220.0, 160.0], [230.0, 140.0], [240.0, 160.0]]);
    s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
    let v = s.execute("document.exportSelection", &json!({"format": "png"})).unwrap();
    let b = node(&s, g).visual_bounds().unwrap();
    assert!(b.x0 > 90.0 && b.x1 < 210.0, "the trimmed line: {b:?}");
    assert_eq!(v["bounds"], json!([b.x0, b.y0, b.width(), b.height()]), "{v}");
}

/// Fit to Artwork Bounds fits the art a Shaper group shows, not its transparent source art.
#[test]
fn shaper_fit_to_artwork_bounds_fits_the_shown_art() {
    let mut s = session();
    s.execute("paint.setFill", &json!({"color":"#ffff00"})).unwrap();
    s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
    s.execute("shape.line", &json!({"x1":50,"y1":150,"x2":250,"y2":150})).unwrap();
    scribble(&mut s, &[[60.0, 140.0], [70.0, 160.0], [80.0, 140.0], [90.0, 160.0]]);
    let g = scribble(&mut s, &[[210.0, 140.0], [220.0, 160.0], [230.0, 140.0], [240.0, 160.0]]);
    s.execute("artboard.fitToArt", &json!({})).unwrap();
    let b = node(&s, g).visual_bounds().unwrap();
    assert_eq!(s.doc().unwrap().doc.artboards[0].rect, b, "the circle and the trimmed line");
}

#[test]
fn shaper_source_move_resize_native_roundtrip_and_release_preserve_live_art() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    let b = rectangle(&mut s, 50.0, "#0000ff");
    let g = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]]);
    s.execute("object.move", &json!({"dx":20,"dy":10})).unwrap();
    assert_eq!(fill(&s, g, 30.0, 15.0).as_deref(), Some("#ff0000"));
    assert_eq!(node(&s, a).geometric_bounds().unwrap().x0, 20.0);
    s.execute("select.set", &json!({"ids":[a.0]})).unwrap();
    s.execute("object.setBounds", &json!({"width":120.0,"height":110.0,"reference":0})).unwrap();
    assert!(matches!(node(&s, a).live_shape(), Some(LiveShape::Rectangle { .. })));
    let doc = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(doc, false)).unwrap();
    assert_eq!(back.layers, doc.layers);
    let originals = [node(&s, a).clone(), node(&s, b).clone()];
    s.execute("shaper.release", &json!({"id":g.0})).unwrap();
    assert!(s.doc().unwrap().doc.node(g).is_none());
    for n in &originals {
        assert_eq!(node(&s, n.id), n);
    }
}

#[test]
fn shaper_face_paint_survives_source_edits_and_expand_discards_only_sources() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    rectangle(&mut s, 50.0, "#0000ff");
    let g = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]]);
    s.execute("shaper.select", &json!({"point":[75,50]})).unwrap();
    s.execute("paint.setFill", &json!({"color":"#00ff00"})).unwrap();
    s.execute("object.move", &json!({"dx":5,"dy":0})).unwrap();
    assert_eq!(fill(&s, g, 75.0, 50.0).as_deref(), Some("#00ff00"));
    s.execute("shaper.expand", &json!({"id":g.0})).unwrap();
    assert!(node(&s, g).extras().shaper.is_none());
    assert!(s.doc().unwrap().doc.node(a).is_none());
    assert_eq!(fill(&s, g, 75.0, 50.0).as_deref(), Some("#00ff00"));
}

#[test]
fn shaper_double_click_selects_an_original_for_construction_editing() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    rectangle(&mut s, 50.0, "#0000ff");
    let g = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]]);
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("shaper", v).unwrap();
    let requests = s.pointer(&PointerEvent::new(PointerKind::DoubleClick, 25.0, 50.0), v).unwrap();
    assert!(requests.iter().any(|r| matches!(r, UiRequest::SwitchTool(tool) if tool == "selection")));
    s.select_tool("selection", v).unwrap();
    assert_eq!(s.tool_id(), "selection");
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().isolation, Some(node(&s, g).children().unwrap()[0].id));
    assert!(s.doc().unwrap().doc.is_editable(a));
    s.execute("object.move", &json!({"dx":10,"dy":0})).unwrap();
    assert_eq!(fill(&s, g, 15.0, 50.0).as_deref(), Some("#ff0000"));
    s.execute("object.exitIsolation", &json!({})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
}

/// The arrow widget beside a selected Shaper Group switches it to Construction mode and back, with
/// the Shaper tool and with the Selection tool. In Construction mode a Shaper tap selects a source
/// shape, and the mode lasts through undo and redo of an edit made in it.
#[test]
fn shaper_widget_switches_between_face_selection_and_construction_mode() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    rectangle(&mut s, 50.0, "#0000ff");
    let g = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]]);
    let sources = node(&s, g).children().unwrap()[0].id;
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    let widget = |s: &Session| {
        let st = s.doc().unwrap();
        vectorcraft_tools::shaperwidget::ShaperWidget::of(&st.doc, &st.selection, st.isolation, v.zoom).unwrap()
    };
    let click = |s: &mut Session, p: Point| {
        for kind in [PointerKind::Down, PointerKind::Up] {
            s.pointer(&PointerEvent::new(kind, p.x, p.y), v).unwrap();
        }
    };
    for tool in ["shaper", "selection"] {
        s.select_tool(tool, v).unwrap();
        s.execute("select.set", &json!({"ids": [g.0]})).unwrap();
        let w = widget(&s);
        assert!(!w.construction);
        click(&mut s, w.at);
        assert_eq!(s.doc().unwrap().isolation, Some(sources), "{tool}");
        assert!(widget(&s).construction);
        if tool == "shaper" {
            click(&mut s, Point::new(25.0, 50.0));
            assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
        } else {
            s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        }
        s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().isolation, Some(sources), "{tool}: undo");
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(s.doc().unwrap().isolation, Some(sources), "{tool}: redo");
        assert_eq!(fill(&s, g, 15.0, 50.0).as_deref(), Some("#ff0000"));
        let w = widget(&s);
        assert!(w.construction);
        click(&mut s, w.at);
        assert_eq!(s.doc().unwrap().isolation, None, "{tool}");
        assert_eq!(s.doc().unwrap().selection.objects, vec![g]);
        s.execute("object.move", &json!({"dx": -10, "dy": 0})).unwrap();
    }
}

/// In Construction mode a Shaper tap picks a source shape of the isolated group only, even where
/// another Shaper Group's source shapes lie above it.
#[test]
fn shaper_construction_taps_stay_in_the_isolated_group() {
    let mut s = session();
    let a = rectangle(&mut s, 0.0, "#ff0000");
    rectangle(&mut s, 50.0, "#0000ff");
    let lower = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]]);
    let c = rectangle(&mut s, 20.0, "#00ff00");
    let d = rectangle(&mut s, 70.0, "#ffff00");
    let upper = NodeId(
        s.execute("shaper.scribble", &json!({"points": [[45, 20], [95, 35], [50, 50], [145, 65]], "ids": [c.0, d.0]})).unwrap()["id"]
            .as_u64()
            .unwrap(),
    );
    assert_ne!(lower, upper);
    let sources = node(&s, lower).children().unwrap()[0].id;
    s.execute("object.isolate", &json!({"id": lower.0})).unwrap();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("shaper", v).unwrap();
    for kind in [PointerKind::Down, PointerKind::Up] {
        s.pointer(&PointerEvent::new(kind, 25.0, 50.0), v).unwrap();
    }
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().isolation, Some(sources));
    // Outside the isolated group's source shapes, a tap changes nothing.
    s.execute("shaper.select", &json!({"point": [160, 50], "source": true})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![a]);
    assert_eq!(s.doc().unwrap().isolation, Some(sources));
}

/// `object.isolate` on a Shaper Group whose source group is locked or hidden is refused, and
/// isolation mode stays off.
#[test]
fn shaper_isolate_refuses_a_locked_or_hidden_source_group() {
    for (key, on) in [("locked", true), ("visible", false)] {
        let mut s = session();
        rectangle(&mut s, 0.0, "#ff0000");
        rectangle(&mut s, 50.0, "#0000ff");
        let g = scribble(&mut s, &[[25.0, 20.0], [75.0, 35.0], [30.0, 50.0], [125.0, 65.0]]);
        let sources = node(&s, g).children().unwrap()[0].id;
        s.execute("object.setProps", &json!({"ids": [sources.0], key: on})).unwrap();
        let r = s.execute("object.isolate", &json!({"id": g.0})).map_err(|e| e.to_string());
        assert_eq!(r, Err("invalid parameters for `object.isolate`: the object is hidden or locked".into()), "{key}");
        assert_eq!(s.doc().unwrap().isolation, None, "{key}");
    }
}

#[test]
fn shaper_lines_keep_free_angles_with_or_without_shift() {
    for shift in [false, true] {
        let mut s = session();
        let v = ViewInfo { smart_guides: false, ..Default::default() };
        s.select_tool("shaper", v).unwrap();
        for (kind, x, y) in [(PointerKind::Down, 30.0, 40.0), (PointerKind::Drag, 130.0, 70.0), (PointerKind::Up, 230.0, 100.0)] {
            s.pointer(&PointerEvent::new(kind, x, y).with_mods(Mods { shift, ..Default::default() }), v).unwrap();
        }
        let n = node(&s, s.doc().unwrap().selection.objects[0]);
        assert!(matches!(n.live_shape(), Some(LiveShape::Line { a, b }) if *a == Point::new(30.0, 40.0) && *b == Point::new(230.0, 100.0)));
    }
}

#[test]
fn shaper_rejects_oversized_or_invalid_gestures_without_changing_art() {
    let mut s = session();
    rectangle(&mut s, 0.0, "#ff0000");
    let before = s.doc().unwrap().doc.clone();
    let history = s.doc().unwrap().history.undo.len();
    for params in [json!({"points":vec![[1.0,1.0];4097]}), json!({"points":[[1,1],[2,2]],"tolerance":0}), json!({"points":[[1e100,0]]})] {
        assert!(s.execute("shaper.scribble", &params).is_err());
        assert_eq!(s.doc().unwrap().doc.layers, before.layers);
        assert_eq!(s.doc().unwrap().history.undo.len(), history);
    }
}

#[test]
fn shaper_tool_trims_both_diagonal_ends_and_only_construction_can_pick_them() {
    use vectorcraft_doc::hit::{HitOptions, hit_test};
    let mut s = session();
    s.execute("paint.setFill", &json!({"color":"#ffff00"})).unwrap();
    s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
    let rotate = |x: f64, y: f64| {
        let (x, y) = (x - 150.0, y - 150.0);
        Point::new(150.0 + (x + y) / 2.0f64.sqrt(), 150.0 + (y - x) / 2.0f64.sqrt())
    };
    let (a, b) = (rotate(50.0, 150.0), rotate(250.0, 150.0));
    let line = NodeId(s.execute("shape.line", &json!({"x1":a.x,"y1":a.y,"x2":b.x,"y2":b.y})).unwrap()["id"].as_u64().unwrap());
    let original = node(&s, line).clone();
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    s.select_tool("shaper", v).unwrap();
    for offset in [0.0, 150.0] {
        let points = [(60.0, 140.0), (70.0, 160.0), (80.0, 140.0), (90.0, 160.0)];
        for (i, (x, y)) in points.into_iter().enumerate() {
            let p = rotate(x + offset, y);
            let kind = if i == 0 {
                PointerKind::Down
            } else if i == 3 {
                PointerKind::Up
            } else {
                PointerKind::Drag
            };
            s.pointer(&PointerEvent::new(kind, p.x, p.y), v).unwrap();
        }
    }
    let g = s.doc().unwrap().selection.objects[0];
    assert!(node(&s, g).extras().shaper.is_some());
    assert_eq!(node(&s, line), &original);
    assert!(stroke(&s, g, 150.0, 150.0));
    assert_eq!(fill(&s, g, 150.0, 130.0).as_deref(), Some("#ffff00"));
    let sources = node(&s, g).children().unwrap()[0].id;
    let doc = &s.doc().unwrap().doc;
    for x in [75.0, 225.0] {
        let p = rotate(x, 150.0);
        assert!(!stroke(&s, g, p.x, p.y));
        assert!(!node(&s, g).geometric_bounds().unwrap().contains(p));
        assert!(!node(&s, g).visual_bounds().unwrap().contains(p));
        assert!(hit_test(doc, p, HitOptions { tol: 0.5, ..Default::default() }).is_none());
        assert_eq!(hit_test(doc, p, HitOptions { tol: 0.5, scope: Some(sources), ..Default::default() }).unwrap().leaf, line);
    }
    let p = rotate(75.0, 150.0);
    s.execute("shaper.select", &json!({"point":[p.x,p.y],"source":true})).unwrap();
    assert_eq!(s.doc().unwrap().selection.objects, vec![line]);
}

/// The Selection tool's marquee, Group Selection's E marquee and Direct Selection's marquee and
/// clicks reach a Shaper Group only by the art it shows: none of them selects the original shape of a
/// face the group erased, or its anchor points. In Construction mode the Selection marquee selects
/// them, and Direct Selection's marquee and clicks select their anchor points.
#[test]
fn shaper_marquees_leave_the_original_shape_of_an_erased_face() {
    let mut s = session();
    rectangle(&mut s, 0.0, "#ff0000");
    let b = rectangle(&mut s, 50.0, "#0000ff");
    let g = scribble(&mut s, &[[110.0, 20.0], [140.0, 35.0], [110.0, 50.0], [140.0, 65.0], [200.0, 70.0]]);
    assert_eq!(fill(&s, g, 125.0, 50.0), None, "the face is erased");
    let v = ViewInfo { smart_guides: false, ..Default::default() };
    let drag = |s: &mut Session, tool: &str, from: [f64; 2], to: [f64; 2]| {
        s.select_tool(tool, v).unwrap();
        for (kind, [x, y]) in [(PointerKind::Down, from), (PointerKind::Drag, to), (PointerKind::Up, to)] {
            s.pointer(&PointerEvent::new(kind, x, y), v).unwrap();
        }
        s.doc().unwrap().selection.clone()
    };
    // Each marquee crosses the second rectangle's right side, where nothing is shown.
    assert!(drag(&mut s, "selection", [140.0, 40.0], [160.0, 60.0]).objects.is_empty());
    let sel = drag(&mut s, "directSelection", [140.0, -10.0], [160.0, 10.0]);
    assert!(sel.objects.is_empty() && sel.anchors.is_empty(), "{sel:?}");
    // Group Selection's E marquee around the whole of that rectangle leaves it out.
    s.select_tool("groupSelection", v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 155.0, -10.0), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Drag, 45.0, 110.0), v).unwrap();
    s.tool_key(vectorcraft_tools::ToolKey::Letter('e'), Mods::default(), v).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Up, 45.0, 110.0), v).unwrap();
    let sel = s.doc().unwrap().selection.clone();
    assert!(!sel.objects.contains(&b) && !sel.anchors.contains_key(&b), "{sel:?}");
    // A Direct Selection click on that rectangle's corner selects nothing either.
    s.execute("select.none", &json!({})).unwrap();
    let sel = drag(&mut s, "directSelection", [150.0, 0.0], [150.0, 0.0]);
    assert!(sel.objects.is_empty() && sel.anchors.is_empty(), "{sel:?}");
    // Over the shown face's left side the marquees take the group and the face's anchors.
    assert_eq!(drag(&mut s, "selection", [-10.0, 40.0], [10.0, 60.0]).objects, vec![g]);
    let sources = node(&s, g).children().unwrap()[0].id;
    let sel = drag(&mut s, "directSelection", [-10.0, -10.0], [10.0, 10.0]);
    assert!(!sel.anchors.is_empty());
    assert!(sel.anchors.keys().all(|id| s.doc().unwrap().doc.parent_of(*id) == Some(g)), "{sel:?}");
    // Construction mode: the original shapes are what the marquees select. These marquees start
    // outside them and cross the second rectangle's top right corner.
    s.execute("object.isolate", &json!({"id": g.0})).unwrap();
    assert_eq!(s.doc().unwrap().isolation, Some(sources));
    assert_eq!(drag(&mut s, "selection", [160.0, -10.0], [140.0, 10.0]).objects, vec![b]);
    assert!(drag(&mut s, "directSelection", [160.0, -10.0], [140.0, 10.0]).anchors.contains_key(&b));
    s.execute("select.none", &json!({})).unwrap();
    assert!(drag(&mut s, "directSelection", [150.0, 0.0], [150.0, 0.0]).anchors.contains_key(&b));
}

#[test]
fn shaper_three_circles_erase_a_visible_fill_and_keep_the_complete_shared_curve() {
    for exits in [false, true] {
        let mut s = session();
        let mut originals = vec![];
        for (x, y, fill) in [(100.0, 20.0, "#ff0000"), (20.0, 130.0, "#ffffff"), (170.0, 130.0, "#ffff00")] {
            s.execute("paint.setFill", &json!({"color":fill,"ids":[]})).unwrap();
            let id = NodeId(s.execute("shape.ellipse", &json!({"x":x,"y":y,"width":200,"height":200})).unwrap()["id"].as_u64().unwrap());
            originals.push(node(&s, id).clone());
        }
        let mut points = vec![[290.0, 210.0], [330.0, 225.0], [290.0, 240.0], [330.0, 255.0]];
        if exits {
            points.push([410.0, 260.0]);
        }
        let g = scribble(&mut s, &points);
        for (x, y) in [(330.0, 230.0), (190.0, 230.0), (250.0, 180.0)] {
            assert_eq!(fill(&s, g, x, y), None, "all of the visible yellow face is erased, including covered intersections");
        }
        assert_eq!(stroke(&s, g, 370.0, 230.0), !exits);
        for angle in [145.0_f64, 160.0, 180.0, 200.0, 220.0, 240.0, 260.0, 280.0] {
            let (sin, cos) = angle.to_radians().sin_cos();
            assert!(stroke(&s, g, 270.0 + 100.0 * cos, 230.0 + 100.0 * sin), "shared curve remains complete at {angle}°, exiting {exits}");
        }
        if exits {
            // The two new corners must be actual path joins, rather than meeting butt caps.
            for corner in [Point::new(298.975, 134.288), Point::new(195.0, 296.144)] {
                assert!(
                    node(&s, g)
                        .children()
                        .unwrap()
                        .iter()
                        .filter(|n| n.name.as_deref().is_some_and(|s| s.starts_with(STROKE_PREFIX)))
                        .flat_map(|n| n.path_data().unwrap().subpaths.iter())
                        .flat_map(|sp| &sp.anchors)
                        .any(|a| a.p.distance(corner) < 0.1 && a.has_in() && a.has_out()),
                    "joined corner at {corner:?}"
                );
            }
        }
        assert_eq!(fill(&s, g, 110.0, 230.0).as_deref(), Some("#ffffff"));
        assert_eq!(fill(&s, g, 200.0, 80.0).as_deref(), Some("#ff0000"));
        for n in originals {
            assert_eq!(node(&s, n.id), &n);
        }
    }
}

#[test]
fn shaper_visible_dividing_stroke_keeps_fill_erasure_on_one_side() {
    let mut s = session();
    s.execute("paint.setFill", &json!({"color":"#ffff00","ids":[]})).unwrap();
    s.execute("shape.ellipse", &json!({"x":100,"y":100,"width":100,"height":100})).unwrap();
    s.execute("shape.line", &json!({"x1":100,"y1":150,"x2":200,"y2":150})).unwrap();
    let g = scribble(&mut s, &[[140.0, 125.0], [160.0, 135.0], [140.0, 140.0], [160.0, 145.0]]);
    assert_eq!(fill(&s, g, 150.0, 130.0), None);
    assert_eq!(fill(&s, g, 150.0, 170.0).as_deref(), Some("#ffff00"));
    assert!(stroke(&s, g, 150.0, 150.0));
}

#[test]
fn shaper_joined_outline_keeps_its_stroke_paint_after_source_edits() {
    let mut s = session();
    let mut ids = vec![];
    for (x, color) in [(0, "#ff0000"), (50, "#ffff00")] {
        s.execute("paint.setFill", &json!({"color":color,"ids":[]})).unwrap();
        ids.push(NodeId(s.execute("shape.ellipse", &json!({"x":x,"y":0,"width":100,"height":100})).unwrap()["id"].as_u64().unwrap()));
    }
    let g = scribble(&mut s, &[[20.0, 40.0], [70.0, 45.0], [30.0, 50.0], [125.0, 55.0], [75.0, 60.0], [125.0, 65.0]]);
    let outlines: Vec<_> =
        node(&s, g).children().unwrap().iter().filter(|n| n.name.as_deref().is_some_and(|s| s.starts_with(STROKE_PREFIX))).collect();
    assert_eq!(outlines.len(), 1);
    assert!(outlines[0].path_data().unwrap().subpaths[0].closed);
    let outline = outlines[0].id;
    s.execute("paint.setStroke", &json!({"color":"#00ff00","ids":[outline.0]})).unwrap();
    s.execute("object.move", &json!({"dx":10,"dy":0})).unwrap();
    let outlines: Vec<_> =
        node(&s, g).children().unwrap().iter().filter(|n| n.name.as_deref().is_some_and(|s| s.starts_with(STROKE_PREFIX))).collect();
    assert_eq!(outlines.len(), 1);
    assert_eq!(outlines[0].appearance.stroke_paint().label(), "#00ff00");
    assert!(outlines[0].path_data().unwrap().subpaths[0].closed);
    for id in ids {
        assert_ne!(node(&s, id).appearance.stroke_paint().label(), "#00ff00");
    }
}
