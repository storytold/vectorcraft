//! The Pen over MCP, headless: `pointer_gesture` with the modifiers held as on the desktop.

use serde_json::{Value, json};

use crate::{Backend, Headless, call_tool};

fn text(r: &crate::ToolResult) -> Value {
    assert!(!r.is_error, "{r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

/// A press at the first point, drags through the others, the release at the last.
fn gesture(pts: &[(f64, f64)]) -> Value {
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    let mut events = vec![json!({"kind": "down", "x": first.0, "y": first.1})];
    events.extend(pts[1..].iter().map(|(x, y)| json!({"kind": "drag", "x": x, "y": y})));
    events.push(json!({"kind": "up", "x": last.0, "y": last.1}));
    json!({ "events": events })
}

/// The anchors of each path, in paint order.
fn paths(h: &Headless) -> Vec<Vec<vectorcraft_geom::Anchor>> {
    let mut v = vec![];
    h.session.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            v.push(p.subpaths.iter().flat_map(|s| s.anchors.clone()).collect());
        }
    });
    v
}

/// #494: Cmd held at the press borrows Direct Selection from the Pen; the path goes on afterwards.
#[test]
fn cmd_with_the_pen_edits_the_path_being_drawn() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 800, "height": 600}})).unwrap();
    text(&call_tool(&mut h, "select_tool", &json!({"tool": "pen"})));
    text(&call_tool(&mut h, "pointer_gesture", &gesture(&[(100.0, 300.0)])));
    text(&call_tool(&mut h, "pointer_gesture", &gesture(&[(200.0, 300.0), (250.0, 300.0)])));
    let mut drag = gesture(&[(250.0, 300.0), (250.0, 280.0), (250.0, 263.0)]);
    drag["mods"] = json!({"cmd": true});
    let r = text(&call_tool(&mut h, "pointer_gesture", &drag));
    assert_eq!(r["tool"], "pen", "{r}");
    assert_eq!(paths(&h)[0][1].h_out, vectorcraft_geom::Point::new(250.0, 263.0));
    text(&call_tool(&mut h, "pointer_gesture", &gesture(&[(300.0, 350.0)])));
    assert_eq!(paths(&h).iter().map(Vec::len).collect::<Vec<_>>(), [3], "the same path, three anchors");
}

/// #501: a drag out of the first anchor closes the path, shaping the closing curve.
#[test]
fn a_drag_from_the_first_anchor_closes_the_path() {
    let mut h = Headless::new();
    h.call("engine.execute", json!({"command": "file.new", "params": {"width": 800, "height": 600}})).unwrap();
    text(&call_tool(&mut h, "select_tool", &json!({"tool": "pen"})));
    for p in [(100.0, 300.0), (200.0, 300.0), (200.0, 400.0)] {
        text(&call_tool(&mut h, "pointer_gesture", &gesture(&[p])));
    }
    let close: Vec<(f64, f64)> = (0..=12).map(|i| (100.0 + 5.0 * f64::from(i), 300.0 - 4.0 * f64::from(i))).collect();
    text(&call_tool(&mut h, "pointer_gesture", &gesture(&close)));
    let d = &h.session.doc().unwrap().doc;
    let mut closed = vec![];
    d.walk(|n| closed.extend(n.path_data().map(|p| p.is_closed())));
    assert_eq!(closed, [true], "one path, closed");
    let first = paths(&h)[0][0];
    assert_eq!((first.h_in, first.h_out), (vectorcraft_geom::Point::new(40.0, 348.0), vectorcraft_geom::Point::new(160.0, 252.0)));
}
