//! What envelopes distort: the envelope's own frame under transforms, images (a raster mesh warp),
//! symbols, linear gradients, pattern fills, strokes and effects (Distort Appearance).

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Gradient, GradientPaint, Paint};
use vectorcraft_doc::live::EnvelopeOptions;
use vectorcraft_doc::{AppearanceItem, Node, NodeKind, PatternDef, Symbol};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_render::effects;

use crate::*;

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

/// A selected embedded 2×2 image, 80×80 pt at (100, 100): a red column left of a blue one.
fn two_color_image(s: &mut Session) -> NodeId {
    let img = image::RgbaImage::from_fn(2, 2, |x, _| if x == 0 { image::Rgba([255, 0, 0, 255]) } else { image::Rgba([0, 0, 255, 255]) });
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, sel| {
        d.images.insert("two".into(), vectorcraft_doc::ImageBlob::png(png));
        let id = d.alloc_id();
        let xf = Affine::translate((100.0, 100.0)) * Affine::scale(40.0);
        let im = vectorcraft_doc::ImageObject { key: "two".into(), width: 2, height: 2, xf, link: None, placement: Default::default(), pixels: None };
        d.insert(d.default_layer(), 0, Node::new(id, NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

/// The pixels well inside the image's frame (the clip edges left out) that are a mix of its red
/// and blue rather than one of them.
fn mixed_pixels(s: &Session) -> usize {
    let img = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(110.0, 110.0, 170.0, 170.0), 2.0, true);
    let mut mixed = 0;
    for y in 0..img.height {
        for x in 0..img.width {
            let p = img.pixel(x, y);
            mixed += (p[0] > 40 && p[2] > 40) as usize;
        }
    }
    mixed
}

#[test]
fn anti_alias_off_draws_distorted_images_without_smoothing() {
    let mut s = session();
    let im = two_color_image(&mut s);
    sel(&mut s, &[im]);
    let e = arch(&mut s, 2.0, json!({}));
    let smooth = mixed_pixels(&s);
    assert!(smooth > 400, "with Anti-Alias on the colors blend where the pixels meet: {smooth}");
    s.execute("object.envelope.options", &json!({"antiAlias": false})).unwrap();
    let mut pieces = 0;
    for n in evaluated(&s, e) {
        n.walk(&mut |c| pieces += (matches!(c.kind, NodeKind::Image(_)) && c.extras().pixelated) as usize);
    }
    assert!(pieces >= 32, "{pieces} pixelated pieces");
    let hard = mixed_pixels(&s);
    assert!(hard * 50 < smooth, "with Anti-Alias off each pixel is red or blue: {hard} mixed of {smooth}");
    // The SVG asks viewers not to smooth the pieces either.
    let svg = |s: &mut Session| {
        let b64 = s.execute("document.export", &json!({"format": "svg"})).unwrap()["dataBase64"].as_str().unwrap().to_string();
        String::from_utf8(vectorcraft_format::base64_decode(&b64).unwrap()).unwrap()
    };
    assert!(svg(&mut s).matches("image-rendering").count() >= 32);
    s.execute("object.envelope.options", &json!({"antiAlias": true})).unwrap();
    assert!(!svg(&mut s).contains("image-rendering"));
    s.execute("object.envelope.options", &json!({"antiAlias": false})).unwrap();
    // Expanded, the pieces keep it through a native save.
    s.execute("object.envelope.expand", &json!({})).unwrap();
    let d = &s.doc().unwrap().doc;
    let back = vectorcraft_format::load(&vectorcraft_format::save(d, false)).unwrap();
    let mut kept = 0;
    back.walk(|c| kept += (matches!(c.kind, NodeKind::Image(_)) && c.extras().pixelated) as usize);
    assert_eq!(kept, pieces);
}

fn clip_groups_in(nodes: &[Node]) -> usize {
    let mut n = 0;
    for c in nodes {
        c.walk(&mut |c| n += matches!(c.kind, NodeKind::Group { clip: true, .. }) as usize);
    }
    n
}

#[test]
fn transparency_masks_distorted_images_instead_of_clipping_them() {
    let mut s = session();
    let im = red_image(&mut s);
    sel(&mut s, &[im]);
    let e = arch(&mut s, 80.0, json!({}));
    let region = Rect::new(80.0, 60.0, 160.0, 160.0);
    let clipped = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, region, 2.0, true);
    assert!(clip_groups_in(&evaluated(&s, e)) >= 32);
    s.execute("object.envelope.options", &json!({"preserveShape": "transparency"})).unwrap();
    let out = evaluated(&s, e);
    assert_eq!(clip_groups_in(&out), 0, "no clipping masks");
    let (mut images, mut masked) = (0, 0);
    for n in &out {
        n.walk(&mut |c| {
            if matches!(c.kind, NodeKind::Image(_)) {
                images += 1;
                masked += c.extras().mask.as_ref().is_some_and(|m| m.clip && !m.invert) as usize;
            }
        });
    }
    assert!(images >= 32 && masked == images, "{masked} of {images} pieces masked");
    // It draws as the clipping masks did.
    let alpha = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, region, 2.0, true);
    let (mut sum, mut n) = (0u64, 0u64);
    for y in 0..clipped.height {
        for x in 0..clipped.width {
            let (a, b) = (clipped.pixel(x, y), alpha.pixel(x, y));
            sum += (0..4).map(|i| (a[i] as i32 - b[i] as i32).unsigned_abs() as u64).sum::<u64>();
            n += 4;
        }
    }
    assert!(sum < n, "mean difference {} per channel", sum as f64 / n as f64);
    assert!(alpha.pixel(80, 74)[0] > 200 && alpha.pixel(80, 74)[1] < 60, "the bent image shows: {:?}", alpha.pixel(80, 74));
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
            d.symbols.push(Symbol { dynamic: false, name: "Box".into(), art: Arc::new(art) });
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

/// What strokes the object in [`squeezed`].
#[derive(Clone, Copy)]
enum Stroked {
    /// Nothing.
    No,
    /// The characters of type, or a path's own stroke.
    Own,
    /// A stroke on type's own appearance (Appearance panel), the characters unstroked.
    Appearance,
}

/// The weight of the strokes in [`squeezed`].
const SQUEEZED_WEIGHT: f64 = 8.0;

/// `make`'s object, stroked by `stroked` with a red stroke [`SQUEEZED_WEIGHT`] wide, in a one-patch
/// mesh envelope with Distort Appearance on whose right column is moved in to squeeze the object to
/// half its width (a horizontal squeeze, the same everywhere): the session, the envelope and the
/// height of the envelope's middle.
fn squeezed(make: fn(&mut Session) -> NodeId, stroked: Stroked) -> (Session, NodeId, f64) {
    use vectorcraft_doc::live::EnvelopeKind;
    let mut s = session();
    let a = make(&mut s);
    sel(&mut s, &[a]);
    match stroked {
        Stroked::No => {}
        Stroked::Own => {
            s.execute("paint.setStroke", &json!({"ids": [a.0], "color": "#ff0000"})).unwrap();
            s.execute("stroke.set", &json!({"ids": [a.0], "weight": SQUEEZED_WEIGHT})).unwrap();
        }
        Stroked::Appearance => {
            s.edit("appearance stroke", |d, _| {
                let n = d.node_mut(a).unwrap();
                let mut appearance = (*n.appearance).clone();
                appearance
                    .items
                    .push(AppearanceItem::Stroke(vectorcraft_doc::StrokeLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), SQUEEZED_WEIGHT)));
                n.appearance = appearance.into();
                Ok(())
            })
            .unwrap();
        }
    }
    let e = id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 1, "cols": 1})).unwrap());
    let NodeKind::Envelope { options, .. } = &node(&s, e).kind else { panic!("an envelope") };
    assert!(options.distort_appearance, "new envelopes distort appearance");
    let mut middle = 0.0;
    s.edit("squeeze", |d, _| {
        let NodeKind::Envelope { kind, .. } = &mut d.node_mut(e).unwrap().kind else { panic!("an envelope") };
        let EnvelopeKind::Mesh { points, handles, .. } = &mut **kind else { panic!("a mesh") };
        let (left, right) = (points[0].x, points[1].x);
        points[1].x = left + (right - left) / 2.0;
        points[3].x = points[1].x;
        handles.clear();
        middle = (points[0].y + points[2].y) / 2.0;
        Ok(())
    })
    .unwrap();
    (s, e, middle)
}

/// Where the ink of the page of `s` starts and ends across height `y`, as the canvas draws it.
fn ink_across(s: &Session, y: f64) -> (f64, f64) {
    let scale = 8.0;
    let region = Rect::new(0.0, y - 0.5, 800.0, y + 0.5);
    let r = vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, region, scale, true);
    let row = (r.height / 2) as usize;
    let inked: Vec<usize> = (0..r.width as usize).filter(|i| r.pixels[(row * r.width as usize + i) * 4..][..3].iter().any(|c| *c < 128)).collect();
    let (first, last) = (inked.first().unwrap(), inked.last().unwrap());
    (*first as f64 / scale, (last + 1) as f64 / scale)
}

/// How wide the ink is across the middle of `make`'s object in [`squeezed`].
fn squeezed_ink(make: fn(&mut Session) -> NodeId, stroked: Stroked) -> f64 {
    let (s, _, middle) = squeezed(make, stroked);
    let (first, last) = ink_across(&s, middle);
    last - first
}

/// A capital I set 300 points tall at (100, 400), filled black.
fn tall_i(s: &mut Session) -> NodeId {
    id_of(&s.execute("text.create", &json!({"x": 100, "y": 400, "text": "I", "size": 300})).unwrap())
}

/// With Distort Appearance on, the characters' stroke of type in an envelope paints along the bent
/// glyph outlines at its own weight, as `.ai` files draw it (#6644): squeezed to half its width, the
/// stem of an I still has half the weight of stroke on each side, not a quarter.
#[test]
fn type_strokes_paint_at_their_own_weight_in_a_squeezed_envelope() {
    let (plain, stroked) = (squeezed_ink(tall_i, Stroked::No), squeezed_ink(tall_i, Stroked::Own));
    let past = stroked - plain;
    assert!((past - SQUEEZED_WEIGHT).abs() < 0.5, "the stroke reaches {past} pt past the squeezed stem, both sides together ({plain} → {stroked})");
}

/// A stroke on type's own appearance paints at its own weight too (#6644).
#[test]
fn a_stroke_on_types_appearance_paints_at_its_own_weight_in_a_squeezed_envelope() {
    let (plain, stroked) = (squeezed_ink(tall_i, Stroked::No), squeezed_ink(tall_i, Stroked::Appearance));
    let past = stroked - plain;
    assert!((past - SQUEEZED_WEIGHT).abs() < 0.5, "the stroke reaches {past} pt past the squeezed stem, both sides together ({plain} → {stroked})");
}

/// A path's stroke still bends with the art: squeezed to half its width, it reaches half as far past
/// the path's left and right sides.
#[test]
fn a_paths_stroke_still_bends_in_a_squeezed_envelope() {
    let (plain, stroked) = (squeezed_ink(rect, Stroked::No), squeezed_ink(rect, Stroked::Own));
    let past = stroked - plain;
    assert!(
        (past - SQUEEZED_WEIGHT / 2.0).abs() < 0.5,
        "the stroke reaches {past} pt past the squeezed rectangle, both sides together ({plain} → {stroked})"
    );
}

/// The type's stroke paints past the envelope's grid, where the glyphs reach its edge: the
/// envelope's visual bounds hold it (#6644).
#[test]
fn the_visual_bounds_of_an_envelope_hold_its_types_strokes() {
    let (s, e, middle) = squeezed(tall_i, Stroked::Own);
    let n = node(&s, e);
    let (visual, grid) = (n.visual_bounds().unwrap(), n.geometric_bounds().unwrap());
    let (first, last) = ink_across(&s, middle);
    assert!(first < grid.x0 - 1.0, "the stroke paints past the grid: {first} vs {grid:?}");
    // The ink is measured to whole pixels (an eighth of a point); miter spikes don't count, as in
    // type's own visual bounds.
    let pixel = 0.125;
    assert!(visual.x0 <= first + pixel && visual.x1 >= last - pixel, "{visual:?} holds the ink from {first} to {last}");
    assert!(first - visual.x0 < 1.0 && visual.x1 - last < 1.0, "{visual:?} reaches no further than the ink from {first} to {last}");
}

/// A repeat of stroked type reaches past its copies by as far as the characters' strokes paint
/// past their glyphs: half the weight of a centred stroke, as type's own visual bounds (#6644).
#[test]
fn the_visual_bounds_of_a_repeat_of_stroked_type_grow_by_half_the_stroke() {
    let mut s = session();
    let t = id_of(&s.execute("text.create", &json!({"x": 100, "y": 200, "text": "I", "size": 100})).unwrap());
    s.execute("paint.setStroke", &json!({"ids": [t.0], "color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [t.0], "weight": SQUEEZED_WEIGHT})).unwrap();
    let r = id_of(&s.execute("object.repeat.grid", &json!({"ids": [t.0], "rows": 2, "cols": 2})).unwrap());
    let n = node(&s, r);
    let (visual, geometric) = (n.visual_bounds().unwrap(), n.geometric_bounds().unwrap());
    let grown = [geometric.x0 - visual.x0, geometric.y0 - visual.y0, visual.x1 - geometric.x1, visual.y1 - geometric.y1];
    assert!(grown.iter().all(|g| (g - SQUEEZED_WEIGHT / 2.0).abs() < 1e-6), "{grown:?}");
}

/// An I stroked by `stroked` in a one-patch mesh envelope that leaves it as it is: the envelope.
fn enveloped_i(s: &mut Session, stroked: bool) -> NodeId {
    let t = tall_i(s);
    if stroked {
        s.execute("paint.setStroke", &json!({"ids": [t.0], "color": "#ff0000"})).unwrap();
        s.execute("stroke.set", &json!({"ids": [t.0], "weight": SQUEEZED_WEIGHT})).unwrap();
    }
    sel(s, &[t]);
    id_of(&s.execute("object.envelope.makeWithMesh", &json!({"rows": 1, "cols": 1})).unwrap())
}

/// Type's stroke in an envelope inside an envelope also paints at its own weight: the outer
/// envelope squeezes the inner one's type to half its width, and the stem still has half the weight
/// of stroke on each side (#6644).
#[test]
fn type_strokes_paint_at_their_own_weight_in_an_envelope_inside_an_envelope() {
    let (plain, stroked) = (squeezed_ink(|s| enveloped_i(s, false), Stroked::No), squeezed_ink(|s| enveloped_i(s, true), Stroked::No));
    let past = stroked - plain;
    assert!((past - SQUEEZED_WEIGHT).abs() < 0.5, "the stroke reaches {past} pt past the squeezed stem, both sides together ({plain} → {stroked})");
}

/// An opacity mask on type bends as other art does (#6644): with Distort Appearance its stroke
/// becomes its bent outline, as on a path, while the type's own stroke stays a stroke.
#[test]
fn an_opacity_mask_on_type_bends_as_other_art() {
    let mut s = session();
    let t = tall_i(&mut s);
    s.execute("paint.setStroke", &json!({"ids": [t.0], "color": "#ff0000"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [t.0], "weight": SQUEEZED_WEIGHT})).unwrap();
    let m = id_of(&s.execute("shape.rectangle", &json!({"x": 90, "y": 150, "width": 120, "height": 300})).unwrap());
    s.execute("paint.setStroke", &json!({"ids": [m.0], "color": "#ffffff"})).unwrap();
    s.execute("stroke.set", &json!({"ids": [m.0], "weight": SQUEEZED_WEIGHT})).unwrap();
    sel(&mut s, &[t, m]);
    s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
    assert!(node(&s, t).extras().mask.is_some(), "the type has the mask");
    sel(&mut s, &[t]);
    let e = arch(&mut s, 60.0, json!({"distortAppearance": true}));
    let (mut masks, mut kept) = (vec![], 0);
    for n in &evaluated(&s, e) {
        n.walk(&mut |c| {
            kept += c.appearance.painted_strokes().count();
            masks.extend(c.extras().mask.as_ref().map(|m| m.art.clone()));
        });
    }
    assert_eq!(masks.len(), 1, "the mask goes with the type");
    let mut in_mask = 0;
    masks[0].walk(&mut |c| in_mask += c.appearance.painted_strokes().count());
    assert_eq!(in_mask, 0, "the mask's stroke became its bent outline");
    assert_eq!(kept, 1, "the type's stroke stays a stroke");
}

/// An envelope bends the art of a linked opacity mask on its contents with them; an unlinked mask
/// stays where it is.
#[test]
fn an_envelope_bends_a_linked_opacity_mask_and_leaves_an_unlinked_one() {
    for linked in [true, false] {
        let mut s = session();
        let a = rect(&mut s);
        let m = rect(&mut s);
        sel(&mut s, &[a, m]);
        s.execute("transparency.makeOpacityMask", &json!({})).unwrap();
        if !linked {
            s.execute("transparency.unlinkOpacityMask", &json!({})).unwrap();
        }
        let before = node(&s, a).extras().mask.as_ref().unwrap().art.path_data().cloned();
        sel(&mut s, &[a]);
        let e = arch(&mut s, 60.0, json!({}));
        let mut found = vec![];
        for n in &evaluated(&s, e) {
            n.walk(&mut |c| found.extend(c.extras().mask.as_ref().map(|k| (c.path_data().cloned(), k.art.path_data().cloned()))));
        }
        let [(bent, mask)] = found.as_slice() else { panic!("one masked object: {found:?}") };
        assert!(bent != &before, "the contents bend");
        assert_eq!(mask, if linked { bent } else { &before }, "linked: {linked}");
    }
}

/// Expand turns an envelope of stroked type into its bent glyph outlines with the stroke on them,
/// at its own weight, as it draws (#6644).
#[test]
fn expanding_an_envelope_of_stroked_type_keeps_the_stroke_at_its_weight() {
    let (mut s, e, _) = squeezed(tall_i, Stroked::Own);
    sel(&mut s, &[e]);
    s.execute("object.envelope.expand", &json!({})).unwrap();
    let mut weights = vec![];
    node(&s, e).walk(&mut |c| weights.extend(c.appearance.painted_strokes().map(|st| st.width)));
    assert_eq!(weights, [SQUEEZED_WEIGHT]);
}
