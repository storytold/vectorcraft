//! Gradients through commands: lossless, validated params (M3.14), transforms (M3.15), the active
//! proxy and type (M3.16) and the interactive annotator (M3.17).

use serde_json::{Value, json};
use vectorcraft_color::{Color, Gradient, GradientGeom, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_geom::Point;
use vectorcraft_tools::{Mods, PointerEvent, PointerKind, ToolKey};

use crate::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
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

#[test]
fn new_gradient_defaults_follow_the_cmyk_document_mode() {
    for kind in ["linear", "radial", "conic", "diamond", "freeform"] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width":200,"height":200,"colorMode":"cmyk"})).unwrap();
        let id = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
        s.execute("paint.editGradient", &json!({"kind":kind})).unwrap();
        let g = fill_gradient(&s, id);
        assert!(g.gradient.stops.iter().all(|stop| matches!(stop.color, Color::Cmyk { .. })), "{kind}: {:?}", g.gradient.stops);
        if let Some(freeform) = g.freeform {
            assert!(freeform.points.iter().all(|point| matches!(point.color, Color::Cmyk { .. })), "freeform points must use the same color model");
        }
    }
}

#[test]
fn editing_an_explicit_rgb_gradient_in_cmyk_keeps_its_color_model() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":200,"height":200,"colorMode":"cmyk"})).unwrap();
    let id = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
    s.execute("paint.editGradient", &json!({"kind":"linear","keepModel":true})).unwrap();
    let before = fill_gradient(&s, id).gradient.stops;
    assert!(before.iter().all(|stop| matches!(stop.color, Color::Rgb { .. })));
    s.execute("paint.editGradient", &json!({"angle":45})).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient.stops, before);
}

#[test]
fn drawing_a_new_gradient_vector_uses_cmyk_default_stops() {
    for stroke in [false, true] {
        let mut s = Session::new();
        s.execute("file.new", &json!({"width":200,"height":200,"colorMode":"cmyk"})).unwrap();
        let id = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
        s.execute("paint.setGradientGeom", &json!({"start":[10,10],"end":[110,10],"stroke":stroke})).unwrap();
        let appearance = &node(&s, id).appearance;
        let paint = if stroke { appearance.stroke_paint() } else { appearance.fill_paint() };
        let Paint::Gradient(g) = paint else { panic!("not a gradient") };
        assert!(g.gradient.stops.iter().all(|stop| matches!(stop.color, Color::Cmyk { .. })), "stroke={stroke}: {:?}", g.gradient.stops);
    }
}

/// A three-stop freeform gradient with a 0.3 midpoint, partial opacity and a placed vector.
fn three_stops() -> GradientPaint {
    let stop = |offset, color, opacity, midpoint| GradientStop { opacity, midpoint, ..GradientStop::new(offset, color) };
    let mut g = GradientPaint::new(Gradient::new(
        GradientKind::Freeform,
        vec![
            stop(0.0, Color::rgb(1.0, 0.0, 0.0), 1.0, 0.3),
            stop(0.4, Color::cmyk(0.1, 0.2, 0.3, 0.4), 0.5, 0.5),
            stop(1.0, Color::gray(0.25), 0.75, 0.5),
        ],
    ));
    g.geom = Some(GradientGeom { start: Point::new(10.0, 20.0), end: Point::new(110.0, 70.0), aspect: 0.5, focal: None });
    g.angle = g.geom.unwrap().angle_deg();
    g
}

fn gradient_param(g: &GradientPaint) -> Value {
    json!({ "gradient": vectorcraft_tools::params::gradient_params(g) })
}

#[test]
fn gradient_params_round_trip_through_fill_and_swatches() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let g = three_stops();
    s.execute("paint.setFill", &gradient_param(&g)).unwrap();
    let got = fill_gradient(&s, id);
    assert_eq!(got.gradient.kind, GradientKind::Freeform);
    assert_eq!(got.gradient.stops, g.gradient.stops);
    let (a, b) = (got.geom.unwrap(), g.geom.unwrap());
    assert!(a.start.distance(b.start) < 1e-9 && a.end.distance(b.end) < 1e-9 && (a.aspect - b.aspect).abs() < 1e-9, "{a:?}");
    // The default for new art keeps all but the placement (new art fits it to itself), and saving
    // it as a swatch is lossless.
    assert_eq!(s.paint.fill, Paint::Gradient(Box::new(GradientPaint { geom: None, ..got.clone() })));
    let mut q = gradient_param(&g);
    q["name"] = json!("Three");
    s.execute("swatch.new", &q).unwrap();
    assert_eq!(s.doc().unwrap().doc.swatch("Three").unwrap().paint, Paint::Gradient(Box::new(got)));
}

#[test]
fn bad_gradients_are_rejected_without_editing() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let before = node(&s, id).appearance.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for bad in [
        json!({"stops": [{"offset": 0, "color": "#ff0000"}]}),
        json!({"stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1}]}),
        json!({"stops": [{"offset": 0, "color": "#ff0000"}, {"color": "#000000"}]}),
        json!({"stops": [{"offset": "a", "color": "#ff0000"}, {"offset": 1, "color": "#000000"}]}),
        json!({"stops": "red"}),
        json!({"kind": "spiral"}),
        json!({"start": [0, 0]}),
        json!({"aspect": "wide"}),
    ] {
        assert!(s.execute("paint.setFill", &json!({ "gradient": bad })).is_err(), "accepted {bad}");
        // `start` isn't a paint.editGradient param (it would just apply the gradient).
        if bad.get("start").is_none() {
            assert!(s.execute("paint.editGradient", &bad).is_err(), "editGradient accepted {bad}");
        }
    }
    assert_eq!(node(&s, id).appearance, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

/// The gradient commands refuse a `reverse` or `stroke` that is not true or false, and an `angle`
/// or a stop's `tint` that is not a number, given as text or as another value of the wrong kind:
/// the object and the undo history stay as they were. True, false, numbers and null still work.
#[test]
fn gradient_commands_refuse_a_number_or_boolean_given_as_text() {
    use crate::tests::{not_a_number, not_a_switch, not_refused};
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    let red = s.execute("swatch.new", &json!({"color": "#ff0000", "global": true})).unwrap()["name"].clone();
    let stops = |tint: Value| json!([{"offset": 0, "color": "#ff0000", "swatch": red, "tint": tint}, {"offset": 1, "color": "#000000"}]);
    let calls = [
        not_a_switch("paint.editGradient", json!({"reverse": "true"}), "reverse"),
        not_a_switch("paint.editGradient", json!({"angle": 45, "stroke": "false"}), "stroke"),
        not_a_switch("paint.setGradientGeom", json!({"aspect": 50, "stroke": 0}), "stroke"),
        not_a_number("paint.editGradient", json!({"angle": "45"}), "angle"),
        ("paint.setFill".to_string(), json!({"gradient": {"angle": "45"}}), "`angle` must be a number, not \"45\"".to_string()),
        ("paint.editGradient".to_string(), json!({"stops": stops(json!("50"))}), "stop 0: `tint` must be a number".to_string()),
        ("paint.setFill".to_string(), json!({"gradient": {"stops": stops(json!([50]))}}), "stop 0: `tint` must be a number".to_string()),
    ];
    let wrong = not_refused(&mut s, &calls);
    assert!(wrong.is_empty(), "not refused: {wrong:#?}");
    s.execute("paint.editGradient", &json!({"angle": 30, "reverse": false, "stroke": null, "stops": stops(json!(50))})).unwrap();
    let g = fill_gradient(&s, id);
    assert_eq!((g.angle, g.gradient.stops[0].tint), (30.0, 0.5));
}

#[test]
fn stop_opacity_and_offsets_are_normalised() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute(
        "paint.setFill",
        &json!({"gradient": {"stops": [{"offset": 1.5, "color": "#000000", "opacity": 50}, {"offset": -1, "color": "#ffffff", "opacity": 0.25, "midpoint": 0.99}]}}),
    )
    .unwrap();
    let g = fill_gradient(&s, id);
    // Sorted, offsets clamped, 0..100 opacity normalised, midpoint clamped to the diamond's range.
    let st = &g.gradient.stops;
    assert_eq!((st[0].offset, st[0].opacity, st[0].midpoint), (0.0, 0.25, 0.87));
    assert_eq!((st[1].offset, st[1].opacity), (1.0, 0.5));
    assert!(g.geom.is_none());
}

#[test]
fn geometry_and_aspect_in_one_call() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial", "start": [200, 150], "end": [260, 150], "aspect": 40}})).unwrap();
    let geom = fill_gradient(&s, id).geom.unwrap();
    assert_eq!((geom.start, geom.end), (Point::new(200.0, 150.0), Point::new(260.0, 150.0)));
    assert!((geom.aspect - 0.4).abs() < 1e-9);
    // An aspect without a vector places the gradient on the object's bounds so the aspect sticks.
    s.execute("paint.setStroke", &json!({"gradient": {"kind": "radial", "aspect": 25}})).unwrap();
    let Paint::Gradient(sg) = node(&s, id).appearance.stroke_paint() else { panic!() };
    let geom = sg.geom.unwrap();
    assert_eq!(geom.start, Point::new(200.0, 150.0));
    assert!((geom.aspect - 0.25).abs() < 1e-9 && (geom.length() - 100.5).abs() < 1e-9, "fits the stroke-inflated bounds: {geom:?}");
    // appearance.setItem takes the same gradient object.
    let g = three_stops();
    let mut q = gradient_param(&g);
    q["index"] = json!(0);
    s.execute("appearance.setItem", &q).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient, g.gradient);
}

// ---------- M3.15: gradients follow every transform ----------

fn fill_geom(s: &Session, id: NodeId) -> GradientGeom {
    fill_gradient(s, id).geom.expect("placed")
}

#[test]
fn rotate_reflect_and_scale_carry_unplaced_gradients() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 50.0);
    s.execute("paint.setFill", &json!({"gradient": {}})).unwrap();
    s.execute("object.move", &json!({"dx": 10, "dy": 0})).unwrap();
    assert!(fill_gradient(&s, id).geom.is_none(), "a move keeps refitting");
    s.execute("object.rotate", &json!({"angle": 90})).unwrap();
    let g = fill_geom(&s, id);
    assert!((g.end.x - g.start.x).abs() < 1e-9 && (g.length() - 100.0).abs() < 1e-9, "vertical after a 90° turn: {g:?}");
    s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
    let r = fill_geom(&s, id);
    // The vertical vector is centred on the object, so the reflection swaps its ends.
    assert!(r.start.distance(g.end) < 1e-9 && r.end.distance(g.start) < 1e-9, "{g:?} → {r:?}");

    // A circle's radial gradient becomes an ellipse under a non-uniform scale.
    let c = s.execute("shape.ellipse", &json!({"x": 300, "y": 300, "width": 100, "height": 100})).unwrap();
    let c = NodeId(c["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial"}})).unwrap();
    s.execute("object.scale", &json!({"sx": 200, "sy": 100})).unwrap();
    let e = fill_geom(&s, c);
    assert!((e.aspect - 0.5).abs() < 1e-9 && (e.length() - 100.0).abs() < 1e-9, "{e:?}");
}

#[test]
fn free_distort_maps_gradients_once() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {}})).unwrap();
    // An affine distort (a sheared parallelogram) maps the gradient exactly like object.transform.
    s.execute("object.distort", &json!({"corners": [[20, 0], [120, 0], [100, 100], [0, 100]]})).unwrap();
    let g = fill_geom(&s, id);
    let mut want = GradientGeom::fit(GradientKind::Linear, vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0), 0.0);
    want.transform(vectorcraft_geom::Affine::new([1.0, 0.0, -0.2, 1.0, 20.0, 0.0]), GradientKind::Linear);
    assert!(g.start.distance(want.start) < 1e-6 && g.end.distance(want.end) < 1e-6, "{g:?} vs {want:?}");
}

// ---------- M3.16: the active proxy and type objects ----------

fn text(s: &mut Session, x: f64, y: f64) -> NodeId {
    let r = s.execute("text.create", &json!({"x": x, "y": y, "text": "Gradient", "size": 40})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn text_obj(s: &Session, id: NodeId) -> &vectorcraft_doc::TextObject {
    match &node(s, id).kind {
        vectorcraft_doc::NodeKind::Text(t) => t,
        k => panic!("not text: {k:?}"),
    }
}

fn run_gradient(p: &Paint) -> &GradientPaint {
    match p {
        Paint::Gradient(g) => g,
        p => panic!("expected a gradient, got {p:?}"),
    }
}

#[test]
fn gradient_vector_follows_the_active_proxy_or_an_item_index() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.toggleActive", &json!({})).unwrap();
    s.execute("paint.setGradientGeom", &json!({"start": [0, 50], "end": [100, 50]})).unwrap();
    let ap = &node(&s, id).appearance;
    assert!(matches!(ap.fill_paint(), Paint::Solid { .. }), "the fill stays solid");
    let Paint::Gradient(g) = ap.stroke_paint() else { panic!("the stroke proxy is in front") };
    assert_eq!(g.geom.map(|g| (g.start, g.end)), Some((Point::new(0.0, 50.0), Point::new(100.0, 50.0))));
    // `index` targets one appearance item (item 0 is the fill), whichever proxy is in front.
    s.execute("paint.setGradientGeom", &json!({"start": [50, 0], "end": [50, 100], "index": 0})).unwrap();
    assert_eq!(fill_geom(&s, id).start, Point::new(50.0, 0.0));
    let err = s.execute("paint.setGradientGeom", &json!({"start": [0, 0], "end": [1, 1], "index": 7})).unwrap_err();
    assert!(err.to_string().contains("item 7"), "{err}");
}

#[test]
fn type_objects_take_the_vector_on_their_runs_in_text_space() {
    let mut s = session();
    let id = text(&mut s, 100.0, 200.0);
    s.execute("paint.setGradientGeom", &json!({"start": [100, 190], "end": [300, 190]})).unwrap();
    let t = text_obj(&s, id);
    let geom = run_gradient(&t.runs[0].style.fill).geom.unwrap();
    // Text space starts at the first baseline (100, 200).
    assert!(geom.start.distance(Point::new(0.0, -10.0)) < 1e-9 && geom.end.distance(Point::new(200.0, -10.0)) < 1e-9, "{geom:?}");
    // The annotator reads it back in document coordinates, and it turns with the text.
    let (_, doc) = node(&s, id).proxy_gradient(false, None).unwrap();
    assert!(doc.start.distance(Point::new(100.0, 190.0)) < 1e-9, "{doc:?}");
    s.execute("object.rotate", &json!({"angle": 90, "origin": [100, 200]})).unwrap();
    let (_, doc) = node(&s, id).proxy_gradient(false, None).unwrap();
    assert!(doc.start.distance(Point::new(90.0, 200.0)) < 1e-9 && doc.end.distance(Point::new(90.0, 0.0)) < 1e-9, "{doc:?}");
    // paint.setFill's vector is in document coordinates too.
    s.execute("paint.setFill", &json!({"gradient": {"start": [90, 200], "end": [90, 100]}})).unwrap();
    let (_, doc) = node(&s, id).proxy_gradient(false, None).unwrap();
    assert!(doc.end.distance(Point::new(90.0, 100.0)) < 1e-9, "{doc:?}");
}

#[test]
fn aspect_works_on_type_and_run_strokes_fit_the_inflated_box() {
    let mut s = session();
    let id = text(&mut s, 100.0, 200.0);
    s.execute("paint.editGradient", &json!({"kind": "radial", "aspect": 50, "stroke": false})).unwrap();
    let t = text_obj(&s, id);
    let lb = t.local_bounds();
    let geom = run_gradient(&t.runs[0].style.fill).geom.expect("an aspect places the gradient");
    assert!((geom.aspect - 0.5).abs() < 1e-9 && geom.start.distance(lb.center()) < 1e-9, "{geom:?} on {lb:?}");
    // A stroke gradient fits the layout box grown by half the run's stroke weight.
    s.execute("paint.setStroke", &json!({"gradient": {}})).unwrap();
    s.execute("paint.editGradient", &json!({"kind": "radial", "aspect": 200, "stroke": true})).unwrap();
    let t = text_obj(&s, id);
    let w = t.runs[0].style.stroke_width;
    let geom = run_gradient(&t.runs[0].style.stroke).geom.unwrap();
    assert!((geom.length() - (lb.width().max(lb.height()) + w) / 2.0).abs() < 1e-9, "{geom:?}");
    // The Stroke proxy's vector lands on the runs' strokes.
    s.execute("paint.setGradientGeom", &json!({"start": [100, 190], "end": [150, 190], "stroke": true})).unwrap();
    let (g, doc) = node(&s, id).proxy_gradient(true, None).unwrap();
    assert!(doc.end.distance(Point::new(150.0, 190.0)) < 1e-9 && (doc.aspect - 2.0).abs() < 1e-9, "aspect kept: {doc:?}");
    assert_eq!(g.gradient.kind, GradientKind::Radial);
}

// ---------- M3.17: the interactive annotator ----------

/// A 100 × 100 rectangle at (100, 100) with a horizontal fill gradient across it, and the
/// Gradient tool.
fn annotated() -> (Session, NodeId) {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"start": [100, 150], "end": [200, 150]}})).unwrap();
    s.select_tool("gradient", crate::ViewInfo::default()).unwrap();
    (s, id)
}

fn gesture(s: &mut Session, events: &[(PointerKind, f64, f64)]) {
    for (k, x, y) in events {
        s.pointer(&PointerEvent::new(*k, *x, *y), crate::ViewInfo::default()).unwrap();
    }
}

fn stop_count(s: &Session, id: NodeId) -> usize {
    fill_gradient(s, id).gradient.stops.len()
}

#[test]
fn select_stop_is_validated_and_shared() {
    let (mut s, _) = annotated();
    assert_eq!(s.execute("gradient.selectStop", &json!({"index": 1})).unwrap(), json!({"index": 1}));
    assert_eq!(s.selected_stop(), Some(1));
    assert!(s.execute("gradient.selectStop", &json!({"index": 2})).is_err(), "two stops");
    // A 32-bit build read 2^32 + 1 as stop 1.
    assert!(s.execute("gradient.selectStop", &json!({"index": (1u64 << 32) + 1})).is_err(), "past u32");
    assert!(s.execute("gradient.selectStop", &json!({"index": "a"})).is_err());
    assert!(s.execute("gradient.selectStop", &json!({})).is_err());
    s.execute("gradient.selectStop", &json!({"index": null})).unwrap();
    assert_eq!(s.selected_stop(), None);
    // The stroke proxy's paint is solid: nothing to select there.
    s.execute("paint.toggleActive", &json!({})).unwrap();
    assert!(s.execute("gradient.selectStop", &json!({"index": 0})).is_err());
}

#[test]
fn annotator_gestures_edit_the_gradient_as_single_undo_steps() {
    let (mut s, id) = annotated();
    // A click on the bar adds a stop there and selects it.
    gesture(&mut s, &[(PointerKind::Down, 130.0, 150.0), (PointerKind::Up, 130.0, 150.0)]);
    let stops = fill_gradient(&s, id).gradient.stops;
    assert_eq!(stops.len(), 3);
    assert!((stops[1].offset - 0.3).abs() < 1e-6);
    assert_eq!(s.selected_stop(), Some(1));
    // Dragging the end handle changes only the end, as one undo step.
    let undo = s.doc().unwrap().history.undo.len();
    gesture(
        &mut s,
        &[(PointerKind::Down, 200.0, 150.0), (PointerKind::Drag, 210.0, 160.0), (PointerKind::Drag, 220.0, 170.0), (PointerKind::Up, 220.0, 170.0)],
    );
    let g = fill_geom(&s, id);
    assert_eq!((g.start, g.end), (Point::new(100.0, 150.0), Point::new(220.0, 170.0)));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    // The selected stop takes Delete ahead of edit.clear; the object stays.
    let view = crate::ViewInfo::default();
    assert!(s.tool_claims_key(ToolKey::Delete, view));
    s.tool_key(ToolKey::Delete, Mods::default(), view).unwrap();
    assert_eq!(stop_count(&s, id), 2);
    assert_eq!(s.selected_stop(), Some(1));
    // Never below two stops.
    s.tool_key(ToolKey::Backspace, Mods::default(), view).unwrap();
    assert_eq!(stop_count(&s, id), 2);
    assert!(s.doc().unwrap().doc.node(id).is_some());
}

#[test]
fn the_selected_stop_belongs_to_its_gradient() {
    let (mut s, a) = annotated();
    let b = rect(&mut s, 300.0, 100.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {}})).unwrap();
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    s.execute("gradient.selectStop", &json!({"index": 1})).unwrap();
    let view = crate::ViewInfo::default();
    assert!(s.tool_claims_key(ToolKey::Delete, view));
    // Other art selected: no stop is selected there, so Delete is not the tool's.
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    assert_eq!(s.selected_stop(), None);
    assert!(!s.tool_claims_key(ToolKey::Delete, view));
    // Back on the art it was selected on, it is selected again; the other proxy has none.
    s.execute("select.set", &json!({"ids": [a.0]})).unwrap();
    assert_eq!(s.selected_stop(), Some(1));
    s.execute("paint.toggleActive", &json!({})).unwrap();
    assert_eq!(s.selected_stop(), None);
}

#[test]
fn swatches_and_new_art_fit_a_placed_gradient_to_themselves() {
    let mut s = session();
    rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let mut g = three_stops();
    g.gradient.kind = GradientKind::Radial;
    let mut q = gradient_param(&g);
    s.execute("paint.setFill", &q).unwrap();
    q["name"] = json!("Placed");
    s.execute("swatch.new", &q).unwrap();
    // New art drawn elsewhere fits the gradient to itself rather than to the first rectangle.
    let b = rect(&mut s, 500.0, 300.0, 100.0, 100.0);
    assert_eq!(fill_gradient(&s, b).geom, None);
    // Applying the swatch fits its gradient to each object, keeping the aspect it was saved with.
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setFill", &json!({"swatch": "Placed"})).unwrap();
    let geom = fill_geom(&s, b);
    assert_eq!(geom.start, Point::new(550.0, 350.0));
    assert!((geom.aspect - 0.5).abs() < 1e-9 && (geom.length() - 50.0).abs() < 1e-9, "{geom:?}");
    // On type, in text space.
    let t = text(&mut s, 100.0, 400.0);
    s.execute("paint.setFill", &json!({"swatch": "Placed"})).unwrap();
    let (_, doc) = node(&s, t).proxy_gradient(false, None).unwrap();
    let tb = node(&s, t).geometric_bounds().unwrap();
    assert!(doc.start.distance(tb.center()) < 1e-9 && (doc.aspect - 0.5).abs() < 1e-9, "{doc:?} on {tb:?}");
}

/// Dither (Illustrator 30's Gradient panel checkbox): set and cleared through the gradient
/// commands, refused on strokes (it applies to fills), kept by swatches, the params and a native
/// save and load.
#[test]
fn dither_is_a_fill_gradient_option_that_saves_and_loads() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    assert!(!fill_gradient(&s, id).gradient.dither, "off by default");
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("paint.editGradient", &json!({"dither": true})).unwrap();
    assert!(fill_gradient(&s, id).gradient.dither);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // The params carry it, so a swatch and a copied paint keep it.
    let g = fill_gradient(&s, id);
    assert_eq!(vectorcraft_tools::params::gradient_params(&g)["dither"], json!(true));
    let mut q = gradient_param(&g);
    q["name"] = json!("Smooth");
    s.execute("swatch.new", &q).unwrap();
    let Some(Paint::Gradient(sw)) = s.doc().unwrap().doc.swatch("Smooth").map(|w| w.paint.clone()) else { panic!("a gradient swatch") };
    assert!(sw.gradient.dither);
    // Saved and opened again.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Paint::Gradient(saved) = back.node(id).unwrap().appearance.fill_paint() else { panic!("a gradient fill") };
    assert!(saved.gradient.dither, "the native file keeps dither");
    // Written only when on, so files without it (older ones) open with it off.
    assert_eq!(serde_json::to_value(&saved.gradient).unwrap()["dither"], json!(true));
    assert!(serde_json::to_value(Gradient::default()).unwrap().get("dither").is_none());
    let old: Gradient = serde_json::from_value(json!({"kind": "Linear", "stops": []})).unwrap();
    assert!(!old.dither);
    s.execute("paint.editGradient", &json!({"dither": false})).unwrap();
    assert!(!fill_gradient(&s, id).gradient.dither);
    // Strokes: refused, nothing changed.
    s.execute("paint.setStroke", &json!({"gradient": {}})).unwrap();
    let before = node(&s, id).appearance.clone();
    assert!(s.execute("paint.editGradient", &json!({"dither": true, "stroke": true})).is_err());
    assert!(s.execute("paint.setStroke", &json!({"gradient": {"dither": true}})).is_err());
    assert!(s.execute("paint.setStroke", &json!({"swatch": null, "gradient": {"dither": true}})).is_err(), "a null swatch names none");
    assert!(s.execute("paint.editGradient", &json!({"dither": "yes"})).is_err());
    assert!(s.execute("paint.setFill", &json!({"gradient": {"dither": 1}})).is_err());
    assert_eq!(node(&s, id).appearance, before);
}

/// A stroke ignores dither, so one that holds it anyway (swapped from a fill) outlines to a fill
/// without it: Outline Stroke keeps the look.
#[test]
fn an_outlined_stroke_leaves_dither_behind() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setStroke", &json!({"gradient": {}})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"dither": true}})).unwrap();
    s.execute("paint.swap", &json!({})).unwrap();
    let Paint::Gradient(st) = node(&s, id).appearance.stroke_paint() else { panic!("a gradient stroke") };
    assert!(st.gradient.dither, "the swapped stroke keeps the option");
    s.execute("object.path.outlineStroke", &json!({})).unwrap();
    let mut fills = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let Paint::Gradient(g) = n.appearance.fill_paint() {
            fills.push(g.gradient.dither);
        }
    });
    assert_eq!(fills, [false, false], "the fill and the outline, neither dithered");
}

/// The Gradient panel's Method (Illustrator 30): `method` set and cleared through the gradient
/// commands, on fills and strokes, kept by the params, swatches and a native save and load;
/// anything but the four method names is refused without editing.
#[test]
fn method_is_a_gradient_option_that_saves_and_loads() {
    use vectorcraft_color::GradientMethod;
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let stops = json!([{"offset": 0, "color": "#0000ff"}, {"offset": 1, "color": "#ffff00"}]);
    s.execute("paint.setFill", &json!({"gradient": {"stops": stops}})).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient.method, GradientMethod::Classic, "classic by default");
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("paint.editGradient", &json!({"method": "perceptual"})).unwrap();
    let g = fill_gradient(&s, id);
    assert_eq!(g.gradient.method, GradientMethod::Perceptual);
    assert_eq!(g.gradient.stops.len(), 2, "the stops stay as they were");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
    // The params carry it, so a swatch and a copied paint keep it.
    assert_eq!(vectorcraft_tools::params::gradient_params(&g)["method"], json!("perceptual"));
    let mut q = gradient_param(&g);
    q["name"] = json!("Even");
    s.execute("swatch.new", &q).unwrap();
    let Some(Paint::Gradient(sw)) = s.doc().unwrap().doc.swatch("Even").map(|w| w.paint.clone()) else { panic!("a gradient swatch") };
    assert_eq!(sw.gradient.method, GradientMethod::Perceptual);
    // Saved and opened again.
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Paint::Gradient(saved) = back.node(id).unwrap().appearance.fill_paint() else { panic!("a gradient fill") };
    assert_eq!(saved.gradient.method, GradientMethod::Perceptual, "the native file keeps the method");
    // Strokes take it too.
    s.execute("paint.setStroke", &json!({"gradient": {"method": "perceptual"}})).unwrap();
    let Paint::Gradient(st) = node(&s, id).appearance.stroke_paint() else { panic!("a gradient stroke") };
    assert_eq!(st.gradient.method, GradientMethod::Perceptual);
    s.execute("paint.editGradient", &json!({"method": "classic", "stroke": true})).unwrap();
    let Paint::Gradient(st) = node(&s, id).appearance.stroke_paint() else { panic!("a gradient stroke") };
    assert_eq!(st.gradient.method, GradientMethod::Classic);
    // Bad values: refused, nothing changed.
    let before = node(&s, id).appearance.clone();
    for bad in [json!("smooth"), json!(1), json!(true)] {
        let e = s.execute("paint.editGradient", &json!({"method": bad})).unwrap_err().to_string();
        assert!(e.contains("`method` must be classic, perceptual, hue-clockwise or hue-counterclockwise"), "{e}");
        assert!(s.execute("paint.setFill", &json!({"gradient": {"method": bad}})).is_err());
    }
    assert_eq!(node(&s, id).appearance, before);
    s.execute("paint.editGradient", &json!({"method": "Classic", "stroke": false})).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient.method, GradientMethod::Classic);
    assert!(vectorcraft_tools::params::gradient_params(&fill_gradient(&s, id)).get("method").is_none());
    // With dither: one edit sets both, and the params carry both back.
    s.execute("paint.editGradient", &json!({"method": "perceptual", "dither": true, "stroke": false})).unwrap();
    let g = fill_gradient(&s, id);
    assert!(g.gradient.dither && g.gradient.method == GradientMethod::Perceptual);
    let p = vectorcraft_tools::params::gradient_params(&g);
    assert_eq!((&p["method"], &p["dither"]), (&json!("perceptual"), &json!(true)));
    s.execute("paint.setFill", &json!({"gradient": p})).unwrap();
    let back = fill_gradient(&s, id);
    assert!(back.gradient.dither && back.gradient.method == GradientMethod::Perceptual);
}

/// The hue methods: `method` hue-clockwise and hue-counterclockwise set through the gradient
/// commands on fills and strokes, as one undo step, kept by the params, a swatch, a saved preset
/// and a native save and load, and written with or without the hyphen.
#[test]
fn the_hue_methods_are_set_saved_and_loaded() {
    use vectorcraft_color::GradientMethod;
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let stops = json!([{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]);
    s.execute("paint.setFill", &json!({"gradient": {"stops": stops}})).unwrap();
    for (name, method, middle) in
        [("hue-clockwise", GradientMethod::HueClockwise, "#ff00ff"), ("hue-counterclockwise", GradientMethod::HueCounterclockwise, "#00ff00")]
    {
        let undo = s.doc().unwrap().history.undo.len();
        s.execute("paint.editGradient", &json!({"method": name})).unwrap();
        let g = fill_gradient(&s, id);
        assert_eq!(g.gradient.method, method);
        assert_eq!(g.gradient.stops.len(), 2, "{name}: the stops stay as they were");
        assert_eq!(g.gradient.sample(0.5).0.to_hex(), middle, "{name}: halfway around the wheel");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{name}: one undo step");
        // The params carry it, so a swatch, a preset and a copied paint keep it.
        let p = vectorcraft_tools::params::gradient_params(&g);
        assert_eq!(p["method"], json!(name));
        let mut q = gradient_param(&g);
        q["name"] = json!(format!("Wheel {name}"));
        s.execute("swatch.new", &q).unwrap();
        let Some(Paint::Gradient(sw)) = s.doc().unwrap().doc.swatch(&format!("Wheel {name}")).map(|w| w.paint.clone()) else {
            panic!("a gradient swatch")
        };
        assert_eq!(sw.gradient.method, method, "{name}: the swatch");
        s.execute("gradient.presets.save", &json!({"name": format!("Preset {name}")})).unwrap();
        s.execute("paint.editGradient", &json!({"method": "classic"})).unwrap();
        s.execute("gradient.presets.apply", &json!({"name": format!("Preset {name}")})).unwrap();
        assert_eq!(fill_gradient(&s, id).gradient.method, method, "{name}: the preset");
        // Saved and opened again.
        let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
        let Paint::Gradient(saved) = back.node(id).unwrap().appearance.fill_paint() else { panic!("a gradient fill") };
        assert_eq!(saved.gradient.method, method, "{name}: the native file keeps the method");
    }
    // Strokes take it too, and the name reads without the hyphen and in any case.
    for (name, method) in [("hue-clockwise", GradientMethod::HueClockwise), ("hue-counterclockwise", GradientMethod::HueCounterclockwise)] {
        for written in [name.replace('-', " "), name.replace('-', "").to_uppercase()] {
            s.execute("paint.setStroke", &json!({"gradient": {"method": "classic"}})).unwrap();
            s.execute("paint.editGradient", &json!({"method": written, "stroke": true})).unwrap();
            let Paint::Gradient(st) = node(&s, id).appearance.stroke_paint() else { panic!("a gradient stroke") };
            assert_eq!(st.gradient.method, method, "{written}");
        }
    }
    // Reverse gives the same ramp the other way round: the hue method turns with the stops, so
    // the middle keeps its color, unless the edit names a method too.
    s.execute("paint.editGradient", &json!({"method": "hue-clockwise", "stroke": false})).unwrap();
    let before = fill_gradient(&s, id).gradient;
    s.execute("paint.editGradient", &json!({"reverse": true, "stroke": false})).unwrap();
    let after = fill_gradient(&s, id).gradient;
    assert_eq!(after.method, GradientMethod::HueCounterclockwise);
    assert_eq!((after.stops[0].color.to_hex(), after.stops[1].color.to_hex()), ("#0000ff".to_string(), "#ff0000".to_string()));
    for t in [0.25, 0.5, 0.75] {
        assert_eq!(after.sample(t).0.to_hex(), before.sample(1.0 - t).0.to_hex(), "reversed at {t}");
    }
    s.execute("paint.editGradient", &json!({"reverse": true, "method": "hue-counterclockwise", "stroke": false})).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient.method, GradientMethod::HueCounterclockwise, "a method in the edit wins");
    // Half a name is refused, nothing changed.
    let before = node(&s, id).appearance.clone();
    for bad in ["hue", "clockwise", "hue-clock"] {
        let e = s.execute("paint.editGradient", &json!({"method": bad})).unwrap_err().to_string();
        assert!(e.contains("`method` must be classic, perceptual, hue-clockwise or hue-counterclockwise"), "{bad}: {e}");
    }
    assert_eq!(node(&s, id).appearance, before);
}

/// A conic gradient through `paint.setFill`, the Gradient panel's edits, transforms, the params
/// agents and swatches read, and the native format.
#[test]
fn a_conic_gradient_is_set_edited_transformed_and_saved() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "conic"}})).unwrap();
    let g = fill_gradient(&s, id);
    assert_eq!(g.gradient.kind, GradientKind::Conic);
    // Unplaced, it sweeps from the right about the box's center.
    let geom = g.resolve(node(&s, id).geometric_bounds().unwrap());
    assert_eq!(geom.start, Point::new(200.0, 150.0));
    assert!((geom.param_at(GradientKind::Conic, Point::new(200.0, 190.0)) - 0.25).abs() < 1e-9);
    // `angle` and `conical` name it too.
    for name in ["angle", "conical", "CONICAL"] {
        s.execute("paint.editGradient", &json!({"kind": "linear"})).unwrap();
        s.execute("paint.editGradient", &json!({"kind": name})).unwrap();
        assert_eq!(fill_gradient(&s, id).gradient.kind, GradientKind::Conic, "{name}");
    }
    let e = s.execute("paint.editGradient", &json!({"kind": "spiral"})).unwrap_err().to_string();
    assert!(e.contains("(linear, radial, conic, diamond, freeform)"), "{e}");
    // Placed, an angle turns the vector about the center.
    s.execute("paint.setGradientGeom", &json!({"start": [200, 150], "end": [260, 150], "aspect": 50})).unwrap();
    s.execute("paint.editGradient", &json!({"angle": 90})).unwrap();
    let g = fill_geom(&s, id);
    assert!(g.start.distance(Point::new(200.0, 150.0)) < 1e-9 && g.end.distance(Point::new(200.0, 90.0)) < 1e-9, "{g:?}");
    // A reflection mirrors the sweep (a negative aspect): each point keeps its color.
    let p = Point::new(230.0, 170.0);
    let before = g.param_at(GradientKind::Conic, p);
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    let m = fill_geom(&s, id);
    assert!(m.is_mirrored(), "{m:?}");
    assert!((m.param_at(GradientKind::Conic, Point::new(170.0, 170.0)) - before).abs() < 1e-9);
    // An aspect edit sets what it is given: the Gradient panel sends a mirrored conic's sign.
    s.execute("paint.editGradient", &json!({"aspect": -80})).unwrap();
    assert_eq!(fill_geom(&s, id).aspect, -0.8);
    s.execute("paint.setGradientGeom", &json!({"aspect": 40})).unwrap();
    assert_eq!(fill_geom(&s, id).aspect, 0.4);
    // A rotation carries it along.
    let g = fill_geom(&s, id);
    let before = g.param_at(GradientKind::Conic, p);
    s.execute("object.rotate", &json!({"angle": 30, "origin": [200, 150]})).unwrap();
    let r = vectorcraft_geom::Affine::rotate_about(-30f64.to_radians(), Point::new(200.0, 150.0));
    let after = fill_geom(&s, id).param_at(GradientKind::Conic, r * p);
    assert!((after - before).abs() < 1e-9, "{before} vs {after}");
    // The params give it back, and the native format keeps it.
    s.execute("paint.editGradient", &json!({"aspect": -60})).unwrap();
    let want = fill_gradient(&s, id);
    let params = vectorcraft_tools::params::gradient_params(&want);
    assert_eq!((params["kind"].as_str(), params["aspect"].as_f64()), (Some("conic"), Some(-60.0)));
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setFill", &json!({ "gradient": params })).unwrap();
    let got = fill_gradient(&s, id);
    assert_eq!((&got.gradient, got.geom), (&want.gradient, want.geom));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Paint::Gradient(g) = back.node(id).unwrap().appearance.fill_paint() else { panic!("the gradient is lost") };
    assert_eq!((g.gradient.kind, g.geom), (GradientKind::Conic, want.geom));
}

/// A free distort that is not affine moves a conic as it moves a radial gradient: by the warp
/// near its center, not near the middle of its vector.
#[test]
fn a_warp_moves_a_conic_by_its_center() {
    let warped = |kind: &str| {
        let mut s = session();
        let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        s.execute("paint.setFill", &json!({"gradient": {"kind": kind, "start": [20, 30], "end": [90, 30], "aspect": 60}})).unwrap();
        s.execute("object.distort", &json!({"corners": [[0, 0], [100, 0], [140, 130], [-20, 100]]})).unwrap();
        fill_geom(&s, id)
    };
    let (conic, radial, linear) = (warped("conic"), warped("radial"), warped("linear"));
    assert!(conic.start.distance(radial.start) < 1e-9, "{conic:?} vs {radial:?}");
    assert!(conic.start.distance(linear.start) > 0.1, "a linear gradient moves by its middle: {linear:?}");
}

/// A negative aspect mirrors a conic's sweep where the gradient is given one, and a mirrored
/// conic swatch lands mirrored on other art. Other kinds refuse a negative aspect, and a conic
/// refuses a focal point.
#[test]
fn a_negative_aspect_mirrors_only_a_conic() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "conic", "aspect": -50}})).unwrap();
    assert_eq!(fill_gradient(&s, id).geom.map(|g| g.aspect), Some(-0.5));
    s.execute("paint.setFill", &json!({"gradient": {"kind": "conic", "start": [0, 0], "end": [10, 0], "aspect": -200}})).unwrap();
    assert_eq!(fill_gradient(&s, id).geom.map(|g| g.aspect), Some(-2.0));
    let mut q = gradient_param(&fill_gradient(&s, id));
    q["name"] = json!("Mirrored");
    s.execute("swatch.new", &q).unwrap();
    let b = rect(&mut s, 500.0, 300.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"swatch": "Mirrored"})).unwrap();
    assert_eq!(fill_gradient(&s, b).geom.map(|g| g.aspect), Some(-2.0));
    // Radial (and linear) gradients have no mirror: a negative aspect is an error, whichever
    // command gives it, and the gradient stays as it was.
    let before = fill_gradient(&s, b);
    for kind in ["radial", "linear"] {
        let e = s.execute("paint.setFill", &json!({"gradient": {"kind": kind, "aspect": -50}})).unwrap_err().to_string();
        assert!(e.contains("aspect"), "{kind}: {e}");
        let e = s.execute("paint.setFill", &json!({"gradient": {"kind": kind, "start": [0, 0], "end": [10, 0], "aspect": -50}})).unwrap_err();
        assert!(e.to_string().contains("aspect"), "{kind}: {e}");
    }
    assert_eq!(fill_gradient(&s, b), before);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial", "aspect": 50}})).unwrap();
    let radial = fill_gradient(&s, b);
    assert!(s.execute("paint.editGradient", &json!({"aspect": -50})).is_err());
    assert!(s.execute("paint.setGradientGeom", &json!({"aspect": -50})).is_err());
    assert!(s.execute("paint.editGradient", &json!({"kind": "linear", "aspect": -50})).is_err());
    assert_eq!(fill_gradient(&s, b), radial);
    // Turned conic in the same edit, a negative aspect mirrors it.
    s.execute("paint.editGradient", &json!({"kind": "conic", "aspect": -50})).unwrap();
    assert_eq!(fill_gradient(&s, b).geom.map(|g| g.aspect), Some(-0.5));
    // A conic has no focal point: one given is an error, and null is allowed.
    let e = s.execute("paint.setFill", &json!({"gradient": {"kind": "conic", "start": [500, 300], "end": [550, 300], "focal": [510, 300]}}));
    assert!(e.unwrap_err().to_string().contains("focal"));
    s.execute("paint.setFill", &json!({"gradient": {"kind": "conic", "start": [500, 300], "end": [550, 300], "focal": null}})).unwrap();
    assert_eq!(fill_gradient(&s, b).geom.unwrap().focal, None);
    assert!(s.execute("paint.setGradientGeom", &json!({"focal": [510, 300]})).unwrap_err().to_string().contains("focal"));
    // A selected conic takes a negative aspect while the default paint is linear: the default
    // takes its size.
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("paint.editGradient", &json!({"kind": "conic"})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("select.set", &json!({"ids": [b.0]})).unwrap();
    s.execute("paint.editGradient", &json!({"aspect": -70})).unwrap();
    assert_eq!(fill_gradient(&s, b).geom.map(|g| g.aspect), Some(-0.7));
    // A linear gradient keeps no focal point, as paint.setGradientGeom leaves it.
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear", "start": [500, 300], "end": [550, 300], "focal": [510, 300]}})).unwrap();
    assert_eq!(fill_gradient(&s, b).geom.unwrap().focal, None);
}

/// Saved as `.ai`, as PDF with its editing data and as SVG with its editing data, then opened, a
/// mirrored, squashed conic comes back as it was; a PDF without editing data gives it back from
/// its sweep shading.
#[test]
fn a_conic_survives_saving_as_ai_pdf_and_svg() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "conic", "start": [180, 140], "end": [240, 110], "aspect": -60}})).unwrap();
    let want = fill_gradient(&s, id);
    let reopened = |s: &mut Session, format: &str, p: Value| {
        let mut p = p;
        p["format"] = json!(format);
        let r = s.execute("file.saveCopy", &p).unwrap();
        let mut back = session();
        back.execute("document.open", &json!({"name": format!("t.{format}"), "dataBase64": r["dataBase64"]})).unwrap();
        let mut found = None;
        back.doc().unwrap().doc.walk(|n| {
            if let (None, Paint::Gradient(g)) = (&found, n.appearance.fill_paint()) {
                found = Some(*g.clone());
            }
        });
        found.unwrap_or_else(|| panic!("{format}: the gradient is lost"))
    };
    for (format, p) in [("ai", json!({})), ("pdf", json!({"preserveEditing": true})), ("svg", json!({"preserveEditing": true}))] {
        let got = reopened(&mut s, format, p);
        assert_eq!((got.gradient.kind, got.geom), (GradientKind::Conic, want.geom), "{format}");
    }
    let got = reopened(&mut s, "pdf", json!({"preserveEditing": false}));
    assert_eq!(got.gradient.kind, GradientKind::Conic);
    let (g, w) = (got.geom.unwrap(), want.geom.unwrap());
    assert!(g.is_mirrored() && g.start.distance(w.start) < 0.05, "{g:?}");
    for p in [Point::new(120.0, 120.0), Point::new(280.0, 180.0), Point::new(200.0, 105.0)] {
        assert!((g.param_at(GradientKind::Conic, p) - w.param_at(GradientKind::Conic, p)).abs() < 2e-3, "{p:?}: {g:?}");
    }
}

/// Turning a gradient conic is one undo step, and undoing it gives the gradient back as it was.
#[test]
fn making_a_gradient_conic_is_one_undo_step() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial"}})).unwrap();
    let (before, steps) = (fill_gradient(&s, id), s.doc().unwrap().history.undo.len());
    s.execute("paint.editGradient", &json!({"kind": "conic", "angle": 45})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_gradient(&s, id), before);
}

/// A diamond gradient through `paint.setFill`, the Gradient panel's edits, transforms, the params
/// agents and swatches read, and the native format.
#[test]
fn a_diamond_gradient_is_set_edited_transformed_and_saved() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "diamond"}})).unwrap();
    let g = fill_gradient(&s, id);
    assert_eq!(g.gradient.kind, GradientKind::Diamond);
    // Unplaced, it runs from the box's center out to a square whose edges pass through the box's
    // corners.
    let geom = g.resolve(node(&s, id).geometric_bounds().unwrap());
    assert_eq!((geom.start, geom.end), (Point::new(200.0, 150.0), Point::new(350.0, 150.0)));
    for (p, t) in [
        (Point::new(300.0, 200.0), 1.0),
        (Point::new(100.0, 100.0), 1.0),
        (Point::new(250.0, 150.0), 1.0 / 3.0),
        (Point::new(200.0, 175.0), 1.0 / 6.0),
    ] {
        assert!((geom.param_at(GradientKind::Diamond, p) - t).abs() < 1e-9, "{p:?}");
    }
    s.execute("paint.editGradient", &json!({"kind": "linear"})).unwrap();
    s.execute("paint.editGradient", &json!({"kind": "DIAMOND"})).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient.kind, GradientKind::Diamond);
    let e = s.execute("paint.editGradient", &json!({"kind": "spiral"})).unwrap_err().to_string();
    assert!(e.contains("(linear, radial, conic, diamond, freeform)"), "{e}");
    // Placed, an angle turns the vector about the center.
    s.execute("paint.setGradientGeom", &json!({"start": [200, 150], "end": [260, 150], "aspect": 50})).unwrap();
    s.execute("paint.editGradient", &json!({"angle": 90})).unwrap();
    let g = fill_geom(&s, id);
    assert!(g.start.distance(Point::new(200.0, 150.0)) < 1e-9 && g.end.distance(Point::new(200.0, 90.0)) < 1e-9, "{g:?}");
    // A reflection keeps each point's color.
    let p = Point::new(230.0, 170.0);
    let before = g.param_at(GradientKind::Diamond, p);
    s.execute("object.reflect", &json!({"axis": "vertical"})).unwrap();
    let m = fill_geom(&s, id);
    assert!(m.aspect > 0.0, "{m:?}");
    assert!((m.param_at(GradientKind::Diamond, Point::new(170.0, 170.0)) - before).abs() < 1e-9);
    // A rotation carries it along.
    let before = m.param_at(GradientKind::Diamond, p);
    s.execute("object.rotate", &json!({"angle": 30, "origin": [200, 150]})).unwrap();
    let r = vectorcraft_geom::Affine::rotate_about(-30f64.to_radians(), Point::new(200.0, 150.0));
    let after = fill_geom(&s, id).param_at(GradientKind::Diamond, r * p);
    assert!((after - before).abs() < 1e-9, "{before} vs {after}");
    // The params give it back, and the native format keeps it.
    s.execute("paint.editGradient", &json!({"aspect": 60})).unwrap();
    let want = fill_gradient(&s, id);
    let params = vectorcraft_tools::params::gradient_params(&want);
    assert_eq!((params["kind"].as_str(), params["aspect"].as_f64()), (Some("diamond"), Some(60.0)));
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setFill", &json!({ "gradient": params })).unwrap();
    let got = fill_gradient(&s, id);
    assert_eq!((&got.gradient, got.geom), (&want.gradient, want.geom));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Paint::Gradient(g) = back.node(id).unwrap().appearance.fill_paint() else { panic!("the gradient is lost") };
    assert_eq!((g.gradient.kind, g.geom), (GradientKind::Diamond, want.geom));
}

/// A diamond refuses a negative aspect and a focal point, from every command that gives one, and
/// the gradient stays as it was; a null focal point is allowed.
#[test]
fn a_diamond_refuses_a_negative_aspect_and_a_focal_point() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "diamond", "start": [200, 150], "end": [260, 150], "aspect": 50}})).unwrap();
    let before = fill_gradient(&s, id);
    for p in [
        json!({"gradient": {"kind": "diamond", "aspect": -50}}),
        json!({"gradient": {"kind": "diamond", "start": [0, 0], "end": [10, 0], "aspect": -50}}),
    ] {
        assert!(s.execute("paint.setFill", &p).unwrap_err().to_string().contains("aspect"), "{p}");
    }
    assert!(s.execute("paint.editGradient", &json!({"aspect": -50})).is_err());
    assert!(s.execute("paint.setGradientGeom", &json!({"aspect": -50})).is_err());
    assert!(s.execute("paint.editGradient", &json!({"kind": "radial", "aspect": -50})).is_err());
    let e = s.execute("paint.setFill", &json!({"gradient": {"kind": "diamond", "start": [200, 150], "end": [260, 150], "focal": [210, 150]}}));
    assert!(e.unwrap_err().to_string().contains("focal"));
    assert!(s.execute("paint.setGradientGeom", &json!({"focal": [210, 150]})).unwrap_err().to_string().contains("focal"));
    assert_eq!(fill_gradient(&s, id), before);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "diamond", "start": [200, 150], "end": [260, 150], "focal": null}})).unwrap();
    assert_eq!(fill_gradient(&s, id).geom.unwrap().focal, None);
}

/// A free distort that is not affine moves a diamond by the warp near its center, as it moves a
/// radial gradient.
#[test]
fn a_warp_moves_a_diamond_by_its_center() {
    let warped = |kind: &str| {
        let mut s = session();
        let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
        s.execute("paint.setFill", &json!({"gradient": {"kind": kind, "start": [20, 30], "end": [90, 30], "aspect": 60}})).unwrap();
        s.execute("object.distort", &json!({"corners": [[0, 0], [100, 0], [140, 130], [-20, 100]]})).unwrap();
        fill_geom(&s, id)
    };
    let (diamond, radial) = (warped("diamond"), warped("radial"));
    assert!(diamond.start.distance(radial.start) < 1e-9, "{diamond:?} vs {radial:?}");
}

/// Saved as `.ai`, as PDF with its editing data and as SVG with its editing data, then opened, a
/// turned, squashed diamond comes back as it was.
#[test]
fn a_diamond_survives_saving_as_ai_pdf_and_svg() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "diamond", "start": [180, 140], "end": [240, 110], "aspect": 60}})).unwrap();
    let want = fill_gradient(&s, id);
    for format in ["ai", "pdf", "svg"] {
        let r = s.execute("file.saveCopy", &json!({"format": format, "preserveEditing": true})).unwrap();
        let mut back = session();
        back.execute("document.open", &json!({"name": format!("t.{format}"), "dataBase64": r["dataBase64"]})).unwrap();
        let mut found = None;
        back.doc().unwrap().doc.walk(|n| {
            if let (None, Paint::Gradient(g)) = (&found, n.appearance.fill_paint()) {
                found = Some(*g.clone());
            }
        });
        let got = found.unwrap_or_else(|| panic!("{format}: the gradient is lost"));
        assert_eq!((got.gradient.kind, got.geom), (GradientKind::Diamond, want.geom), "{format}");
    }
}

/// Turning a gradient diamond is one undo step, and undoing it gives the gradient back as it was.
#[test]
fn making_a_gradient_diamond_is_one_undo_step() {
    let mut s = session();
    let id = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial"}})).unwrap();
    let (before, steps) = (fill_gradient(&s, id), s.doc().unwrap().history.undo.len());
    s.execute("paint.editGradient", &json!({"kind": "diamond", "angle": 45})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_gradient(&s, id), before);
}

/// `paint.setFill` and `paint.editGradient` take a spread and a count for each kind of gradient
/// that has ends; the params give them back, the native format keeps them, and a change is one
/// undo step while an edit that changes nothing records none.
#[test]
fn a_gradient_takes_a_spread_and_a_count() {
    use vectorcraft_color::GradientSpread;
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    for kind in ["linear", "radial", "conic", "diamond"] {
        s.execute("paint.setFill", &json!({"gradient": {"kind": kind, "spread": "Reflect", "repeats": 4}})).unwrap();
        let g = fill_gradient(&s, id);
        assert_eq!((g.gradient.spread, g.gradient.repeats), (GradientSpread::Reflect, 4), "{kind}");
        let params = vectorcraft_tools::params::gradient_params(&g);
        assert_eq!((params["spread"].as_str(), params["repeats"].as_u64()), (Some("reflect"), Some(4)), "{kind}");
        s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
        assert_eq!(fill_gradient(&s, id).gradient.spread, GradientSpread::Pad, "a new gradient pads");
        s.execute("paint.setFill", &json!({ "gradient": params })).unwrap();
        assert_eq!(fill_gradient(&s, id).gradient, g.gradient, "{kind}");
    }
    let steps = s.doc().unwrap().history.undo.len();
    s.execute("paint.editGradient", &json!({"spread": "repeat", "repeats": 2})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "one step");
    let g = fill_gradient(&s, id);
    assert_eq!((g.gradient.spread, g.gradient.repeats), (GradientSpread::Repeat, 2));
    s.execute("paint.editGradient", &json!({"spread": "repeat", "repeats": 2})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), steps + 1, "nothing changed: no step");
    s.execute("paint.editGradient", &json!({"spread": "pad"})).unwrap();
    assert_eq!(fill_gradient(&s, id).gradient.repeats, 2, "Pad keeps the count for later");
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_gradient(&s, id), g);
    let back = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    let Paint::Gradient(saved) = back.node(id).unwrap().appearance.fill_paint() else { panic!("the gradient is lost") };
    assert_eq!((saved.gradient.spread, saved.gradient.repeats), (GradientSpread::Repeat, 2));
    // The params of a gradient that pads give back the count it keeps.
    s.execute("paint.editGradient", &json!({"spread": "pad"})).unwrap();
    let params = vectorcraft_tools::params::gradient_params(&fill_gradient(&s, id));
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("paint.setFill", &json!({ "gradient": params })).unwrap();
    assert_eq!((fill_gradient(&s, id).gradient.spread, fill_gradient(&s, id).gradient.repeats), (GradientSpread::Pad, 2), "{params}");
}

/// A spread or count a gradient can't take is an error, from every command that sets one, and the
/// gradient stays as it was: an unknown spread, a count that is not a whole number from 1 to 100,
/// and a spread other than pad on a freeform gradient.
#[test]
fn a_bad_spread_or_count_is_refused() {
    let mut s = session();
    let id = rect(&mut s, 100.0, 100.0, 200.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "radial", "spread": "repeat", "repeats": 3}})).unwrap();
    let before = fill_gradient(&s, id);
    let steps = s.doc().unwrap().history.undo.len();
    for bad in [json!({"spread": "mirror"}), json!({"spread": 1}), json!({"spread": true})] {
        let e = s.execute("paint.editGradient", &bad).unwrap_err().to_string();
        assert!(e.contains("`spread` must be pad, repeat or reflect"), "{bad}: {e}");
    }
    for n in [json!(0), json!(-2), json!(1.5), json!(101), json!(1e9), json!("3"), json!(f64::MAX)] {
        let e = s.execute("paint.editGradient", &json!({ "repeats": n })).unwrap_err().to_string();
        assert!(e.contains("`repeats` must be a whole number from 1 to 100"), "{n}: {e}");
        let e = s.execute("paint.setFill", &json!({"gradient": {"kind": "linear", "repeats": n}})).unwrap_err().to_string();
        assert!(e.contains("`repeats`"), "{n}: {e}");
    }
    let freeform = json!({"kind": "freeform", "spread": "reflect"});
    let e = s.execute("paint.editGradient", &freeform).unwrap_err().to_string();
    assert!(e.contains("`spread` applies to linear, radial, conic and diamond gradients"), "{e}");
    let e = s.execute("paint.setFill", &json!({ "gradient": freeform })).unwrap_err().to_string();
    assert!(e.contains("`spread` applies to"), "{e}");
    assert_eq!(fill_gradient(&s, id), before);
    assert_eq!(s.doc().unwrap().history.undo.len(), steps);
    // Pad on a freeform gradient is allowed (it has nothing to spread), and 100 is the most.
    s.execute("paint.editGradient", &json!({"repeats": 100})).unwrap();
    s.execute("paint.editGradient", &json!({"kind": "freeform", "spread": "pad"})).unwrap();
    // A repeating gradient turned freeform: its params, which carry no spread, set it again.
    s.execute("paint.editGradient", &json!({"kind": "radial", "spread": "reflect"})).unwrap();
    s.execute("paint.editGradient", &json!({"kind": "freeform"})).unwrap();
    let params = vectorcraft_tools::params::gradient_params(&fill_gradient(&s, id));
    assert!(params.get("spread").is_none(), "{params}");
    s.execute("paint.setFill", &json!({ "gradient": params })).unwrap();
}

/// A spread for a selected gradient applies while the gradient used last, the default, is a
/// freeform one, which keeps no spread.
#[test]
fn a_spread_for_a_selected_gradient_leaves_a_freeform_default_alone() {
    use vectorcraft_color::GradientSpread;
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "freeform"}})).unwrap();
    s.execute("paint.editGradient", &json!({"ids": [a.0], "spread": "repeat", "repeats": 3})).unwrap();
    let g = fill_gradient(&s, a);
    assert_eq!((g.gradient.spread, g.gradient.repeats), (GradientSpread::Repeat, 3));
}

/// A preset keeps its spread and count: saved from a gradient, applied to another object.
#[test]
fn a_gradient_preset_keeps_its_spread() {
    use vectorcraft_color::GradientSpread;
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "conic", "spread": "reflect", "repeats": 5}})).unwrap();
    s.execute("gradient.presets.save", &json!({"name": "Fan"})).unwrap();
    let b = rect(&mut s, 200.0, 0.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"kind": "linear"}})).unwrap();
    s.execute("gradient.presets.apply", &json!({"name": "Fan"})).unwrap();
    let g = fill_gradient(&s, b);
    assert_eq!((g.gradient.kind, g.gradient.spread, g.gradient.repeats), (GradientKind::Conic, GradientSpread::Reflect, 5));
    assert_eq!(fill_gradient(&s, a).gradient.repeats, 5);
}

/// A gradient set back to Pad keeps its count for later, and still matches a saved preset and a
/// swatch of the same gradient; while it repeats, the spread tells them apart.
#[test]
fn a_padding_gradient_matches_whatever_count_it_keeps() {
    let mut s = session();
    let a = rect(&mut s, 0.0, 0.0, 100.0, 100.0);
    let ramp = json!({"kind": "linear", "stops": [{"offset": 0, "color": "#ff0000"}, {"offset": 1, "color": "#0000ff"}]});
    s.execute("paint.setFill", &json!({ "gradient": ramp })).unwrap();
    s.execute("gradient.presets.save", &json!({"name": "Plain"})).unwrap();
    s.execute("swatch.new", &json!({"name": "Plain", "paint": {"gradient": ramp}})).unwrap();
    let unused = |s: &mut Session| s.execute("swatch.unused", &json!({})).unwrap()["names"].as_array().unwrap().contains(&json!("Plain"));
    assert!(!unused(&mut s));
    s.execute("paint.editGradient", &json!({"spread": "repeat", "repeats": 5})).unwrap();
    assert_eq!(s.active_saved_gradient_preset(), None, "a repeating gradient is another ramp");
    assert!(unused(&mut s), "nothing paints the swatch's gradient");
    s.execute("paint.editGradient", &json!({"spread": "pad"})).unwrap();
    assert_eq!(fill_gradient(&s, a).gradient.repeats, 5);
    assert_eq!(s.active_saved_gradient_preset(), Some("Plain"));
    assert!(!unused(&mut s), "the gradient paints as the swatch does");
}
