//! Freeform gradients through commands: the first application (M3.81), transforms and warps, and
//! the point and line commands (M3.82).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Freeform, GradientKind, GradientPaint, Paint};
use vectorcraft_geom::Point;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn shape(s: &mut Session, cmd: &str, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute(cmd, &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> &vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap()
}

fn fill_gradient(s: &Session, id: NodeId) -> GradientPaint {
    match node(s, id).appearance.fill_paint() {
        Paint::Gradient(g) => *g,
        p => panic!("expected a gradient fill, got {p:?}"),
    }
}

fn points(s: &Session, id: NodeId) -> Freeform {
    fill_gradient(s, id).freeform.expect("placed freeform points")
}

/// A selected 100 × 100 rectangle at (100, 100) with a freeform fill.
fn freeform_rect() -> (Session, NodeId) {
    let mut s = session();
    let id = shape(&mut s, "shape.rectangle", 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.editGradient", &json!({"kind": "freeform"})).unwrap();
    (s, id)
}

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd} {p}: {e}"))
}

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-6
}

// ---------- M3.81: the first application, transforms, warps and saving ----------

#[test]
fn the_first_application_places_points_inside_the_shape() {
    let mut s = session();
    // An ellipse: the box's corners are outside it.
    let id = shape(&mut s, "shape.ellipse", 100.0, 100.0, 300.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.editGradient", &json!({"kind": "freeform"})).unwrap();
    let g = fill_gradient(&s, id);
    assert_eq!(g.gradient.kind, GradientKind::Freeform);
    let f = g.freeform.as_ref().unwrap();
    let inside = node(&s, id).contains_fn();
    assert!(f.points.len() >= 4 && f.points.iter().all(|p| inside(p.at)), "{:?}", f.points);
    // The stops follow the points' colours (swatch chips and exports show them).
    assert_eq!(g.gradient.stops.iter().map(|st| st.color).collect::<Vec<_>>(), f.points.iter().map(|p| p.color).collect::<Vec<_>>());
    // Draw mode: freeform only.
    s.execute("paint.editGradient", &json!({"mode": "lines"})).unwrap();
    assert_eq!(points(&s, id).mode, vectorcraft_color::FreeformMode::Lines);
    assert_eq!(points(&s, id).points, f.points, "the points stay");
    s.execute("paint.editGradient", &json!({"kind": "linear"})).unwrap();
    assert!(fill_gradient(&s, id).freeform.is_none(), "a linear gradient drops the points");
    let e = s.execute("paint.editGradient", &json!({"mode": "lines"})).unwrap_err().to_string();
    assert!(e.contains("freeform"), "{e}");
}

#[test]
fn new_stops_recolour_the_points() {
    let (mut s, id) = freeform_rect();
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    s.execute("paint.editGradient", &json!({"stops": stops})).unwrap();
    let f = points(&s, id);
    assert_eq!((f.points[0].color.to_hex(), f.points.last().unwrap().color.to_hex()), ("#ff0000".into(), "#0000ff".into()));
}

/// A CMYK color's inks.
fn inks(c: Color) -> [f32; 4] {
    match c {
        Color::Cmyk { c, m, y, k } => [c, m, y, k],
        c => panic!("a CMYK color expected: {c:?}"),
    }
}

/// Cyan and black inks only, adding up to one.
fn cyan_and_black(c: Color) -> bool {
    let [c, m, y, k] = inks(c);
    m.abs() < 1e-5 && y.abs() < 1e-5 && (c + k - 1.0).abs() < 1e-5
}

#[test]
fn cmyk_stops_recolor_the_points_with_inks() {
    let cyan_to_black = json!([{"offset": 0, "color": {"c": 1, "m": 0, "y": 0, "k": 0}}, {"offset": 1, "color": {"c": 0, "m": 0, "y": 0, "k": 1}}]);
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.editGradient", json!({"stops": cyan_to_black}));
    let f = points(&s, id);
    let n = f.points.len();
    for (i, p) in f.points.iter().enumerate() {
        let k = i as f32 / (n - 1) as f32;
        let [c, m, y, kk] = inks(p.color);
        assert!((c - (1.0 - k)).abs() < 1e-5 && m.abs() < 1e-5 && y.abs() < 1e-5 && (kk - k).abs() < 1e-5, "point {i}: {:?}", p.color);
    }
    // A CMYK gradient switched to freeform places points with inks.
    let mut s = session();
    let id = shape(&mut s, "shape.rectangle", 100.0, 100.0, 100.0, 100.0);
    run(&mut s, "paint.setFill", json!({"gradient": {"kind": "linear", "stops": cyan_to_black}}));
    run(&mut s, "paint.editGradient", json!({"kind": "freeform"}));
    assert!(points(&s, id).points.iter().all(|p| cyan_and_black(p.color)), "{:?}", points(&s, id).points);
}

#[test]
fn points_added_between_cmyk_points_take_inks() {
    let (mut s, id) = freeform_rect();
    let n = points(&s, id).points.len();
    for i in 0..n {
        let c = if i % 2 == 0 { json!({"c": 1, "m": 0, "y": 0, "k": 0}) } else { json!({"c": 0, "m": 0, "y": 0, "k": 1}) };
        run(&mut s, "paint.freeform.setPoint", json!({"index": i, "color": c}));
    }
    run(&mut s, "paint.freeform.addLine", json!({"points": [0, 1]}));
    let i = run(&mut s, "paint.freeform.splitLine", json!({"line": 0, "segment": 0}))["index"].as_u64().unwrap() as usize;
    let split = points(&s, id).points[i].color;
    assert!(cyan_and_black(split), "the split point: {split:?}");
    let i = run(&mut s, "paint.freeform.addPoint", json!({"at": [150, 150]}))["index"].as_u64().unwrap() as usize;
    let added = points(&s, id).points[i].color;
    assert!(cyan_and_black(added), "the added point: {added:?}");
}

#[test]
fn transforms_and_warps_move_the_points() {
    let (mut s, id) = freeform_rect();
    let before = points(&s, id);
    run(&mut s, "object.move", json!({"dx": 10, "dy": 5}));
    let moved = points(&s, id);
    assert!(before.points.iter().zip(&moved.points).all(|(a, b)| close(b.at, a.at + vectorcraft_geom::Vec2::new(10.0, 5.0))));
    run(&mut s, "object.rotate", json!({"angle": 90, "origin": [160, 155]}));
    let turned = points(&s, id);
    // The y-down document turns (x, y) about the origin to (x', y') = (160 + (y − 155), 155 − (x − 160)).
    for (a, b) in moved.points.iter().zip(&turned.points) {
        let want = Point::new(160.0 + (a.at.y - 155.0), 155.0 - (a.at.x - 160.0));
        assert!(close(b.at, want), "{:?} → {:?}, want {want:?}", a.at, b.at);
    }
    // A perspective distort maps each point exactly: a point on a corner stays on it.
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.freeform.addPoint", json!({"at": [200, 200]}));
    run(&mut s, "object.distort", json!({"corners": [[100, 100], [200, 100], [260, 240], [100, 200]]}));
    let f = points(&s, id);
    assert!(close(f.points.last().unwrap().at, Point::new(260.0, 240.0)), "{:?}", f.points.last());
}

#[test]
fn freeform_paints_survive_saving_and_old_files_load() {
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.editGradient", json!({"mode": "lines"}));
    run(&mut s, "paint.freeform.addLine", json!({"points": [0, 1, 2]}));
    vectorcraft_testkit::invariants::check_native_roundtrip_exact(&s.doc().unwrap().doc).unwrap();
    // A gradient saved before freeform points existed has none.
    let old: GradientPaint = serde_json::from_value(json!({"gradient": {"kind": "Freeform", "stops": []}, "angle": 0.0})).unwrap();
    assert!(old.freeform.is_none());
    // The command params are lossless: setting the paint read back reproduces it.
    let g = fill_gradient(&s, id);
    let other = shape(&mut s, "shape.rectangle", 300.0, 300.0, 50.0, 50.0);
    run(&mut s, "paint.setFill", json!({"gradient": vectorcraft_tools::params::gradient_params(&g), "ids": [other.0]}));
    assert_eq!(fill_gradient(&s, other), g);
}

#[test]
fn reapplied_gradients_place_their_points_afresh() {
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.freeform.setPoint", json!({"index": 0, "color": "#ff0000"}));
    // The last gradient (`.`) on other art: points inside it, coloured like the first object's.
    let other = shape(&mut s, "shape.rectangle", 400.0, 300.0, 100.0, 100.0);
    run(&mut s, "paint.lastGradient", json!({}));
    let (a, (b, _)) = (points(&s, id), node(&s, other).proxy_freeform(false, None).unwrap());
    assert!(b.points.iter().all(|p| p.at.x >= 400.0 && p.at.y >= 300.0), "{:?}", b.points);
    assert_eq!(a.points.iter().map(|p| p.color.to_hex()).collect::<Vec<_>>(), b.points.iter().map(|p| p.color.to_hex()).collect::<Vec<_>>());
}

// ---------- M3.82: points and lines ----------

#[test]
fn add_set_and_delete_points() {
    let (mut s, id) = freeform_rect();
    let n = points(&s, id).points.len();
    let field = points(&s, id);
    let r = run(&mut s, "paint.freeform.addPoint", json!({"at": [150, 150]}));
    assert_eq!(r["index"], json!(n));
    assert_eq!(s.selected_freeform_point(), Some(n), "the new point is selected");
    let p = points(&s, id).points[n];
    // Without a colour it takes the gradient's colour there.
    let (want, _) = field.sample(Point::new(150.0, 150.0), 50.0);
    assert!(close(p.at, Point::new(150.0, 150.0)) && p.color.to_hex() == want.to_hex(), "{p:?} vs {want:?}");
    // The selected point takes edits without an index; opacity and spread take 0..1 or percentages.
    run(&mut s, "paint.freeform.setPoint", json!({"color": "#00ff00", "opacity": 50, "spread": 0.4, "at": [160, 170]}));
    let p = points(&s, id).points[n];
    assert_eq!((p.color.to_hex().as_str(), p.opacity, p.spread, p.at), ("#00ff00", 0.5, 0.4, Point::new(160.0, 170.0)));
    assert_eq!(fill_gradient(&s, id).gradient.stops[n].color.to_hex(), "#00ff00", "the stops follow");
    // One undo step per command.
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(points(&s, id).points[n].color.to_hex(), want.to_hex());
    run(&mut s, "edit.redo", json!({}));
    run(&mut s, "paint.freeform.deletePoint", json!({}));
    assert_eq!(points(&s, id).points.len(), n);
    assert_eq!(s.selected_freeform_point(), None);
    for i in (1..n).rev() {
        run(&mut s, "paint.freeform.deletePoint", json!({"index": i}));
    }
    let e = s.execute("paint.freeform.deletePoint", &json!({"index": 0})).unwrap_err().to_string();
    assert!(e.contains("at least one"), "{e}");
    let e = s.execute("paint.freeform.setPoint", &json!({"index": 5, "color": "#000000"})).unwrap_err().to_string();
    assert!(e.contains("no point 5"), "{e}");
}

#[test]
fn lines_join_split_and_follow_the_selection() {
    let (mut s, id) = freeform_rect();
    assert_eq!(run(&mut s, "paint.freeform.addLine", json!({"points": [0, 1]}))["line"], json!(0));
    run(&mut s, "paint.freeform.selectPoint", json!({"index": 1}));
    // Adding a point with `line` extends the line ending at the selected point.
    let i = run(&mut s, "paint.freeform.addPoint", json!({"at": [190, 190], "line": true}))["index"].as_u64().unwrap() as usize;
    assert_eq!(points(&s, id).lines, vec![vec![0, 1, i]]);
    let j = run(&mut s, "paint.freeform.splitLine", json!({"line": 0, "segment": 1}))["index"].as_u64().unwrap() as usize;
    assert_eq!((points(&s, id).lines[0].clone(), s.selected_freeform_point()), (vec![0, 1, j, i], Some(j)));
    assert!(s.execute("paint.freeform.splitLine", &json!({"line": 0, "segment": 3})).is_err());
    assert!(s.execute("paint.freeform.addLine", &json!({"points": [0, 99]})).is_err());
    // The query reports points in document coordinates, the lines and the selection.
    let g = run(&mut s, "paint.freeform.get", json!({}));
    assert_eq!((g["lines"].clone(), g["selected"].clone(), g["mode"].clone()), (json!([[0, 1, j, i]]), json!(j), json!("points")));
    assert_eq!(g["points"][i]["at"], json!([190.0, 190.0]));
}

/// Split Line refuses a `t` that is not a number, and the point commands and the query a `stroke`
/// that is not true or false, given as text or as another value of the wrong kind; the gradient and
/// the undo history stay as they were. Numbers, false and null still work.
#[test]
fn freeform_commands_refuse_a_number_or_boolean_given_as_text() {
    use super::tests::{not_a_number, not_a_switch, not_refused};
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.freeform.addLine", json!({"points": [0, 1]}));
    let calls = [
        not_a_number("paint.freeform.splitLine", json!({"line": 0, "segment": 0, "t": "0.25"}), "t"),
        not_a_switch("paint.freeform.addPoint", json!({"at": [150, 150], "stroke": "false"}), "stroke"),
        not_a_switch("paint.freeform.get", json!({"stroke": [false]}), "stroke"),
    ];
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    run(&mut s, "paint.freeform.splitLine", json!({"line": 0, "segment": 0, "t": null}));
    let g = run(&mut s, "paint.freeform.get", json!({"stroke": false}));
    assert_eq!(g["lines"][0].as_array().map(Vec::len), Some(3));
    assert_eq!(points(&s, id).lines[0].len(), 3);
}

#[test]
fn point_commands_need_a_freeform_gradient() {
    let mut s = session();
    shape(&mut s, "shape.rectangle", 0.0, 0.0, 50.0, 50.0);
    for paint in [json!({"color": "#ff0000"}), json!({"gradient": {"kind": "radial"}})] {
        run(&mut s, "paint.setFill", paint);
        for (cmd, p) in [
            ("paint.freeform.addPoint", json!({"at": [10, 10]})),
            ("paint.freeform.setPoint", json!({"index": 0, "color": "#000000"})),
            ("paint.freeform.deletePoint", json!({"index": 0})),
            ("paint.freeform.addLine", json!({"points": [0, 1]})),
            ("paint.freeform.splitLine", json!({"line": 0, "segment": 0})),
            ("paint.freeform.selectPoint", json!({"index": 0})),
            ("paint.freeform.get", json!({})),
        ] {
            let e = s.execute(cmd, &p).unwrap_err().to_string();
            assert!(e.contains("not a freeform gradient"), "{cmd}: {e}");
        }
    }
    // Nothing selected: nothing to edit.
    run(&mut s, "select.none", json!({}));
    assert!(s.execute("paint.freeform.addPoint", &json!({"at": [1, 1]})).is_err());
}

/// The point and line commands refuse a point, line or segment of 2^32 + k, where k names one that
/// exists, and the points and the selected point stay as they were. A 32-bit build read the number
/// as k.
#[test]
fn point_commands_refuse_an_index_past_u32() {
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.freeform.addLine", json!({"points": [0, 1]}));
    let (before, selected) = (points(&s, id), s.selected_freeform_point());
    let past = |k: u64| (1u64 << 32) + k;
    for (cmd, p) in [
        ("paint.freeform.setPoint", json!({"index": past(1), "color": "#000000"})),
        ("paint.freeform.deletePoint", json!({"index": past(1)})),
        ("paint.freeform.selectPoint", json!({"index": past(1)})),
        ("paint.freeform.splitLine", json!({"line": past(0), "segment": 0})),
        ("paint.freeform.splitLine", json!({"line": 0, "segment": past(0)})),
    ] {
        assert!(s.execute(cmd, &p).is_err(), "{cmd} {p}");
        assert_eq!((points(&s, id), s.selected_freeform_point()), (before.clone(), selected), "{cmd} {p}");
    }
}

#[test]
fn selection_follows_the_object_and_the_proxy() {
    let (mut s, id) = freeform_rect();
    run(&mut s, "paint.freeform.selectPoint", json!({"index": 2}));
    assert_eq!(s.selected_freeform_point(), Some(2));
    run(&mut s, "paint.toggleActive", json!({}));
    assert_eq!(s.selected_freeform_point(), None, "the Stroke proxy has its own paint");
    run(&mut s, "paint.toggleActive", json!({}));
    assert_eq!(s.selected_freeform_point(), Some(2));
    run(&mut s, "select.none", json!({}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    assert_eq!(s.selected_freeform_point(), Some(2), "same object, same proxy");
    assert!(s.execute("paint.freeform.selectPoint", &json!({"index": 99})).is_err());
}

#[test]
fn type_objects_keep_points_in_text_space() {
    let mut s = session();
    let r = run(&mut s, "text.create", json!({"x": 100, "y": 200, "text": "Freeform", "size": 40}));
    let id = NodeId(r["id"].as_u64().unwrap());
    run(&mut s, "paint.editGradient", json!({"kind": "freeform"}));
    run(&mut s, "object.rotate", json!({"angle": 90, "origin": [100, 200]}));
    let i = run(&mut s, "paint.freeform.addPoint", json!({"at": [90, 150], "color": "#ff00ff"}))["index"].as_u64().unwrap() as usize;
    let (f, _) = node(&s, id).proxy_freeform(false, None).unwrap();
    assert!(close(f.points[i].at, Point::new(90.0, 150.0)), "back in document coordinates: {:?}", f.points[i].at);
    assert_eq!(f.points[i].color, Color::rgb(1.0, 0.0, 1.0));
}
