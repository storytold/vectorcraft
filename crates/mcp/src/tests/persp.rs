//! The Perspective Selection tool over MCP: a drag with 5 pressed half-way moves perpendicular to
//! the plane, the arrows nudge in perspective.

use serde_json::{Value, json};
use vectorcraft_tools::distort::perspective::{self as persp, PerspectiveGrid, Plane};

use crate::{Headless, call_tool};

fn text(r: &crate::ToolResult) -> Value {
    assert!(!r.is_error, "{r:?}");
    serde_json::from_str(r.content[0]["text"].as_str().unwrap()).unwrap()
}

fn run(h: &mut Headless, command: &str, params: Value) -> Value {
    text(&call_tool(h, "run_command", &json!({"command": command, "params": params})))
}

#[test]
fn perspective_selection_takes_5_and_the_arrows_over_mcp() {
    let mut h = Headless::with_document();
    run(&mut h, "perspective.grid.preset", json!({"kind": 2}));
    let id = run(&mut h, "shape.rectangle", json!({"x": 330, "y": 500, "width": 60, "height": 60}))["id"].as_u64().unwrap();
    run(&mut h, "perspective.attach", json!({"ids": [id], "plane": "right"}));
    let depth = |h: &Headless| persp::attachment(h.session.doc().unwrap().doc.node(vectorcraft_doc::NodeId(id)).unwrap()).unwrap();
    let doc = &h.session.doc().unwrap().doc;
    let g = PerspectiveGrid::from_doc(doc).unwrap();
    let (_, _, rect) = g.plane_bounds(doc, &[vectorcraft_doc::NodeId(id)]).unwrap();
    let from = g.homography_at(Plane::Right, 0.0).unwrap().apply(rect.center()).unwrap();
    let to = g.homography_at(Plane::Right, 30.0).unwrap().apply(rect.center()).unwrap();
    let ev = |kind: &str, p: vectorcraft_geom::Point| json!({"kind": kind, "x": p.x, "y": p.y});
    text(&call_tool(&mut h, "pointer_gesture", &json!({"tool": "perspectiveSelection", "events": [ev("down", from), ev("drag", to)]})));
    assert_eq!(text(&call_tool(&mut h, "press_key", &json!({"key": "5"})))["handledBy"], "tool");
    text(&call_tool(&mut h, "pointer_gesture", &json!({"events": [ev("up", to)]})));
    let (plane, d) = depth(&h);
    assert_eq!(plane, Plane::Right);
    assert!((d - 30.0).abs() < 1e-6, "{d}");
    assert_eq!(text(&call_tool(&mut h, "press_key", &json!({"key": "Right"})))["handledBy"], "tool");
    assert_eq!(h.session.journal.last().unwrap().0, "perspective.nudge");
}
