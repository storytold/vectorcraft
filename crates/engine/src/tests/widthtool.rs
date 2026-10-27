//! The Width tool on compound paths: the stroke is the compound's, so its members are edited
//! through it, by the tool and by the width point commands given a member.

use serde_json::json;
use vectorcraft_tools::{PointerEvent, PointerKind};

use crate::*;

/// Two 200 pt lines made into a compound path with a 10 pt stroke → (compound, members).
fn compound() -> (Session, NodeId, Vec<NodeId>) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 500, "height": 500})).unwrap();
    let members: Vec<NodeId> = [200, 300]
        .iter()
        .map(|y| NodeId(s.execute("shape.line", &json!({"x1": 100, "y1": y, "x2": 300, "y2": y})).unwrap()["id"].as_u64().unwrap()))
        .collect();
    s.execute("select.set", &json!({"ids": [members[0].0, members[1].0]})).unwrap();
    let c = NodeId(s.execute("object.compoundPath.make", &json!({})).unwrap()["id"].as_u64().unwrap());
    s.execute("select.set", &json!({"ids": [c.0]})).unwrap();
    s.execute("stroke.set", &json!({"weight": 10})).unwrap();
    (s, c, members)
}

fn profile(s: &Session, id: NodeId) -> Option<Vec<(f64, f64, f64)>> {
    s.doc().unwrap().doc.node(id).unwrap().appearance.stroke().and_then(|st| st.profile.as_ref().map(|p| p.points.clone()))
}

#[test]
fn the_width_tool_edits_a_compound_paths_stroke() {
    let (mut s, c, members) = compound();
    assert!(s.doc().unwrap().doc.node(members[1]).is_some(), "members keep their ids");
    let v = ViewInfo::default();
    s.select_tool("width", v).unwrap();
    // Drag out from the second member's middle: 20 pt either side.
    for (k, y) in [(PointerKind::Down, 301.0), (PointerKind::Drag, 310.0), (PointerKind::Drag, 320.0), (PointerKind::Up, 320.0)] {
        s.pointer(&PointerEvent::new(k, 200.0, y), v).unwrap();
    }
    let p = profile(&s, c).expect("the compound's stroke has the width point");
    assert!(p.iter().any(|q| (q.0 - 0.5).abs() < 1e-3 && (q.1 - 4.0).abs() < 1e-6), "{p:?}");
    assert_eq!(s.journal.last().unwrap().1["id"], json!(c.0));
}

#[test]
fn width_point_commands_given_a_member_edit_the_compound() {
    let (mut s, c, members) = compound();
    s.execute("stroke.widthPoint.set", &json!({"id": members[0].0, "t": 0.25, "left": 10, "right": 10})).unwrap();
    assert_eq!(profile(&s, c).unwrap().len(), 3);
    s.execute("stroke.widthPoint.remove", &json!({"id": members[1].0, "index": 1})).unwrap();
    assert_eq!(profile(&s, c).unwrap().len(), 2);
    s.execute("stroke.widthProfile.set", &json!({"ids": [members[0].0, members[1].0], "points": [[0, 0, 0], [1, 2, 2]]})).unwrap();
    assert_eq!(profile(&s, c).unwrap(), vec![(0.0, 0.0, 0.0), (1.0, 2.0, 2.0)]);
}
