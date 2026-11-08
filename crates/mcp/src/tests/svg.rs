//! SVG Options through the MCP `export` tool: format options pass through to the engine.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool};

fn svg_of(h: &mut Headless, args: Value) -> String {
    let r = call_tool(h, "export", &args);
    assert!(!r.is_error, "{r:?}");
    let v: Value = serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap();
    String::from_utf8(vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()).unwrap()
}

#[test]
fn export_tool_takes_svg_options() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 100, "height": 50}})).unwrap();
    h.call("engine.execute", json!({"command": "shape.rectangle", "params": {"x": 10, "y": 10, "width": 20, "height": 20}})).unwrap();
    let plain = svg_of(&mut h, json!({"format": "svg"}));
    assert!(!plain.contains("<style>"));
    assert!(svg_of(&mut h, json!({"format": "svg", "options": {"styling": "css"}})).contains("<style>"));
    assert!(svg_of(&mut h, json!({"format": "svg", "options": {"svg": {"styling": "css", "responsive": true}}})).contains("<style>"));
    let r = call_tool(&mut h, "export", &json!({"format": "svg", "options": {"svg": {"styling": "fancy"}}}));
    assert!(r.is_error, "junk options are rejected");
}
