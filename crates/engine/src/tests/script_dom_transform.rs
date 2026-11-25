//! The script object model: a page item's `translate`, `rotate`, `resize` and `transform`.

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

/// A 400 × 300 pt document with a 30 × 40 pt rectangle and a 50 × 50 pt one above it.
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

/// The numbers of a logged array, or a logged number as one.
fn numbers(v: &Value) -> Vec<f64> {
    match v.as_array() {
        Some(a) => a.iter().map(|n| n.as_f64().unwrap()).collect(),
        None => vec![v.as_f64().unwrap()],
    }
}

fn close(a: &[f64], b: &[f64]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6)
}

#[test]
fn translate_moves_by_the_offsets_with_y_up_in_one_undo_step() {
    let (mut s, a, _) = doc();
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var i = app.activeDocument.pageItems[0]; var l = i.left, t = i.top;\
         i.translate(10, 0); vc.log([i.left - l, i.top - t]);\
         i.translate(0, 10); vc.log([i.left - l, i.top - t]);\
         app.coordinateSystem = CoordinateSystem.ARTBOARDCOORDINATESYSTEM; var t2 = i.top;\
         i.translate(0, 5); vc.log(i.top - t2);",
    );
    assert!(close(&numbers(&out[0]), &[10.0, 0.0]), "{out:?}");
    assert!(close(&numbers(&out[1]), &[10.0, 10.0]), "{out:?}");
    // With Y down from the artboard's top, a move up reads as a smaller top.
    assert!(close(&numbers(&out[2]), &[-5.0]), "{out:?}");
    assert_eq!(undo_steps(&s), steps + 1, "one undo step for the run");
    s.execute("edit.undo", &json!({})).unwrap();
    let r = s.active().unwrap().doc.bounds_of(&[a], false).unwrap();
    assert!(close(&[r.x0, r.y0], &[10.0, 20.0]), "{r:?}");
}

#[test]
fn rotate_turns_counterclockwise_about_the_center_or_the_named_point() {
    let (mut s, a, b) = doc();
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("select.none", &json!({})).unwrap();
    let out = log(
        &mut s,
        "var g = app.activeDocument.pageItems[0], r = g.geometricBounds; vc.log(g.typename);\
         vc.log([g.width, g.height, (r[0] + r[2]) / 2, (r[1] + r[3]) / 2]);\
         g.rotate(90); r = g.geometricBounds; vc.log([g.width, g.height, (r[0] + r[2]) / 2, (r[1] + r[3]) / 2]);",
    );
    assert_eq!(out[0], json!("GroupItem"));
    let (before, after) = (numbers(&out[1]), numbers(&out[2]));
    // The width and height trade places, and the center stays.
    assert!(close(&after, &[before[1], before[0], before[2], before[3]]), "{before:?} {after:?}");

    // A positive angle about the bottom-left corner swings a rectangle to the left of that corner.
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var i = app.activeDocument.pageItems[0]; vc.log(i.geometricBounds);\
         i.rotate(90, true, true, true, true, Transformation.BOTTOMLEFT); vc.log(i.geometricBounds);",
    );
    let (b0, b1) = (numbers(&out[0]), numbers(&out[1]));
    // [left, top, right, bottom], Y up: the 30 pt wide side now stands up, and the 40 pt tall side
    // lies to the left.
    assert!(close(&b1, &[b0[0] - 40.0, b0[3] + 30.0, b0[0], b0[3]]), "{b0:?} {b1:?}");
}

#[test]
fn resize_scales_about_the_center_and_line_widths_follow_their_argument() {
    let (mut s, _, _) = doc();
    let out = log(
        &mut s,
        "var p = app.activeDocument.pageItems, i = p[0], j = p[1];\
         function stroke(o) { return (o.visibleBounds[2] - o.visibleBounds[0]) - o.width; }\
         var r = i.geometricBounds, s0 = stroke(i); i.resize(50, 50); var q = i.geometricBounds;\
         vc.log([i.width, i.height, (q[0] + q[2]) - (r[0] + r[2]), (q[1] + q[3]) - (r[1] + r[3]), stroke(i) - s0]);\
         var s1 = stroke(j); j.resize(50, 50, true, true, true, true, 50); vc.log([s0, stroke(j) / s1]);",
    );
    let first = numbers(&out[0]);
    assert!(close(&first, &[15.0, 20.0, 0.0, 0.0, 0.0]), "{first:?}");
    let second = numbers(&out[1]);
    assert!(second[0] > 0.0, "the rectangles have a stroke: {second:?}");
    assert!(close(&second[1..], &[0.5]), "{second:?}");
    // A line width that is neither 100 nor the scale itself is refused.
    let e = failure(&mut s, "app.activeDocument.pageItems[0].resize(50, 50, true, true, true, true, 70);");
    assert!(e.contains("changeLineWidths") && e.contains("not available yet"), "{e}");
}

#[test]
fn transform_applies_a_matrix_record_about_the_center() {
    let (mut s, _, _) = doc();
    let steps = undo_steps(&s);
    let out = log(
        &mut s,
        "var i = app.activeDocument.pageItems[0], r = i.geometricBounds;\
         i.transform(app.getRotationMatrix(45)); var q = i.geometricBounds;\
         vc.log([i.width, i.height, (q[0] + q[2]) - (r[0] + r[2]), (q[1] + q[3]) - (r[1] + r[3])]);\
         var l = i.left; i.transform(app.getTranslationMatrix(5, 0)); vc.log(i.left - l);",
    );
    let side = 70.0 / 2f64.sqrt();
    assert!(close(&numbers(&out[0]), &[side, side, 0.0, 0.0]), "{out:?}");
    assert!(close(&numbers(&out[1]), &[5.0]), "{out:?}");
    assert_eq!(undo_steps(&s), steps + 1);
}

#[test]
fn transforms_that_change_nothing_add_no_undo_step() {
    let (mut s, _, _) = doc();
    let steps = undo_steps(&s);
    log(
        &mut s,
        "var i = app.activeDocument.pageItems[0]; i.translate(); i.translate(0, 0); i.rotate(0); i.rotate(360);\
         i.resize(100, 100); i.transform(app.getIdentityMatrix()); i.translate(10, 0, false, false, false, false);\
         i.translate(7000000, 0, false, false, false, false);",
    );
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn locked_art_and_art_on_a_locked_artboard_are_refused() {
    let (mut s, a, b) = doc();
    let before = s.active().unwrap().doc.clone();
    let steps = undo_steps(&s);
    for call in ["translate(5, 0)", "rotate(30)", "resize(50, 50)", "transform(app.getScaleMatrix(200))"] {
        let e = failure(&mut s, &format!("var i = app.activeDocument.pageItems[0]; i.locked = true; i.{call};"));
        assert!(e.contains(&format!("cannot transform locked objects: {}", a.0)), "{call}: {e}");
    }
    assert_eq!(undo_steps(&s), steps);
    assert_eq!(*s.active().unwrap().doc, *before);
    // A group that holds a locked object is refused too.
    s.execute("select.set", &json!({"ids": [a.0, b.0]})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": true})).unwrap();
    let (before, steps) = (s.active().unwrap().doc.clone(), undo_steps(&s));
    let e = failure(&mut s, "app.activeDocument.pageItems[0].translate(5, 0);");
    assert!(e.contains("cannot transform locked objects"), "{e}");
    assert_eq!(undo_steps(&s), steps);
    assert_eq!(*s.active().unwrap().doc, *before);
    s.execute("object.setProps", &json!({"ids": [b.0], "locked": false})).unwrap();
    s.edit("Lock Artboard", |d, _| {
        d.artboards[0].locked = true;
        Ok(())
    })
    .unwrap();
    let steps = undo_steps(&s);
    let e = failure(&mut s, "app.activeDocument.pageItems[1].rotate(45);");
    assert!(e.contains("cannot transform objects on a locked artboard"), "{e}");
    assert_eq!(undo_steps(&s), steps);
}

#[test]
fn bad_arguments_throw_and_change_nothing() {
    let (mut s, _, _) = doc();
    let before = s.active().unwrap().doc.clone();
    let steps = undo_steps(&s);
    for (call, says) in [
        ("rotate('x')", "angle must be a finite number"),
        ("rotate()", "angle must be a finite number"),
        ("resize(50)", "scaleY must be a finite number"),
        ("resize(0, 50)", "must not be 0"),
        ("transform({})", "transformationMatrix must be a Matrix"),
        ("transform(app.getScaleMatrix(0))", "singular"),
        ("translate(1, 0, 'yes')", "transformObjects must be true or false"),
        ("translate(7000000, 0)", "past the canvas"),
        ("translate('x')", "deltaX must be a finite number"),
        ("translate(NaN)", "is not a finite number"),
        ("rotate(5, true, false, false)", "changeFillGradients different from changePositions"),
        ("rotate(5, true, true, true, false)", "changeFillPatterns and changeStrokePattern"),
        // Left out, the stroke pattern follows the Transform Pattern Tiles preference (off), and
        // gradients default to true.
        ("rotate(5, true, true)", "changeFillPatterns and changeStrokePattern"),
        ("translate(10, 0, false)", "transformFillGradients different from transformObjects"),
        ("rotate(5, true, true, true, true, 'Transformation.NOPE')", "Transformation value"),
        ("rotate(5, true, true, true, true, 3)", "Transformation value"),
        ("resize(1e9, 1e9)", "past the canvas"),
    ] {
        let e = failure(&mut s, &format!("app.activeDocument.pageItems[0].{call};"));
        assert!(e.contains(says), "{call}: {e}");
    }
    assert_eq!(undo_steps(&s), steps);
    assert_eq!(*s.active().unwrap().doc, *before);
    // A deleted object is stale before its arguments are used.
    let e = failure(&mut s, "var i = app.activeDocument.pageItems[0]; i.remove(); i.rotate(10);");
    assert!(e.contains("Stale page item"), "{e}");
}
