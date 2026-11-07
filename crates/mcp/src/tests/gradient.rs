//! The Gradient tool's annotator through MCP: `pointer_gesture` edits stops and the vector,
//! `press_key` reaches the selected stop.

use serde_json::{Value, json};
use vectorcraft_doc::color::{GradientPaint, Paint};
use vectorcraft_geom::Point;

use crate::{Backend, Headless, call_tool};

fn ok(h: &mut Headless, name: &str, args: Value) -> Value {
    let r = call_tool(h, name, &args);
    assert!(!r.is_error, "{name} {args}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

/// The fill gradient of the selected object.
fn gradient(h: &Headless) -> GradientPaint {
    let st = h.session.doc().unwrap();
    match st.doc.node(st.selection.objects[0]).unwrap().appearance.fill_paint() {
        Paint::Gradient(g) => *g,
        p => panic!("not a gradient: {p:?}"),
    }
}

fn offsets(g: &GradientPaint) -> Vec<f32> {
    g.gradient.stops.iter().map(|s| (s.offset * 100.0).round()).collect()
}

#[test]
fn gradient_annotator_end_to_end() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    ok(&mut h, "draw_shape", json!({"shape": "rectangle", "x": 100, "y": 100, "width": 100, "height": 100}));
    ok(&mut h, "run_command", json!({"command": "paint.setFill", "params": {"gradient": {"start": [100, 150], "end": [200, 150]}}}));
    // A click on the bar adds a stop at 30 %.
    let click = json!({"tool": "gradient", "events": [{"kind": "down", "x": 130, "y": 150}, {"kind": "up", "x": 130, "y": 150}]});
    ok(&mut h, "pointer_gesture", click);
    assert_eq!(offsets(&gradient(&h)), vec![0.0, 30.0, 100.0]);
    // Drag the new stop (its chip sits 10 px under the bar) to 60 %.
    let drag = json!({"events": [{"kind": "down", "x": 130, "y": 160}, {"kind": "drag", "x": 145, "y": 160}, {"kind": "drag", "x": 160, "y": 160}, {"kind": "up", "x": 160, "y": 160}]});
    ok(&mut h, "pointer_gesture", drag);
    assert_eq!(offsets(&gradient(&h)), vec![0.0, 60.0, 100.0]);
    // The dragged stop stays selected: Delete removes it (and leaves the rectangle).
    let r = ok(&mut h, "press_key", json!({"key": "Delete"}));
    assert_eq!(r["handledBy"], "tool", "{r}");
    assert_eq!(offsets(&gradient(&h)), vec![0.0, 100.0]);
    // The end handle changes only the end.
    let end = json!({"events": [{"kind": "down", "x": 200, "y": 150}, {"kind": "drag", "x": 230, "y": 120}, {"kind": "up", "x": 230, "y": 120}]});
    ok(&mut h, "pointer_gesture", end);
    let g = gradient(&h).geom.unwrap();
    assert_eq!((g.start, g.end), (Point::new(100.0, 150.0), Point::new(230.0, 120.0)));
    // Double-clicking a stop asks for its popover.
    let dbl = ok(&mut h, "pointer_gesture", json!({"events": [{"kind": "doubleclick", "x": 100, "y": 160}]}));
    assert_eq!(dbl["requests"][0]["dialog"], "gradientStop", "{dbl}");
}
