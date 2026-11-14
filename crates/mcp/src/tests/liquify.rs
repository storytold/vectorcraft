//! The Liquify tools through MCP: their options (`tool.setOption`, kept across tool switches) and
//! brush strokes with `pointer_gesture`.

use serde_json::{Value, json};

use crate::{Headless, call_tool};

fn ok(h: &mut Headless, name: &str, args: Value) -> Value {
    let r = call_tool(h, name, &args);
    assert!(!r.is_error, "{name} {args}: {r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap_or(Value::Null)
}

fn command(h: &mut Headless, command: &str, params: Value) -> Value {
    ok(h, "run_command", json!({"command": command, "params": params}))
}

#[test]
fn tool_options_are_set_headless_and_kept_across_tool_switches() {
    let mut h = Headless::with_document();
    command(&mut h, "tool.select", json!({"tool": "warp"}));
    let o = command(&mut h, "tool.setOption", json!({"values": {"width": 64, "detail": 6}}));
    assert_eq!((o["width"].as_f64(), o["detail"].as_f64()), (Some(64.0), Some(6.0)));
    command(&mut h, "tool.select", json!({"tool": "bloat"}));
    let o = command(&mut h, "tool.setOption", json!({}));
    assert_eq!((o["tool"].as_str(), o["width"].as_f64(), o["detail"].as_f64()), (Some("bloat"), Some(64.0), Some(2.0)));
    command(&mut h, "tool.select", json!({"tool": "warp"}));
    assert_eq!(command(&mut h, "tool.setOption", json!({}))["detail"], json!(6.0));
}

/// A value the tool refuses is an error result naming the tool and the option, and the options
/// stay as they were, the good value given with it included.
#[test]
fn a_refused_option_value_is_an_error_result() {
    let mut h = Headless::with_document();
    command(&mut h, "tool.select", json!({"tool": "warp"}));
    let before = command(&mut h, "tool.setOption", json!({}));
    for values in [json!({"width": 64, "detail": 99}), json!({"width": 64, "detail": "fine"})] {
        let r = call_tool(&mut h, "run_command", &json!({"command": "tool.setOption", "params": {"values": values}}));
        assert!(r.is_error, "{r:?}");
        assert!(format!("{:?}", r.content).contains("tool `warp`: `detail` must be a number from 1 to 10"), "{r:?}");
    }
    assert_eq!(command(&mut h, "tool.setOption", json!({})), before);
}

#[test]
fn pointer_gestures_carry_pen_pressure() {
    let bloat = |pressure: f64| {
        let mut h = Headless::with_document();
        let id = command(&mut h, "shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 200}))["id"].clone();
        command(&mut h, "select.set", json!({"ids": []}));
        command(&mut h, "tool.select", json!({"tool": "bloat"}));
        command(&mut h, "tool.setOption", json!({"key": "usePressure", "value": true}));
        let ev = |kind: &str, y: f64| json!({"kind": kind, "x": 280, "y": y, "pressure": pressure});
        ok(&mut h, "pointer_gesture", json!({"events": [ev("down", 150.0), ev("drag", 200.0), ev("up", 250.0)]}));
        command(&mut h, "document.node", json!({"id": id}))
    };
    let (none, full) = (bloat(0.0), bloat(1.0));
    assert_eq!(none, bloat(0.0));
    assert_ne!(none, full);
}

#[test]
fn holding_still_through_mcp_and_skips_are_reported() {
    let pucker = |hold: u64| {
        let mut h = Headless::with_document();
        let id = command(&mut h, "shape.rectangle", json!({"x": 100, "y": 100, "width": 200, "height": 200}))["id"].clone();
        command(&mut h, "text.create", json!({"x": 290, "y": 200, "text": "Hi"}));
        command(&mut h, "select.set", json!({"ids": []}));
        let ev = |kind: &str| json!({"kind": kind, "x": 290, "y": 200, "holdMs": hold});
        let r = ok(&mut h, "pointer_gesture", json!({"tool": "pucker", "events": [ev("down"), ev("up")]}));
        (r, command(&mut h, "document.node", json!({"id": id})))
    };
    let (r, still) = pucker(0);
    assert!(r["requests"].as_array().unwrap().iter().any(|q| q["status"].as_str().is_some_and(|m| m.contains("type"))), "{r}");
    let (_, held) = pucker(500);
    assert_ne!(still, held);
    assert_eq!(held, pucker(500).1, "the same hold, the same result");
}
