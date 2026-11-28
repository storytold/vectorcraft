//! Linked images in `.vectorcraft` files: their link details round-trip, and an image only linked
//! images show is saved as its preview.

use std::sync::Arc;

use serde_json::Value;
use vectorcraft_brush::{Brush, BrushKind, PatternBrush, Scatter};
use vectorcraft_doc::{Document, ImageBlob, ImageObject, LinkInfo, Node, NodeId, NodeKind};
use vectorcraft_format::{SaveOptions, base64_decode, load, save, save_with};
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

/// An image object for a stored brush's art.
fn brush_image(key: &str, link: Option<LinkInfo>) -> Node {
    let im = ImageObject { key: key.into(), width: 600, height: 300, xf: Affine::IDENTITY, link, placement: Default::default(), pixels: None };
    Node::new(NodeId(0), NodeKind::Image(Box::new(im)))
}

/// Stores a scatter brush with `scatter` as its art and a pattern brush with `tile` as its side tile.
fn brushes(d: &mut Document, scatter: Node, tile: Node) {
    vectorcraft_brush::store(
        d,
        &[
            Brush { name: "Dots".into(), kind: BrushKind::Scatter(Scatter { art: scatter, ..Default::default() }) },
            Brush { name: "Frame".into(), kind: BrushKind::Pattern(PatternBrush { side: tile, ..Default::default() }) },
        ],
    );
}

/// `key`'s entry in the saved file `v`: its bytes and its `proxy` flag.
fn saved_image(v: &Value, key: &str) -> (Vec<u8>, Value) {
    (base64_decode(v["images"][key]["data"].as_str().unwrap()).unwrap(), v["images"][key]["proxy"].clone())
}

/// A linked image shown only in a stored brush's art (a scatter brush's art, a pattern brush's tile)
/// is saved as its preview, as one in the layers is, and with Include Linked Files as its file's
/// bytes; the brushes are saved as they were, their links kept.
#[test]
fn brush_only_link_saves_preview_unless_included() {
    let mut d = Document::new(100.0, 100.0);
    d.images.insert("dot".into(), blob(&[1, 2, 3], Some(&[9])));
    d.images.insert("tile".into(), blob(&[4, 5, 6], Some(&[8])));
    brushes(&mut d, brush_image("dot", Some(LinkInfo::new("/work/dot.jpg"))), brush_image("tile", Some(LinkInfo::new("/work/tile.jpg"))));
    let stored = d.unknown[vectorcraft_brush::DOC_KEY].clone();

    let bytes = save(&d, false);
    let v: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(saved_image(&v, "dot"), (vec![9], Value::Bool(true)));
    assert_eq!(saved_image(&v, "tile"), (vec![8], Value::Bool(true)));
    let back = load(&bytes).unwrap();
    assert_eq!(back.unknown[vectorcraft_brush::DOC_KEY], stored, "the brushes are saved as they were");
    assert!(back.mounted_art.is_empty());
    let mut links = vec![];
    for b in vectorcraft_brush::library(&back) {
        for art in b.art() {
            art.walk(&mut |n| {
                if let NodeKind::Image(im) = &n.kind {
                    links.extend(im.link.as_ref().map(|l| l.path.clone()));
                }
            });
        }
    }
    links.sort();
    assert_eq!(links, ["/work/dot.jpg", "/work/tile.jpg"]);

    let included = save_with(&d, &SaveOptions { include_linked: true, ..SaveOptions::default() }).unwrap();
    let v: Value = serde_json::from_slice(&included).unwrap();
    assert_eq!(saved_image(&v, "dot"), (vec![1, 2, 3], Value::Null));
    assert_eq!(saved_image(&v, "tile"), (vec![4, 5, 6], Value::Null));
}

/// An image key shown linked in one place and embedded in another keeps its full bytes, whichever
/// of the layers and a stored brush's art embeds it; the links stay as they were.
#[test]
fn shared_brush_image_key_keeps_full_bytes_for_embedded_use() {
    for layer_linked in [true, false] {
        let mut d = Document::new(100.0, 100.0);
        d.images.insert("k".into(), blob(&[1, 2, 3], Some(&[9])));
        let link = Some(LinkInfo::new("/work/k.jpg"));
        let (in_layer, in_brush) = if layer_linked { (link, None) } else { (None, link) };
        image(&mut d, "k", in_layer.clone());
        brushes(&mut d, brush_image("k", in_brush.clone()), brush_image("k", in_brush.clone()));
        let bytes = save(&d, false);
        let v: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(saved_image(&v, "k"), (vec![1, 2, 3], Value::Null), "linked in the layer: {layer_linked}");
        let back = load(&bytes).unwrap();
        let mut layer_links = vec![];
        for l in &back.layers {
            l.walk(&mut |n| {
                if let NodeKind::Image(im) = &n.kind {
                    layer_links.push(im.link.clone());
                }
            });
        }
        assert_eq!(layer_links, [in_layer]);
        let brush_links: Vec<Option<LinkInfo>> = vectorcraft_brush::library(&back)
            .iter()
            .flat_map(|b| b.art().into_iter().map(|n| if let NodeKind::Image(im) = &n.kind { im.link.clone() } else { None }).collect::<Vec<_>>())
            .collect();
        assert_eq!(brush_links, [in_brush.clone(), in_brush]);
    }
}
