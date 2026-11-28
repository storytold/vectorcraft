//! The Gradient tool on a freeform gradient through MCP: `pointer_gesture` adds, moves and
//! removes points, drags the spread and draws lines; `press_key` Escape ends a line.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool};

fn ok(h: &mut Headless, name: &str, args: Value) -> Value {
    let r = call_tool(h, name, &args);
    assert!(!r.is_error, "{name} {args}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

fn freeform(h: &mut Headless) -> Value {
    ok(h, "run_command", json!({"command": "paint.freeform.get"}))
}

fn click(h: &mut Headless, x: f64, y: f64) -> Value {
    ok(h, "pointer_gesture", json!({"tool": "gradient", "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}))
}

fn drag(h: &mut Headless, from: [f64; 2], to: [f64; 2]) {
    let mid = [(from[0] + to[0]) / 2.0, (from[1] + to[1]) / 2.0];
    let ev = |k: &str, p: [f64; 2]| json!({"kind": k, "x": p[0], "y": p[1]});
    ok(h, "pointer_gesture", json!({"events": [ev("down", from), ev("drag", mid), ev("drag", to), ev("up", to)]}));
}

#[test]
fn freeform_points_and_lines_on_the_canvas() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    ok(&mut h, "draw_shape", json!({"shape": "rectangle", "x": 100, "y": 100, "width": 100, "height": 100}));
    ok(&mut h, "run_command", json!({"command": "paint.editGradient", "params": {"kind": "freeform", "mode": "lines"}}));
    let n = freeform(&mut h)["points"].as_array().unwrap().len();
    // Lines mode: three clicks draw one line through three new points.
    for (x, y) in [(110.0, 190.0), (150.0, 170.0), (190.0, 190.0)] {
        click(&mut h, x, y);
    }
    let f = freeform(&mut h);
    assert_eq!(f["lines"], json!([[n, n + 1, n + 2]]), "{f}");
    // Escape ends it: the next click starts afresh.
    let r = ok(&mut h, "press_key", json!({"key": "Escape"}));
    assert_eq!(r["handledBy"], "tool", "{r}");
    click(&mut h, 150.0, 120.0);
    assert_eq!(freeform(&mut h)["lines"].as_array().unwrap().len(), 1);
    // Dragging the selected point's spread handle (16 px right of it at 100 %) spreads it.
    drag(&mut h, [166.0, 120.0], [176.0, 120.0]);
    let f = freeform(&mut h);
    let spread = f["points"][n + 3]["spread"].as_f64().unwrap();
    assert!((spread - 0.2).abs() < 1e-6, "10 pt of half the 100 pt box: {f}");
    // A point dragged out of the rectangle goes.
    drag(&mut h, [150.0, 120.0], [40.0, 40.0]);
    assert_eq!(freeform(&mut h)["points"].as_array().unwrap().len(), n + 3);
    // Double-clicking a point asks for its popover.
    let dbl = ok(&mut h, "pointer_gesture", json!({"events": [{"kind": "doubleclick", "x": 150, "y": 170}]}));
    assert_eq!(dbl["requests"][0]["dialog"], "gradientStop", "{dbl}");
    assert_eq!(freeform(&mut h)["selected"], json!(n + 1));
}
