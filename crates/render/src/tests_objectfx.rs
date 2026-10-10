//! Effects on every object kind: raster effects on type, images and symbol instances through an
//! offscreen layer of their art, geometry effects through their outlines, and one fill or
//! stroke's own raster effects.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, Effect, ImageBlob, ImageObject, Node, NodeKind, Symbol};
use vectorcraft_geom::{Point, shapes};

use super::*;

fn fx(id: &str, params: Value) -> Effect {
    Effect { id: id.into(), params, visible: true }
}

/// A 100×100 document with `n` (its id allocated) on the first layer.
fn doc_with(mut d: Document, mut n: Node) -> Document {
    n.id = d.alloc_id();
    let l = d.layers[0].id;
    d.insert(Some(l), 0, n).unwrap();
    d
}

fn render_threads(d: &Document, threads: u16) -> Rendered {
    let mut r = Renderer::new();
    r.threads = threads;
    r.render(d, 100, 100, Affine::IDENTITY, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
}

fn render(d: &Document) -> Rendered {
    render_threads(d, 0)
}

fn lum(p: [u8; 4]) -> u32 {
    p[0] as u32 + p[1] as u32 + p[2] as u32
}

const WHITE: u32 = 765;

#[test]
fn revolve_canvas_matches_baked_art_and_shades_its_surface() {
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((50.0, 15.0));
    bp.line_to((75.0, 30.0));
    bp.line_to((75.0, 75.0));
    bp.line_to((50.0, 90.0));
    let mut n = Node::path(
        NodeId(0),
        vectorcraft_geom::PathData::from_bezpath(&bp),
        Appearance::basic(Paint::solid(Color::rgb(0.9, 0.2, 0.1)), Paint::None, 0.0),
    );
    n.appearance.effects.push(fx("threeD.revolve", json!({"rotationX":0,"rotationY":0})));
    let d = doc_with(Document::new(100.0, 100.0), n);
    let live = render(&d);
    let baked = vectorcraft_effects::bake_document(&d).unwrap();
    let exported = render(&baked);
    assert_eq!(live.pixels, exported.pixels, "canvas and baked/exported faces agree");
    let reds: Vec<_> =
        live.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] as u16 > p[1] as u16 * 2 && p[0] as u16 > p[2] as u16 * 2).map(|p| p[0]).collect();
    assert!(reds.len() > 1500, "a filled surface, rather than the open source path");
    assert!(reds.iter().max().unwrap() - reds.iter().min().unwrap() > 80, "lighting varies across the surface");
    assert_eq!(live.pixels, render_threads(&d, 4).pixels);
}

#[test]
fn revolve_visible_surface_expansion_preserves_the_view_and_transparent_art() {
    let mut bp = vectorcraft_geom::BezPath::new();
    bp.move_to((50.0, 15.0));
    bp.curve_to((80.0, 25.0), (80.0, 75.0), (50.0, 90.0));
    let mut n = Node::path(
        NodeId(0),
        vectorcraft_geom::PathData::from_bezpath(&bp),
        Appearance::basic(Paint::solid(Color::rgb(0.2, 0.55, 0.9)), Paint::None, 0.0),
    );
    for (angle, rotation, perspective, opacity) in
        [(360.0, 0.0, 0.0, 1.0), (360.0, -35.0, 55.0, 1.0), (150.0, -28.0, 40.0, 1.0), (360.0, -35.0, 55.0, 0.5)]
    {
        n.opacity = opacity;
        n.appearance.effects = vec![fx(
            "threeD.revolve",
            json!({"angle":angle,"rotationX":rotation,"rotationY":25,"perspective":perspective,"segments":32,"expandVisibleOnly":true}),
        )];
        let d = doc_with(Document::new(100.0, 100.0), n.clone());
        let live = render(&d);
        let mut expanded = d.clone();
        let id = expanded.layers[0].children().unwrap()[0].id;
        let source = expanded.node(id).unwrap().clone();
        let mut stroke = |_: &mut Document, _: &vectorcraft_geom::PathData, _: vectorcraft_geom::FillRule, _: &vectorcraft_doc::StrokeLayer| None;
        let art = vectorcraft_effects::expand_leaf(&mut expanded, &source, &mut stroke).unwrap();
        *expanded.node_mut(id).unwrap() = art;
        let flat = render(&expanded);
        let difference: u64 = live.pixels.iter().zip(&flat.pixels).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        let average = difference as f64 / live.pixels.len() as f64;
        assert!(average < 1.0, "appearance differs by {average} channel levels: angle {angle}, rotation {rotation}, opacity {opacity}");
        if opacity < 1.0 {
            assert_eq!(live.pixels, flat.pixels);
        }
        // Export baking keeps every face, even when the expansion checkbox is checked.
        assert_eq!(live.pixels, render(&vectorcraft_effects::bake_document(&d).unwrap()).pixels);
    }
}

/// A 30×30 red image placed at (20, 20).
fn image_doc(effects: Vec<Effect>) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let img = image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    d.images.insert("red".into(), ImageBlob::new("image/png", png));
    let mut n = Node::new(
        NodeId(0),
        NodeKind::Image(ImageObject {
            key: "red".into(),
            width: 30,
            height: 30,
            xf: Affine::translate((20.0, 20.0)),
            link: None,
            placement: Default::default(),
        }),
    );
    n.appearance.effects = effects;
    doc_with(d, n)
}

/// An instance at (50, 50) of a symbol whose art is a black 20×20 square centred on its origin.
fn symbol_doc(effects: Vec<Effect>) -> Document {
    let mut d = Document::new(100.0, 100.0);
    let art = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
    );
    d.symbols.push(Symbol { name: "Square".into(), art: Arc::new(art) });
    let mut n = Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Square".into(), xf: Affine::translate((50.0, 50.0)) });
    n.appearance.effects = effects;
    doc_with(d, n)
}

/// Black 40 pt "IIII" with its baseline at (5, 60).
fn text_doc(effects: Vec<Effect>) -> Document {
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
    let mut n = Node::new(NodeId(0), NodeKind::Text(Box::new(TextObject::point(Point::new(5.0, 60.0), "IIII", style))));
    n.appearance.effects = effects;
    doc_with(Document::new(100.0, 100.0), n)
}

/// Columns (x) holding any dark pixel.
fn inked_columns(img: &Rendered) -> Vec<u32> {
    (0..100).filter(|x| (0..100).any(|y| lum(img.pixel(*x, y)) < 300)).collect()
}

#[test]
fn a_shadow_on_an_image_paints_offset_pixels() {
    let plain = render(&image_doc(vec![]));
    assert_eq!(lum(plain.pixel(55, 55)), WHITE);
    let d = image_doc(vec![fx("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 0, "opacity": 100, "mode": "normal"}))]);
    let img = render(&d);
    assert!(lum(img.pixel(55, 55)) < 100, "shadow below-right: {:?}", img.pixel(55, 55));
    assert_eq!(img.pixel(35, 35), [255, 0, 0, 255], "the image stays on top");
    assert_eq!(lum(img.pixel(15, 15)), WHITE);
    // The multithreaded pipeline filters offscreen and matches.
    let mt = render_threads(&d, 3);
    let worst = img.pixels.iter().zip(&mt.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 4, "max channel difference {worst}");
}

#[test]
fn blur_on_a_symbol_instance_softens_its_edges() {
    let plain = render(&symbol_doc(vec![]));
    assert_eq!(lum(plain.pixel(38, 50)), WHITE);
    assert_eq!(lum(plain.pixel(50, 50)), 0);
    let img = render(&symbol_doc(vec![fx("blur.gaussian", json!({"radius": 8}))]));
    let edge = lum(img.pixel(38, 50));
    assert!(edge > 0 && edge < WHITE, "the blur spreads past the edge: {edge}");
    assert!(lum(img.pixel(50, 50)) < 200, "the centre stays dark");
}

#[test]
fn geometry_effects_reshape_type_through_its_outlines() {
    let plain = inked_columns(&render(&text_doc(vec![])));
    assert!(!plain.is_empty() && plain[0] < 30, "{plain:?}");
    // Transform moves the outlines as one piece.
    let moved = inked_columns(&render(&text_doc(vec![fx("distort.transform", json!({"moveH": 30}))])));
    assert_eq!(moved.first().copied(), plain.first().map(|x| x + 30));
    // Roughen redraws the glyphs as rough outlines.
    let d = text_doc(vec![fx("distort.roughen", json!({"size": 4, "relative": false, "detail": 30, "points": "corner"}))]);
    let rough = render(&d);
    let smooth = render(&text_doc(vec![]));
    let changed = rough.pixels.chunks(4).zip(smooth.pixels.chunks(4)).filter(|(a, b)| a != b).count();
    assert!(changed > 100, "{changed} pixels changed");
    // The reshaped art is plain paths painted with the text's fill.
    let n = &d.layers[0].children().unwrap()[0];
    let art = vectorcraft_effects::reshape(n, None).unwrap();
    let (mut text, mut black) = (0, 0);
    art.walk(&mut |m| {
        text += usize::from(matches!(m.kind, NodeKind::Text(_)));
        black += usize::from(m.appearance.fill_paint() == Paint::solid(Color::BLACK));
    });
    assert!(text == 0 && black > 0, "{art:?}");
}

#[test]
fn a_strokes_own_glow_paints_around_that_stroke_only() {
    let mut d = Document::new(100.0, 100.0);
    let mut n = Node::path(
        NodeId(0),
        shapes::rectangle(Rect::new(30.0, 30.0, 70.0, 70.0)),
        Appearance::basic(Paint::solid(Color::WHITE), Paint::solid(Color::BLACK), 2.0),
    );
    n.appearance.items[1].effects_mut().push(fx("stylize.outerGlow", json!({"color": "#00ff00", "mode": "normal", "opacity": 100, "blur": 6})));
    d = doc_with(d, n);
    let img = render(&d);
    let g = img.pixel(27, 50);
    assert!(g[1] > g[0] + 20 && g[1] > g[2] + 20, "green glow outside the stroke: {g:?}");
    assert_eq!(img.pixel(50, 50), [255, 255, 255, 255], "the fill has no glow");
    let mt = render_threads(&d, 3);
    let worst = img.pixels.iter().zip(&mt.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
    assert!(worst <= 4, "max channel difference {worst}");
}
