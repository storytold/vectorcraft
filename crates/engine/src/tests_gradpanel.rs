//! Gradient panel and gradient behaviour: sampling a colour into a gradient stop (M3.33) and
//! copied appearances placing their gradients on the target's own bounds (M3.34) and the
//! Gradient tool's snapping and Eyedropper Options (M3.35).

use serde_json::json;
use vectorcraft_color::{GradientGeom, Paint};
use vectorcraft_geom::Point;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn rect(s: &mut Session, x: f64, y: f64, w: f64, h: f64) -> NodeId {
    let r = s.execute("shape.rectangle", &json!({"x": x, "y": y, "width": w, "height": h})).unwrap();
    NodeId(r["id"].as_u64().unwrap())
}

fn stop_hexes(s: &Session, id: NodeId, stroke: bool) -> Vec<String> {
    let a = &s.doc().unwrap().doc.node(id).unwrap().appearance;
    match if stroke { a.stroke_paint() } else { a.fill_paint() } {
        Paint::Gradient(g) => g.gradient.stops.iter().map(|s| s.color.to_hex()).collect(),
        p => panic!("expected a gradient, got {p:?}"),
    }
}

#[test]
fn sample_color_with_a_stop_recolours_that_stop_in_one_undo_step() {
    let mut s = session();
    let id = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
    // A solid fill: no stop to recolour.
    let err = s.execute("paint.sampleColor", &json!({"color": "#ff0000", "stop": 0})).unwrap_err();
    assert!(err.to_string().contains("not a gradient"), "{err}");
    s.execute("paint.setFill", &json!({"gradient": {}})).unwrap();
    s.execute("paint.sampleColor", &json!({"color": "#ff0000", "stop": 1})).unwrap();
    assert_eq!(stop_hexes(&s, id, false), ["#ffffff", "#ff0000"]);
    assert!(s.execute("paint.sampleColor", &json!({"color": "#ff0000", "stop": 2})).unwrap_err().to_string().contains("no stop 2"));
    // A 32-bit build read 2^32 + 1 as stop 1.
    assert!(s.execute("paint.sampleColor", &json!({"color": "#00ff00", "stop": (1u64 << 32) + 1})).is_err());
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(stop_hexes(&s, id, false), ["#ffffff", "#000000"]);
    // The stroke's gradient through `stroke`, the fill left alone.
    s.execute("paint.setStroke", &json!({"gradient": {}})).unwrap();
    s.execute("paint.sampleColor", &json!({"color": "#00ff00", "stop": 0, "stroke": true})).unwrap();
    assert_eq!(
        (stop_hexes(&s, id, true), stop_hexes(&s, id, false)),
        (vec!["#00ff00".into(), "#000000".into()], vec!["#ffffff".into(), "#000000".into()])
    );
}

fn fill_geom(s: &Session, id: NodeId) -> Option<GradientGeom> {
    match s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint() {
        Paint::Gradient(g) => g.geom,
        p => panic!("expected a gradient, got {p:?}"),
    }
}

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-9
}

/// A 100 × 100 source at (10, 10) with a placed gradient from 20 % to 80 % across its middle,
/// and a 200 × 100 target at x = 500.
fn source_and_target() -> (Session, NodeId, NodeId) {
    let mut s = session();
    let src = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"gradient": {"start": [30, 60], "end": [90, 60]}})).unwrap();
    let dst = rect(&mut s, 500.0, 10.0, 200.0, 100.0);
    (s, src, dst)
}

#[test]
fn the_eyedropper_places_copied_gradients_on_the_target() {
    let (mut s, src, dst) = source_and_target();
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [dst.0]})).unwrap();
    let g = fill_geom(&s, dst).expect("placed");
    assert!(close(g.start, Point::new(540.0, 60.0)) && close(g.end, Point::new(660.0, 60.0)), "{g:?}");
    // The source keeps its own; new art fits the gradient to itself.
    assert!(close(fill_geom(&s, src).unwrap().start, Point::new(30.0, 60.0)));
    assert!(matches!(&s.paint.fill, Paint::Gradient(g) if g.geom.is_none()));
    // The eyedropper tool's click runs the same command.
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    let ev = vectorcraft_tools::PointerEvent::new(vectorcraft_tools::PointerKind::Down, 50.0, 50.0);
    s.pointer(&ev, ViewInfo::default()).unwrap();
    assert!(close(fill_geom(&s, dst).unwrap().start, Point::new(540.0, 60.0)));
}

#[test]
fn graphic_styles_keep_gradients_relative_to_the_bounds() {
    let (mut s, src, dst) = source_and_target();
    s.execute("select.set", &json!({"ids": [src.0]})).unwrap();
    s.execute("graphicStyle.new", &json!({"name": "Glow"})).unwrap();
    let style = s.doc().unwrap().doc.graphic_styles.iter().find(|g| g.name == "Glow").cloned().unwrap();
    let Paint::Gradient(g) = style.appearance.fill_paint() else { panic!() };
    assert!(style.unit_box && close(g.geom.unwrap().start, Point::new(0.2, 0.5)), "stored in unit-box space: {g:?}");
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    s.execute("graphicStyle.apply", &json!({"name": "Glow"})).unwrap();
    let g = fill_geom(&s, dst).unwrap();
    assert!(close(g.start, Point::new(540.0, 60.0)) && close(g.end, Point::new(660.0, 60.0)), "{g:?}");
    // Both stay linked (each has the style's look on its own bounds); redefining the style from an
    // edited source moves the new placement onto the linked target's bounds too.
    let linked = |s: &mut Session| {
        let list = s.execute("graphicStyle.list", &json!({})).unwrap();
        list["styles"].as_array().unwrap().iter().find(|g| g["name"] == "Glow").unwrap()["linked"].clone()
    };
    assert_eq!(linked(&mut s), json!([src.0, dst.0]));
    s.execute("select.set", &json!({"ids": [src.0]})).unwrap();
    s.execute("paint.setFill", &json!({"gradient": {"start": [13.3, 47.7], "end": [97.1, 71.9]}})).unwrap();
    assert_eq!(linked(&mut s), json!([dst.0]), "the edited source is no longer linked");
    s.execute("graphicStyle.redefine", &json!({"name": "Glow"})).unwrap();
    let g = fill_geom(&s, dst).unwrap();
    assert!(close(g.start, Point::new(506.6, 47.7)) && close(g.end, Point::new(674.2, 71.9)), "{g:?}");
    assert_eq!(linked(&mut s), json!([src.0, dst.0]));
    s.execute("select.set", &json!({"ids": [dst.0]})).unwrap();
    // The flag survives a save; a style from an older file (no flag) keeps document coordinates.
    let bytes = vectorcraft_format::save(&s.doc().unwrap().doc, false);
    let back = vectorcraft_format::load(&bytes).unwrap();
    assert_eq!(back.graphic_styles, s.doc().unwrap().doc.graphic_styles);
    let legacy: vectorcraft_doc::GraphicStyle = serde_json::from_value(json!({"name": "Old", "appearance": style.appearance})).unwrap();
    assert!(!legacy.unit_box);
    s.edit("Add", |d, _| {
        d.graphic_styles.push(legacy);
        Ok(())
    })
    .unwrap();
    s.execute("graphicStyle.apply", &json!({"name": "Old"})).unwrap();
    assert!(close(fill_geom(&s, dst).unwrap().start, Point::new(0.2, 0.5)), "legacy styles are applied as stored");
}

#[test]
fn eyedropper_options_choose_what_copy_from_takes() {
    let mut s = session();
    let src = rect(&mut s, 10.0, 10.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("transparency.set", &json!({"opacity": 40})).unwrap();
    let dst = rect(&mut s, 200.0, 10.0, 100.0, 100.0);
    s.execute("paint.setFill", &json!({"color": "#ffffff"})).unwrap();
    let o = s.execute("eyedropper.setOptions", &json!({})).unwrap();
    assert_eq!((&o["sampleSize"], &o["pickUp"]["appearance"]["fill"]["color"]), (&json!(1), &json!(true)));
    let o = s.execute("eyedropper.setOptions", &json!({"pickUp": {"appearance": {"fill": false, "stroke": false}}})).unwrap();
    assert_eq!((&o["pickUp"]["appearance"]["fill"]["overprint"], &o["apply"]["appearance"]["fill"]["color"]), (&json!(false), &json!(true)));
    let n = |s: &Session| s.doc().unwrap().doc.node(dst).unwrap().clone();
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [dst.0]})).unwrap();
    assert_eq!((n(&s).opacity, n(&s).appearance.fill_paint().color().unwrap().to_hex()), (0.4, "#ffffff".into()));
    // Params override the options for one call.
    s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [dst.0], "pickUp": {"appearance": {"fill": true}}})).unwrap();
    assert_eq!(n(&s).appearance.fill_paint().color().unwrap().to_hex(), "#ff0000");
    // Nothing to copy: no edit.
    let undo = s.doc().unwrap().history.undo.len();
    let r = s.execute("appearance.copyFrom", &json!({"source": src.0, "ids": [dst.0], "apply": false})).unwrap();
    assert_eq!(r, json!({"ids": []}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
}

#[test]
fn the_gradient_tool_reads_the_constrain_angle_preference() {
    let mut s = session();
    rect(&mut s, 100.0, 100.0, 100.0, 100.0);
    s.execute("prefs.set", &json!({"key": "constrainAngle", "value": 30})).unwrap();
    s.select_tool("gradient", ViewInfo::default()).unwrap();
    let ev = |kind, x, y, shift| vectorcraft_tools::PointerEvent::new(kind, x, y).with_mods(vectorcraft_tools::Mods { shift, ..Default::default() });
    use vectorcraft_tools::PointerKind::{Down, Drag, Up};
    s.pointer(&ev(Down, 120.0, 180.0, false), ViewInfo::default()).unwrap();
    s.pointer(&ev(Drag, 190.0, 140.0, true), ViewInfo::default()).unwrap();
    s.pointer(&ev(Up, 190.0, 140.0, true), ViewInfo::default()).unwrap();
    let id = s.doc().unwrap().selection.objects[0];
    assert!((fill_geom(&s, id).unwrap().angle_deg() - 30.0).abs() < 1e-9);
}
