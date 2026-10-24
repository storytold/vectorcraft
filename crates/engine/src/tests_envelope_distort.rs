//! What envelopes distort: the envelope's own frame under transforms, images (a raster mesh warp),
//! symbols, linear gradients, pattern fills, strokes and effects (Distort Appearance).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::live::EnvelopeOptions;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind, PatternDef, Symbol};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_render::effects;

use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 800, "height": 600})).unwrap();
    s
}

fn id_of(v: &Value) -> NodeId {
    NodeId(v["id"].as_u64().unwrap())
}

fn node(s: &Session, id: NodeId) -> Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn sel(s: &mut Session, ids: &[NodeId]) {
    s.execute("select.set", &json!({"ids": ids.iter().map(|i| i.0).collect::<Vec<_>>()})).unwrap();
}

/// A 200×100 rectangle at (100, 100), filled red, no stroke.
fn rect(s: &mut Session) -> NodeId {
    let a = id_of(&s.execute("shape.rectangle", &json!({"x": 100, "y": 100, "width": 200, "height": 100})).unwrap());
    s.edit("red", |d, _| {
        d.node_mut(a).unwrap().appearance = vectorcraft_doc::Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0).into();
        Ok(())
    })
    .unwrap();
    a
}

/// Envelope the selection with an arch warp (`bend` %) and `options` → the envelope.
fn arch(s: &mut Session, bend: f64, options: Value) -> NodeId {
    let e = id_of(&s.execute("object.envelope.makeWithWarp", &json!({"style": "arch", "bend": bend})).unwrap());
    s.execute("object.envelope.options", &options).unwrap();
    e
}

/// The envelope evaluated as the canvas and the exporters do.
fn evaluated(s: &Session, e: NodeId) -> Vec<Node> {
    effects::expand_live(Some(&s.doc().unwrap().doc), &node(s, e))
}

fn anchors(nodes: &[Node]) -> Vec<Point> {
    let mut v = vec![];
    for n in nodes {
        n.walk(&mut |c| v.extend(c.path_data().into_iter().flat_map(|p| p.anchors().map(|(_, _, a)| a.p))));
    }
    v
}

fn frame(s: &Session, e: NodeId) -> Affine {
    match node(s, e).kind {
        NodeKind::Envelope { frame, .. } => *frame,
        k => panic!("{k:?}"),
    }
}

/// Regression: rotating a warp envelope rotated its content and re-warped it along the page axes.
#[test]
fn a_rotated_warp_envelope_turns_with_its_warp() {
    let mut s = session();
    let a = rect(&mut s);
    sel(&mut s, &[a]);
    let e = arch(&mut s, 50.0, json!({}));
    let before = anchors(&evaluated(&s, e));
    let c = Point::new(200.0, 150.0);
    s.execute("object.rotate", &json!({"angle": 90, "origin": [c.x, c.y]})).unwrap();
    let after = anchors(&evaluated(&s, e));
    let turn = Affine::rotate_about(-std::f64::consts::FRAC_PI_2, c);
    assert_eq!(before.len(), after.len());
    for (p, q) in before.iter().zip(&after) {
        assert!((turn * *p).distance(*q) < 1e-6, "{:?} vs {q:?}", turn * *p);
    }
    assert_ne!(frame(&s, e), Affine::IDENTITY);
    // Turning back squares the frame with the page again; a plain move or scale never tilts it.
    s.execute("object.rotate", &json!({"angle": -90, "origin": [c.x, c.y]})).unwrap();
    assert_eq!(frame(&s, e), Affine::IDENTITY);
    s.execute("object.scale", &json!({"sx": 150, "sy": 50})).unwrap();
    assert_eq!(frame(&s, e), Affine::IDENTITY);
    // A reflection keeps a frame: the arch flips with the art.
    s.execute("object.reflect", &json!({"axis": "horizontal"})).unwrap();
    let flipped = anchors(&evaluated(&s, e));
    let b = node(&s, e).geometric_bounds().unwrap();
    assert!(flipped.iter().any(|p| p.y > b.y1 - 1.0) && frame(&s, e) != Affine::IDENTITY);
}

#[test]
fn the_frame_round_trips_natively() {
    let mut s = session();
    let a = rect(&mut s);
    sel(&mut s, &[a]);
    let e = arch(&mut s, 50.0, json!({}));
    s.execute("object.rotate", &json!({"angle": 30})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    assert_eq!(back.node(e), d.node(e));
    let mut old = serde_json::to_value(node(&s, e)).unwrap();
    old["kind"].as_object_mut().unwrap().remove("frame");
    let n: Node = serde_json::from_value(old).unwrap();
    assert!(matches!(n.kind, NodeKind::Envelope { frame, .. } if frame == Box::new(Affine::IDENTITY)), "older envelopes stand square to the page");
}

/// A selected embedded 40×40 pt red image at (100, 100).
fn red_image(s: &mut Session) -> NodeId {
    let img = image::RgbaImage::from_pixel(4, 4, image::Rgba([255, 0, 0, 255]));
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, sel| {
        d.images.insert("pic".into(), vectorcraft_doc::ImageBlob::png(png));
        let id = d.alloc_id();
        let xf = Affine::translate((100.0, 100.0)) * Affine::scale(10.0);
        let im = vectorcraft_doc::ImageObject { key: "pic".into(), width: 4, height: 4, xf, link: None, placement: Default::default(), pixels: None };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

#[test]
fn images_bend_as_a_raster_mesh_warp() {
    let mut s = session();
    let im = red_image(&mut s);
    sel(&mut s, &[im]);
    let e = arch(&mut s, 80.0, json!({}));
    let out = evaluated(&s, e);
    let mut pieces = 0;
    out[0].walk(&mut |c| pieces += matches!(c.kind, NodeKind::Image(_)) as usize);
    assert!(pieces >= 32, "{pieces} pieces");
    // The arch lifts the image's middle above its top edge, not its corners.
    let img = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true);
    let b = node(&s, e).geometric_bounds().unwrap();
    assert!(b.y0 < 95.0, "{b:?}");
    let mid = img.pixel(120, 97);
    assert!(mid[0] > 200 && mid[1] < 60, "the middle rose: {mid:?}");
    assert!(img.pixel(101, 95)[1] > 200, "the corner stayed: {:?}", img.pixel(101, 95));
    // The pieces export as clipped images, bent in the SVG too.
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(&svg).unwrap()).unwrap();
    assert!(svg.matches("<use").count() >= 32 && svg.contains("clip"), "{}", &svg[..svg.len().min(400)]);
    assert_eq!(svg.matches("<image").count(), 1, "the pieces share the image's pixels");
}

#[test]
fn symbols_bend_with_the_envelope() {
    let mut s = session();
    let inst = s
        .edit("symbol", |d, sel| {
            // Symbol art is normalised to ±10 (its instance's transform sizes it).
            let art = Node::path(
                NodeId(0),
                shapes::rectangle(Rect::new(-10.0, -10.0, 10.0, 10.0)),
                vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
            );
            d.symbols.push(Symbol { name: "Box".into(), art: Arc::new(art) });
            let id = d.alloc_id();
            let xf = Affine::translate((200.0, 150.0)) * Affine::scale_non_uniform(10.0, 5.0);
            d.insert(d.default_layer(), 0, Node::new(id, NodeKind::SymbolInstance { symbol: "Box".into(), xf }))?;
            sel.set([id]);
            Ok(id)
        })
        .unwrap();
    sel(&mut s, &[inst]);
    let e = arch(&mut s, 60.0, json!({}));
    let pts = anchors(&evaluated(&s, e));
    assert!(pts.iter().any(|p| p.y < 90.0), "the symbol's art rose in the middle");
    // Editing the symbol redraws the envelope (the renderer's cache follows the symbol's art).
    let mut r = vectorcraft_render::Renderer::new();
    let region = Rect::new(0.0, 0.0, 400.0, 300.0);
    assert!(r.render_region(&s.doc().unwrap().doc, region, 1.0, true).pixel(200, 150)[1] < 60);
    s.edit("edit symbol", |d, _| {
        let art = Node::path(NodeId(0), shapes::rectangle(Rect::new(-10.0, -10.0, -8.0, -8.0)), Default::default());
        d.symbols[0].art = Arc::new(art);
        Ok(())
    })
    .unwrap();
    assert!(r.render_region(&s.doc().unwrap().doc, region, 1.0, true).pixel(200, 150)[1] > 200);
}

fn strokes(nodes: &[Node]) -> usize {
    let mut n = 0;
    for c in nodes {
        c.walk(&mut |c| n += c.appearance.items.iter().filter(|i| matches!(i, AppearanceItem::Stroke(_))).count());
    }
    n
}

#[test]
fn distort_appearance_bends_strokes_and_effects_with_the_art() {
    let mut s = session();
    let a = rect(&mut s);
    s.execute("paint.setStroke", &json!({"ids": [a.0], "color": "#000000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [a.0], "weight": 8})).unwrap();
    sel(&mut s, &[a]);
    let e = arch(&mut s, 60.0, json!({"distortAppearance": true}));
    let out = evaluated(&s, e);
    assert_eq!(strokes(&out), 0, "the stroke became its bent outline");
    // Off: the stroke paints along the bent path at its own weight.
    s.execute("object.envelope.options", &json!({"distortAppearance": false})).unwrap();
    assert_eq!(strokes(&evaluated(&s, e)), 1);
    // Geometry effects bake into the art first too.
    s.execute("object.envelope.editContents", &json!({"editing": true})).unwrap();
    s.execute("effect.apply", &json!({"effect": "distort.zigZag", "params": {"size": 5, "ridges": 10}})).unwrap();
    s.execute("object.envelope.editContents", &json!({"editing": false})).unwrap();
    s.execute("object.envelope.options", &json!({"distortAppearance": true})).unwrap();
    let out = evaluated(&s, e);
    let mut effects_left = 0;
    out[0].walk(&mut |c| effects_left += c.appearance.effects.len());
    assert_eq!(effects_left, 0);
    assert!(anchors(&out).len() > anchors(&[node(&s, a)]).len());
}

fn gradient_geom(nodes: &[Node]) -> Option<vectorcraft_color::GradientGeom> {
    let mut g = None;
    for n in nodes {
        n.walk(&mut |c| {
            if let Paint::Gradient(gp) = c.appearance.fill_paint() {
                g = g.or(gp.geom);
            }
        });
    }
    g
}

#[test]
fn linear_gradients_bend_when_asked() {
    let mut s = session();
    let a = rect(&mut s);
    s.edit("gradient", |d, _| {
        d.node_mut(a).unwrap().appearance.set_fill(Paint::Gradient(Box::new(GradientPaint::new(Gradient::default()))));
        Ok(())
    })
    .unwrap();
    sel(&mut s, &[a]);
    let e = arch(&mut s, 60.0, json!({"distortAppearance": true}));
    assert!(gradient_geom(&evaluated(&s, e)).is_none(), "off: the gradient fits the bent shape");
    s.execute("object.envelope.options", &json!({"distortLinearGradients": true})).unwrap();
    let g = gradient_geom(&evaluated(&s, e)).expect("on: the gradient is placed and follows the warp");
    // Arch lifts the middle: the vector (left to right through the middle) moved up.
    assert!(g.start.midpoint(g.end).y < 150.0 - 1.0, "{g:?}");
    // It needs Distort Appearance.
    s.execute("object.envelope.options", &json!({"distortAppearance": false})).unwrap();
    assert!(gradient_geom(&evaluated(&s, e)).is_none());
}

#[test]
fn pattern_fills_bend_when_asked() {
    let mut s = session();
    let a = rect(&mut s);
    s.edit("pattern", |d, _| {
        let dot = Node::path(
            NodeId(0),
            shapes::rectangle(Rect::new(0.0, 0.0, 5.0, 5.0)),
            vectorcraft_doc::Appearance::basic(Paint::solid(Color::BLACK), Paint::None, 0.0),
        );
        let mut def = PatternDef::new("Dots", vec![Arc::new(dot)]);
        def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
        d.patterns.push(def);
        d.node_mut(a).unwrap().appearance.set_fill(vectorcraft_doc::pattern::pattern_paint("Dots"));
        Ok(())
    })
    .unwrap();
    sel(&mut s, &[a]);
    let e = arch(&mut s, 60.0, json!({"distortAppearance": true}));
    let clip_groups = |out: &[Node]| {
        let mut n = 0;
        out[0].walk(&mut |c| n += matches!(c.kind, NodeKind::Group { clip: true, .. }) as usize);
        n
    };
    assert_eq!(clip_groups(&evaluated(&s, e)), 0, "off: the pattern fills the bent shape unbent");
    s.execute("object.envelope.options", &json!({"distortPatternFills": true})).unwrap();
    let out = evaluated(&s, e);
    assert_eq!(clip_groups(&out), 1);
    assert!(anchors(&out).len() > 50 * 4, "the tiles are art");
    assert!(!matches!(out[0].appearance.fill_paint(), Paint::Pattern { .. }));
}

#[test]
fn new_envelopes_distort_appearance_and_old_ones_keep_their_look() {
    let mut s = session();
    let a = rect(&mut s);
    sel(&mut s, &[a]);
    let e = arch(&mut s, 50.0, json!({}));
    assert!(matches!(node(&s, e).kind, NodeKind::Envelope { options, .. } if options == EnvelopeOptions::NEW));
}
