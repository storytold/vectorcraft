//! `press_key` with the Width and Puppet Warp tools: Delete and Backspace reach the selected width
//! point or pin, and clear the selected art when the tool has nothing selected. Cmd+A while the
//! Type tool edits selects the text, not the art.

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

fn exists(h: &Headless, id: u64) -> bool {
    h.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).is_some()
}

fn click(h: &mut Headless, tool: &str, x: f64, y: f64) {
    ok(h, "pointer_gesture", json!({"tool": tool, "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}));
}

fn session() -> Headless {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 400, "height": 400}})).unwrap();
    h
}

#[test]
fn delete_and_backspace_reach_the_width_tool() {
    for key in ["Delete", "Backspace"] {
        let mut h = session();
        let id = run(&mut h, "shape.line", json!({"x1": 100, "y1": 200, "x2": 300, "y2": 200}))["id"].as_u64().unwrap();
        run(&mut h, "stroke.set", json!({"weight": 10}));
        run(&mut h, "stroke.widthPoint.set", json!({"id": id, "t": 0.5, "left": 12, "right": 12}));
        click(&mut h, "width", 200.0, 200.0);
        let r = ok(&mut h, "press_key", json!({"key": key}));
        assert_eq!(r["handledBy"], "tool", "{key}: {r}");
        let n = h.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap().appearance.stroke().unwrap().profile.clone().unwrap();
        assert_eq!(n.points.len(), 2, "{key} removed the width point");
        // With no width point selected the key clears the selected line.
        ok(&mut h, "press_key", json!({"key": key}));
        assert!(!exists(&h, id), "{key} cleared the line");
    }
}

#[test]
fn delete_and_backspace_reach_the_puppet_warp_tool() {
    for key in ["Delete", "Backspace"] {
        let mut h = session();
        let id = run(&mut h, "shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 100}))["id"].as_u64().unwrap();
        ok(&mut h, "select_tool", json!({"tool": "puppetWarp"}));
        let pins = |h: &Headless| h.session.tool_options()["pins"].as_array().map_or(0, Vec::len);
        click(&mut h, "puppetWarp", 112.0, 188.0);
        let n = pins(&h);
        let r = ok(&mut h, "press_key", json!({"key": key}));
        assert_eq!(r["handledBy"], "tool", "{key}: {r}");
        assert!(exists(&h, id), "{key} kept the art");
        assert_eq!(pins(&h), n - 1, "{key} removed the pin");
        ok(&mut h, "press_key", json!({"key": key}));
        assert!(!exists(&h, id), "{key} cleared the art");
    }
}

#[test]
fn cmd_a_while_the_type_tool_edits_selects_the_text() {
    let mut h = session();
    run(&mut h, "shape.rectangle", json!({"x": 300, "y": 300, "width": 20, "height": 20}));
    let id = run(&mut h, "text.create", json!({"x": 100, "y": 100, "text": "Hello", "size": 20}))["id"].as_u64().unwrap();
    click(&mut h, "type", 102.0, 95.0);
    let r = ok(&mut h, "press_key", json!({"key": "A", "mods": {"cmd": true}}));
    assert_eq!(r["command"], "select.all");
    assert_eq!(r["result"], json!({"editing": id, "start": 0, "end": 5}));
    assert_eq!(h.session.doc().unwrap().selection.objects, [vectorcraft_doc::NodeId(id)], "the art selection stays");
}
