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
        json!({"kind": "conic"}),
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
/// anything but classic or perceptual is refused without editing.
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
        assert!(e.contains("`method` must be classic or perceptual"), "{e}");
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
