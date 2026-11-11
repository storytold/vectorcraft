use serde_json::json;
use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeId, NodeKind};
use vectorcraft_geom::Affine;

use crate::Session;

/// Create two embedded images and one ordinary rectangle.
fn fixture() -> (Session, [NodeId; 2], NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 100, "height": 100})).unwrap();

    let rect = s.execute("shape.rectangle", &json!({"x": 50, "y": 50, "width": 20, "height": 20})).unwrap();
    let rect_id = NodeId(rect["id"].as_u64().unwrap());

    let mut image_ids = Vec::new();

    s.edit("Create test images", |d, _| {
        let layer = d.layers[0].id;
        let key = "pixelated-test";

        d.images.insert(key.into(), ImageBlob::new("image/png", crate::tests_links::png(2, 2, [255, 0, 0])));

        for x in [0.0, 20.0] {
            let id = d.alloc_id();
            let image = ImageObject {
                key: key.into(),
                width: 2,
                height: 2,
                xf: Affine::translate((x, 0.0)),
                link: None,
                placement: Default::default(),
                pixels: None,
            };
            d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(image)))).unwrap();
            image_ids.push(id);
        }
        Ok(())
    })
    .unwrap();

    (s, [image_ids[0], image_ids[1]], rect_id)
}

fn pixelated(s: &Session, id: NodeId) -> bool {
    s.doc().unwrap().doc.node(id).unwrap().extras().pixelated
}

#[test]
fn pixelated_sets_multiple_images_and_undoes() {
    let (mut s, [a, b], _) = fixture();

    assert!(!pixelated(&s, a));
    assert!(!pixelated(&s, b));

    let before = s.doc().unwrap().history.undo.len();

    // Both images change in one operation.
    s.execute("object.setProps", &json!({"ids": [a.0, b.0], "pixelated": true})).unwrap();

    assert!(pixelated(&s, a));
    assert!(pixelated(&s, b));
    assert_eq!(s.doc().unwrap().history.undo.len(), before + 1);

    // Setting the same value does not add an undo step.
    let revision = s.doc().unwrap().revision;
    s.execute("object.setProps", &json!({"ids": [a.0, b.0], "pixelated": true})).unwrap();

    assert_eq!(s.doc().unwrap().revision, revision);
    assert_eq!(s.doc().unwrap().history.undo.len(), before + 1);

    // One Undo restores both images.
    s.execute("edit.undo", &json!({})).unwrap();
    assert!(!pixelated(&s, a));
    assert!(!pixelated(&s, b));

    s.execute("edit.redo", &json!({})).unwrap();
    assert!(pixelated(&s, a));
    assert!(pixelated(&s, b));

    // A single image can be switched back to smooth.
    s.execute("object.setProps", &json!({"id": a.0, "pixelated": false})).unwrap();

    assert!(!pixelated(&s, a));
    assert!(pixelated(&s, b));

    // Updating a mixed selection applies the same value to both images.
    s.execute("object.setProps", &json!({"ids": [b.0, a.0], "pixelated": true})).unwrap();

    assert!(pixelated(&s, a));
    assert!(pixelated(&s, b));
}

#[test]
fn pixelated_rejects_non_images_and_invalid_values() {
    let (mut s, [a, b], rect) = fixture();

    let revision = s.doc().unwrap().revision;
    let history_len = s.doc().unwrap().history.undo.len();

    // A mixed selection must not be partially modified.
    assert!(s.execute("object.setProps", &json!({"ids": [a.0, rect.0], "pixelated": true}),).is_err());

    // Invalid parameter types must also be rejected.
    for value in [json!("true"), json!(1)] {
        assert!(s.execute("object.setProps", &json!({"ids": [a.0, b.0], "pixelated": value}),).is_err());
    }

    assert!(!pixelated(&s, a));
    assert!(!pixelated(&s, b));
    assert_eq!(s.doc().unwrap().revision, revision);
    assert_eq!(s.doc().unwrap().history.undo.len(), history_len);
}
