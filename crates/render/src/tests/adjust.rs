//! Live colour adjustments (Effect → Color Adjustments) as drawn: on paths, type, embedded images,
//! groups and symbol instances, alone and with other effects.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, Effect, ImageBlob, ImageObject, Node, NodeKind, Symbol};
use vectorcraft_geom::{Point, shapes};

use crate::*;

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// Hue +120°: red becomes green, green blue.
fn hue() -> Effect {
    fx("adjust.hueSaturation", json!({"hue": 120}))
}

fn doc_with(nodes: Vec<Node>) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let l = d.layers[0].id;
    for mut n in nodes {
        n.id = d.alloc_id();
        d.insert(Some(l), 0, n).unwrap();
    }
    d
}

fn render(d: &Document) -> Rendered {
    let mut r = Renderer::new();
    r.threads = 0;
    r.render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn red_square(x: f64, y: f64) -> Node {
    Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(x, y, x + 30.0, y + 30.0)),
        Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0),
    )
}

fn red_image() -> (String, ImageBlob) {
    let img = image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    ("red".into(), ImageBlob::new("image/png", png))
}

#[test]
fn a_path_takes_its_adjusted_colour_and_keeps_its_other_effects() {
    let mut n = red_square(10.0, 10.0);
    n.appearance.effects = vec![hue(), fx("distort.transform", json!({"moveH": 40}))];
    let img = render(&doc_with(vec![n]));
    assert_eq!(img.pixel(65, 25), [0, 255, 0, 255], "green, and moved by the geometry effect");
    assert_eq!(img.pixel(25, 25), [255, 255, 255, 255]);
    // Hidden, it changes nothing.
    let mut n = red_square(10.0, 10.0);
    n.appearance.effects = vec![Effect { visible: false, ..hue() }];
    assert_eq!(render(&doc_with(vec![n])).pixel(25, 25), [255, 0, 0, 255]);
}

#[test]
fn a_group_recolours_its_members_and_type() {
    let style = CharStyle { size: 60.0, fill: Paint::solid(Color::rgb(1.0, 0.0, 0.0)), ..Default::default() };
    let text = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(50.0, 90.0), "I", style))));
    let mut g = Node::group(NodeId(0), vec![Arc::new(red_square(5.0, 5.0)), Arc::new(text)]);
    g.appearance.effects.push(hue());
    let d = doc_with(vec![g]);
    let img = render(&d);
    assert_eq!(img.pixel(20, 20), [0, 255, 0, 255]);
    let inked: Vec<[u8; 4]> =
        (50..70).flat_map(|x| (50..90).map(move |y| (x, y))).map(|(x, y)| img.pixel(x, y)).filter(|p| p[0] < 128 || p[2] < 128).collect();
    assert!(inked.contains(&[0, 255, 0, 255]), "the type turns green too");
    assert!(inked.iter().all(|p| p[1] == 255 && p[0] == p[2]), "no red left: {:?}", inked.iter().find(|p| p[1] != 255 || p[0] != p[2]));
}

#[test]
fn an_embedded_image_is_recoloured_and_the_original_pixels_stay() {
    let mut d = doc_with(vec![]);
    let (key, blob) = red_image();
    d.images.insert(key.clone(), blob.clone());
    let mut n = Node::new(
        NodeId(0),
        NodeKind::Image(Box::new(ImageObject {
            key,
            width: 30,
            height: 30,
            xf: Affine::translate((20.0, 20.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    n.appearance.effects.push(hue());
    d = {
        let l = d.layers[0].id;
        n.id = d.alloc_id();
        d.insert(Some(l), 0, n).unwrap();
        d
    };
    let img = render(&d);
    assert_eq!(img.pixel(35, 35), [0, 255, 0, 255]);
    assert_eq!(d.images.get("red"), Some(&blob), "non-destructive");
}

#[test]
fn a_symbol_instance_recolours_its_art() {
    let mut d = Document::new(100.0, 100.0);
    let art = red_square(-15.0, -15.0);
    d.symbols.push(Symbol { name: "Red".into(), art: Arc::new(art) });
    let mut n = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Red".into(), xf: Affine::translate((50.0, 50.0)) });
    n.appearance.effects.push(hue());
    let l = d.layers[0].id;
    n.id = d.alloc_id();
    d.insert(Some(l), 0, n).unwrap();
    assert_eq!(render(&d).pixel(50, 50), [0, 255, 0, 255]);
}
