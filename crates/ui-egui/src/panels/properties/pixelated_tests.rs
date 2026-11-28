use super::*;
use crate::tests::panel_harness::Panel;
use vectorcraft_doc::{ImageBlob, ImageObject, NodeId};
use vectorcraft_engine::Session;
use vectorcraft_geom::Affine;

fn fixture() -> (VectorcraftApp, [NodeId; 2]) {
    let mut app = VectorcraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::new_rgba8(2, 2).write_to(&mut encoded, image::ImageFormat::Png).unwrap();
    let mut ids = Vec::new();
    app.session
        .edit("Create images", |doc, _| {
            doc.images.insert("pixelated-panel".into(), ImageBlob::new("image/png", encoded.into_inner()));
            let layer = doc.layers[0].id;
            for x in [0.0, 20.0] {
                let id = doc.alloc_id();
                let image = ImageObject {
                    key: "pixelated-panel".into(),
                    width: 2,
                    height: 2,
                    xf: Affine::translate((x, 0.0)),
                    link: None,
                    placement: Default::default(),
                    pixels: None,
                };
                doc.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(image)))).unwrap();
                ids.push(id);
            }
            Ok(())
        })
        .unwrap();
    app.run("select.set", json!({"ids": [ids[0].0, ids[1].0]})).unwrap();
    (app, [ids[0], ids[1]])
}

#[test]
fn pixelated_panel_updates_mixed_images_in_one_undo_step() {
    let (mut app, [a, b]) = fixture();
    app.run("object.setProps", json!({"id": b.0, "pixelated": true})).unwrap();
    assert_eq!(object_sections::selected_image_pixelated(&app).unwrap().1, None);
    let before = app.session.doc().unwrap().history.undo.len();
    let mut panel = Panel::new(app, show);
    assert!(panel.shows("Pixelated"));
    panel.click("Pixelated");
    {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert!(st.doc.node(a).unwrap().extras().pixelated);
        assert!(st.doc.node(b).unwrap().extras().pixelated);
        assert_eq!(st.history.undo.len(), before + 1);
    }
    panel.app().run("edit.undo", json!({})).unwrap();
    {
        let app = panel.app();
        let st = app.session.doc().unwrap();
        assert!(!st.doc.node(a).unwrap().extras().pixelated);
        assert!(st.doc.node(b).unwrap().extras().pixelated);
    }
    let rectangle = panel.app().run("shape.rectangle", json!({"x": 50, "y": 50, "width": 10, "height": 10})).unwrap()["id"].clone();
    panel.app().run("select.set", json!({"ids": [a.0, rectangle.as_u64().unwrap()]})).unwrap();
    panel.frame();
    assert!(!panel.shows("Pixelated"));
    assert!(object_sections::selected_image_pixelated(&panel.app()).is_none());
}
