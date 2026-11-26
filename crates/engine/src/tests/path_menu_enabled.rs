//! Object → Path items that work on paths are disabled while only non-path objects are selected.

use serde_json::json;

use crate::*;

const PATH_ITEMS: [&str; 9] = [
    "path.reverse",
    "path.join",
    "path.average",
    "path.averageJoin",
    "object.path.simplify",
    "object.path.smooth",
    "object.path.addAnchorPoints",
    "object.path.divideObjectsBelow",
    "object.path.splitIntoGrid",
];

fn enabled(s: &Session, id: &str) -> bool {
    command_specs().iter().find(|c| c.id == id).is_some_and(|c| (c.enabled)(s).is_ok())
}

#[test]
fn path_items_need_a_path_selected() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s.execute("text.create", &json!({"x": 10, "y": 50, "text": "Hi"})).unwrap();
    for id in PATH_ITEMS {
        assert!(!enabled(&s, id), "{id} enabled for text");
    }
    s.execute("shape.rectangle", &json!({"x": 10, "y": 10, "width": 50, "height": 50})).unwrap();
    for id in PATH_ITEMS {
        assert!(enabled(&s, id), "{id} disabled for a path");
    }
}
