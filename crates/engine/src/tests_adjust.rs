//! Live colour adjustments (Effect → Color Adjustments) through the commands: applied, listed,
//! exported, expanded and kept in the native file.

use serde_json::json;
use vectorcraft_doc::{Node, NodeKind};

use super::*;

pub(crate) fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    s
}

pub(crate) fn red_rect(s: &mut Session) -> NodeId {
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    id
}

pub(crate) fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).cloned().unwrap()
}

pub(crate) fn fill_hex(n: &Node) -> String {
    n.appearance.fill_paint().color().unwrap().to_hex()
}

/// A selected embedded 4×4 red image.
fn red_image(s: &mut Session) -> NodeId {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, sel| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png));
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key: "pic".into(),
            width: 4,
            height: 4,
            xf: vectorcraft_geom::Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn the_effects_are_in_the_catalog_under_color_adjustments() {
    let mut s = session();
    let v = s.execute("effect.list", &json!({})).unwrap();
    let ids: Vec<&str> =
        v["catalog"].as_array().unwrap().iter().filter(|e| e["menu"][1] == "Color Adjustments").map(|e| e["id"].as_str().unwrap()).collect();
    for id in vectorcraft_render::effects::ADJUSTMENTS {
        assert!(ids.contains(&id), "{id} in {ids:?}");
    }
}

#[test]
fn an_adjustment_is_live_and_one_undo_step() {
    let mut s = session();
    let id = red_rect(&mut s);
    let undo = s.doc().unwrap().history.undo.len();
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Non-destructive: the fill stays red, the effect sits on the object.
    let n = node(&s, id);
    assert_eq!(fill_hex(&n), "#ff0000");
    assert_eq!(n.appearance.effects.len(), 1);
    // Drawn green.
    let img =
        vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, true);
    assert_eq!(img.pixel(50, 50), [0, 255, 0, 255]);
    // Editing the parameters changes it live.
    s.execute("effect.setParams", &json!({"index": 0, "item": null, "params": {"hue": -120}})).unwrap();
    let img =
        vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, vectorcraft_geom::Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, true);
    assert_eq!(img.pixel(50, 50), [0, 0, 255, 255]);
}

#[test]
fn export_and_expand_write_the_adjusted_colours() {
    let mut s = session();
    let id = red_rect(&mut s);
    s.execute("effect.apply", &json!({"effect": "adjust.levels", "params": {"outputWhite": 128, "channel": "red"}})).unwrap();
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.contains("#800000") && !svg.contains("#ff0000"), "{svg}");
    // Expand Appearance makes it permanent.
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let n = node(&s, id);
    assert!(n.appearance.effects.is_empty());
    assert_eq!(fill_hex(&n), "#800000");
}

#[test]
fn an_embedded_image_is_adjusted_for_export_without_changing_its_pixels() {
    let mut s = session();
    let id = red_image(&mut s);
    s.execute("effect.apply", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    let doc = s.doc().unwrap().doc.clone();
    let baked = vectorcraft_render::effects::bake_document(&doc).unwrap();
    let NodeKind::Image(im) = &baked.node(id).unwrap().kind else { panic!("not an image") };
    assert_ne!(im.key, "pic", "a recoloured copy");
    let px = image::load_from_memory(&baked.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(px.get_pixel(1, 1).0, [0, 255, 0, 255]);
    assert_eq!(doc.images.len(), 1, "the document keeps its one image");
    // Through Expand Appearance too.
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let NodeKind::Image(im) = &node(&s, id).kind else { panic!("not an image") };
    let px = image::load_from_memory(&s.doc().unwrap().doc.images[&im.key].bytes).unwrap().to_rgba8();
    assert_eq!(px.get_pixel(0, 0).0, [0, 255, 0, 255]);
}

#[test]
fn adjustments_survive_the_native_file() {
    let mut s = session();
    red_rect(&mut s);
    s.execute("effect.apply", &json!({"effect": "adjust.curves", "params": {"points": "0,0 64,32 255,255", "channel": "green"}})).unwrap();
    let saved = s.execute("document.export", &json!({"format": "vectorcraft"})).unwrap()["dataBase64"].clone();
    let mut o = Session::new();
    o.execute("document.open", &json!({"name": "a.vectorcraft", "dataBase64": saved})).unwrap();
    let mut found = None;
    o.doc().unwrap().doc.walk(|n| {
        if let Some(e) = n.appearance.effects.first() {
            found = Some(e.clone());
        }
    });
    let e = found.expect("the effect is kept");
    assert_eq!(e.id, "adjust.curves");
    assert_eq!(e.params["points"], "0,0 64,32 255,255");
}
