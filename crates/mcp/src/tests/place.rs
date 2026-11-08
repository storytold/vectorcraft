//! File → Place over MCP, headless: `file.place` through `run_command`, and the place cursor driven
//! by pointer gestures and keys.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool};

fn text(r: &crate::ToolResult) -> Value {
    assert!(!r.is_error, "{r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

/// A headless session with a 400×300 document, and a 30×20 PNG as base64.
fn setup() -> (Headless, String) {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 30, "height": 20}})).unwrap();
    let png = h.call("engine.execute", json!({"command": "document.serialize", "params": {"format": "png"}})).unwrap();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 400, "height": 300}})).unwrap();
    (h, png["dataBase64"].as_str().unwrap().to_string())
}

fn images(h: &Headless) -> usize {
    let mut n = 0;
    h.session.doc().unwrap().doc.walk(|x| n += usize::from(matches!(x.kind, vectorcraft_doc::NodeKind::Image(_))));
    n
}

#[test]
fn headless_run_command_file_place() {
    let (mut h, png) = setup();
    let r = text(&call_tool(
        &mut h,
        "run_command",
        &json!({"command": "file.place", "params": {"name": "tile.png", "dataBase64": png, "at": [100, 50]}}),
    ));
    assert_eq!((r["format"].as_str(), r["width"].as_f64(), r["height"].as_f64()), (Some("png"), Some(30.0), Some(20.0)), "{r}");
    assert_eq!(images(&h), 1);
    let sel = h.session.doc().unwrap().selection.objects.clone();
    assert_eq!(sel.len(), 1);
    assert_eq!(sel[0].0, r["ids"][0].as_u64().unwrap());
}

#[test]
fn headless_place_cursor_places_on_clicks_and_esc_ends_it() {
    let (mut h, png) = setup();
    let files = json!([{"name": "a.png", "dataBase64": png}, {"name": "b.png", "dataBase64": png}, {"name": "c.png", "dataBase64": png}]);
    let r = text(&call_tool(&mut h, "run_command", &json!({"command": "file.place.queue", "params": {"files": files}})));
    assert_eq!(r["count"], 3);
    assert_eq!(h.session.tool_id(), "place");
    let click = |x: f64| json!({"events": [{"kind": "down", "x": x, "y": 10}, {"kind": "up", "x": x, "y": 10}]});
    text(&call_tool(&mut h, "pointer_gesture", &click(10.0)));
    text(&call_tool(&mut h, "pointer_gesture", &click(60.0)));
    assert_eq!(images(&h), 2);
    let r = text(&call_tool(&mut h, "press_key", &json!({"key": "Escape"})));
    assert_eq!(r["handledBy"], "tool");
    assert_eq!(h.session.tool_id(), "selection", "the last file discarded: back to the previous tool");
    assert_eq!(images(&h), 2);
}
