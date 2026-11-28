//! The script object model's paths, path points and colors.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{LineCap, LineJoin};
use vectorcraft_geom::path::AnchorKind;

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
    // `pathItems` lists the topmost path first, so `a`, the first one a script reads, is made last.
    let b = rect(&mut s, 100.0, 100.0, 50.0, 50.0);
    let a = rect(&mut s, 10.0, 20.0, 30.0, 40.0);
    s.execute("select.none", &json!({})).unwrap();
    (s, a, b)
}

fn undo_steps(s: &Session) -> usize {
    s.active().unwrap().history.undo.len()
}

/// The layers of the active document, to show that a failed run changed nothing.
fn art(s: &Session) -> String {
    format!("{:?}", s.active().unwrap().doc.layers)
}

fn node(s: &Session, id: NodeId) -> crate::doc::Node {
    s.active().unwrap().doc.node(id).unwrap().clone()
}

/// Each number of `v` rounded to 3 decimals, in place.
fn rounded(v: &Value) -> Value {
    match v {
        Value::Number(n) => {
            let r = (n.as_f64().unwrap() * 1000.0).round() / 1000.0;
            if r.fract() == 0.0 { json!(r as i64) } else { json!(r) }
        }
        Value::Array(a) => Value::Array(a.iter().map(rounded).collect()),
        v => v.clone(),
    }
}

#[test]
fn path_items_are_live_and_list_only_paths() {
    let (mut s, a, b) = doc();
    s.execute("object.setProps", &json!({"ids": [a.0], "name": "first"})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 250, "text": "hi"})).unwrap();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pathItems;\
         vc.log(p.length); vc.log(d.pageItems.length); vc.log(p.typename); vc.log(p.parent.typename);\
         vc.log(p[0].typename); vc.log(p[1].parent.typename); vc.log(p.getByName('first').name);\
         vc.log(p[2] === undefined && p[-1] === undefined);\
         var r = p.rectangle(100, 10, 5, 5); vc.log(p.length); vc.log(p[2].typename);",
    );
    let expected = [json!(2), json!(4), json!("PathItems"), json!("Document"), json!("PathItem"), json!("GroupItem"), json!("first"), json!(true)];
    assert_eq!(out[..8], expected);
    assert_eq!(out[8..], [json!(3), json!("PathItem")]);
    assert!(failure(&mut s, "app.activeDocument.pathItems.getByName('none');").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.pathItems.index('none');").contains("Not found"));
}

#[test]
fn path_items_remove_all_refuses_locked_paths_and_changes_nothing() {
    let (mut s, a, _) = doc();
    s.execute("object.setProps", &json!({"ids": [a.0], "locked": true})).unwrap();
    let before = art(&s);
    let e = failure(&mut s, "app.activeDocument.pathItems.removeAll();");
    assert!(e.contains("hidden or locked paths"), "{e}");
    assert_eq!(art(&s), before);
    s.execute("object.setProps", &json!({"ids": [a.0], "locked": false})).unwrap();
    log(&mut s, "app.activeDocument.pathItems.removeAll();");
    assert_eq!(log(&mut s, "vc.log(app.activeDocument.pathItems.length);"), vec![json!(0)]);
}

#[test]
fn shape_creators_use_documented_defaults_and_y_up() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems;\
         vc.log(p.rectangle(250, 10, 30, 40).geometricBounds);\
         vc.log(p.ellipse().geometricBounds);\
         var g = p.polygon(); vc.log(g.pathPoints.length); var b = g.geometricBounds; vc.log([(b[0] + b[2]) / 2, b[1]]);\
         vc.log(p.star().pathPoints.length); vc.log(p.star(0, 0, 10, 5, 7).pathPoints.length);\
         var r = p.roundedRectangle(250, 10, 100, 60); vc.log(r.pathPoints.length); vc.log(r.geometricBounds); vc.log(r.closed);\
         vc.log(p.roundedRectangle(250, 10, 100, 60, 0, 0).pathPoints.length);\
         vc.log(p.length);",
    );
    let out: Vec<Value> = out.iter().map(rounded).collect();
    assert_eq!(out[0], json!([10, 250, 40, 210]));
    assert_eq!(out[1], json!([100, 100, 150, 0]));
    assert_eq!(out[2], json!(8));
    assert_eq!(out[3], json!([200, 350]), "the polygon is centered on (200, 300) and reaches 50 pt up");
    assert_eq!(out[4], json!(10));
    assert_eq!(out[5], json!(14));
    assert_eq!(out[6], json!(8));
    assert_eq!(out[7], json!([10, 250, 110, 190]));
    assert_eq!(out[8], json!(true));
    assert_eq!(out[9], json!(4), "corners of radius 0 leave four points");
    assert_eq!(out[10], json!(7));
    assert_eq!(undo_steps(&s), steps + 1, "one undo step for the run");
    assert!(s.active().unwrap().selection.objects.is_empty(), "the selection stays as it was");
}

#[test]
fn shape_creators_reject_bad_sizes_without_change() {
    let (mut s, _, _) = doc();
    let before = art(&s);
    let steps = undo_steps(&s);
    for (source, part) in [
        ("app.activeDocument.pathItems.rectangle(10, 10, -5, 10);", "width"),
        ("app.activeDocument.pathItems.rectangle(10, 10, 5);", "height"),
        ("app.activeDocument.pathItems.rectangle();", "top"),
        ("app.activeDocument.pathItems.ellipse(0, 0, NaN);", "finite"),
        ("app.activeDocument.pathItems.polygon(0, 0, 10, 2);", "sides"),
        ("app.activeDocument.pathItems.polygon(0, 0, 10, 5.5);", "sides"),
        ("app.activeDocument.pathItems.star(0, 0, 10, 5, 1001);", "points"),
        ("app.activeDocument.pathItems.star(0, 0, 10, -5);", "innerRadius"),
        ("app.activeDocument.pathItems.roundedRectangle(0, 0, 10, 10, -1);", "horizontalRadius"),
        ("app.activeDocument.pathItems.rectangle(0, 0, 10, 10, 'yes');", "reversed"),
        ("app.activeDocument.pathItems.rectangle(1e12, 0, 10, 10);", "outside the canvas"),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(art(&s), before);
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn set_entire_path_replaces_points_and_keeps_closed() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0];\
         p.setEntirePath([[0, 0], [10, 0], [10, 10]]); vc.log(p.pathPoints.length); vc.log(p.closed); vc.log(p.pathPoints[2].anchor);\
         var n = app.activeDocument.pathItems.add(); vc.log(n.pathPoints.length); vc.log(n.closed);\
         n.setEntirePath([[50, 50], [60, 50]]); vc.log(n.pathPoints.length); vc.log(n.closed); vc.log(n.length);\
         n.closed = true; vc.log(n.closed);",
    );
    assert_eq!(out, vec![json!(3), json!(true), json!([10, 10]), json!(0), json!(false), json!(2), json!(false), json!(10), json!(true)]);
    let n = node(&s, a);
    let pd = n.path_data().unwrap();
    assert_eq!(pd.subpaths.len(), 1);
    assert!(pd.subpaths[0].closed);
    assert!(pd.subpaths[0].anchors.iter().all(|an| an.h_in == an.p && an.h_out == an.p && an.kind == AnchorKind::Corner));
}

#[test]
fn hostile_set_entire_path_input_is_bounded() {
    let (mut s, _, _) = doc();
    let before = art(&s);
    for (source, part) in [
        ("var a = []; for (var i = 0; i < 100001; i++) a.push([i % 100, 1]); app.activeDocument.pathItems[0].setEntirePath(a);", "at most"),
        ("app.activeDocument.pathItems[0].setEntirePath([[0, 0], [NaN, 1]]);", "finite"),
        ("app.activeDocument.pathItems[0].setEntirePath([[0, 0], [1]]);", "two numbers"),
        ("app.activeDocument.pathItems[0].setEntirePath('0,0');", "array"),
        ("app.activeDocument.pathItems[0].setEntirePath([[0, 1e13]]);", "outside the canvas"),
        (
            "var o = {}; Object.defineProperty(o, 0, {get: function () { return 1; }}); app.activeDocument.pathItems[0].setEntirePath([o]);",
            "two numbers",
        ),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(art(&s), before);
}

#[test]
fn path_point_setters_edit_one_anchor() {
    let (mut s, a, _) = doc();
    let anchors_before = node(&s, a).path_data().unwrap().subpaths[0].anchors.to_vec();
    let out = log(
        &mut s,
        "var pp = app.activeDocument.pathItems[0].pathPoints, q = pp[1];\
         vc.log(pp.length); vc.log(pp.typename); vc.log(q.typename); vc.log(q.parent.typename); vc.log(q.anchor);\
         q.anchor = [45, 285]; vc.log(q.anchor); vc.log(q.leftDirection); vc.log(q.rightDirection);\
         q.rightDirection = [55, 285]; q.leftDirection = [35, 285]; vc.log(q.pointType);\
         q.pointType = PointType.SMOOTH; vc.log(q.pointType); q.pointType = PointType.CORNER; vc.log(q.pointType);\
         vc.log(pp.index(1).anchor);",
    );
    assert_eq!(out[0], json!(4));
    assert_eq!(out[1], json!("PathPoints"));
    assert_eq!(out[2], json!("PathPoint"));
    assert_eq!(out[3], json!("PathItem"));
    assert_eq!(out[5], json!([45, 285]));
    assert_eq!(out[6], json!([40, 280]), "moving the anchor leaves its handles");
    assert_eq!(out[8], json!("PointType.CORNER"), "moving handles leaves the point type");
    assert_eq!(out[9], json!("PointType.SMOOTH"));
    assert_eq!(out[10], json!("PointType.CORNER"), "a corner with handles in line stays a corner");
    assert_eq!(out[11], json!([45, 285]));
    let anchors = node(&s, a).path_data().unwrap().subpaths[0].anchors.to_vec();
    for i in [0, 2, 3] {
        assert_eq!(anchors[i], anchors_before[i], "point {i} did not change");
    }
    assert!(failure(&mut s, "app.activeDocument.pathItems[0].pathPoints.index('a');").contains("Not found"));
    assert!(failure(&mut s, "app.activeDocument.pathItems[0].pathPoints[0].pointType = 'SMOOTH';").contains("PointType"));
    assert!(failure(&mut s, "app.activeDocument.pathItems[0].pathPoints[0].anchor = [1, 'x'];").contains("finite"));
    assert_eq!(log(&mut s, "vc.log(app.activeDocument.pathItems[0].pathPoints.getByName === undefined);"), vec![json!(true)]);
}

#[test]
fn path_point_wrapper_goes_stale_after_remove() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0], pp = p.pathPoints, last = pp[3];\
         pp[0].remove(); vc.log(pp.length); vc.log(pp[0].anchor);\
         try { last.anchor; } catch (e) { vc.log(e.message); } try { last.selected; } catch (e) { vc.log(e.message); }\
         var n = pp.add(); vc.log(pp.length); vc.log(n.anchor);\
         pp.removeAll(); vc.log(pp.length); vc.log(app.activeDocument.pathItems.length);\
         p.remove(); try { pp.length; } catch (e) { vc.log(e.message); } try { p.closed; } catch (e) { vc.log(e.message); }\
         try { p.polarity; } catch (e) { vc.log(e.message); }",
    );
    assert_eq!(
        out,
        vec![
            json!(3),
            json!([40, 280]),
            json!("Stale path point"),
            json!("Stale path point"),
            json!(4),
            json!([10, 240]),
            json!(0),
            json!(2),
            json!("Stale page item"),
            json!("Stale page item"),
            json!("Stale page item")
        ]
    );
}

#[test]
fn fill_color_round_trips_rgb_cmyk_gray_lab() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0], c = new RGBColor(); vc.log(c.typename); vc.log([c.red, c.green, c.blue]);\
         c.red = 255; c.blue = 128; p.fillColor = c; var f = p.fillColor; vc.log([f.typename, f.red, f.green, f.blue]);\
         var k = new cmykColor(); k.cyan = 10; k.black = 50; p.strokeColor = k; var t = p.strokeColor; vc.log([t.typename, t.cyan, t.black]);\
         var g = new GrayColor(); g.gray = 25; p.fillColor = g; vc.log([p.fillColor.typename, p.fillColor.gray]);\
         var l = new LabColor(); l.l = 50; l.a = -20; l.b = 30; p.strokeColor = l; vc.log([p.strokeColor.l, p.strokeColor.a, p.strokeColor.b]);",
    );
    assert_eq!(out[0], json!("RGBColor"));
    assert_eq!(out[1], json!([0, 0, 0]));
    assert_eq!(rounded(&out[2]), json!(["RGBColor", 255, 0, 128]));
    assert_eq!(rounded(&out[3]), json!(["CMYKColor", 10, 50]));
    assert_eq!(rounded(&out[4]), json!(["GrayColor", 25]));
    assert_eq!(rounded(&out[5]), json!([50, -20, 30]));
    let n = node(&s, a);
    match n.appearance.fill_paint() {
        Paint::Solid { color: Color::Gray { k }, .. } => assert!((k - 0.75).abs() < 1e-6, "gray 25 is 75% black ink, got {k}"),
        p => panic!("fill is {p:?}"),
    }
}

#[test]
fn no_color_unfills_and_unstrokes() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0]; vc.log([p.filled, p.stroked]);\
         p.fillColor = new NoColor(); p.stroked = false; vc.log([p.filled, p.stroked, p.fillColor.typename]);\
         p.filled = true; vc.log(p.filled);",
    );
    assert_eq!(out, vec![json!([true, true]), json!([false, false, "NoColor"]), json!(true)]);
    let n = node(&s, a);
    assert!(n.appearance.stroke_paint().is_none());
    assert!(!n.appearance.fill_paint().is_none());
}

#[test]
fn path_paint_edits_leave_new_art_and_recent_colors() {
    let (mut s, _, _) = doc();
    let (paint, active, last, recent) = (s.paint.clone(), s.fill_active, s.last_solid, s.recent_colors.clone());
    log(
        &mut s,
        "var p = app.activeDocument.pathItems[0], c = new RGBColor(); c.red = 200; c.green = 10; c.blue = 30;\
         p.fillColor = c; p.strokeColor = c; p.strokeWidth = 7; p.strokeCap = StrokeCap.ROUNDENDCAP; p.filled = false;",
    );
    assert_eq!(s.paint, paint, "the paint for new art");
    assert_eq!(s.fill_active, active);
    assert_eq!(s.last_solid, last);
    assert_eq!(s.recent_colors, recent);
}

#[test]
fn compound_path_members_read_the_paint_they_are_given() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.compoundPath.make", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0], c = new RGBColor(); c.red = 200; p.fillColor = c; p.strokeWidth = 4;\
         vc.log([p.fillColor.red, p.fillColor.green, p.strokeWidth]); vc.log(app.activeDocument.pathItems[1].fillColor.red);\
         p.evenodd = true; vc.log([p.evenodd, app.activeDocument.pathItems[1].evenodd]);",
    );
    assert_eq!(rounded(&out[0]), json!([200, 0, 4]));
    assert_eq!(rounded(&out[1]), json!(200), "the compound path paints every member");
    assert_eq!(out[2], json!([true, true]), "the compound path holds the fill rule");
}

#[test]
fn a_spot_color_reads_as_a_spot_color_and_keeps_its_link() {
    let (mut s, a) = doc_with_paints();
    s.execute("paint.setFill", &json!({"ids": [a.0], "swatch": "Ink", "tint": 40})).unwrap();
    let before = node(&s, a).appearance.fill_paint();
    let out = log(&mut s, "var p = app.activeDocument.pathItems[0]; vc.log([p.fillColor.typename, p.fillColor.spot.name, p.fillColor.tint]);");
    assert_eq!(rounded(&out[0]), json!(["SpotColor", "Ink", 40]));
    let steps = undo_steps(&s);
    log(&mut s, "var p = app.activeDocument.pathItems[0]; p.fillColor = p.fillColor;");
    assert_eq!(undo_steps(&s), steps, "assigning the spot color the fill has records no step");
    assert_eq!(node(&s, a).appearance.fill_paint(), before, "the fill stays linked to its swatch");
}

#[test]
fn a_spot_color_paints_its_spot_at_its_tint() {
    let (mut s, a) = doc_with_paints();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pathItems[0], c = new SpotColor(); c.spot = d.spots.getByName('Ink'); c.tint = 50;\
         p.fillColor = c; p.strokeColor = c; vc.log([p.fillColor.typename, p.fillColor.spot.name, p.fillColor.tint, p.strokeColor.tint]);",
    );
    assert_eq!(rounded(&out[0]), json!(["SpotColor", "Ink", 50, 50]));
    let n = node(&s, a);
    for paint in [n.appearance.fill_paint(), n.appearance.stroke_paint()] {
        match paint {
            Paint::Solid { swatch: Some(name), tint, .. } => assert!(name == "Ink" && (tint - 0.5).abs() < 1e-6, "{name} at {tint}"),
            p => panic!("the paint is {p:?}"),
        }
    }
}

#[test]
fn a_spot_color_left_without_a_tint_paints_at_its_default() {
    let (mut s, a) = doc_with_paints();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pathItems[0], c = new SpotColor(); c.spot = d.spots.getByName('Ink');\
         p.fillColor = c; vc.log([p.fillColor.spot.name, p.fillColor.tint]);\
         p.strokeColor = {typename: 'SpotColor', spot: d.spots.getByName('Ink')}; vc.log([p.strokeColor.spot.name, p.strokeColor.tint]);",
    );
    assert_eq!(rounded(&Value::Array(out)), json!([["Ink", 100], ["Ink", 0]]), "new SpotColor() starts at 100, a color without a tint is 0");
    let n = node(&s, a);
    for (paint, want) in [(n.appearance.fill_paint(), 1.0), (n.appearance.stroke_paint(), 0.0)] {
        match paint {
            Paint::Solid { swatch: Some(name), tint, .. } => assert!(name == "Ink" && (tint - want).abs() < 1e-6, "{name} at {tint}"),
            p => panic!("the paint is {p:?}"),
        }
    }
}

#[test]
fn a_global_process_link_read_as_a_spot_color_is_not_assigned() {
    let (mut s, a) = doc_with_paints();
    s.execute("swatch.new", &json!({"name": "Leaf", "color": "#3a9a40", "global": true})).unwrap();
    s.execute("swatch.new", &json!({"name": "Moss", "color": "#405a20", "global": true})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [a.0], "swatch": "Leaf", "tint": 60})).unwrap();
    let b = rect(&mut s, 30.0, 30.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [b.0], "swatch": "Moss", "tint": 60})).unwrap();
    let c = rect(&mut s, 60.0, 60.0, 10.0, 10.0);
    s.execute("paint.setFill", &json!({"ids": [c.0], "color": "#ff0000"})).unwrap();
    let paints = |s: &Session| [a, b, c].map(|id| node(s, id).appearance.fill_paint());
    let before = paints(&s);
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var q = app.activeDocument.pathItems; vc.log([q[2].fillColor.typename, q[2].fillColor.spot === undefined, q[1].fillColor.typename, q[0].fillColor.typename]);",
    );
    // `pathItems` lists c (unlinked), b (Moss), a (Leaf) and the document's first rectangle.
    assert_eq!(out, vec![json!(["SpotColor", true, "SpotColor", "RGBColor"])]);
    for script in [
        "var q = app.activeDocument.pathItems; q[2].fillColor = q[2].fillColor;",
        "var q = app.activeDocument.pathItems; q[1].fillColor = q[2].fillColor;",
        "var q = app.activeDocument.pathItems; q[0].fillColor = q[2].fillColor;",
    ] {
        let e = failure(&mut s, script);
        assert!(e.contains("global process swatch cannot be assigned yet"), "{script}: {e}");
        assert_eq!(paints(&s), before, "{script}");
        assert_eq!(undo_steps(&s), steps, "{script}");
    }
}

#[test]
fn a_path_painted_with_registration_reads_and_takes_its_spot() {
    let (mut s, a) = doc_with_paints();
    let out = log(
        &mut s,
        "var d = app.activeDocument, p = d.pathItems[0], c = new SpotColor(); c.spot = d.spots.getByName('[Registration]'); c.tint = 100;\
         p.strokeColor = c; vc.log([p.strokeColor.typename, p.strokeColor.spot.name, p.strokeColor.tint]);",
    );
    assert_eq!(rounded(&out[0]), json!(["SpotColor", "[Registration]", 100]));
    assert!(node(&s, a).appearance.stroke_paint().is_registration(), "the stroke is linked to Registration");
}

#[test]
fn a_spot_color_without_a_spot_of_the_document_is_refused() {
    let (mut s, _) = doc_with_paints();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    let layers = |s: &Session| s.documents().iter().map(|d| format!("{:?}", d.doc.layers)).collect::<Vec<_>>();
    let before = layers(&s);
    let steps = undo_steps(&s);
    for (source, part) in [
        ("app.documents[0].pathItems[0].fillColor = new SpotColor();", "SpotColor.spot must be a Spot; a color read"),
        ("var c = new SpotColor(); c.spot = 5; app.documents[0].pathItems[0].fillColor = c;", "SpotColor.spot must be a Spot"),
        // `app.documents` lists the active document, the new one, first: the document with `Ink` is
        // `documents[1]`.
        ("var a = app.documents[1], c = new SpotColor(); c.spot = a.swatches.getByName('Ink'); a.pathItems[0].fillColor = c;", "must be a Spot"),
        (
            "var a = app.documents[1], c = new SpotColor(); c.spot = a.spots.getByName('Ink'); c.tint = 101; a.pathItems[0].fillColor = c;",
            "SpotColor.tint",
        ),
        (
            "var a = app.documents[1], b = app.documents[0], c = new SpotColor(); c.spot = a.spots.getByName('Ink'); b.pathItems[0].fillColor = c;",
            "Cross-document",
        ),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(layers(&s), before);
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn filled_true_on_a_filled_path_runs_no_command() {
    let (mut s, _, _) = doc();
    let steps = undo_steps(&s);
    let r = run(
        &mut s,
        "var p = app.activeDocument.pathItems[0]; p.filled = true; p.stroked = true; p.fillColor = p.fillColor;\
         p.strokeWidth = p.strokeWidth; p.strokeCap = p.strokeCap; p.closed = true; p.evenodd = false; p.strokeDashes = [];\
         p.pathPoints[0].anchor = p.pathPoints[0].anchor; p.translate(0, 0); p.rotate(360); p.resize(100, 100);",
    )
    .unwrap();
    assert_eq!(undo_steps(&s), steps, "values that change nothing add no step");
    assert_eq!(r["slowestCommands"], json!([]), "and run no command");
}

#[test]
fn identity_transforms_of_an_empty_path_do_nothing() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    log(
        &mut s,
        "var n = app.activeDocument.pathItems.add(); n.rotate(360); n.rotate(0, true, true, true, true, Transformation.TOPLEFT);\
         n.resize(100, 100); n.resize(100, 100, true, true, true, true, 100, Transformation.BOTTOM); n.translate(0, 0);",
    );
    for source in [
        "app.activeDocument.pathItems[0].rotate(360, true, true, true, true, 'CENTER');",
        "app.activeDocument.pathItems[0].resize(100, 100, true, true, true, true, 100, 'x');",
    ] {
        assert!(failure(&mut s, source).contains("Transformation"), "{source}");
    }
}

/// The paint of the fill (`fill`) or stroke of `id`.
fn paint_of(s: &Session, id: NodeId, fill: bool) -> Paint {
    let n = node(s, id);
    if fill { n.appearance.fill_paint() } else { n.appearance.stroke_paint() }
}

#[test]
fn a_color_read_from_a_path_assigns_back_unchanged() {
    let colors = [
        "var c = new RGBColor(); c.red = 0.00001; c.green = 200.123456789; c.blue = 1 / 3;",
        "var c = new CMYKColor(); c.cyan = 100 / 3; c.magenta = 0.00001; c.yellow = 99.99999; c.black = 12.3456789;",
        "var c = new GrayColor(); c.gray = 100 / 3;",
        "var c = new GrayColor(); c.gray = 99.99999;",
        "var c = new LabColor(); c.l = 12.3456789; c.a = -0.00001; c.b = 127.99999;",
    ];
    for color in colors {
        for prop in ["fillColor", "strokeColor"] {
            let (mut s, a, _) = doc();
            log(&mut s, &format!("{color} app.activeDocument.pathItems[0].{prop} = c;"));
            let (before, steps) = (paint_of(&s, a, prop == "fillColor"), undo_steps(&s));
            let r = run(&mut s, &format!("var p = app.activeDocument.pathItems[0]; p.{prop} = p.{prop};")).unwrap();
            assert_eq!(paint_of(&s, a, prop == "fillColor"), before, "{color} {prop}");
            assert_eq!((undo_steps(&s), &r["slowestCommands"]), (steps, &json!([])), "{color} {prop}");
        }
    }
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0], c = new RGBColor(); c.red = 200; c.green = 0.5; p.fillColor = c;\
         var g = new GrayColor(); g.gray = 25; p.strokeColor = g; vc.log([p.fillColor.red, p.fillColor.green, p.strokeColor.gray]);",
    );
    assert_eq!(out, vec![json!([200, 0.5, 25])], "whole and short values read as they were given");
}

#[test]
fn many_point_edits_run_one_command() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let anchors: Vec<Value> = (0..2000).map(|i| json!({"x": f64::from(i) * 0.1, "y": 100})).collect();
    let id = NodeId(s.execute("path.create", &json!({"anchors": anchors, "closed": false})).unwrap()["id"].as_u64().unwrap());
    let steps = undo_steps(&s);
    let r = run(
        &mut s,
        "var p = app.activeDocument.pathItems[0], pp = p.pathPoints;\
         for (var i = 0; i < pp.length; i++) { var a = pp[i].anchor; pp[i].rightDirection = [a[0] + 1, a[1]]; pp[i].anchor = [a[0], a[1] + 5]; }\
         vc.log(pp[1999].anchor); vc.log(pp[1999].rightDirection); pp[0].remove(); vc.log(pp.length); vc.log(p.pathPoints[0].anchor);",
    )
    .unwrap();
    let out: Vec<Value> = r["log"].as_array().unwrap().iter().map(rounded).collect();
    assert_eq!(out, vec![json!([199.9, 205]), json!([200.9, 200]), json!(1999), json!([0.1, 205])]);
    assert_eq!(
        r["slowestCommands"].as_array().map(|c| c.iter().map(|c| (c["id"].clone(), c["calls"].clone())).collect::<Vec<_>>()),
        Some(vec![(json!("path.setAnchors"), json!(1))]),
        "one command writes every point"
    );
    let pd = node(&s, id).path_data().unwrap().clone();
    assert_eq!(pd.subpaths[0].anchors.len(), 1999);
    assert!(pd.subpaths[0].anchors.iter().all(|a| (a.h_out.x - a.p.x - 1.0).abs() < 1e-9 && (a.h_out.y - a.p.y - 5.0).abs() < 1e-9));
    assert_eq!(undo_steps(&s), steps + 1);
}

#[test]
fn point_edits_are_written_before_the_next_other_member_reads() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0], q = app.activeDocument.pathItems[1];\
         p.pathPoints[0].anchor = [0, 300]; vc.log(p.geometricBounds); q.pathPoints[0].anchor = [90, 210];\
         p.pathPoints[1].anchor = [50, 300]; vc.log(q.geometricBounds); vc.log(p.geometricBounds);",
    );
    let out: Vec<Value> = out.iter().map(rounded).collect();
    assert_eq!(out, vec![json!([0, 300, 40, 240]), json!([90, 210, 150, 150]), json!([0, 300, 50, 240])]);
    assert_eq!(node(&s, a).path_data().unwrap().subpaths[0].anchors[1].p, vectorcraft_geom::Point::new(50.0, 0.0));
}

#[test]
fn closed_and_set_entire_path_join_the_point_edits_in_one_command() {
    let (mut s, a, _) = doc();
    let r = run(
        &mut s,
        "var p = app.activeDocument.pathItems[0]; p.setEntirePath([[0, 300], [40, 300], [40, 240]]); p.closed = false;\
         p.pathPoints[1].anchor = [50, 300]; vc.log(p.closed); p.closed = true; p.setEntirePath([[0, 300], [60, 300], [60, 240]]); vc.log(p.closed);",
    )
    .unwrap();
    assert_eq!(r["log"], json!([false, true]));
    assert_eq!(
        r["slowestCommands"].as_array().map(|c| c.iter().map(|c| (c["id"].clone(), c["calls"].clone())).collect::<Vec<_>>()),
        Some(vec![(json!("path.setAnchors"), json!(1))]),
        "one command writes every change"
    );
    let pd = node(&s, a).path_data().unwrap().clone();
    assert!(pd.subpaths[0].closed);
    assert_eq!(pd.subpaths[0].anchors.iter().map(|a| (a.p.x, a.p.y)).collect::<Vec<_>>(), vec![(0.0, 0.0), (60.0, 0.0), (60.0, 60.0)]);
}

#[test]
fn point_edits_of_a_pixel_aligned_path_read_as_the_grid_snaps_them() {
    let (mut s, a, _) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("object.setPixelAligned", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0]; p.setEntirePath([[10.3, 280.3], [40.3, 280.3], [40.3, 240.3], [10.3, 240.3]]);\
         vc.log(p.pathPoints[0].anchor); p.pathPoints[1].anchor = [40.4, 280.3]; vc.log(p.pathPoints[1].anchor);",
    );
    let anchors = node(&s, a).path_data().unwrap().subpaths[0].anchors.to_vec();
    let stored = |i: usize| rounded(&json!([anchors[i].p.x, 300.0 - anchors[i].p.y]));
    assert_ne!(rounded(&out[0]), json!([10.3, 280.3]), "the grid moves the point");
    assert_eq!((rounded(&out[0]), rounded(&out[1])), (stored(0), stored(1)), "points read as the path has them");
}

#[test]
fn added_paths_go_on_top_of_the_layer_whatever_the_drawing_mode() {
    for mode in ["behind", "inside"] {
        let (mut s, a, _) = doc();
        s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
        s.execute("view.drawMode", &json!({"mode": mode})).unwrap();
        let drawing = s.draw_mode;
        log(
            &mut s,
            "var d = app.activeDocument; d.pathItems.add().name = 'n'; d.pathItems.rectangle(10, 10, 5, 5).name = 'r';\
             d.pathItems.roundedRectangle(10, 10, 5, 5).name = 'q';",
        );
        let st = s.active().unwrap();
        let children = st.doc.layers[0].children().unwrap();
        let names: Vec<Option<&str>> = children.iter().rev().take(3).map(|c| c.name.as_deref()).collect();
        assert_eq!(names, [Some("q"), Some("r"), Some("n")], "{mode}: the new paths are on top of the layer");
        assert_eq!(s.draw_mode, drawing, "{mode}: the drawing mode stays");
    }
}

#[test]
fn overprint_changes_the_top_fill_or_stroke_only() {
    let (mut s, a, _) = doc();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("appearance.addFill", &json!({})).unwrap();
    s.execute("appearance.addStroke", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0]; p.fillOverprint = true; p.strokeOverprint = true; vc.log([p.fillOverprint, p.strokeOverprint]);",
    );
    assert_eq!(out, vec![json!([true, true])]);
    let items = node(&s, a).appearance.items.clone();
    for fill in [true, false] {
        let flags: Vec<bool> = items.iter().filter(|i| i.is_fill() == fill).map(|i| i.overprint()).collect();
        assert!(flags.len() >= 2, "{flags:?}");
        let (top, lower) = flags.split_last().unwrap();
        assert!(*top && lower.iter().all(|f| !f), "fill {fill}: only the top row overprints: {flags:?}");
    }
}

#[test]
fn filled_true_on_a_compound_member_places_the_gradient_on_the_compound() {
    for member in 0..2 {
        let (mut s, a, b) = doc();
        s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
        let compound = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
        s.execute("paint.setFill", &json!({"ids": [compound.0], "none": true})).unwrap();
        s.execute("select.none", &json!({})).unwrap();
        let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
        s.execute("paint.setFill", &json!({"gradient": {"kind": "linear", "stops": stops}})).unwrap();
        if let Paint::Gradient(g) = &mut s.paint.fill {
            g.geom = Some(vectorcraft_color::GradientGeom {
                start: vectorcraft_geom::Point::new(0.0, 0.5),
                end: vectorcraft_geom::Point::new(1.0, 0.5),
                aspect: 1.0,
                focal: None,
            });
        }
        log(&mut s, &format!("app.activeDocument.pathItems[{member}].filled = true;"));
        let b = node(&s, compound).geometric_bounds().unwrap();
        let Paint::Gradient(g) = node(&s, compound).appearance.fill_paint() else { panic!("member {member}: the compound has a gradient") };
        let geom = g.geom.unwrap();
        assert!((geom.start.x - b.x0).abs() < 1e-6 && (geom.end.x - b.x1).abs() < 1e-6, "member {member}: {geom:?} spans {b:?}");
    }
}

#[test]
fn set_anchors_reads_smooth_false_as_a_corner() {
    let (mut s, a, _) = doc();
    let anchor = |smooth: Value| {
        let mut v = json!({"x": 20, "y": 20, "in": [10, 20], "out": [30, 20]});
        if !smooth.is_null() {
            v["smooth"] = smooth;
        }
        v
    };
    for (smooth, kind, read) in [(json!(false), AnchorKind::Corner, "PointType.CORNER"), (Value::Null, AnchorKind::Smooth, "PointType.SMOOTH")] {
        let subpaths = json!([{"anchors": [anchor(smooth.clone()), {"x": 60, "y": 20}], "closed": false}]);
        s.execute("path.setAnchors", &json!({"id": a.0, "subpaths": subpaths})).unwrap();
        assert_eq!(node(&s, a).path_data().unwrap().subpaths[0].anchors[0].kind, kind, "smooth {smooth}");
        assert_eq!(log(&mut s, "vc.log(app.activeDocument.pathItems[0].pathPoints[0].pointType);"), vec![json!(read)], "smooth {smooth}");
    }
}

#[test]
fn an_added_point_follows_the_last_point_before_an_empty_subpath() {
    let (mut s, a, _) = doc();
    let subpaths = json!([{"anchors": [{"x": 10, "y": 20}, {"x": 40, "y": 20}], "closed": false}, {"anchors": [], "closed": false}]);
    s.execute("path.setAnchors", &json!({"id": a.0, "subpaths": subpaths})).unwrap();
    assert!(node(&s, a).path_data().is_some_and(|pd| pd.subpaths.len() == 2), "the empty subpath is kept");
    let out = log(
        &mut s,
        "var pp = app.activeDocument.pathItems[0].pathPoints; var n = pp.add(); vc.log(pp.length); vc.log(n.anchor); vc.log(pp[1].anchor);",
    );
    assert_eq!(out[0], json!(3));
    assert_eq!(out[1], out[2], "the new point is at the last point");
}

#[test]
fn transforms_change_the_path_and_not_the_selected_one() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    let (before_a, before_b) = (node(&s, a).geometric_bounds(), node(&s, b).geometric_bounds());
    log(&mut s, "var p = app.activeDocument.pathItems[0]; p.translate(5, 0); p.rotate(90); p.resize(50, 50);");
    assert_ne!(node(&s, a).geometric_bounds(), before_a, "the path the script named changes");
    assert_eq!(node(&s, b).geometric_bounds(), before_b, "the selected path stays");
    assert_eq!(s.active().unwrap().selection.objects, [b], "the selection stays");
}

#[test]
fn an_added_path_takes_new_art_without_a_fill_or_stroke() {
    let (mut s, _, _) = doc();
    s.execute("paint.setFill", &json!({"none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    let unpainted = |s: &Session| {
        let ap = s.new_art();
        matches!(ap.fill_paint(), Paint::None) && matches!(ap.stroke_paint(), Paint::None)
    };
    assert!(unpainted(&s));
    let (steps, recent) = (undo_steps(&s), s.recent_colors.clone());
    let out = log(&mut s, "var n = app.activeDocument.pathItems.add(); vc.log(n.filled); vc.log(n.stroked); vc.log(n.pathPoints.length);");
    assert_eq!(out, [json!(false), json!(false), json!(0)]);
    assert_eq!(undo_steps(&s), steps + 1, "one step for the run");
    assert!(unpainted(&s), "new art keeps no fill and no stroke");
    assert_eq!(s.recent_colors, recent);
    s.execute("paint.setStroke", &json!({"color": "#ff0000"})).unwrap();
    let out = log(&mut s, "var n = app.activeDocument.pathItems.add(); vc.log(n.filled); vc.log(n.stroked); vc.log(n.strokeColor.red);");
    assert_eq!(out, [json!(false), json!(true), json!(255)], "a new-art stroke stays");
}

/// [`doc`] with pattern `Dots` and spot swatch `Ink`, and path `a` with no fill or stroke.
fn doc_with_paints() -> (Session, NodeId) {
    let (mut s, a, _) = doc();
    let dot = rect(&mut s, 0.0, 0.0, 10.0, 10.0);
    s.execute("object.pattern.make", &json!({"ids": [dot.0], "name": "Dots", "width": 20, "height": 20, "edit": false})).unwrap();
    s.execute("select.set", &json!({"ids": [dot.0]})).unwrap();
    s.execute("edit.clear", &json!({})).unwrap();
    s.execute("swatch.new", &json!({"name": "Ink", "color": "#2a6fb0", "spot": true})).unwrap();
    s.execute("paint.setFill", &json!({"ids": [a.0], "none": true})).unwrap();
    s.execute("paint.setStroke", &json!({"ids": [a.0], "none": true})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    (s, a)
}

#[test]
fn filled_and_stroked_true_give_the_new_art_paint() {
    let kinds = [
        json!({"color": "#00ff00"}),
        json!({"swatch": "Ink", "tint": 40}),
        json!({"gradient": {"kind": "radial", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]}}),
        json!({"swatch": "Dots"}),
    ];
    for fill in [true, false] {
        for params in &kinds {
            let (mut s, a) = doc_with_paints();
            s.execute(if fill { "paint.setFill" } else { "paint.setStroke" }, params).unwrap();
            let new_art = s.new_art();
            let want = if fill { new_art.fill_paint() } else { new_art.stroke_paint() };
            assert!(!matches!(want, Paint::None), "{params}: new art takes the paint");
            log(&mut s, if fill { "app.activeDocument.pathItems[0].filled = true;" } else { "app.activeDocument.pathItems[0].stroked = true;" });
            let n = node(&s, a);
            let got = if fill { n.appearance.fill_paint() } else { n.appearance.stroke_paint() };
            assert_eq!(crate::cmd::gradient::unplaced(&got), want, "fill {fill}, {params}");
        }
    }
}

#[test]
fn filled_true_throws_for_a_new_art_pattern_it_cannot_give() {
    let placed = vectorcraft_color::Paint::Pattern { pattern: "Dots".into(), xf: vectorcraft_geom::Affine::scale(2.0) };
    for (paint, says) in [(placed, "placement of its own"), (vectorcraft_doc::pattern::pattern_paint("Gone"), "not in this document")] {
        for fill in [true, false] {
            let (mut s, _) = doc_with_paints();
            if fill {
                s.paint.fill = paint.clone();
            } else {
                (s.paint.stroke, s.paint.stroke_width) = (paint.clone(), 1.0);
            }
            let (before, steps) = (art(&s), undo_steps(&s));
            let e = failure(
                &mut s,
                if fill { "app.activeDocument.pathItems[0].filled = true;" } else { "app.activeDocument.pathItems[0].stroked = true;" },
            );
            assert!(e.contains(says), "{e}");
            assert_eq!((art(&s), undo_steps(&s)), (before, steps), "nothing changes");
        }
    }
}

#[test]
fn stroke_options_map_to_stroke_set() {
    let (mut s, a, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pathItems[0];\
         p.strokeWidth = 4; p.strokeCap = StrokeCap.ROUNDENDCAP; p.strokeJoin = StrokeJoin.BEVELENDJOIN; p.strokeMiterLimit = 7;\
         p.strokeDashes = [6, 3]; p.strokeDashOffset = 2; p.evenodd = true; p.fillOverprint = true;\
         vc.log([p.strokeWidth, p.strokeCap, p.strokeJoin, p.strokeMiterLimit, p.strokeDashes, p.strokeDashOffset, p.evenodd, p.fillOverprint]);\
         p.strokeDashes = {}; vc.log(p.strokeDashes);",
    );
    assert_eq!(out[0], json!([4, "StrokeCap.ROUNDENDCAP", "StrokeJoin.BEVELENDJOIN", 7, [6, 3], 2, true, true]));
    assert_eq!(out[1], json!([]));
    let n = node(&s, a);
    let st = n.appearance.stroke().unwrap();
    assert_eq!((st.width, st.cap, st.join, st.miter_limit, st.dash.is_none()), (4.0, LineCap::Round, LineJoin::Bevel, 7.0, true));
    for (source, part) in [
        ("app.activeDocument.pathItems[0].strokeMiterLimit = 600;", "strokeMiterLimit"),
        ("app.activeDocument.pathItems[0].strokeCap = 'round';", "StrokeCap"),
        ("app.activeDocument.pathItems[0].strokeJoin = StrokeCap.ROUNDENDCAP;", "StrokeJoin"),
        ("app.activeDocument.pathItems[0].strokeWidth = -1;", "strokeWidth"),
        ("app.activeDocument.pathItems[0].strokeDashes = [0, 0];", "not 0"),
        ("app.activeDocument.pathItems[0].strokeDashes = [1, -2];", "dash length"),
        ("var d = []; for (var i = 0; i < 65; i++) d.push(1); app.activeDocument.pathItems[0].strokeDashes = d;", "at most 64"),
        ("app.activeDocument.pathItems[0].strokeDashOffset = 3;", "strokeDashes first"),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
}

#[test]
fn bad_color_components_throw_and_change_nothing() {
    let (mut s, _, _) = doc();
    let before = art(&s);
    for (source, part) in [
        ("var c = new RGBColor(); c.red = 300; app.activeDocument.pathItems[0].fillColor = c;", "RGBColor.red"),
        ("var c = new CMYKColor(); c.cyan = NaN; app.activeDocument.pathItems[0].fillColor = c;", "finite"),
        ("var c = new CMYKColor(); c.cyan = -1; app.activeDocument.pathItems[0].fillColor = c;", "CMYKColor.cyan"),
        ("var c = new GrayColor(); c.gray = '5'; app.activeDocument.pathItems[0].fillColor = c;", "GrayColor.gray"),
        ("var c = new LabColor(); c.a = 200; app.activeDocument.pathItems[0].strokeColor = c;", "LabColor.a"),
        ("app.activeDocument.pathItems[0].fillColor = {typename: 'Foo'};", "color object"),
        ("app.activeDocument.pathItems[0].fillColor = 5;", "color object"),
        ("app.activeDocument.pathItems[0].fillColor = new GradientColor();", "not available yet"),
        ("app.activeDocument.pathItems[0].strokeColor = new SpotColor();", "SpotColor.spot must be a Spot; a color read"),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(art(&s), before);
}

#[test]
fn locked_paths_and_wrong_objects_are_refused() {
    let (mut s, a, _) = doc();
    s.execute("object.setProps", &json!({"ids": [a.0], "locked": true})).unwrap();
    let before = art(&s);
    for (source, part) in [
        ("app.activeDocument.pathItems[0].fillColor = new NoColor();", "Cannot edit PathItem"),
        ("app.activeDocument.pathItems[0].setEntirePath([[0, 0], [1, 1]]);", "Cannot edit PathItem"),
        ("app.activeDocument.pathItems[0].pathPoints[0].anchor = [1, 1];", "Cannot edit PathItem"),
        ("app.activeDocument.pathItems[0].strokeWidth = 3;", "Cannot edit PathItem"),
        ("app.activeDocument.pathItems[0].translate(5, 5);", "Cannot edit PathItem"),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(art(&s), before);
    s.execute("text.create", &json!({"x": 10, "y": 250, "text": "hi"})).unwrap();
    let e = failure(&mut s, "var t = app.activeDocument.pageItems[0]; Object.getOwnPropertyDescriptor(PathItem.prototype, 'closed').get.call(t);");
    assert!(e.contains("not a PathItem"), "{e}");
    let e = failure(&mut s, "Object.getOwnPropertyDescriptor(PathPoint.prototype, 'anchor').get.call({});");
    assert!(e.contains("not a PathPoint"), "{e}");
}

#[test]
fn rotate_positive_is_counterclockwise_and_transforms_move_and_scale() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let out = log(
        &mut s,
        "var n = app.activeDocument.pathItems.add(); n.setEntirePath([[100, 100], [110, 100]]);\
         n.rotate(90, true, true, true, true, Transformation.LEFT); vc.log(n.pathPoints[1].anchor);\
         n.translate(5, 10); vc.log(n.pathPoints[0].anchor);\
         var r = app.activeDocument.pathItems.rectangle(250, 10, 30, 40); r.strokeWidth = 2;\
         r.resize(200, 50, true, true, true, true, 300, Transformation.TOPLEFT); vc.log(r.geometricBounds); vc.log(r.strokeWidth);",
    );
    let out: Vec<Value> = out.iter().map(rounded).collect();
    assert_eq!(out[0], json!([100, 110]), "a positive angle turns counterclockwise");
    assert_eq!(out[1], json!([105, 110]), "translate moves up for a positive deltaY");
    assert_eq!(out[2], json!([10, 250, 70, 230]));
    assert_eq!(out[3], json!(6));
    assert!(failure(&mut s, "app.activeDocument.pathItems[0].rotate(10, true, true, true, true, 'CENTER');").contains("Transformation"));
    assert!(failure(&mut s, "app.activeDocument.pathItems[0].resize(0, 100);").contains("scaleX"));
    assert!(failure(&mut s, "app.activeDocument.pathItems[0].translate(1, 2, 'no');").contains("changePositions"));
    let before = art(&s);
    for (source, part) in [
        ("app.activeDocument.pathItems[0].translate(1, 2, true, true, false);", "changeFillGradients false"),
        ("app.activeDocument.pathItems[0].rotate(10, true, true, true, false);", "must be the same"),
        ("app.activeDocument.pathItems.ellipse(10, 10, 5, 5, false, true);", "inscribed"),
        (
            "var ps = app.activeDocument.pathItems;\
             for (var i = 0; i < ps.length; i++) { if (ps[i].strokeWidth == 6) ps[i].resize(150, 100, true, true, true, true, 20000); }",
            "more than 1000 points",
        ),
    ] {
        let e = failure(&mut s, source);
        assert!(e.contains(part), "{source}: {e}");
    }
    assert_eq!(art(&s), before);
}

#[test]
fn area_sign_follows_direction() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300})).unwrap();
    let out = log(
        &mut s,
        "var n = app.activeDocument.pathItems.add(); n.setEntirePath([[0, 0], [10, 0], [10, 10], [0, 10]]); n.closed = true;\
         vc.log(n.area); vc.log(n.length);\
         var r = app.activeDocument.pathItems.rectangle(100, 0, 10, 10), q = app.activeDocument.pathItems.rectangle(100, 0, 10, 10, true);\
         vc.log(r.area + q.area); vc.log(Math.abs(r.area));",
    );
    let out: Vec<Value> = out.iter().map(rounded).collect();
    assert_eq!(out, vec![json!(-100), json!(40), json!(0), json!(100)]);
}

#[test]
fn many_path_edits_are_one_undo_step() {
    let (mut s, a, b) = doc();
    let steps = undo_steps(&s);
    log(
        &mut s,
        "var p = app.activeDocument.pathItems, c = new RGBColor(); c.green = 200;\
         for (var i = 0; i < p.length; i++) { p[i].fillColor = c; p[i].strokeWidth = 3; p[i].pathPoints[0].anchor = [0, 0]; }",
    );
    assert_eq!(undo_steps(&s), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    for id in [a, b] {
        assert!((node(&s, id).appearance.stroke_width() - 1.0).abs() < 1e-9);
    }
}
