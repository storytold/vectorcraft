//! Linked images in `.vectorcraft` files: their link details round-trip, and an image only linked
//! images show is saved as its preview.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_doc::{Document, ImageBlob, ImageObject, LinkInfo, Node, NodeKind};
use vectorcraft_format::{base64_decode, load, save};
use vectorcraft_geom::Affine;

fn image(d: &mut Document, key: &str, link: Option<LinkInfo>) {
    let (id, layer) = (d.alloc_id(), d.layers[0].id);
    let im = ImageObject { key: key.into(), width: 600, height: 300, xf: Affine::IDENTITY, link, placement: Default::default(), pixels: None };
    d.insert(Some(layer), 0, Node::new(id, NodeKind::Image(Box::new(im)))).unwrap();
}

fn blob(full: &[u8], proxy: Option<&[u8]>) -> ImageBlob {
    ImageBlob { proxy: proxy.map(|p| Arc::new(p.to_vec())), ..ImageBlob::new("image/jpeg", full.to_vec()) }
}

#[test]
fn link_details_round_trip_and_linked_only_images_save_their_preview() {
    let mut d = Document::new(100.0, 100.0);
    let link = LinkInfo {
        relative: Some("art/photo.jpg".into()),
        modified: Some(1_700_000_000_000),
        size: Some(4),
        hash: Some("img0123456789abcdef".into()),
        ..LinkInfo::new("/work/art/photo.jpg")
    };
    d.images.insert("linked".into(), blob(&[1, 2, 3, 4], Some(&[9, 9])));
    d.images.insert("both".into(), blob(&[5, 6, 7], Some(&[8])));
    d.images.insert("small".into(), blob(&[4, 4], None));
    image(&mut d, "linked", Some(link.clone()));
    image(&mut d, "both", Some(LinkInfo::new("/work/b.jpg")));
    image(&mut d, "both", None);
    image(&mut d, "small", Some(LinkInfo::new("/work/s.jpg")));
    let bytes = save(&d, false);
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    let saved = |k: &str| {
        (
            v["images"][k]["mime"].as_str().unwrap().to_string(),
            base64_decode(v["images"][k]["data"].as_str().unwrap()).unwrap(),
            v["images"][k]["proxy"].clone(),
        )
    };
    assert_eq!(saved("linked"), ("image/png".into(), vec![9, 9], Value::Bool(true)), "only linked images show it: the preview");
    assert_eq!(saved("both"), ("image/jpeg".into(), vec![5, 6, 7], Value::Null), "an embedded image shows it too: the full bytes");
    assert_eq!(saved("small"), ("image/jpeg".into(), vec![4, 4], Value::Null), "no preview: the full bytes");

    let back = load(&bytes).unwrap();
    let linked = &back.images["linked"];
    assert_eq!((linked.bytes.as_slice(), linked.is_proxy()), (&[9u8, 9][..], true), "the preview stands in until the file is read");
    assert!(!back.images["both"].is_proxy() && back.images["both"].proxy.is_none());
    let mut links = vec![];
    back.visit_images(|_, im| links.push(im.link.clone()));
    assert!(links.contains(&Some(link)), "{links:?}");
    // Saved again unread, the preview stays the preview.
    let again: Value = serde_json::from_slice(&save(&back, false)).unwrap();
    assert_eq!(again["images"]["linked"], v["images"]["linked"]);
}
