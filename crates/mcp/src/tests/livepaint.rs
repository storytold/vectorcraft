//! The Live Paint Bucket through MCP: `pointer_gesture` sends a triple-click as `tripleclick`.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool};

fn ok(h: &mut Headless, name: &str, args: Value) -> Value {
    let r = call_tool(h, name, &args);
    assert!(!r.is_error, "{name} {args}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    ok(h, "run_command", json!({"command": command, "params": params}))
}

/// The fill of each face of Live Paint group `g`, left to right.
fn fills(h: &mut Headless, g: &Value) -> Vec<Value> {
    let info = run(h, "livePaint.info", json!({"group": g}));
    let mut faces = info["faces"].as_array().unwrap().clone();
    faces.sort_by(|a, b| a["bounds"][0].as_f64().unwrap().total_cmp(&b["bounds"][0].as_f64().unwrap()));
    faces.iter().map(|f| f["fill"].clone()).collect()
}

#[test]
fn a_triple_click_fills_every_face_of_the_same_fill() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    let a = run(&mut h, "shape.rectangle", json!({"x": 0, "y": 0, "width": 50, "height": 50}))["id"].clone();
    let b = run(&mut h, "shape.rectangle", json!({"x": 100, "y": 0, "width": 50, "height": 50}))["id"].clone();
    let g = run(&mut h, "livePaint.make", json!({"ids": [a, b]}))["id"].clone();
    for x in [25, 125] {
        run(&mut h, "livePaint.fill", json!({"group": g, "point": [x, 25], "color": "#ff0000"}));
    }
    let red = fills(&mut h, &g);
    assert_eq!(red[0], red[1]);
    // With nothing selected, the current fill changes and the group keeps its faces' fills.
    run(&mut h, "select.none", json!({}));
    run(&mut h, "paint.setFill", json!({"color": "#00ff00"}));
    assert_eq!(fills(&mut h, &g), red);
    let ev = |kind: &str| json!({"kind": kind, "x": 25, "y": 25});
    let clicks = [ev("down"), ev("up"), ev("down"), ev("up"), ev("doubleclick"), ev("down"), ev("up")];
    ok(&mut h, "pointer_gesture", json!({"tool": "livePaintBucket", "events": clicks}));
    let after_two = fills(&mut h, &g);
    assert_eq!(after_two[1], red[1], "the face apart is left by the clicks and the double-click");
    assert_ne!(after_two[0], red[0]);
    ok(&mut h, "pointer_gesture", json!({"events": [ev("tripleclick")]}));
    assert_eq!(fills(&mut h, &g), [after_two[0].clone(), after_two[0].clone()], "both faces were red");
}
