//! A shape tool's double-click on a selected live shape's corner widget through MCP: the
//! gesture's `requests` carry the Transform panel.

use serde_json::{Value, json};
use vectorcraft_tools::corners::CornerWidgets;

use crate::{Backend, Headless, call_tool};

fn ok(h: &mut Headless, name: &str, args: Value) -> Value {
    let r = call_tool(h, name, &args);
    assert!(!r.is_error, "{name} {args}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

#[test]
fn a_corner_widget_double_click_with_a_shape_tool_requests_the_transform_panel() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 300, "height": 300}})).unwrap();
    ok(&mut h, "draw_shape", json!({"shape": "rectangle", "x": 100, "y": 100, "width": 100, "height": 100}));
    let st = h.session.doc().unwrap();
    let w = CornerWidgets::showing(&st.doc, &st.selection, h.view.zoom, false, true, 180.0).expect("corner widgets show");
    let at = w.visible().next().expect("a widget");
    let steps = st.history.undo.len();
    let dbl = ok(&mut h, "pointer_gesture", json!({"tool": "rectangle", "events": [{"kind": "doubleclick", "x": at.x, "y": at.y}]}));
    assert_eq!(dbl["requests"], json!([{"panel": "transform"}]), "{dbl}");
    assert_eq!(h.session.doc().unwrap().history.undo.len(), steps, "no undo step");
    // Off the widgets, the double-click asks for nothing.
    let off = ok(&mut h, "pointer_gesture", json!({"events": [{"kind": "doubleclick", "x": 280, "y": 280}]}));
    assert_eq!(off["requests"], json!([]), "{off}");
}
