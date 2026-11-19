//! Engine-only controls for explicit automatic Align references.

use serde_json::{Value, json};
use vectorcraft_doc::NodeId;
use vectorcraft_engine::Session;
use vectorcraft_geom::Point;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 300, "profile": null})).unwrap();
    s
}

fn align_with(s: &mut Session, mut params: Value, automatic: bool) {
    if automatic {
        params["to"] = json!("auto");
    }
    s.execute("object.align", &params).unwrap();
}

#[test]
fn automatic_align_keeps_one_objects_key_guide_reference() {
    for automatic in [false, true] {
        let mut s = session();
        let id = NodeId(s.execute("shape.rectangle", &json!({"x": 20, "y": 40, "width": 40, "height": 20})).unwrap()["id"].as_u64().unwrap());
        s.execute("guide.add", &json!({"vertical": false, "pos": 210})).unwrap();
        s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
        s.execute("guide.select", &json!({"indexes": [0], "toggle": true})).unwrap();
        s.execute("select.key", &json!({"guide": 0})).unwrap();
        let selection = s.doc().unwrap().selection.clone();
        assert_eq!(selection.len(), 1);
        assert!(selection.has_key());
        let guide = selection.key_guide.unwrap();

        align_with(&mut s, json!({"horizontal": "right", "vertical": "bottom", "bounds": "geometric"}), automatic);
        let bounds = s.doc().unwrap().doc.bounds_of(&[id], false).unwrap();
        assert_eq!(
            (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
            (20.0, 190.0, 60.0, 210.0),
            "the horizontal key guide overrides the one-object artboard rule and leaves x alone"
        );
        assert_eq!(s.doc().unwrap().doc.ruler_guide(guide).unwrap().pos, 210.0);
        assert_eq!(s.doc().unwrap().selection, selection, "reference resolution does not create or clear a key");
    }
}

#[test]
fn automatic_align_keeps_a_single_paths_selected_anchor_bounds() {
    for automatic in [false, true] {
        let mut s = session();
        let id = NodeId(s.execute("path.create", &json!({"d": "M20 40 L60 100 L100 40", "closed": false})).unwrap()["id"].as_u64().unwrap());
        s.execute("select.anchors", &json!({"id": id.0, "anchors": [[0, 0], [0, 1]], "mode": "set"})).unwrap();
        let selection = s.doc().unwrap().selection.clone();
        assert_eq!(selection.len(), 1);
        assert!(!selection.anchors.is_empty());
        assert!(!selection.has_key());

        align_with(&mut s, json!({"horizontal": "center", "bounds": "geometric"}), automatic);
        let document = &s.doc().unwrap().doc;
        let path = document.node(id).unwrap().path_data().unwrap();
        let points: Vec<Point> = path.subpaths.first().unwrap().anchors.iter().map(|anchor| anchor.p).collect();
        assert_eq!(
            points,
            [Point::new(40.0, 40.0), Point::new(40.0, 100.0), Point::new(100.0, 40.0)],
            "only picked anchors use their own center; the whole path is not centered on the artboard"
        );
        assert_eq!(s.doc().unwrap().selection, selection);
    }
}
