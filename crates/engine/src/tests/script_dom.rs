//! The script object model: `app`, its documents, their selection and page items.

use serde_json::{Value, json};

use crate::*;

fn run(s: &mut Session, source: &str) -> Result<Value> {
    s.execute("script.run", &json!({"source": source, "name": "t.js"}))
}

/// What the run logged.
fn log(s: &mut Session, source: &str) -> Vec<Value> {
    let r = run(s, source).unwrap_or_else(|e| panic!("{source}: {e}"));
    r["log"].as_array().cloned().unwrap_or_default()
}

/// The message of the run's failure.
fn failure(s: &mut Session, source: &str) -> String {
    run(s, source).map(|r| panic!("{source} ran: {r}")).unwrap_err().to_string()
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    NodeId(s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap()["id"].as_u64().unwrap())
}

/// A 400 × 300 pt document with two rectangles.
fn doc() -> (Session, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    let b = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    (s, a, b)
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

#[test]
fn app_documents_track_active_tab_and_new_document() {
    let mut s = Session::new();
    assert_eq!(log(&mut s, "vc.log(app.documents.length); vc.log(app.typename);"), vec![json!(0), json!("Application")]);
    assert!(failure(&mut s, "app.activeDocument;").contains("No active document"));
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    s.execute("file.new", &json!({"width": 200, "height": 100})).unwrap();
    let out = log(
        &mut s,
        "var d = app.documents; vc.log(d.length); vc.log(app.activeDocument.width);\
         app.activeDocument = d[0]; vc.log(app.activeDocument.width); vc.log(d[0].parent.typename);\
         vc.log(d[5] === undefined); vc.log(app.activeDocument instanceof Document);\
         var n = d.add(DocumentColorSpace.CMYK, 50, 60, 2);\
         vc.log(d.length); vc.log(app.activeDocument.name === n.name); vc.log(n.documentColorSpace); vc.log(n.height);\
         vc.log(d.getByName(n.name).width);",
    );
    let expected = [json!(2), json!(200), json!(400), json!("Application"), json!(true), json!(true), json!(3), json!(true)];
    assert_eq!(out[..8], expected);
    assert_eq!(out[8..], [json!("DocumentColorSpace.CMYK"), json!(60), json!(50)]);
    assert_eq!(s.documents().len(), 3);
    assert_eq!(s.active_index(), Some(2));
    assert_eq!(s.documents()[2].doc.artboards.len(), 2);
    // A bad argument changes nothing.
    assert!(failure(&mut s, "app.documents.add(DocumentColorSpace.RGB, -5);").contains("width"));
    assert!(failure(&mut s, "app.documents.add('RGB');").contains("colorSpace"));
    assert!(failure(&mut s, "app.documents.add(undefined, 10, 10, 101);").contains("numArtBoards"));
    // The spacing is refused at file.new's limit, with the script's own name for it.
    let spacing = "app.documents.add(undefined, 10, 10, 2, DocumentArtboardLayout.Row, 100001);";
    assert!(failure(&mut s, spacing).contains("artboardSpacing must be from 0 to 100000 points"));
    assert!(failure(&mut s, "app.documents.index('none');").contains("Not found"));
    assert_eq!(s.documents().len(), 3);
}

#[test]
fn page_items_are_live_and_typed_for_all_node_kinds() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    rect(&mut s, 200.0, 200.0, 10.0, 10.0);
    s.execute("guide.add", &json!({"vertical": true, "pos": 120})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 250, "text": "hi"})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, t = [];\
         for (var i = 0; i < p.length; i++) t.push(p[i].typename);\
         vc.log(t.join(','));\
         vc.log(p[3].parent.typename); vc.log(p[0].parent.typename); vc.log(p.parent.typename);\
         p[1].remove(); vc.log(p.length); vc.log(p[-1] === undefined && p[1.5] === undefined && p[99] === undefined);\
         vc.log(p[2] instanceof PathItem && p[2] instanceof PageItem);",
    );
    // Topmost first, each group before the objects in it.
    assert_eq!(
        out,
        vec![
            json!("TextFrame,PathItem,GroupItem,PathItem,PathItem"),
            json!("GroupItem"),
            json!("Layer"),
            json!("Document"),
            json!(4),
            json!(true),
            json!(true)
        ]
    );
}

#[test]
fn page_items_list_the_art_inside_a_blend() {
    let (mut s, a, b) = doc();
    log(&mut s, "app.activeDocument.pageItems[0].name = 'first';");
    s.execute("object.blend.make", &json!({"ids": [a.0, b.0], "steps": 2})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, t = [];\
         for (var i = 0; i < p.length; i++) t.push(p[i].typename);\
         vc.log(t.join(','));\
         vc.log(p.getByName('first').parent.typename); vc.log(p.index('first').typename);",
    );
    assert_eq!(out, vec![json!("BlendItem,PathItem,PathItem"), json!("BlendItem"), json!("PathItem")]);
}

#[test]
fn page_item_geometry_uses_document_coordinates_with_y_up() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var i = app.activeDocument.pageItems[1];\
         vc.log(i.geometricBounds); vc.log(i.position); vc.log([i.left, i.top, i.width, i.height]);\
         app.coordinateSystem = CoordinateSystem.ARTBOARDCOORDINATESYSTEM; vc.log(i.position);\
         app.coordinateSystem = CoordinateSystem.DOCUMENTCOORDINATESYSTEM;\
         i.position = [20, 250]; i.width = 60; vc.log(i.geometricBounds);",
    );
    assert_eq!(out[0], json!([10, 280, 40, 240]));
    assert_eq!(out[1], json!([10, 280]));
    assert_eq!(out[2], json!([10, 280, 30, 40]));
    assert_eq!(out[3], json!([10, 20]));
    assert_eq!(out[4], json!([20, 250, 80, 210]));
}

#[test]
fn page_item_size_setters_on_a_rotated_object_give_the_page_aligned_size() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let other = format!("{:?}", s.active().unwrap().doc.node(b));
    let out = log(
        &mut s,
        "var i = app.activeDocument.selection[0], h = i.height;\
         i.width = 80; vc.log([i.width, i.height - h]);\
         i.height = 90; vc.log([i.width, i.height]);\
         var g = i.geometricBounds; vc.log([g[2] - g[0], g[1] - g[3]]);",
    );
    let pairs: Vec<[f64; 2]> = out.iter().map(|v| [v[0].as_f64().unwrap(), v[1].as_f64().unwrap()]).collect();
    for (got, want) in pairs.iter().zip([[80.0, 0.0], [80.0, 90.0], [80.0, 90.0]]) {
        assert!((got[0] - want[0]).abs() < 1e-6 && (got[1] - want[1]).abs() < 1e-6, "{out:?}");
    }
    assert_eq!(pairs.len(), 3, "{out:?}");
    assert_eq!(format!("{:?}", s.active().unwrap().doc.node(b)), other, "the other object stays as it was");
}

#[test]
fn convert_coordinate_pair_is_inverse_and_y_up() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, D = CoordinateSystem.DOCUMENTCOORDINATESYSTEM, A = CoordinateSystem.ARTBOARDCOORDINATESYSTEM;\
         var p = d.convertCoordinate([10, 100], A, D); vc.log(p); vc.log(d.convertCoordinate(p, D, A));\
         vc.log(d.convertCoordinate([3, 4], D, D));",
    );
    assert_eq!(out, vec![json!([10, 200]), json!([10, 100]), json!([3, 4])]);
    assert!(
        failure(
            &mut s,
            "app.activeDocument.convertCoordinate([1, NaN], CoordinateSystem.DOCUMENTCOORDINATESYSTEM, CoordinateSystem.DOCUMENTCOORDINATESYSTEM);"
        )
        .contains("finite")
    );
    assert!(
        failure(&mut s, "app.activeDocument.convertCoordinate([1, 2], 'x', CoordinateSystem.DOCUMENTCOORDINATESYSTEM);").contains("CoordinateSystem")
    );
}

#[test]
fn ruler_origin_round_trips_without_moving_art() {
    let (mut s, a, _) = doc();
    let art = format!("{:?}", s.active().unwrap().doc.layers);
    let out = log(&mut s, "var d = app.activeDocument; vc.log(d.rulerOrigin); d.rulerOrigin = [50, 60]; vc.log(d.rulerOrigin);");
    assert_eq!(out, vec![json!([0, 300]), json!([50, 60])]);
    assert_eq!(s.active().unwrap().doc.ruler_origin_for(0), vectorcraft_geom::Point::new(50.0, 240.0));
    assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), art);
    assert!(s.active().unwrap().doc.node(a).is_some());
    let steps = undo_steps(&s);
    log(&mut s, "var d = app.activeDocument; d.rulerOrigin = d.rulerOrigin;");
    assert_eq!(undo_steps(&s), steps, "setting the same origin adds no step");
    assert!(failure(&mut s, "app.activeDocument.rulerOrigin = [1e12, 0];").contains("outside the canvas"));
}

/// A window on the document of [`doc`] that shows 300 × 200 pt around (200, 150) at 200%.
fn window() -> WindowView {
    WindowView {
        center: vectorcraft_geom::Point::new(200.0, 150.0),
        zoom: 2.0,
        zoom_baseline: 1.0,
        rotation: 0.0,
        size: [600.0, 400.0],
        screen_mode: 0,
    }
}

/// Shows the active document in the window `v`.
fn show(s: &mut Session, v: WindowView) {
    let uid = s.active().unwrap().uid;
    s.window_views = vec![((uid, 1), v)];
}

/// The window of the active document.
fn shown(s: &Session) -> Option<WindowView> {
    let uid = s.active().unwrap().uid;
    s.window_views.iter().find(|((u, _), _)| *u == uid).map(|(_, v)| *v)
}

#[test]
fn the_view_shows_the_part_of_the_document_its_window_shows() {
    let (mut s, ..) = doc();
    let none = "var v = app.activeDocument.views; vc.log(v.length); vc.log(v[0] === undefined); vc.log(0 in v); vc.log('a' in v);";
    assert_eq!(log(&mut s, none), vec![json!(0), json!(true), json!(false), json!(false)], "a document without a window has no view");
    show(&mut s, window());
    // A rectangle drawn over the visible part of the document has the view's bounds.
    let r = window().visible();
    rect(&mut s, r.x0, r.y0, r.width(), r.height());
    let out = log(
        &mut s,
        "var d = app.activeDocument; var v = d.views[0]; vc.log(d.views.length); vc.log(v.typename);\
         vc.log(v.parent === d); vc.log(v.bounds); var r = d.pageItems[0];\
         vc.log(r.geometricBounds); vc.log(v.centerPoint); vc.log(r.position[0] + r.width / 2);\
         vc.log(r.position[1] - r.height / 2);",
    );
    assert_eq!(out[..3], [json!(1), json!("View"), json!(true)]);
    assert_eq!(out[3], out[4], "the view's bounds are the visible area");
    assert_eq!(out[5], json!([out[6], out[7]]), "the centre is the middle of the visible area");
    // Turning the view shows more of the document around the same centre.
    show(&mut s, WindowView { rotation: 30.0, ..window() });
    let turned = log(&mut s, "vc.log(app.activeDocument.views[0].bounds);");
    let width = |b: &Value| b[2].as_f64().unwrap() - b[0].as_f64().unwrap();
    assert!(width(&turned[0]) > width(&out[3]));
    let names = log(&mut s, "var d = app.activeDocument; vc.log(String(d.views) + String(d.views[0]));");
    assert_eq!(names, vec![json!("[Views][View]")]);
    assert_eq!(log(&mut s, "var v = app.activeDocument.views; vc.log(0 in v); vc.log(1 in v);"), vec![json!(true), json!(false)]);
    // A named engine keeps the views from an earlier run, but they are stale.
    run(&mut s, "#targetengine keep\nvar vs = app.activeDocument.views, v = vs[0];").unwrap();
    for member in ["String(vs)", "String(v)"] {
        let e = failure(&mut s, &format!("#targetengine keep\n{member};"));
        assert!(e.contains("Stale document"), "{member}: {e}");
    }
}

#[test]
fn setting_the_zoom_changes_the_view_without_an_undo_step() {
    let (mut s, ..) = doc();
    show(&mut s, window());
    // Another document's window.
    s.window_views.push(((u64::MAX, 1), window()));
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var v = app.activeDocument.views[0]; var b = v.bounds; v.zoom = 4; vc.log(v.zoom);\
         vc.log(v.bounds[2] - v.bounds[0] < b[2] - b[0]); v.centerPoint = [100, 100]; vc.log(v.centerPoint);\
         v.screenMode = ScreenMode.FULLSCREEN; vc.log(v.screenMode);",
    );
    assert_eq!(out, vec![json!(4), json!(true), json!([100, 100]), json!("ScreenMode.FULLSCREEN")]);
    let now = shown(&s).unwrap();
    assert!(now.zoom > window().zoom);
    assert!(now.center != window().center && now.screen_mode != window().screen_mode);
    assert_eq!(s.window_views[1].1.screen_mode, now.screen_mode, "the screen mode is the same in every window");
    assert_eq!(s.window_views[1].1.zoom, window().zoom, "the zoom is each window's own");
    assert_eq!(undo_steps(&s), steps, "a view change is not undoable");
    // A bad value throws and leaves the view as it is.
    let bad = [
        ("v.zoom = 0;", "RangeError"),
        ("v.zoom = 1000;", "RangeError"),
        ("v.zoom = '2';", "TypeError"),
        ("v.centerPoint = [1];", "TypeError"),
        ("v.centerPoint = [1e12, 0];", "RangeError"),
        ("v.screenMode = 'FULLSCREEN';", "TypeError"),
    ];
    for (bad, name) in bad {
        let out = log(&mut s, &format!("var v = app.activeDocument.views[0]; try {{ {bad} vc.log('set'); }} catch (e) {{ vc.log(e.name); }}"));
        assert_eq!(out, vec![json!(name)], "{bad}");
        assert_eq!(shown(&s), Some(now), "{bad}");
    }
    // A run that fails after changing the view keeps the change.
    failure(&mut s, "app.activeDocument.views[0].zoom = 1; throw new Error('stop');");
    assert_eq!(shown(&s).map(|v| v.zoom), Some(1.0));
}

#[test]
fn view_centers_stay_inside_the_canvas_after_coordinate_conversion() {
    let (mut s, ..) = doc();
    let origin = vectorcraft_geom::Point::new(3.0e6, 3.0e6);
    s.edit("Move Artboard", |d, _| {
        d.artboards[0].rect = vectorcraft_geom::Rect::from_origin_size(origin, (400.0, 300.0));
        Ok(())
    })
    .unwrap();
    show(&mut s, window());
    let steps = undo_steps(&s);
    for coordinates in ["ARTBOARDCOORDINATESYSTEM", "DOCUMENTCOORDINATESYSTEM"] {
        let script_point = |p: vectorcraft_geom::Point| {
            let y = if coordinates == "ARTBOARDCOORDINATESYSTEM" { p.y - origin.y } else { origin.y + 300.0 - p.y };
            [p.x - origin.x, y]
        };
        for bound in [-MAX_COORD, MAX_COORD] {
            for horizontal in [false, true] {
                let point = |n| if horizontal { vectorcraft_geom::Point::new(n, 0.0) } else { vectorcraft_geom::Point::new(0.0, n) };
                let before = shown(&s);
                let [x, y] = script_point(point(bound + bound.signum()));
                let source = format!(
                    "app.coordinateSystem = CoordinateSystem.{coordinates}; var v = app.activeDocument.views[0]; \
                     try {{ v.centerPoint = [{x}, {y}]; vc.log('set'); }} catch (e) {{ vc.log(e.name); }}"
                );
                assert_eq!(log(&mut s, &source), vec![json!("RangeError")], "{source}");
                assert_eq!(shown(&s), before, "a rejected center leaves the view unchanged");
                let [x, y] = script_point(point(bound));
                log(&mut s, &format!("app.coordinateSystem = CoordinateSystem.{coordinates}; app.activeDocument.views[0].centerPoint = [{x}, {y}];"));
                assert_eq!(shown(&s).unwrap().center, point(bound), "the canvas boundary is allowed");
            }
        }
    }
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn page_item_properties_join_one_undo_step() {
    let (mut s, a, _) = doc();
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var i = app.activeDocument.pageItems[1];\
         i.name = 'box'; i.opacity = 40; i.locked = true; i.locked = false; i.hidden = true; i.hidden = false;\
         vc.log([i.name, i.opacity, i.locked, i.hidden, i.editable, i.zOrderPosition]);\
         vc.log(app.activeDocument.pageItems.getByName('box').name);",
    );
    assert_eq!(out, vec![json!(["box", 40, false, false, true, 1]), json!("box")]);
    let st = s.active().unwrap();
    let n = st.doc.node(a).unwrap();
    assert_eq!((n.name.as_deref(), (n.opacity * 100.0).round()), (Some("box"), 40.0));
    assert_eq!(undo_steps(&s), steps + 1, "one undo step for the run");
    let steps = undo_steps(&s);
    log(&mut s, "var i = app.activeDocument.pageItems[1]; i.name = 'box'; i.opacity = 40; i.hidden = false;");
    assert_eq!(undo_steps(&s), steps, "values that change nothing add no step");
    for (source, part) in [
        ("app.activeDocument.pageItems[0].opacity = 101;", "opacity"),
        ("app.activeDocument.pageItems[0].hidden = 'yes';", "hidden"),
        ("app.activeDocument.pageItems[0].width = 0;", "width"),
        ("app.activeDocument.pageItems[0].position = [1];", "position"),
    ] {
        assert!(failure(&mut s, source).contains(part), "{source}");
    }
}

/// An opacity a graph mark already has still replaces the opacity another mark of its series left on
/// the graph, as `object.setProps` does.
#[test]
fn an_opacity_a_graph_mark_already_has_replaces_the_one_its_series_keeps() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 800})).unwrap();
    let csv = ",2024,2025\nQ1,3,2\nQ2,5,4";
    let r = s.execute("graph.create", &json!({"type": "column", "x": 100, "y": 100, "width": 300, "height": 200, "csv": csv})).unwrap();
    let id = NodeId(r["id"].as_u64().unwrap());
    let kept =
        |s: &Session| s.active().unwrap().doc.node(id).unwrap().extras().graph.as_deref().unwrap().series_paints.first().and_then(|p| p.opacity);
    let series = |s: &Session| {
        let n = s.active().unwrap().doc.node(id).unwrap().clone();
        let group = n.children().unwrap().iter().find(|c| c.extras().series_index == Some(0)).unwrap().clone();
        group.children().unwrap().iter().map(|m| m.id).collect::<Vec<_>>()
    };
    let marks = series(&s);
    s.execute("object.setProps", &json!({"id": marks[0].0, "opacity": 50})).unwrap();
    assert_eq!(kept(&s), Some(0.5));
    s.execute("object.setProps", &json!({"id": series(&s)[1].0, "name": "mark"})).unwrap();
    // In a hidden or locked layer the same assignment would still change the graph, so it is refused.
    let layer = s.active().unwrap().doc.layers[0].id;
    for (on, off) in [
        (json!({"ids": [layer.0], "visible": false}), json!({"ids": [layer.0], "visible": true})),
        (json!({"ids": [layer.0], "locked": true}), json!({"ids": [layer.0], "locked": false})),
    ] {
        s.execute("object.setProps", &on).unwrap();
        let steps = undo_steps(&s);
        let e = failure(&mut s, "var m = app.activeDocument.pageItems.getByName('mark'); m.opacity = m.opacity;");
        assert!(e.contains("Cannot edit"), "{on}: {e}");
        assert_eq!(kept(&s), Some(0.5), "{on}");
        assert_eq!(undo_steps(&s), steps, "{on}");
        s.execute("object.setProps", &off).unwrap();
    }
    log(&mut s, "app.activeDocument.pageItems.getByName('mark').opacity = 100;");
    assert_eq!(kept(&s), None);
}

/// Assigning an item the opacity it reports, rounded to four decimals, changes nothing.
#[test]
fn assigning_the_reported_opacity_changes_nothing() {
    let (mut s, a, _) = doc();
    s.execute("object.setProps", &json!({"id": a.0, "opacity": 100.0 / 3.0})).unwrap();
    let (before, steps) = (s.active().unwrap().doc.node(a).unwrap().opacity, undo_steps(&s));
    let out = log(&mut s, "var i = app.activeDocument.pageItems[1]; vc.log(i.opacity); i.opacity = i.opacity;");
    assert_eq!(out, vec![json!(33.3333)]);
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().opacity, before);
    assert_eq!(undo_steps(&s), steps);
    // A value close to the reported one but not equal to it is still applied.
    s.execute("object.setProps", &json!({"id": a.0, "opacity": 50})).unwrap();
    let (before, steps) = (s.active().unwrap().doc.node(a).unwrap().opacity, undo_steps(&s));
    log(&mut s, "app.activeDocument.pageItems[1].opacity = 50.00005;");
    assert!(s.active().unwrap().doc.node(a).unwrap().opacity > before, "the assigned opacity was applied");
    assert_eq!(undo_steps(&s), steps + 1);
}

#[test]
fn selection_assignment_is_atomic_across_invalid_items() {
    let (mut s, a, b) = doc();
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": true})).unwrap();
    let e = failure(&mut s, "var p = app.activeDocument.pageItems; app.activeDocument.selection = [p[0], p[1]];");
    assert!(e.contains("Cannot select PathItem"), "{e}");
    assert!(s.active().unwrap().selection.objects.is_empty());
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": false})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pageItems; vc.log(d.selection instanceof Array && d.selection.length === 0);\
         d.selection = [p[1], p[0], p[1]]; vc.log(d.selection.length); vc.log(app.selection[0].selected);\
         p[0].selected = false; vc.log(d.selection.length);\
         d.selection = p[1]; vc.log(d.selection.length);",
    );
    assert_eq!(out, vec![json!(true), json!(2), json!(true), json!(1), json!(1)]);
    assert_eq!(s.active().unwrap().selection.objects, vec![a]);
    // An item of another document cannot be selected here.
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    let e = failure(&mut s, "app.activeDocument.selection = app.documents[0].pageItems[0];");
    assert!(e.contains("Cannot select"), "{e}");
}

#[test]
fn a_guide_path_is_not_selected_while_guides_are_locked() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("view.guides.make", &json!({})).unwrap();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("view.guides.lock", &json!({"locked": true})).unwrap();
    // The assignment and `selected` refuse the guide path, and the selection stays as it was.
    for source in ["app.activeDocument.selection = app.activeDocument.pageItems[1];", "app.activeDocument.pageItems[1].selected = true;"] {
        let e = failure(&mut s, source);
        assert!(e.contains(&format!("Cannot select PathItem {}", a.0)), "{source}: {e}");
        assert_eq!(s.active().unwrap().selection.objects, vec![b], "{source}");
    }
    // With the guides unlocked, the guide path is selected.
    s.execute("view.guides.lock", &json!({"locked": false})).unwrap();
    log(&mut s, "app.activeDocument.selection = app.activeDocument.pageItems[1];");
    assert_eq!(s.active().unwrap().selection.objects, vec![a]);
}

#[test]
fn an_inactive_document_changes_without_switching_tabs() {
    let (mut s, a, _) = doc();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    log(&mut s, "var d = app.documents[0]; d.pageItems[1].name = 'far'; d.selection = d.pageItems[0];");
    assert_eq!(s.active_index(), Some(1));
    let first = &s.documents()[0];
    assert_eq!(first.doc.node(a).unwrap().name.as_deref(), Some("far"));
    assert_eq!(first.selection.objects.len(), 1);
}

#[test]
fn move_duplicate_and_zorder_obey_parent_order() {
    let (mut s, a, b) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, first = p[1];\
         first.zOrder(ZOrderMethod.BRINGTOFRONT); vc.log(first.zOrderPosition);\
         var c = first.duplicate(); vc.log(c.zOrderPosition); vc.log(p.length); vc.log(app.activeDocument.selection.length);\
         c.zOrder(ZOrderMethod.SENDTOBACK); vc.log(c.zOrderPosition);",
    );
    assert_eq!(out, vec![json!(2), json!(3), json!(3), json!(1), json!(1)]);
    assert_eq!(s.active().unwrap().selection.objects, vec![b], "the selection stays as it was");
    let layer = &s.active().unwrap().doc.layers[0];
    let order: Vec<NodeId> = layer.children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(order[1..], [b, a]);
    assert!(failure(&mut s, "app.activeDocument.pageItems[0].zOrder('front');").contains("ZOrderMethod"));
}

/// The ids of the children of the first layer, bottom first.
fn layer_order(s: &Session) -> Vec<NodeId> {
    s.active().unwrap().doc.layers[0].children().unwrap().iter().map(|n| n.id).collect()
}

#[test]
fn move_and_duplicate_place_by_element_placement() {
    let (mut s, a, b) = doc();
    let c = rect(&mut s, 300.0, 10.0, 5.0, 5.0);
    s.execute("select.none", &json!({})).unwrap();
    let steps = undo_steps(&s);
    // Before puts it directly above the relative object, after directly below.
    let r =
        log(&mut s, "var p = app.activeDocument.pageItems; var a = p[2], c = p[0]; vc.log(a.move(c, ElementPlacement.PLACEBEFORE) === undefined);");
    assert_eq!(r, vec![json!(false)]);
    assert_eq!(layer_order(&s), [b, c, a]);
    log(&mut s, "var p = app.activeDocument.pageItems; p[0].move(p[2], ElementPlacement.PLACEAFTER);");
    assert_eq!(layer_order(&s), [a, b, c]);
    // At the end of a container is under its contents, at the beginning on top.
    log(&mut s, "var p = app.activeDocument.pageItems; p[0].move(p[0].parent, ElementPlacement.PLACEATEND);");
    assert_eq!(layer_order(&s), [c, a, b]);
    log(&mut s, "var p = app.activeDocument.pageItems; p[2].move(p[2].parent, ElementPlacement.PLACEATBEGINNING);");
    assert_eq!(layer_order(&s), [a, b, c]);
    assert_eq!(undo_steps(&s), steps + 4, "one step for each run that changed the order");
    // Moving to where it is runs nothing.
    let steps = undo_steps(&s);
    log(&mut s, "var p = app.activeDocument.pageItems; p[0].move(p[1], ElementPlacement.PLACEBEFORE);");
    assert_eq!(undo_steps(&s), steps);
    // Into a group, and a copy beside an object.
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.none", &json!({})).unwrap();
    log(&mut s, "var p = app.activeDocument.pageItems; p[0].move(p[1], ElementPlacement.INSIDE);");
    let inside: Vec<NodeId> = s.active().unwrap().doc.node(g).unwrap().children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(inside, [a, b, c]);
    let out = log(
        &mut s,
        "var g = app.activeDocument.pageItems[0], a = app.activeDocument.pageItems[3];\
         var copy = a.duplicate(g, ElementPlacement.PLACEATEND); vc.log(copy.parent.typename); vc.log(copy.zOrderPosition);",
    );
    assert_eq!(out, vec![json!("GroupItem"), json!(1)]);
    assert_eq!(s.active().unwrap().doc.node(g).unwrap().children().unwrap().len(), 4);
    // Refused moves change nothing.
    let before = format!("{:?}", s.active().unwrap().doc.layers);
    for (source, part) in [
        ("var g = app.activeDocument.pageItems[0]; g.move(app.activeDocument.pageItems[1], ElementPlacement.INSIDE);", "into itself"),
        ("var p = app.activeDocument.pageItems; p[1].move(p[2], ElementPlacement.INSIDE);", "cannot hold objects"),
        ("var p = app.activeDocument.pageItems; p[1].move(p[2], 'inside');", "ElementPlacement"),
        ("var p = app.activeDocument.pageItems; p[1].move();", "relative object"),
        ("var p = app.activeDocument.pageItems; p[1].move(app.activeDocument);", "not available yet"),
        ("var p = app.activeDocument.pageItems; p[1].duplicate(undefined, ElementPlacement.INSIDE);", "relative object"),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), before);
    // A locked group takes nothing.
    s.execute("object.setProps", &json!({"ids": [g.0], "locked": true})).unwrap();
    rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    let e = failure(&mut s, "var p = app.activeDocument.pageItems; p[0].move(p[1], ElementPlacement.INSIDE);");
    assert!(e.contains("hidden or locked"), "{e}");
}

#[test]
fn a_copy_goes_beside_its_source() {
    let (mut s, a, b) = doc();
    let out = log(
        &mut s,
        "var a = app.activeDocument.pageItems[1], c = a.duplicate(a, ElementPlacement.PLACEAFTER);\
         vc.log([c.zOrderPosition, a.zOrderPosition]);",
    );
    assert_eq!(out, vec![json!([1, 2])]);
    let order = layer_order(&s);
    assert_eq!(order[1..], [a, b]);
    // An object moved directly above or below itself stays where it is with no undo step, and it
    // cannot go into itself; a copy cannot go into its source.
    let (order, steps) = (layer_order(&s), undo_steps(&s));
    log(&mut s, "var p = app.activeDocument.pageItems; p[1].move(p[1], ElementPlacement.PLACEAFTER); p[0].move(p[0], ElementPlacement.PLACEBEFORE);");
    assert_eq!((layer_order(&s), undo_steps(&s)), (order, steps));
    assert!(failure(&mut s, "var p = app.activeDocument.pageItems; p[1].move(p[1], ElementPlacement.PLACEATBEGINNING);").contains("into itself"));
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    let e = failure(&mut s, "var p = app.activeDocument.pageItems, g = p[0]; g.duplicate(g, ElementPlacement.INSIDE);");
    assert!(e.contains("into itself"), "{e}");
}

#[test]
fn a_copy_that_cannot_be_placed_is_deleted() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let a = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    let b = rect(&mut s, 30.0, 30.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("livePaint.make", &json!({})).unwrap();
    s.execute("text.create", &json!({"x": 200, "y": 200, "text": "Hi"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let before = format!("{:?}", s.active().unwrap().doc.layers);
    let (steps, step) = (undo_steps(&s), s.active().unwrap().history.step());
    // A Live Paint group takes no type, so the copy is refused after it was made.
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, n = p.length, g = p[1], t = p[0], m = '';\
         try { t.duplicate(g, ElementPlacement.INSIDE); } catch (e) { m = e.message; }\
         vc.log(m); vc.log(p.length === n);",
    );
    assert!(out[0].as_str().is_some_and(|m| m.contains("Live Paint takes paths")), "{out:?}");
    assert_eq!(out[1], json!(true));
    assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), before);
    // The refused copy leaves no undo step and does not change the document's step.
    assert_eq!((undo_steps(&s), s.active().unwrap().history.step()), (steps, step));
}

#[test]
fn a_path_inside_a_compound_path_is_not_duplicated() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let before = format!("{:?}", s.active().unwrap().doc.layers);
    let steps = undo_steps(&s);
    let e = failure(&mut s, "var p = app.activeDocument.pageItems; vc.log(p[0].typename); p[1].duplicate();");
    assert!(e.contains("Duplicating a path inside a compound path is not available yet"), "{e}");
    assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), before);
    assert_eq!(undo_steps(&s), steps);
    // The compound path itself is copied whole.
    let out = log(&mut s, "var p = app.activeDocument.pageItems, c = p[0].duplicate(); vc.log([c.typename, p.length]);");
    assert_eq!(out, vec![json!(["CompoundPathItem", 6])]);
}

#[test]
fn a_path_inside_a_compound_path_is_not_reordered() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    rect(&mut s, 200.0, 200.0, 20.0, 20.0);
    let before = format!("{:?}", s.active().unwrap().doc.layers);
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, m = [], out = [];\
         for (var i = 0; i < p.length; i++) if (p[i].parent.typename == 'CompoundPathItem') m.push(p[i]);\
         var ways = [ZOrderMethod.BRINGTOFRONT, ZOrderMethod.BRINGFORWARD, ZOrderMethod.SENDBACKWARD, ZOrderMethod.SENDTOBACK];\
         for (var j = 0; j < m.length; j++) for (var k = 0; k < ways.length; k++) {\
           try { m[j].zOrder(ways[k]); out.push('ran'); } catch (e) { out.push(e.message); }\
         }\
         vc.log(m.length); vc.log(out);",
    );
    assert_eq!(out[0], json!(2));
    let messages = out[1].as_array().cloned().unwrap_or_default();
    assert_eq!(messages.len(), 8, "{out:?}");
    for m in &messages {
        assert!(m.as_str().is_some_and(|m| m.contains("Reordering a path inside a compound path is not available yet")), "{out:?}");
    }
    assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), before);
    assert_eq!(undo_steps(&s), steps);
    // The compound path itself still moves.
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, c;\
         for (var i = 0; i < p.length; i++) if (p[i].typename == 'CompoundPathItem') c = p[i];\
         var at = c.zOrderPosition; c.zOrder(ZOrderMethod.BRINGTOFRONT); vc.log([at, c.zOrderPosition]);",
    );
    assert_eq!(out, vec![json!([1, 2])]);
}

#[test]
fn moving_the_last_path_out_of_a_compound_path_keeps_the_compound_path() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let compound = layer_order(&s)[0];
    let layer = s.active().unwrap().doc.layers[0].id;
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var d = app.activeDocument, c = d.pageItems[0], m = [d.pageItems[2], d.pageItems[1]];\
         m[0].move(c, ElementPlacement.PLACEBEFORE); vc.log(d.pageItems.length);\
         m[1].move(c, ElementPlacement.PLACEBEFORE); vc.log(d.pageItems.length);\
         vc.log([c.typename, c === d.pageItems[2]]);\
         var bad = []; for (var k in c) { try { c[k]; } catch (e) { bad.push(k + ': ' + e.message); } } vc.log(bad);\
         vc.log([c.geometricBounds, c.visibleBounds, c.position, c.width, c.height]);\
         c.left = 5; c.top = 5; c.position = [1, 2]; c.width = 10; c.height = 10; vc.log(c.geometricBounds);",
    );
    let zeros = json!([0, 0, 0, 0]);
    assert_eq!(out[..3], [json!(3), json!(3), json!(["CompoundPathItem", true])]);
    // Every property `for`-`in` lists reads, and the emptied compound path has zero bounds that
    // moving or resizing leaves as they are.
    assert_eq!(out[3..], [json!([]), json!([zeros, zeros, [0, 0], 0, 0]), zeros]);
    let doc = &s.active().unwrap().doc;
    assert!(doc.node(compound).is_some_and(|n| n.children().is_some_and(|c| c.is_empty())), "the compound path stays, empty");
    // Each path went directly above the compound path, the second above the first.
    assert_eq!(layer_order(&s), [compound, b, a]);
    assert!([a, b].iter().all(|id| doc.parent_of(*id) == Some(layer)));
    assert_eq!(undo_steps(&s), steps + 1, "the run is one undo step");
}

/// `remove()` on the last path of a compound path keeps the compound path, empty, as `move` does,
/// and a document holding it saves, reopens, exports and releases it.
#[test]
fn removing_the_last_path_of_a_compound_path_keeps_the_compound_path() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let compound = layer_order(&s)[0];
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var d = app.activeDocument, c = d.pageItems[0]; d.pageItems[2].remove(); d.pageItems[1].remove();\
         vc.log([c.typename, d.pageItems.length, c === d.pageItems[0]]);",
    );
    assert_eq!(out, vec![json!(["CompoundPathItem", 1, true])]);
    let doc = s.active().unwrap().doc.clone();
    assert!(doc.node(compound).is_some_and(|n| n.children().is_some_and(|c| c.is_empty())), "the compound path stays, empty");
    assert_eq!(undo_steps(&s), steps + 1, "the run is one undo step");
    assert!(s.execute("edit.clear", &json!({"ids": [compound.0], "keepEmpty": 1})).is_err(), "keepEmpty must be a boolean");
    // The native format keeps it, and the other writers and Release Compound Path accept it.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&doc, false)).unwrap();
    assert!(back.node(compound).is_some_and(|n| matches!(n.kind, NodeKind::Compound { .. }) && n.children().is_some_and(|c| c.is_empty())));
    for format in ["svg", "pdf", "vectorcraft"] {
        s.execute("document.export", &json!({"format": format})).unwrap_or_else(|e| panic!("{format}: {e}"));
    }
    s.execute("select.set", &json!({"ids": [compound.0]})).unwrap();
    s.execute("object.compoundPath.release", &json!({})).unwrap();
}

/// Clear with `keepEmpty` keeps the emptied compound path also when it deletes direct-selected
/// anchors, and without it deletes the compound path as before.
#[test]
fn clearing_anchors_with_keep_empty_keeps_the_compound_path() {
    for keep in [true, false] {
        let (mut s, a, b) = doc();
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        s.execute("object.compoundPath.make", &json!({})).unwrap();
        let compound = layer_order(&s)[0];
        for (id, mode) in [(a, "set"), (b, "add")] {
            s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0], [0, 1], [0, 2]], "mode": mode})).unwrap();
        }
        s.execute("edit.clear", &json!({"keepEmpty": keep})).unwrap();
        let d = &s.active().unwrap().doc;
        assert!(d.node(a).is_none() && d.node(b).is_none(), "keepEmpty {keep}: both paths are deleted");
        assert_eq!(d.node(compound).is_some_and(|n| n.children().is_some_and(|c| c.is_empty())), keep, "keepEmpty {keep}");
    }
}

#[test]
fn a_copy_of_an_object_with_a_large_id_is_the_copy() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    // The copy's id, 2^53 + 1, is not exact as a number.
    let big = 1u64 << 53;
    s.active_mut().unwrap().change_unsaved(|d| std::sync::Arc::make_mut(d).raise_next_id(big));
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    assert_eq!(a.0, big);
    let out = log(&mut s, "var i = app.activeDocument.pageItems[0], c = i.duplicate(); c.name = 'copy'; vc.log(c === i); vc.log(i.name);");
    assert_eq!(out, vec![json!(false), json!("")]);
    let d = &s.active().unwrap().doc;
    assert_eq!(d.node(NodeId(big + 1)).and_then(|n| n.name.clone()).as_deref(), Some("copy"));
    assert_eq!(d.node(a).unwrap().name, None);
}

#[test]
fn a_grid_by_column_takes_the_number_of_rows() {
    let mut s = Session::new();
    // With five artboards in four rows, the columns hold four and one.
    for (layout, count, given, rows, cols) in
        [("GridByCol", 6, 2, 2, 3), ("RLGridByCol", 6, 2, 2, 3), ("GridByRow", 6, 2, 3, 2), ("GridByCol", 5, 4, 4, 2), ("RLGridByCol", 5, 4, 4, 2)]
    {
        run(&mut s, &format!("app.documents.add(DocumentColorSpace.RGB, 100, 100, {count}, DocumentArtboardLayout.{layout}, 10, {given});")).unwrap();
        let boards = &s.active().unwrap().doc.artboards;
        let distinct = |mut v: Vec<i64>| {
            v.sort_unstable();
            v.dedup();
            v.len()
        };
        let tops = distinct(boards.iter().map(|b| b.rect.y0.round() as i64).collect());
        let lefts = distinct(boards.iter().map(|b| b.rect.x0.round() as i64).collect());
        assert_eq!((tops, lefts), (rows, cols), "{layout} {count}");
    }
}

#[test]
fn file_new_takes_the_rows_of_a_grid_by_column() {
    let mut s = Session::new();
    let first_column = |s: &mut Session, layout: Value| {
        s.execute("file.new", &json!({"width": 100, "height": 100, "artboards": 5, "artboardLayout": layout})).unwrap();
        let boards = &s.active().unwrap().doc.artboards;
        boards.iter().filter(|b| b.rect.x0.round() as i64 == boards[0].rect.x0.round() as i64).count()
    };
    assert_eq!(first_column(&mut s, json!({"layout": "gridByColumn", "rows": 4})), 4);
    // Rows take the place of columns given beside them.
    assert_eq!(first_column(&mut s, json!({"layout": "gridByColumn", "rows": 4, "columns": 1})), 4);
    // A preset keeps and lists the rows, with the columns they give.
    use crate::cmd::newdoc::{ArtboardLayout, DocSettings};
    let settings = DocSettings { artboards: 5, layout: ArtboardLayout::GridByColumn, ..DocSettings::default() };
    let settings = settings.with_params(&json!({"artboardLayout": {"rows": 4}}), "file.new").unwrap();
    let listed = settings.to_json();
    assert_eq!((&listed["artboardLayout"]["columns"], &listed["artboardLayout"]["rows"]), (&json!(2), &json!(4)), "{listed}");
    let stored: DocSettings = serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
    assert_eq!(stored.rows, 4);
    // More artboards keep the rows; columns alone, or another layout, replace them.
    let more = stored.clone().with_params(&json!({"artboards": 9, "artboardLayout": listed["artboardLayout"]}), "file.new").unwrap();
    assert_eq!(more.boards().iter().filter(|b| b.x0 == more.boards()[0].x0).count(), 4);
    let by_columns = stored.clone().with_params(&json!({"artboardLayout": {"columns": 1}}), "file.new").unwrap();
    assert_eq!((by_columns.rows, by_columns.boards().iter().filter(|b| b.x0 == by_columns.boards()[0].x0).count()), (0, 5));
    for layout in [json!({"layout": "gridByRow"}), json!("gridByRow")] {
        let by_row = stored.clone().with_params(&json!({"artboardLayout": layout}), "file.new").unwrap();
        assert_eq!((by_row.rows, by_row.to_json()["artboardLayout"].get("rows")), (0, None), "{layout}");
    }
    let e = s.execute("file.new", &json!({"artboardLayout": {"layout": "gridByColumn", "rows": 0}})).unwrap_err().to_string();
    assert!(e.contains("rows must be 1 or more"), "{e}");
    for layout in ["gridByRow", "row", "column"] {
        let e = s.execute("file.new", &json!({"artboardLayout": {"layout": layout, "rows": 2}})).unwrap_err().to_string();
        assert!(e.contains("rows needs the gridByColumn layout"), "{layout}: {e}");
    }
}

#[test]
fn a_listed_preset_keeps_more_rows_than_artboards() {
    use crate::cmd::newdoc::{ArtboardLayout, DocSettings};
    let settings = DocSettings { artboards: 2, layout: ArtboardLayout::GridByColumn, ..DocSettings::default() };
    let settings = settings.with_params(&json!({"artboardLayout": {"rows": 4}}), "file.new").unwrap();
    let listed = settings.to_json();
    assert_eq!((&listed["artboardLayout"]["columns"], &listed["artboardLayout"]["rows"]), (&json!(1), &json!(4)), "{listed}");
    // Five artboards from the listing fill four rows, not two.
    let more = settings.with_params(&json!({"artboards": 5, "artboardLayout": listed["artboardLayout"]}), "file.new").unwrap();
    assert_eq!(more.boards().iter().filter(|b| b.x0 == more.boards()[0].x0).count(), 4);
}

#[test]
fn stale_documents_throw_on_every_member() {
    let (mut s, _, _) = doc();
    run(&mut s, "#targetengine keep\nvar d = app.activeDocument, p = d.pageItems, l = p[0].parent;").unwrap();
    for (member, message) in [
        ("d.typename", "Stale document"),
        ("d.parent", "Stale document"),
        ("p.typename", "Stale document"),
        ("p.toString()", "Stale document"),
        ("l.typename", "Stale"),
    ] {
        let e = failure(&mut s, &format!("#targetengine keep\n{member};"));
        assert!(e.contains(message), "{member}: {e}");
    }
}

#[test]
fn hiding_or_locking_a_selected_item_deselects_it() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    log(&mut s, "app.activeDocument.pageItems[1].hidden = true;");
    assert_eq!(s.active().unwrap().selection.objects, vec![b]);
    log(&mut s, "app.activeDocument.pageItems[0].locked = true;");
    assert!(s.active().unwrap().selection.objects.is_empty());
}

#[test]
fn an_edit_keeps_direct_selected_anchors() {
    let (mut s, a, _) = doc();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 1]], "mode": "set"})).unwrap();
    let before = s.active().unwrap().selection.clone();
    assert!(!before.anchors.is_empty());
    log(&mut s, "var p = app.activeDocument.pageItems; p[1].duplicate(); p[1].zOrder(ZOrderMethod.SENDTOBACK);");
    assert_eq!(s.active().unwrap().selection.anchors, before.anchors);
    assert_eq!(s.active().unwrap().selection.objects, before.objects);
    assert!(s.journal.iter().any(|(id, _)| id == "select.anchorsMany"), "the journal holds the restore");
}

#[test]
fn an_edit_keeps_the_order_of_a_mixed_selection() {
    let (mut s, a, b) = doc();
    rect(&mut s, 200.0, 200.0, 20.0, 20.0);
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 1]], "mode": "set"})).unwrap();
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    let before = s.active().unwrap().selection.clone();
    assert_eq!(before.objects, vec![a, b]);
    log(&mut s, "app.activeDocument.pageItems[2].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.anchors.clone()), (before.objects, before.anchors));
}

#[test]
fn assigning_the_selection_clears_a_layers_panel_target() {
    let (mut s, a, _) = doc();
    s.execute("layer.target", &json!({"id": a.0})).unwrap();
    assert_eq!(s.active().unwrap().selection.target, Some(a));
    log(&mut s, "var d = app.activeDocument; d.selection = d.pageItems[1];");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.target), (vec![a], None));
}

#[test]
fn an_edit_keeps_a_layers_panel_target() {
    let (mut s, a, b) = doc();
    s.execute("layer.target", &json!({"id": a.0})).unwrap();
    log(&mut s, "app.activeDocument.pageItems[1].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.target), (vec![a], Some(a)));
    // A targeted layer stays targeted, with its art selected.
    let layer = s.active().unwrap().doc.layers[0].id;
    s.execute("layer.target", &json!({"id": layer.0})).unwrap();
    log(&mut s, "app.activeDocument.pageItems[1].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.target, sel.objects.len(), sel.objects.contains(&b)), (Some(layer), 4, true));
}

#[test]
fn an_edit_keeps_the_current_layer() {
    let (mut s, a, b) = doc();
    let first = s.active().unwrap().doc.layers[0].id;
    let back = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let layer = |s: &Session| (s.active().unwrap().active_layer, s.active().unwrap().highlighted_rows());
    assert_eq!(layer(&s), (Some(back), vec![back]));
    // Moving an object of the first layer selects it for the command; the new layer stays current.
    log(&mut s, "app.activeDocument.pageItems[0].left = 50;");
    assert_eq!(layer(&s), (Some(back), vec![back]));
    assert_eq!(s.active().unwrap().selection.objects, vec![b]);
    // With nothing selected and no row highlighted, the layer stays current and no row is highlighted.
    s.execute("select.none", &json!({})).unwrap();
    s.execute("layer.setCurrent", &json!({"id": back.0})).unwrap();
    s.execute("layer.highlight", &json!({"ids": [], "mode": "set"})).unwrap();
    assert_eq!(layer(&s), (Some(back), vec![]));
    log(&mut s, "app.activeDocument.pageItems[1].left = 120;");
    assert_eq!(layer(&s), (Some(back), vec![]));
    assert!(s.active().unwrap().selection.objects.is_empty());
    // Targeting a layer makes it current and leaves another layer's row highlighted; both stay.
    s.execute("layer.highlight", &json!({"ids": [first.0], "mode": "set"})).unwrap();
    s.execute("layer.target", &json!({"id": back.0})).unwrap();
    assert_eq!(layer(&s), (Some(back), vec![first]));
    log(&mut s, "app.activeDocument.pageItems[0].left = 70;");
    assert_eq!(layer(&s), (Some(back), vec![first]));
    // Assigning the selection makes the layer of the selected object current, as a click does.
    log(&mut s, "app.activeDocument.selection = app.activeDocument.pageItems[1];");
    assert_eq!(s.active().unwrap().active_layer, Some(first));
    assert_eq!(s.active().unwrap().selection.objects, vec![a]);
}

/// A document with a rectangle at the bottom of the stack, then a dynamic symbol instance of two
/// rectangles and an Objects on Path group of one rectangle: (session, instance, group, the
/// group's rectangle).
fn chosen_parts() -> (Session, NodeId, NodeId, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    rect(&mut s, 300.0, 200.0, 20.0, 20.0);
    let (x, y) = (rect(&mut s, 10.0, 10.0, 20.0, 20.0), rect(&mut s, 50.0, 10.0, 20.0, 20.0));
    let id = |v: Value| NodeId(v["id"].as_u64().unwrap());
    let inst = id(s.execute("symbol.new", &json!({"name": "Pair", "ids": [x.0, y.0], "type": "dynamic"})).unwrap());
    let member = rect(&mut s, 10.0, 100.0, 20.0, 20.0);
    let line = id(s.execute("shape.line", &json!({"x1": 50, "y1": 250, "x2": 350, "y2": 250})).unwrap());
    s.execute("select.set", &json!({"ids": [member.0]})).unwrap();
    let group = id(s.execute("object.objectsOnPath.attach", &json!({"path": line.0})).unwrap());
    (s, inst, group, member)
}

#[test]
fn an_edit_keeps_an_object_chosen_inside_a_symbol_or_on_a_path() {
    let (mut s, inst, group, member) = chosen_parts();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    let before = s.active().unwrap().selection.clone();
    log(&mut s, "app.activeDocument.pageItems[0].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.symbol_part.clone()), (before.objects, before.symbol_part));
    s.execute("object.objectsOnPath.selectMember", &json!({"member": member.0})).unwrap();
    log(&mut s, "app.activeDocument.pageItems[0].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.on_path_member), (vec![group], Some(member)));
}

#[test]
fn assigning_the_selection_selects_a_symbol_or_on_path_group_as_a_whole() {
    let (mut s, inst, group, member) = chosen_parts();
    s.execute("symbol.instance.selectPart", &json!({"id": inst.0, "path": [1]})).unwrap();
    log(&mut s, "var d = app.activeDocument; d.selection = d.selection[0];");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.symbol_part.clone()), (vec![inst], None));
    s.execute("object.objectsOnPath.selectMember", &json!({"member": member.0})).unwrap();
    log(&mut s, "var d = app.activeDocument; d.selection = d.selection[0];");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.on_path_member), (vec![group], None));
}

#[test]
fn a_held_image_stays_the_same_object_when_it_is_embedded() {
    let dir = super::links::Folder::new("dom-embed");
    let photo = dir.file("photo.png");
    super::links::write(&photo, &super::links::png(20, 10, [255, 0, 0]));
    let mut s = super::links::session();
    let id = super::links::place(&mut s, &photo);
    let out = log(
        &mut s,
        &format!(
            "var i = app.activeDocument.pageItems[0]; vc.log(i.typename); vc.run('links.embed', {{ids: [{}]}});\
             var j = app.activeDocument.pageItems[0]; vc.log([i === j, i.typename, i instanceof RasterItem, i instanceof PlacedItem]);",
            id.0
        ),
    );
    assert_eq!(out, vec![json!("PlacedItem"), json!([true, "RasterItem", true, false])]);
}

#[test]
fn an_edit_keeps_the_key_object() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    log(&mut s, "var p = app.activeDocument.pageItems; p[0].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.key), (vec![a, b], Some(b)));
}

#[test]
fn an_edit_keeps_selected_slices() {
    let (mut s, a, _) = doc();
    s.execute("object.slice.make", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.slice.select", &json!({"slices": [a.0]})).unwrap();
    log(&mut s, "app.activeDocument.pageItems[1].duplicate();");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.slices.clone(), sel.objects.len()), (vec![a], 0));
}

#[test]
fn a_startup_preset_alone_gives_its_first_size() {
    let mut s = Session::new();
    for profile in ["Web", "Print", "Mobile"] {
        let first = crate::cmd::newdoc::category(&s, profile).unwrap().remove(0);
        let out = log(&mut s, &format!("var d = app.documents.addDocument('{profile}'); vc.log([d.width, d.height]);"));
        let got = (out[0][0].as_f64(), out[0][1].as_f64());
        assert_eq!(got, (Some(first.width), Some(first.height)), "{profile}");
    }
}

#[test]
fn page_item_members_not_built_yet_throw_on_read_and_assignment() {
    let (mut s, _, _) = doc();
    for source in
        ["app.activeDocument.pageItems[0].blendingMode;", "app.activeDocument.pageItems[0].uuid = 'x';", "app.activeDocument.pageItems[0].rotate(5);"]
    {
        assert!(failure(&mut s, source).contains("is not available yet"), "{source}");
    }
    assert!(failure(&mut s, "app.activeDocument.layers = 1;").contains("Document.layers is not available yet"));
    // A deleted item or an earlier run's document is stale first.
    let e = failure(&mut s, "var i = app.activeDocument.pageItems[0]; i.remove(); i.blendingMode;");
    assert!(e.contains("Stale page item"), "{e}");
    run(&mut s, "#targetengine keep\nvar d = app.activeDocument;").unwrap();
    let e = failure(&mut s, "#targetengine keep\nd.layers;");
    assert!(e.contains("Stale document"), "{e}");
}

#[test]
fn assigning_the_same_objects_clears_the_key_and_guides() {
    let (mut s, a, b) = doc();
    s.execute("guide.add", &json!({"vertical": true, "pos": 120})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("select.key", &json!({"id": b.0})).unwrap();
    s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
    assert!(!s.active().unwrap().selection.guides.is_empty());
    log(&mut s, "var d = app.activeDocument; d.selection = [d.pageItems[1], d.pageItems[0]];");
    let sel = &s.active().unwrap().selection;
    assert_eq!((sel.objects.clone(), sel.key, sel.guides.len()), (vec![a, b], None, 0));
    s.execute("guide.select", &json!({"indexes": [0]})).unwrap();
    log(&mut s, "app.activeDocument.selection = null;");
    assert!(s.active().unwrap().selection.guides.is_empty());
}

#[test]
fn a_page_item_with_a_large_id_keeps_its_id() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let big = (1u64 << 53) + 1;
    s.active_mut().unwrap().change_unsaved(|d| std::sync::Arc::make_mut(d).raise_next_id(big));
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    assert_eq!(a.0, big);
    let out = log(&mut s, "var i = app.activeDocument.pageItems[0]; i.name = 'big'; vc.log(i.name); i.remove();");
    assert_eq!(out, vec![json!("big")]);
    assert!(s.active().unwrap().doc.node(a).is_none());
}

#[test]
fn only_class_constructors_construct() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var m = [];\
         try { new app.documents.add(); } catch (e) { m.push(e.name); }\
         try { new app.activeDocument.pageItems[0].remove(); } catch (e) { m.push(e.name); }\
         vc.log(m); vc.log(app.documents.length); vc.log(app.activeDocument.pageItems.length);",
    );
    assert_eq!(out, vec![json!(["TypeError", "TypeError"]), json!(1), json!(2)]);
}

#[test]
fn a_document_preset_is_an_instance_of_its_class() {
    let mut s = Session::new();
    let out = log(&mut s, "var p = new DocumentPreset(); vc.log(p instanceof DocumentPreset); vc.log(p.typename); vc.log(p.width);");
    assert_eq!(out, vec![json!(true), json!("DocumentPreset"), json!(612)]);
}

#[test]
fn the_same_document_or_item_is_the_same_object() {
    let (mut s, a, _) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pageItems;\
         vc.log([d === app.documents[0], p[1] === p[1], p[1] === d.selection[0], p[0] !== p[1], p === d.pageItems]);",
    );
    assert_eq!(out, vec![json!([true, true, true, true, true])]);
}

#[test]
fn collection_entries_are_found_by_in_and_array_methods() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, d = app.documents;\
         vc.log([0 in p, '1' in p, 2 in p, 0 in d, 1 in d]);\
         vc.log(Array.prototype.map.call(p, function (i) { return i.typename; }).join(','));",
    );
    assert_eq!(out, vec![json!([true, true, false, true, false]), json!("PathItem,PathItem")]);
}

#[test]
fn placement_in_another_document_is_refused() {
    let (mut s, _, _) = doc();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();
    rect(&mut s, 0.0, 0.0, 5.0, 5.0);
    let before: Vec<String> = s.documents().iter().map(|d| format!("{:?}", d.doc.layers)).collect();
    for source in [
        "app.documents[0].pageItems[0].move(app.documents[1].pageItems[0], ElementPlacement.PLACEBEFORE);",
        "app.documents[0].pageItems[0].duplicate(app.documents[1].pageItems[0].parent, ElementPlacement.INSIDE);",
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains("Cross-document placement is not available"), "{source}: {e}");
    }
    let after: Vec<String> = s.documents().iter().map(|d| format!("{:?}", d.doc.layers)).collect();
    assert_eq!(after, before);
}

#[test]
fn note_is_the_attributes_note() {
    let (mut s, a, _) = doc();
    let steps = undo_steps(&s);
    let out = log(&mut s, "var i = app.activeDocument.pageItems[1]; vc.log(i.note); i.note = 'check bleed'; vc.log(i.note);");
    assert_eq!(out, vec![json!(""), json!("check bleed")]);
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().extras().attrs.as_ref().unwrap().note, "check bleed");
    assert_eq!(undo_steps(&s), steps + 1);
    log(&mut s, "app.activeDocument.pageItems[1].note = 'check bleed';");
    assert_eq!(undo_steps(&s), steps + 1, "the same note adds no step");
    // A number or a Boolean becomes its text, as it does for `name`.
    let out = log(&mut s, "var i = app.activeDocument.pageItems[1]; i.note = 5; vc.log([typeof i.note, i.note]); i.note = true; vc.log(i.note);");
    assert_eq!(out, vec![json!(["string", "5"]), json!("true")]);
    for value in ["null", "undefined", "{}"] {
        assert!(failure(&mut s, &format!("app.activeDocument.pageItems[0].note = {value};")).contains("note must be a string"), "{value}");
    }
    assert!(failure(&mut s, "app.activeDocument.pageItems[0].note = new Array(5000).join('n');").contains("too long"));
}

#[test]
fn document_preset_applies_only_its_given_fields() {
    let mut s = Session::new();
    let out = log(
        &mut s,
        "var p = new DocumentPreset(); vc.log([p.typename, p.width, p.height, p.numArtboards, p.artboardSpacing, p.artboardLayout]);\
         vc.log(p.colorMode === undefined && p.units === undefined);\
         p.width = 300; p.height = 200; p.numArtboards = 2; p.colorMode = DocumentColorSpace.CMYK; p.title = 'Flyer';\
         p.units = RulerUnits.Millimeters; p.rasterResolution = DocumentRasterResolution.HighResolution;\
         var d = app.documents.addDocument('Print', p); vc.log([d.name, d.width, d.height, d.documentColorSpace, d.rulerUnits]);\
         var e = app.documents.addDocument(DocumentPresetType.BasicRGB); vc.log(e.documentColorSpace);",
    );
    assert_eq!(out[0], json!(["DocumentPreset", 612, 792, 1, 20, "DocumentArtboardLayout.GridByRow"]));
    assert_eq!(out[1], json!(true));
    assert_eq!(out[2], json!(["Flyer", 300, 200, "DocumentColorSpace.CMYK", "RulerUnits.Millimeters"]));
    assert_eq!(out[3], json!("DocumentColorSpace.RGB"));
    assert_eq!(s.documents().len(), 2);
    assert_eq!(s.documents()[0].doc.artboards.len(), 2);
    assert!((s.documents()[0].doc.raster_effects_ppi - 300.0).abs() < 1e-9);
    for (source, part) in [
        ("app.documents.addDocument('Nope');", "no startup preset"),
        ("var p = new DocumentPreset(); p.numArtboards = 101; app.documents.addDocument('Print', p);", "numArtBoards"),
        ("var p = new DocumentPreset(); p.units = RulerUnits.Qs; app.documents.addDocument('Print', p);", "units"),
        (
            "var p = new DocumentPreset(); p.transparencyGrid = DocumentTransparencyGrid.TransparencyGridRed; app.documents.addDocument('Print', p);",
            "not available yet",
        ),
        ("app.documents.addDocument('Print', 5);", "DocumentPreset"),
        ("app.documents.addDocument(7);", "startup preset"),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(s.documents().len(), 2, "a refused preset makes no document");
}

#[test]
fn headless_add_document_dialog_fails_without_creating() {
    let mut s = Session::new();
    let e = failure(&mut s, "app.documents.addDocument('Print', new DocumentPreset(), true);");
    assert!(e.contains("Dialog unavailable"), "{e}");
    assert!(s.documents().is_empty());
    let out = log(&mut s, "vc.log(app.documents.addDocument('Web', undefined, false).typename);");
    assert_eq!(out, vec![json!("Document")]);
    assert_eq!(s.documents().len(), 1);
}

#[test]
fn setting_selected_keeps_how_the_other_objects_are_selected() {
    let (mut s, a, b) = doc();
    let c = rect(&mut s, 200.0, 100.0, 50.0, 50.0);
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("select.anchors", &json!({"id": a.0, "anchors": [[0, 0]], "mode": "add"})).unwrap();
    let anchors = s.active().unwrap().selection.anchors.get(&a).cloned();
    assert!(anchors.is_some());
    log(
        &mut s,
        "var p = app.activeDocument.pageItems;\
         for (var i = 0; i < p.length; i++) { if (p[i].left == 200) p[i].selected = true; if (p[i].left == 100) p[i].selected = false; }",
    );
    let sel = &s.active().unwrap().selection;
    assert_eq!(sel.objects, vec![a, c]);
    assert_eq!(sel.anchors.get(&a).cloned(), anchors, "the direct-selected anchors stay");
}

#[test]
fn add_document_takes_only_a_document_preset() {
    let mut s = Session::new();
    for source in [
        "app.documents.addDocument('Print', {width: 500});",
        "app.documents.addDocument('Print', {});",
        "app.documents.addDocument('Print', {typename: 'DocumentPreset', width: 500});",
        "var p = new DocumentPreset(); var q = {}; for (var k in p) q[k] = p[k]; app.documents.addDocument('Print', q);",
        "app.documents.addDocument('Print', [612, 792]);",
        "app.documents.addDocument('Print', 'DocumentPreset');",
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains("TypeError") && e.contains("presetSettings must be a DocumentPreset"), "{source}: {e}");
    }
    assert!(s.documents().is_empty(), "a refused preset makes no document");
    let out = log(&mut s, "var p = new DocumentPreset(); p.width = 500; vc.log(app.documents.addDocument('Print', p).width);");
    assert_eq!(out, vec![json!(500)]);
}

#[test]
fn add_document_applies_the_profile_file_and_throws_when_it_cannot_be_read() {
    let dir = std::env::temp_dir().join(format!("vc-script-add-document-profile-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.attach_new_doc_profiles(dir.clone());
    // A profile with no file gives the defaults.
    let out = log(&mut s, "vc.log(app.documents.addDocument('Web').typename);");
    assert_eq!(out, vec![json!("Document")]);
    assert_eq!(s.doc().unwrap().links_from, None);
    // A script reads the profile's file in the app's own folder.
    s.execute("file.newProfiles.edit", &json!({"name": "Print"})).unwrap();
    let file = dir.join("Print.vectorcraft").to_string_lossy().to_string();
    log(&mut s, "app.documents.addDocument('Print');");
    assert_eq!(s.doc().unwrap().links_from.as_deref(), Some(file.as_str()));
    while !s.documents().is_empty() {
        s.execute("file.close", &json!({})).unwrap();
    }
    std::fs::write(dir.join("Print.vectorcraft"), b"not a document").unwrap();
    // file.new itself makes the document with the defaults and reports the profile it could not read.
    let r = s.execute("file.new", &json!({"preset": "Letter"})).unwrap();
    assert_eq!(r["profileError"]["profile"], "Print", "{r}");
    s.execute("file.close", &json!({})).unwrap();
    let recent = s.execute("file.newPresets", &json!({"category": "Recent"})).unwrap();
    let e = failure(&mut s, "app.documents.addDocument('Print');");
    assert!(e.contains("Error: the New Document profile Print could not be read"), "{e}");
    assert!(s.documents().is_empty(), "the document made with the defaults is put back");
    // A script that catches the error finds nothing changed.
    let out = log(
        &mut s,
        "var p = new DocumentPreset(); p.width = 333;\
         try { app.documents.addDocument('Print', p); } catch (e) { vc.log(e.name); } vc.log(app.documents.length);",
    );
    assert_eq!(out, vec![json!("Error"), json!(0)]);
    assert_eq!(s.execute("file.newPresets", &json!({"category": "Recent"})).unwrap(), recent, "the Recent sizes are as they were");
    // A startup preset with no profile reads no file.
    let out = log(&mut s, "vc.log(app.documents.addDocument('Basic RGB').documentColorSpace);");
    assert_eq!(out, vec![json!("DocumentColorSpace.RGB")]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn add_document_names_a_color_profile_of_the_profile_file_that_cannot_be_used() {
    let dir = std::env::temp_dir().join(format!("vc-script-add-document-warning-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut s = Session::new();
    s.attach_new_doc_profiles(dir.clone());
    s.execute("file.newProfiles.edit", &json!({"name": "Print"})).unwrap();
    while !s.documents().is_empty() {
        s.execute("file.close", &json!({})).unwrap();
    }
    let file = dir.join("Print.vectorcraft");
    let mut v: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    let broken = format!("Script profile broken {}", std::process::id());
    v["profiles"] = json!({ &broken: {"mime": "application/vnd.iccprofile", "data": "AAAA"} });
    std::fs::write(&file, serde_json::to_vec(&v).unwrap()).unwrap();
    let r = run(&mut s, "vc.log(app.documents.addDocument('Print').typename);").unwrap();
    assert_eq!(r["log"], json!(["Document"]));
    let lines = r["console"]["lines"].as_array().cloned().unwrap_or_default();
    let named = lines.iter().filter_map(Value::as_str).any(|l| l.starts_with("addDocument: ") && l.contains(&broken));
    assert!(named, "{lines:?}");
    assert_eq!(s.documents().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn add_document_refuses_pixel_preview() {
    let mut s = Session::new();
    let e = failure(&mut s, "var p = new DocumentPreset(); p.previewMode = DocumentPreviewMode.PixelPreview; app.documents.addDocument('Web', p);");
    assert!(e.contains("DocumentPreviewMode.PixelPreview is not available yet"), "{e}");
    assert!(s.documents().is_empty());
    let out = log(
        &mut s,
        "var p = new DocumentPreset(); p.previewMode = DocumentPreviewMode.DefaultPreview; vc.log(app.documents.addDocument('Web', p).typename);",
    );
    assert_eq!(out, vec![json!("Document")]);
}

#[test]
fn document_preset_values_count_toward_the_data_a_run_passes() {
    // A 64 KiB title, set once, then read or handed to addDocument 32 times: 2 MiB, over a 1 MiB limit.
    crate::script::TEST_BRIDGE_BYTES.with(|b| b.set(Some(1 << 20)));
    let mut s = Session::new();
    let set = "var p = new DocumentPreset(); p.title = new Array(65537).join('t');";
    let read = failure(&mut s, &format!("{set} for (var i = 0; i < 32; i++) p.title;"));
    let added = failure(&mut s, &format!("{set} for (var i = 0; i < 32; i++) try {{ app.documents.addDocument('Print', p, 5); }} catch (e) {{}}"));
    let within = run(&mut s, &format!("{set} for (var i = 0; i < 8; i++) p.title;"));
    crate::script::TEST_BRIDGE_BYTES.with(|b| b.set(None));
    for e in [read, added] {
        assert!(e.contains("passed more data"), "{e}");
    }
    within.unwrap();
    assert!(s.documents().is_empty());
}

#[test]
fn wrappers_go_stale_after_delete_and_undo() {
    let (mut s, _, _) = doc();
    let e = failure(&mut s, "var i = app.activeDocument.pageItems[0]; i.remove(); i.name;");
    assert!(e.contains("Stale page item"), "{e}");
    let e = failure(&mut s, "var c = app.activeDocument.pageItems[0].duplicate(); vc.run('edit.undo'); c.position;");
    assert!(e.contains("Stale page item"), "{e}");
    let e = failure(&mut s, "var p = app.activeDocument.pageItems; p.removeAll(); vc.log(p.length); p[0].name;");
    assert!(e.contains("undefined"), "{e}");
    // A named engine keeps its globals, but an object from an earlier run is stale.
    run(&mut s, "#targetengine keep\nvar held = app.activeDocument.pageItems[0];").unwrap();
    let e = failure(&mut s, "#targetengine keep\nheld.name;");
    assert!(e.contains("Stale page item"), "{e}");
}

#[test]
fn remove_all_refuses_locked_art_before_deleting_anything() {
    let (mut s, a, b) = doc();
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": true})).unwrap();
    let e = failure(&mut s, "app.activeDocument.pageItems.removeAll();");
    assert!(e.contains(&b.0.to_string()), "{e}");
    assert_eq!(s.active().unwrap().doc.layers[0].children().unwrap().len(), 2);
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": false})).unwrap();
    // A locked object inside an unlocked group stops it too.
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": true})).unwrap();
    let e = failure(&mut s, "app.activeDocument.pageItems.removeAll();");
    assert!(e.contains(&g.to_string()), "{e}");
    assert!(s.active().unwrap().doc.node(b).is_some());
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": false})).unwrap();
    let out = log(&mut s, "var p = app.activeDocument.pageItems; p.removeAll(); vc.log(p.length);");
    assert_eq!(out, vec![json!(0)]);
}

#[test]
fn remove_refuses_hidden_or_locked_art_as_remove_all_does() {
    let (mut s, a, b) = doc();
    for (on, off) in [
        (json!({"ids": [b.0], "locked": true}), json!({"ids": [b.0], "locked": false})),
        (json!({"ids": [b.0], "visible": false}), json!({"ids": [b.0], "visible": true})),
    ] {
        s.execute("object.setProps", &on).unwrap();
        let e = failure(&mut s, "app.activeDocument.pageItems[0].remove();");
        assert!(e.contains(&format!("cannot delete hidden or locked objects: {}", b.0)), "{on}: {e}");
        assert!(s.active().unwrap().doc.node(b).is_some(), "{on}");
        s.execute("object.setProps", &off).unwrap();
    }
    // A group that holds a locked object is refused too.
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": true})).unwrap();
    let e = failure(&mut s, "app.activeDocument.pageItems[0].remove();");
    assert!(e.contains(&g.to_string()), "{e}");
    assert!(s.active().unwrap().doc.node(b).is_some());
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": false})).unwrap();
    log(&mut s, "app.activeDocument.pageItems[0].remove();");
    assert!(s.active().unwrap().doc.node(b).is_none());
}

/// Art on a locked artboard stays: `remove()` and `removeAll()` throw and delete nothing.
#[test]
fn remove_and_remove_all_refuse_art_on_a_locked_artboard() {
    let (mut s, a, b) = doc();
    let lock = |s: &mut Session, locked: bool| {
        s.edit("Lock Artboard", |d, _| {
            d.artboards[0].locked = locked;
            Ok(())
        })
        .unwrap();
    };
    lock(&mut s, true);
    // An object on a second, unlocked artboard is not deleted either.
    s.execute("artboard.new", &json!({"x": 1000, "y": 0, "width": 400, "height": 300})).unwrap();
    let c = rect(&mut s, 1100.0, 100.0, 50.0, 50.0);
    let steps = undo_steps(&s);
    let e = failure(&mut s, "app.activeDocument.pageItems[1].remove();");
    assert!(e.contains(&format!("cannot delete objects on a locked artboard: {}", b.0)), "{e}");
    let e = failure(&mut s, "app.activeDocument.pageItems.removeAll();");
    assert!(e.contains("cannot delete objects on a locked artboard") && e.contains(&a.0.to_string()), "{e}");
    let out =
        log(&mut s, "try { app.activeDocument.pageItems.removeAll(); } catch (e) { vc.log(e.name); } vc.log(app.activeDocument.pageItems.length);");
    assert_eq!(out, vec![json!("Error"), json!(3)]);
    let doc = &s.active().unwrap().doc;
    assert!(doc.node(a).is_some() && doc.node(b).is_some() && doc.node(c).is_some());
    assert_eq!(undo_steps(&s), steps);
    // `editable` is false for art on the locked artboard and true for art on the unlocked one.
    let editable = "var p = app.activeDocument.pageItems; vc.log([p[0].editable, p[1].editable, p[2].editable]);";
    assert_eq!(log(&mut s, editable), vec![json!([true, false, false])]);
    lock(&mut s, false);
    assert_eq!(log(&mut s, editable), vec![json!([true, true, true])]);
    let out = log(&mut s, "var p = app.activeDocument.pageItems; p[1].remove(); p.removeAll(); vc.log(p.length);");
    assert_eq!(out, vec![json!(0)]);
}

#[test]
fn move_and_duplicate_refuse_art_on_a_locked_artboard() {
    let (mut s, a, b) = doc();
    let lock = |s: &mut Session, locked: bool| {
        s.edit("Lock Artboard", |d, _| {
            d.artboards[0].locked = locked;
            Ok(())
        })
        .unwrap();
    };
    lock(&mut s, true);
    let (order, steps) = (layer_order(&s), undo_steps(&s));
    let e = failure(&mut s, "var p = app.activeDocument.pageItems; p[1].move(p[0], ElementPlacement.PLACEBEFORE);");
    assert!(e.contains(&format!("Cannot move PathItem {}", a.0)), "{e}");
    assert_eq!((layer_order(&s), undo_steps(&s)), (order.clone(), steps));
    // `duplicate` refuses it too, with or without a relative object.
    for source in ["app.activeDocument.pageItems[1].duplicate();", "var p = app.activeDocument.pageItems; p[1].duplicate(p[0]);"] {
        let e = failure(&mut s, source);
        assert!(e.contains(&format!("Cannot duplicate PathItem {}", a.0)), "{source}: {e}");
    }
    assert_eq!((layer_order(&s), undo_steps(&s)), (order, steps));
    lock(&mut s, false);
    log(&mut s, "var p = app.activeDocument.pageItems; p[1].move(p[0], ElementPlacement.PLACEBEFORE);");
    assert_eq!(layer_order(&s), [b, a]);
}

#[test]
fn duplicate_refuses_hidden_or_locked_art() {
    let (mut s, a, b) = doc();
    for (on, off) in [
        (json!({"ids": [b.0], "locked": true}), json!({"ids": [b.0], "locked": false})),
        (json!({"ids": [b.0], "visible": false}), json!({"ids": [b.0], "visible": true})),
    ] {
        s.execute("object.setProps", &on).unwrap();
        let (order, steps) = (layer_order(&s), undo_steps(&s));
        // With no argument and with a relative object.
        for source in ["app.activeDocument.pageItems[0].duplicate();", "var p = app.activeDocument.pageItems; p[0].duplicate(p[1]);"] {
            let e = failure(&mut s, source);
            assert!(e.contains(&format!("Cannot duplicate PathItem {}", b.0)), "{on} {source}: {e}");
        }
        assert_eq!((layer_order(&s), undo_steps(&s)), (order, steps), "{on}");
        s.execute("object.setProps", &off).unwrap();
    }
    log(&mut s, "app.activeDocument.pageItems[0].duplicate();");
    let order = layer_order(&s);
    assert_eq!(order.len(), 3);
    assert!(order.contains(&a) && order.contains(&b));
}

/// Reading `editable` on every member of a group on a locked artboard measures the group once.
#[test]
fn editable_measures_a_group_on_a_locked_artboard_once() {
    let (mut s, a, b) = doc();
    let mut ids = vec![a.0, b.0];
    ids.extend((0..20).map(|i| rect(&mut s, 10.0 + f64::from(i), 200.0, 5.0, 5.0).0));
    s.execute("select.set", &json!({"ids": ids})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.edit("Lock Artboard", |d, _| {
        d.artboards[0].locked = true;
        Ok(())
    })
    .unwrap();
    let before = vectorcraft_doc::locked_artboard_measures();
    let out = log(&mut s, "var p = app.activeDocument.pageItems, n = 0; for (var i = 0; i < p.length; i++) if (!p[i].editable) n++; vc.log(n);");
    assert_eq!(out, vec![json!(23)], "the group and its 22 members are on the locked artboard");
    assert_eq!(vectorcraft_doc::locked_artboard_measures() - before, 1);
}

#[test]
fn get_is_file_open_cannot_probe_outside_script_scope() {
    let (mut s, _, _) = doc();
    let path = std::env::temp_dir().join(format!("vc-dom-open-{}.svg", std::process::id()));
    s.execute("document.save", &json!({"path": path.to_string_lossy()})).unwrap();
    let p = path.to_string_lossy().replace('\\', "\\\\");
    let query = format!("vc.log(app.getIsFileOpen('{p}')); vc.log(app.getIsFileOpen('{p}.other'));");
    let out = log(&mut s, &query);
    assert_eq!(out, vec![json!(true), json!(false)]);
    // Read roots that leave out the document's file: the same path does not count.
    let elsewhere = std::env::temp_dir().join(format!("vc-dom-roots-{}", std::process::id()));
    std::fs::create_dir_all(&elsewhere).unwrap();
    let roots = crate::file_access::AutomationRoots::new(&[&elsewhere], &[]).unwrap().unwrap();
    let out = crate::file_access::confine(Some(&roots), || log(&mut s, &query));
    assert_eq!(out, vec![json!(false), json!(false)]);
    let _ = std::fs::remove_dir_all(&elsewhere);
    for bad in ["../x.svg", "a/../b.svg", "\\\\\\\\host\\\\share\\\\x.svg", "//host/share/x.svg", "a\\u0000b", ""] {
        assert!(failure(&mut s, &format!("app.getIsFileOpen('{bad}');")).contains("cannot check"), "{bad}");
    }
    assert!(failure(&mut s, "app.getIsFileOpen(new Array(5000).join('a'));").contains("too long"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn a_menu_command_runs_its_command_on_the_selection() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let g = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let steps = undo_steps(&s);
    let out = log(&mut s, "app.executeMenuCommand('ungroup'); vc.log(app.activeDocument.selection.length);");
    assert_eq!(out, vec![json!(2)]);
    let st = s.active().unwrap();
    assert!(st.doc.node(g).is_none(), "the group is gone");
    assert!(st.doc.node(a).is_some() && st.doc.node(b).is_some());
    assert_eq!(undo_steps(&s), steps + 1, "one undo step for the run");
    // With nothing selected the menu item does nothing and adds no undo step.
    s.execute("select.none", &json!({})).unwrap();
    let steps = undo_steps(&s);
    let before = s.active().unwrap().doc.clone();
    log(&mut s, "app.executeMenuCommand('ungroup'); app.executeMenuCommand('releaseMask');");
    assert_eq!(undo_steps(&s), steps);
    assert!(std::sync::Arc::ptr_eq(&before, &s.active().unwrap().doc), "nothing changed");
    // selectall then group makes one group of both rectangles.
    let out = log(&mut s, "app.executeMenuCommand('selectall'); app.executeMenuCommand('group'); vc.log(app.activeDocument.selection[0].typename);");
    assert_eq!(out, vec![json!("GroupItem")]);
}

#[test]
fn the_fit_to_artwork_menu_command_fits_the_artboard_to_the_art() {
    let (mut s, _, _) = doc();
    let before = s.active().unwrap().doc.artboards[0].rect;
    log(&mut s, "app.executeMenuCommand('Fit Artboard to artwork bounds');");
    let after = s.active().unwrap().doc.artboards[0].rect;
    assert!(after.width() < before.width() && after.height() < before.height(), "{before:?} -> {after:?}");
    // An artboard that is locked, or has no art on it, leaves the menu item dimmed: it does nothing.
    let (mut s, _, _) = doc();
    s.execute("artboard.setProps", &json!({"index": 0, "locked": true})).unwrap();
    let steps = undo_steps(&s);
    assert_eq!(log(&mut s, "app.executeMenuCommand('Fit Artboard to artwork bounds'); vc.log(1);"), vec![json!(1)]);
    assert_eq!(s.active().unwrap().doc.artboards[0].rect, before);
    assert_eq!(undo_steps(&s), steps, "locked");
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let empty = s.active().unwrap().doc.artboards[0].rect;
    assert_eq!(log(&mut s, "app.executeMenuCommand('Fit Artboard to artwork bounds'); vc.log(1);"), vec![json!(1)]);
    assert_eq!(s.active().unwrap().doc.artboards[0].rect, empty);
    assert_eq!(undo_steps(&s), 0, "no art");
}

#[test]
fn an_unknown_menu_command_throws_and_names_it() {
    let (mut s, a, _) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let steps = undo_steps(&s);
    let e = failure(&mut s, "app.executeMenuCommand('acmeNoSuchMenu');");
    assert!(e.contains("acmeNoSuchMenu"), "{e}");
    assert!(failure(&mut s, "app.executeMenuCommand('Ungroup');").contains("Ungroup"), "names are matched exactly");
    assert!(failure(&mut s, "app.executeMenuCommand(5);").contains("string"));
    assert!(failure(&mut s, "app.executeMenuCommand();").contains("string"));
    assert!(failure(&mut s, "app.executeMenuCommand(new Array(5000).join('a'));").contains("too long"));
    assert_eq!(undo_steps(&s), steps, "nothing changed");
    // With no document open a known menu item does nothing, and an unknown one still throws.
    let mut s = Session::new();
    assert_eq!(log(&mut s, "app.executeMenuCommand('selectall'); vc.log(app.documents.length);"), vec![json!(0)]);
    assert!(failure(&mut s, "app.executeMenuCommand('acmeNoSuchMenu');").contains("acmeNoSuchMenu"));
}

#[test]
fn hostile_dom_values_do_not_reenter_or_exceed_limits() {
    let (mut s, _, _) = doc();
    let before = s.active().unwrap().selection.objects.clone();
    // A plain object shaped like an object of the model is not one.
    let e = failure(&mut s, "var f = {}; f['\\u0000hostRef'] = ['PathItem', 0, 1, 1]; app.activeDocument.selection = f;");
    assert!(e.contains("not plain data"), "{e}");
    // A getter in a value is refused before it runs.
    let out = log(
        &mut s,
        "var hit = 0, v = {}; Object.defineProperty(v, 0, {get: function () { hit++; return 1; }, enumerable: true});\
        try { app.activeDocument.rulerOrigin = v; } catch (e) { vc.log(e.message); } vc.log(hit);",
    );
    assert!(out[0].as_str().is_some_and(|m| m.contains("rulerOrigin")), "{out:?}");
    assert_eq!(out[1], json!(0));
    let e = failure(&mut s, "var a = []; for (var i = 0; i < 10001; i++) a.push(app.activeDocument.pageItems[0]); app.activeDocument.selection = a;");
    assert!(e.contains("at most 10000"), "{e}");
    assert_eq!(s.active().unwrap().selection.objects, before);
}

#[test]
fn prototype_changes_cannot_forge_a_page_item() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var f = Object.create(PathItem.prototype); var m = '';\
         try { f.name = 'x'; } catch (e) { m = e.message; } vc.log(m);\
         try { PathItem.prototype.name = 'y'; } catch (e) { m = e.message; } vc.log(m); PageItem = null; Document.prototype.pageItems = null;\
         vc.log(Object.isFrozen(PathItem.prototype) || !Object.isExtensible(PathItem.prototype));\
         vc.log(app.activeDocument.pageItems.length);\
         try { new PathItem(); } catch (e) { m = e.name; } vc.log(m);",
    );
    assert_eq!(out, vec![json!("this is not a page item"), json!("this is not a page item"), json!(true), json!(2), json!("TypeError")]);
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().name, None);
}

#[test]
fn unsupported_members_say_so() {
    let (mut s, _, _) = doc();
    assert!(failure(&mut s, "app.activeDocument.layers;").contains("Document.layers is not available yet"));
}

/// A script that names each visible selected object after its place in `app.selection`, testing
/// the members it uses with `hasOwnProperty` and `propertyIsEnumerable` first. A member that is not
/// available yet, such as `blendingMode`, is read only when `hasOwnProperty` reports it, so it is
/// never read.
const NAME_PICKED: &str = "\
    var picked = app.selection, k = picked.length, named = 0;\
    if (!k) alert('Select something first.');\
    while (k--) {\
        var obj = picked[k];\
        var usable = obj.propertyIsEnumerable('name') && obj.hasOwnProperty('hidden') && !obj.hidden;\
        if (obj.hasOwnProperty('blendingMode') && obj.blendingMode) usable = false;\
        if (usable) { obj.name = 'picked ' + k; named += 1; }\
    }\
    if (picked.length && named < picked.length) alert('Some objects were skipped.');";

#[test]
fn a_script_that_checks_has_own_property_changes_every_kind_of_selected_object() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 600, "height": 400})).unwrap();
    let path = rect(&mut s, 10.0, 10.0, 20.0, 20.0);
    let (g1, g2) = (rect(&mut s, 50.0, 10.0, 20.0, 20.0), rect(&mut s, 80.0, 10.0, 20.0, 20.0));
    s.execute("select.set", &json!({"ids": [g1.0, g2.0]})).unwrap();
    let group = NodeId(s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap());
    let text = NodeId(s.execute("text.create", &json!({"x": 10, "y": 100, "text": "hi"})).unwrap()["id"].as_u64().unwrap());
    let (c1, c2) = (rect(&mut s, 150.0, 10.0, 20.0, 20.0), rect(&mut s, 155.0, 15.0, 20.0, 20.0));
    s.execute("select.set", &json!({"ids": [c1.0, c2.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    let compound = s.active().unwrap().selection.objects[0];
    let objects = [path, group, text, compound];
    s.execute("select.set", &json!({"ids": objects.iter().map(|n| n.0).collect::<Vec<_>>()})).unwrap();
    let names = |s: &Session| -> Vec<Option<String>> {
        let d = &s.active().unwrap().doc;
        objects.iter().map(|n| d.node(*n).unwrap().name.clone()).collect()
    };
    let before = names(&s);
    let steps = undo_steps(&s);
    // The run shows no alert: without an answer for one, it would stop there.
    let r = run(&mut s, NAME_PICKED).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(r["dialogs"], json!([]), "{r}");
    let after = names(&s);
    assert!(after.iter().all(|n| n.as_deref().is_some_and(|n| n.starts_with("picked "))), "every kind was named: {after:?}");
    assert_eq!(after.iter().collect::<std::collections::HashSet<_>>().len(), objects.len(), "each got its own name: {after:?}");
    assert_eq!(undo_steps(&s), steps + 1, "the run is one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(names(&s), before);
    // Opacity reads and writes on every kind too: each object goes to its own value, read back from the script.
    s.execute("select.set", &json!({"ids": objects.iter().map(|n| n.0).collect::<Vec<_>>()})).unwrap();
    let opacities = |s: &Session| -> Vec<f32> {
        let d = &s.active().unwrap().doc;
        objects.iter().map(|n| d.node(*n).unwrap().opacity).collect()
    };
    let before = opacities(&s);
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var sel = app.activeDocument.selection, read = [];\
         for (var i = 0; i < sel.length; i++) { sel[i].opacity = 20 + 10 * i; read.push(sel[i].opacity); }\
         vc.log(read);",
    );
    assert_eq!(out, vec![json!([20, 30, 40, 50])]);
    let after = opacities(&s);
    assert!(after.iter().zip(&before).all(|(a, b)| a < b), "every kind got a lower opacity: {before:?} -> {after:?}");
    assert_eq!(undo_steps(&s), steps + 1, "the run is one undo step");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(opacities(&s), before);
}

#[test]
fn the_selection_is_an_empty_list_when_nothing_is_selected() {
    let (mut s, _, _) = doc();
    s.execute("select.set", &json!({"ids": []})).unwrap();
    let out = log(&mut s, "var d = app.activeDocument; vc.log([d.selection instanceof Array, d.selection.length, app.selection.length]);");
    assert_eq!(out, vec![json!([true, 0, 0])]);
    let steps = undo_steps(&s);
    let r = s.execute("script.run", &json!({"source": NAME_PICKED, "name": "t.js", "answers": [{"kind": "alert", "value": "ok"}]})).unwrap();
    assert_eq!(r["dialogs"][0]["message"], json!("Select something first."), "{r}");
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn page_items_report_their_available_members_as_their_own() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems[0], d = app.activeDocument, keys = [];\
         for (var k in p) keys.push(k);\
         vc.log([p.hasOwnProperty('opacity'), p.hasOwnProperty('typename'), p.hasOwnProperty('duplicate'),\
           Object.prototype.hasOwnProperty.call(p, 'name'), p.propertyIsEnumerable('opacity'), d.hasOwnProperty('selection'),\
           app.hasOwnProperty('activeDocument')]);\
         vc.log([p.hasOwnProperty('blendingMode'), p.hasOwnProperty('rotate'), d.hasOwnProperty('layers'),\
           p.hasOwnProperty('nosuch'), p.hasOwnProperty('hasOwnProperty'), p.hasOwnProperty('constructor')]);\
         vc.log([keys.indexOf('opacity') >= 0, keys.indexOf('typename') >= 0, keys.indexOf('duplicate'), keys.indexOf('blendingMode')]);",
    );
    assert_eq!(out[0], json!([true, true, true, true, true, true, true]));
    // Members that are not available yet, unknown names and inherited ones are not the object's own.
    assert_eq!(out[1], json!([false, false, false, false, false, false]));
    // `for`-`in` lists the properties, not the methods or the members that are not available yet.
    assert_eq!(out[2], json!([true, true, -1, -1]));
}

/// A name a page item class's prototype has, such as `constructor`, is not an object's own, and a
/// script cannot put a name on the prototype.
#[test]
fn a_property_added_to_a_class_prototype_is_not_an_objects_own() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems[0]; PathItem.prototype.foo = 1; PageItem.prototype.bar = 2;\
         vc.log([PathItem.prototype.foo === undefined, PageItem.prototype.bar === undefined, 'foo' in p]);\
         vc.log([p.hasOwnProperty('foo'), p.hasOwnProperty('bar'), p.propertyIsEnumerable('foo'), p.hasOwnProperty('constructor')]);",
    );
    assert_eq!(out, vec![json!([true, true, false]), json!([false, false, false, false])]);
}

/// Reading every property `for`-`in` lists calls each getter, and each call counts against the run:
/// a loop of such reads ends at `maxFuel` and at `script.stop`.
#[test]
fn reading_every_listed_property_is_charged_and_can_be_stopped() {
    let (mut s, _, _) = doc();
    let mut calls = |body: &str| {
        let source = format!("var p = app.activeDocument.pageItems[0], n = 0; for (var k in p) {{ {body} n++; }} vc.log(n);");
        let r = run(&mut s, &source).unwrap_or_else(|e| panic!("{e}"));
        (r["hostCalls"].as_u64().unwrap(), r["log"][0].as_u64().unwrap())
    };
    let (listed, n) = calls("");
    let (read, m) = calls("p[k];");
    assert_eq!(n, m);
    assert!(n > 10, "{n} properties");
    assert!(read >= listed + n, "{listed} host calls to list, {read} to read {n} properties");
    let spin = "var p = app.activeDocument.pageItems[0]; while (true) for (var k in p) p[k];";
    let e = s.execute("script.run", &json!({"source": spin, "name": "t.js", "maxFuel": 200_000})).unwrap_err();
    assert!(e.to_string().ends_with("(limit)"), "{e}");
    s.script_frames = true;
    assert_eq!(run(&mut s, spin).unwrap()["running"], true);
    assert!(s.script_pump().is_none());
    assert_eq!(s.execute("script.stop", &json!({})).unwrap()["stopped"], true);
    assert!(s.script_pump().unwrap().is_err());
    assert!(!s.script_running());
}

/// `pageItems` lists art topmost first, and the selection lists the selected objects in that
/// order whatever order they were selected or made in.
#[test]
fn page_items_and_the_selection_list_the_topmost_object_first() {
    let (mut s, a, b) = doc();
    s.execute("object.setProps", &json!({"ids": [a.0], "name": "bottom"})).unwrap();
    s.execute("object.setProps", &json!({"ids": [b.0], "name": "top"})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("select.add", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(s.active().unwrap().selection.objects, vec![a, b], "picked bottom first");
    let out = log(
        &mut s,
        "var d = app.activeDocument; vc.log([d.pageItems[0].name, d.pageItems[1].name, d.selection[0].name, d.selection[1].name]);\
         d.selection = [d.pageItems[1], d.pageItems[0]]; vc.log(d.selection[0].name);",
    );
    assert_eq!(out, vec![json!(["top", "bottom", "top", "bottom"]), json!("top")]);
    // An object made last and sent to the back comes after the others it is selected with.
    let c = rect(&mut s, 200.0, 200.0, 20.0, 20.0);
    s.execute("object.setProps", &json!({"ids": [c.0], "name": "last"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, c = d.pageItems.getByName('last'); c.zOrder(ZOrderMethod.SENDTOBACK);\
         d.selection = null; c.selected = true; d.pageItems.getByName('top').selected = true;\
         vc.log([d.selection[0].name, d.selection[1].name, d.pageItems[2].name]);",
    );
    assert_eq!(out, vec![json!(["top", "last", "last"])]);
}

/// The selection lists a group before the objects in it, in `pageItems`' order, and reading it
/// costs as much in a large document as in a small one.
#[test]
fn the_selection_follows_page_items_and_costs_the_same_in_a_large_document() {
    let (mut s, a, b) = doc();
    let c = rect(&mut s, 200.0, 200.0, 20.0, 20.0);
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    let group = s.execute("object.group", &json!({})).unwrap()["id"].as_u64().unwrap();
    s.execute("select.set", &json!({"ids": [a.0, c.0, group]})).unwrap();
    let source = "var d = app.activeDocument, want = [], got = [];\
                  for (var i = 0; i < d.pageItems.length; i++) if (d.pageItems[i].selected) want.push(d.pageItems[i].typename + i);\
                  for (var i = 0; i < d.pageItems.length; i++) for (var j = 0; j < d.selection.length; j++)\
                    if (d.selection[j] === d.pageItems[i]) got[j] = d.pageItems[i].typename + i;\
                  vc.log([want, got]);";
    let out = log(&mut s, source);
    assert_eq!(out[0][0], out[0][1], "the selection is in pageItems' order");
    assert_eq!(out[0][0].as_array().map(Vec::len), Some(3));
    // Each pass edits the document, so nothing kept from the last read can be reused.
    let edits = "var d = app.activeDocument; for (var i = 0; i < 50; i++) d.selection[0].opacity = 50 + i % 2;";
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    let small = run(&mut s, edits).unwrap()["fuelUsed"].as_u64().unwrap();
    // 3 objects doubled 11 times: 6,144.
    s.execute("select.all", &json!({})).unwrap();
    for _ in 0..11 {
        s.execute("edit.duplicate", &json!({})).unwrap();
        s.execute("select.all", &json!({})).unwrap();
    }
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    crate::script::TEST_BRIDGE_BYTES.with(|b| b.set(Some(1 << 16)));
    let large = run(&mut s, edits);
    crate::script::TEST_BRIDGE_BYTES.with(|b| b.set(None));
    let large = large.unwrap_or_else(|e| panic!("the reads in a large document went over: {e}"))["fuelUsed"].as_u64().unwrap();
    assert_eq!(small, large);
}

/// A collection read with a name gives what `getByName` gives, and throws as it does for no match.
#[test]
fn a_collection_read_with_a_name_gives_the_named_object() {
    let (mut s, a, _) = doc();
    s.execute("object.setProps", &json!({"ids": [a.0], "name": "n"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pageItems, m = [];\
         vc.log([p['n'] === p.getByName('n'), p['n'].name, 'n' in p, 'nosuch' in p, p['length'] === p.length]);\
         vc.log([app.documents[d.name] === d, d.name in app.documents, 'nosuch' in app.documents]);\
         try { p['nosuch']; } catch (e) { m.push(e.message); }\
         try { app.documents.nosuch; } catch (e) { m.push(e.message); }\
         vc.log(m);",
    );
    assert_eq!(out, vec![json!([true, "n", true, false, true]), json!([true, true, false]), json!(["Not found", "Not found"])]);
}

/// `opacity`, `hidden`, `locked` and the position and size setters on an object in a hidden or
/// locked layer throw and the run changes nothing, while its name and note still change; an object
/// that is hidden or locked itself takes them.
#[test]
fn setters_refuse_art_in_a_hidden_or_locked_layer() {
    let (mut s, a, b) = doc();
    let layer = NodeId(s.execute("layer.new", &json!({})).unwrap()["id"].as_u64().unwrap());
    let c = rect(&mut s, 200.0, 200.0, 20.0, 20.0);
    rect(&mut s, 300.0, 200.0, 20.0, 20.0);
    for (id, name) in [(a, "locked"), (b, "free"), (c, "closed")] {
        s.execute("object.setProps", &json!({"ids": [id.0], "name": name})).unwrap();
    }
    for (on, off) in [
        (json!({"ids": [layer.0], "visible": false}), json!({"ids": [layer.0], "visible": true})),
        (json!({"ids": [layer.0], "locked": true}), json!({"ids": [layer.0], "locked": false})),
    ] {
        s.execute("object.setProps", &on).unwrap();
        // `hidden` and `locked` are refused even when the value is the one the object has.
        for set in [
            "opacity = 50",
            "hidden = true",
            "hidden = false",
            "locked = true",
            "locked = false",
            "left = 5",
            "top = 5",
            "position = [5, 5]",
            "width = 10",
            "height = 10",
            "zOrder(ZOrderMethod.BRINGTOFRONT)",
        ] {
            let (before, steps) = (format!("{:?}", s.active().unwrap().doc.layers), undo_steps(&s));
            // The change made before the refusal is put back with the rest of the run.
            let e = failure(&mut s, &format!("var p = app.activeDocument.pageItems; p.getByName('free').opacity = 20; p.getByName('closed').{set};"));
            assert!(e.contains(&format!("Cannot edit PathItem {}", c.0)), "{on} {set}: {e}");
            assert_eq!(format!("{:?}", s.active().unwrap().doc.layers), before, "{on} {set}");
            assert_eq!(undo_steps(&s), steps, "{on} {set}");
        }
        let out = log(
            &mut s,
            "var c = app.activeDocument.pageItems.getByName('closed'); c.note = 'n'; c.name = 'renamed'; c.opacity = c.opacity;\
             vc.log([c.name, c.note, c.opacity]); c.name = 'closed'; c.note = '';",
        );
        assert_eq!(out, vec![json!(["renamed", "n", 100])], "{on}");
        s.execute("object.setProps", &off).unwrap();
    }
    s.execute("object.setProps", &json!({"ids": [a.0], "locked": true})).unwrap();
    s.execute("object.setProps", &json!({"ids": [b.0], "visible": false})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems; p.getByName('locked').opacity = 50; p.getByName('free').opacity = 40;\
         vc.log([p.getByName('locked').opacity, p.getByName('free').opacity]);",
    );
    assert_eq!(out, vec![json!([50, 40])]);
}

/// `name` takes a number or a Boolean as the text `String` gives it, and nothing else.
#[test]
fn a_name_given_as_a_number_or_a_boolean_becomes_its_text() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var i = app.activeDocument.pageItems[1]; i.name = 5; vc.log([typeof i.name, i.name]); i.name = 2.5e-7; vc.log(i.name);\
         i.name = true; vc.log(i.name);",
    );
    assert_eq!(out, vec![json!(["string", "5"]), json!("2.5e-7"), json!("true")]);
    assert_eq!(s.active().unwrap().doc.node(a).unwrap().name.as_deref(), Some("true"));
    for value in ["null", "undefined", "{}"] {
        assert!(failure(&mut s, &format!("app.activeDocument.pageItems[1].name = {value};")).contains("name must be a string"), "{value}");
    }
}

/// `String` of an object of the object model names its class, and its name when it has one.
#[test]
fn objects_of_the_object_model_print_their_class_and_name() {
    let (mut s, a, _) = doc();
    s.execute("object.setProps", &json!({"ids": [a.0], "name": "Logo"})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pageItems;\
         vc.log([String(p[0]), String(p[1]), '' + p[1], p[1].toString(), String(p[1].parent) === '[Layer ' + p[1].parent.name + ']']);\
         vc.log([String(d) === '[Document ' + d.name + ']', String(app), String(p), String(app.documents)]);",
    );
    assert_eq!(out[0], json!(["[PathItem ]", "[PathItem Logo]", "[PathItem Logo]", "[PathItem Logo]", true]));
    assert_eq!(out[1], json!([true, "[Application]", "[PageItems]", "[Documents]"]));
}

/// `activeDocument`, `documents` and `selection` without `app.` read the application's members, and
/// assigning one assigns the member: `selection` selects, `documents` keeps its value.
#[test]
fn the_application_members_can_be_named_without_app() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    let out = log(
        &mut s,
        "vc.log([activeDocument === app.activeDocument, documents === app.documents, documents.length,\
           selection.length, selection[0] === app.activeDocument.pageItems[1], typeof activeDocument]);",
    );
    assert_eq!(out, vec![json!([true, true, 1, 1, true, "object"])]);
    let out = log(
        &mut s,
        "var d = app.activeDocument; selection = null; vc.log(d.selection.length);\
         selection = [d.pageItems[0], d.pageItems[1]]; vc.log(d.selection.length);\
         app.selection = d.pageItems[0]; vc.log([d.selection.length, selection[0] === d.pageItems[0]]);\
         var selection = d.selection; documents = 5; activeDocument = d; vc.log([selection.length, typeof documents]);",
    );
    assert_eq!(out, vec![json!(0), json!(2), json!([1, true]), json!([1, "object"])]);
    assert_eq!(s.active().unwrap().selection.objects, vec![b]);
    // A top-level `var selection = []` clears the selection; a push goes to a list nobody keeps.
    let out = log(&mut s, "var selection = []; selection.push(app.activeDocument.pageItems[0]); vc.log(selection.length);");
    assert_eq!(out, vec![json!(0)]);
    assert!(s.active().unwrap().selection.objects.is_empty());
    assert!(failure(&mut s, "activeDocument = 5;").contains("not a Document"));
    let e = failure(&mut s, "'use strict'; documents = 5;");
    assert!(e.contains("TypeError") && e.contains("cannot assign to property 'documents': it has only a getter"), "{e}");
    // With no document open, reading `activeDocument` throws as `app.activeDocument` does.
    let mut s = Session::new();
    assert!(failure(&mut s, "activeDocument;").contains("No active document"));
}

/// Assigning to a name a collection does not have throws as reading it does, and assigning an
/// index does not replace the object there.
#[test]
fn assigning_a_name_a_collection_lacks_throws() {
    let (mut s, _, _) = doc();
    let steps = undo_steps(&s);
    for source in ["app.activeDocument.pageItems.qux = 4;", "app.documents.qux = 4;"] {
        assert!(failure(&mut s, source).contains("Not found"), "{source}");
    }
    let out = log(&mut s, "var p = app.activeDocument.pageItems; p[0] = 3; vc.log(typeof p[0]);");
    assert_eq!(out, vec![json!("object")]);
    // On a page item the value is not kept; strict-mode code throws instead.
    let out = log(&mut s, "var i = app.activeDocument.pageItems[0]; i.qux = 4; vc.log(typeof i.qux);");
    assert_eq!(out, vec![json!("undefined")]);
    let e = failure(&mut s, "'use strict'; app.activeDocument.pageItems[0].qux = 4;");
    assert!(e.contains("TypeError") && e.contains("qux"), "{e}");
    assert_eq!(undo_steps(&s), steps);
}

/// `JSON.stringify` writes a collection, or data that holds one, without reading a `toJSON` the
/// collection lacks, while a script that reads `toJSON` itself gets `Not found`.
#[test]
fn json_stringify_writes_a_collection() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, m = ''; vc.log([JSON.stringify(p), JSON.stringify({list: [p, app.documents]})]);\
         try { p.toJSON; } catch (e) { m = e.message; } vc.log(m);",
    );
    assert_eq!(out, vec![json!(["{}", "{\"list\":[{},{}]}"]), json!("Not found")]);
}
