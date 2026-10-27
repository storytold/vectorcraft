//! Effects on every object kind: raster effects on type, images and symbol instances through an
//! offscreen layer of their art, geometry effects through their outlines, and one fill or
//! stroke's own raster effects.

use std::sync::Arc;

use serde_json::{Value, json};
use vectorcraft_color::{Color, Gradient, GradientKind, GradientPaint, GradientStop, Paint};
use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
use vectorcraft_doc::text::{CharStyle, TextObject};
use vectorcraft_doc::{Appearance, AppearanceItem, Effect, FillLayer, ImageBlob, ImageObject, Node, NodeKind, StrokeLayer, Symbol};
use vectorcraft_geom::{Point, shapes};

use crate::*;

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
fn revolve_caps_cover_the_opening_and_match_export_and_visible_only_expansion() {
    let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M70 30 L70 70").unwrap());
    let mut n = Node::path(NodeId(0), path, Appearance::basic(Paint::solid(Color::rgb(0.9, 0.2, 0.1)), Paint::None, 0.0));
    for (angle, opacity) in [(360.0, 1.0), (90.0, 1.0), (360.0, 0.5)] {
        n.opacity = opacity;
        n.appearance.effects = vec![fx(
            "threeD.revolve",
            json!({"offset":20,"rotationX":-35,"lightAzimuth":0,
            "angle":angle,"segments":32,"cap":false,"expandVisibleOnly":true}),
        )];
        let open = render(&doc_with(Document::new(100.0, 100.0), n.clone()));
        n.appearance.effects[0].params["cap"] = json!(true);
        let d = doc_with(Document::new(100.0, 100.0), n.clone());
        let live = render(&d);
        assert_ne!(live.pixels, open.pixels, "caps change the opening: {angle}, {opacity}");
        if angle == 360.0 && opacity == 1.0 {
            let pixel = (34 * 100 + 50) * 4;
            assert!(live.pixels[pixel] as u16 > open.pixels[pixel] as u16 + 30, "the lit cap covers the visible interior");
        }
        assert_eq!(live.pixels, render(&vectorcraft_effects::bake_document(&d).unwrap()).pixels);
        let mut expanded = d.clone();
        let id = expanded.layers[0].children().unwrap()[0].id;
        let source = expanded.node(id).unwrap().clone();
        let mut stroke = |_: &mut Document, _: &vectorcraft_geom::PathData, _: vectorcraft_geom::FillRule, _: &vectorcraft_doc::StrokeLayer| None;
        let art = vectorcraft_effects::expand_leaf(&mut expanded, &source, &mut stroke).unwrap();
        *expanded.node_mut(id).unwrap() = art;
        let flat = render(&expanded);
        let difference: u64 = live.pixels.iter().zip(&flat.pixels).map(|(a, b)| a.abs_diff(*b) as u64).sum();
        assert!(difference as f64 / (live.pixels.len() as f64) < 1.0, "visible-only expansion preserves cap appearance");
        if opacity < 1.0 {
            assert_eq!(live.pixels, flat.pixels);
        }
    }
}

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

/// Extrude & Bevel, Inflate (also twisted, tapered and hollow) and Rotate draw on the canvas exactly
/// as they export, filling more than the flat square with faces of differing shades (Rotate: one
/// shade, narrower).
#[test]
fn solid_effects_canvas_matches_baked_art_and_shades_their_faces() {
    for (effect, params) in [
        ("threeD.extrude", json!({"depth": 30, "bevel": "rounded"})),
        ("threeD.inflate", json!({"depth": 10})),
        ("threeD.inflate", json!({"depth": 10, "twist": 90, "taper": 60, "cap": false, "rotationX": -20, "rotationY": 35})),
        ("threeD.rotate", json!({"rotationX": 0, "rotationY": 55, "rotationZ": 0})),
    ] {
        let mut n = Node::path(
            NodeId(0),
            shapes::rectangle(vectorcraft_geom::Rect::new(30.0, 30.0, 70.0, 70.0)),
            Appearance::basic(Paint::solid(Color::rgb(0.9, 0.2, 0.1)), Paint::None, 0.0),
        );
        n.appearance.effects.push(fx(effect, params));
        let d = doc_with(Document::new(100.0, 100.0), n);
        let live = render(&d);
        let exported = render(&vectorcraft_effects::bake_document(&d).unwrap());
        assert_eq!(live.pixels, exported.pixels, "{effect}: canvas and baked/exported faces agree");
        assert_eq!(live.pixels, render_threads(&d, 4).pixels, "{effect}");
        let reds: Vec<_> =
            live.pixels.as_chunks::<4>().0.iter().filter(|p| p[0] as u16 > p[1] as u16 * 2 && p[0] as u16 > p[2] as u16 * 2).map(|p| p[0]).collect();
        let shades = reds.iter().max().unwrap() - reds.iter().min().unwrap();
        if effect == "threeD.rotate" {
            assert!(reds.len() < 40 * 40 && reds.len() > 600, "{effect}: the square foreshortened, {} px", reds.len());
        } else {
            assert!(reds.len() > 40 * 40, "{effect}: a solid larger than its front, {} px", reds.len());
            assert!(shades > 60, "{effect}: lighting varies across the faces ({shades})");
        }
    }
}

#[test]
fn smooth_revolve_reduces_banding_without_changing_radial_resolution() {
    let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M70 20 L70 80").unwrap());
    let mut n = Node::path(NodeId(0), path, Appearance::basic(Paint::solid(Color::rgb(0.2, 0.55, 0.9)), Paint::None, 0.0));
    n.appearance
        .effects
        .push(fx("threeD.revolve", json!({"offset":30,"segments":16,"lightAzimuth":0,"lightElevation":0,"ambient":10,"lightIntensity":80})));
    let d = doc_with(Document::new(100.0, 100.0), n);
    let smooth = render(&d);
    let levels = |r: &Rendered| (14..66).map(|x| r.pixels[(50 * 100 + x) * 4 + 2]).collect::<std::collections::BTreeSet<_>>().len();
    // More colour levels than radial facets proves interpolation inside the faces.
    assert!(levels(&smooth) > 16, "default rendering interpolates lighting: {} blue levels", levels(&smooth));
    let jump = |r: &Rendered| (15..66).map(|x| r.pixels[(50 * 100 + x) * 4 + 2].abs_diff(r.pixels[(50 * 100 + x - 1) * 4 + 2])).max().unwrap();
    assert!(jump(&smooth) < 20, "default lighting has no abrupt changes between face colours");
    assert_eq!(smooth.pixels, render(&vectorcraft_effects::bake_document(&d).unwrap()).pixels);
    assert_eq!(smooth.pixels, render_threads(&d, 4).pixels);
}

#[test]
fn revolve_rim_has_no_spikes_at_high_zoom_and_keeps_seams_covered() {
    use vectorcraft_doc::AppearanceItem;
    let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M70 30 L70 70").unwrap());
    let mut n = Node::path(NodeId(0), path, Appearance::basic(Paint::solid(Color::rgb(0.2, 0.55, 0.9)), Paint::None, 0.0));
    n.appearance.effects.push(fx("threeD.revolve", json!({"offset":20,"rotationX":-18,"segments":24,"cap":true})));
    let d = doc_with(Document::new(100.0, 100.0), n);
    let baked = vectorcraft_effects::bake_document(&d).unwrap();
    let mut fills = baked.clone();
    fn remove_edges(n: &mut Node) {
        n.appearance.items.retain(|i| matches!(i, AppearanceItem::Fill(_)));
        for child in n.children_mut().into_iter().flatten() {
            remove_edges(Arc::make_mut(child));
        }
    }
    for layer in &mut fills.layers {
        remove_edges(Arc::make_mut(layer));
    }
    let bounds = fills.layers[0].geometric_bounds().unwrap();
    let zoom = 24.0;
    let xf = Affine::translate((600.0 - 50.0 * zoom, 50.0 - bounds.y0 * zoom)) * Affine::scale(zoom);
    let draw =
        |doc: &Document| Renderer::new().render(doc, 1200, 400, xf, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() });
    let reference = draw(&fills);
    let live = draw(&d);
    // The seam stroke may extend half its width, plus antialiasing, past the fill
    // silhouette. Miter spikes previously reached several times that distance.
    let reach = (0.375 * zoom) as u32 + 2;
    let top = |img: &Rendered| (0..1200).map(|x| (0..400).find(|y| lum(img.pixel(x, *y)) < 600).unwrap_or(400)).collect::<Vec<_>>();
    let fill_top = top(&reference);
    for (x, y) in top(&live).into_iter().enumerate().filter(|(_, y)| *y < 400) {
        let near = *fill_top[x.saturating_sub(reach as usize)..=(x + reach as usize).min(1199)].iter().min().unwrap();
        assert!(y + reach >= near, "rim spike at x {x}: ink starts at {y}, nearest fill at {near}");
    }
    assert!((200..1000).all(|x| lum(live.pixel(x, 200)) < 600), "seam coverage leaves no white gaps across the cap");
    assert_eq!(live.pixels, draw(&baked).pixels, "live and exported geometry agree at high zoom");
}

#[test]
fn continuous_revolve_caps_keep_one_colour_along_the_curved_rim_at_high_zoom() {
    let path = vectorcraft_geom::PathData::from_bezpath(&vectorcraft_geom::BezPath::from_svg("M70 30 L70 70").unwrap());
    for rotation in [-18.0f64, 18.0] {
        for shade in [false, true] {
            let mut n = Node::path(NodeId(0), path.clone(), Appearance::basic(Paint::solid(Color::rgb(0.2, 0.55, 0.9)), Paint::None, 0.0));
            n.appearance.effects.push(fx("threeD.revolve", json!({"offset":20,"rotationX":rotation,"segments":8,"cap":true,"shade":shade})));
            let d = doc_with(Document::new(100.0, 100.0), n);
            let center_y = 50.0 + if rotation < 0.0 { -20.0 } else { 20.0 } * rotation.to_radians().cos();
            let zoom = 24.0;
            let xf = Affine::translate((600.0 - 50.0 * zoom, 220.0 - center_y * zoom)) * Affine::scale(zoom);
            let draw = |doc: &Document| {
                Renderer::new().render(doc, 1200, 440, xf, &RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() })
            };
            let live = draw(&d);
            let color = live.pixel(600, 220);
            assert!(lum(color) < 600, "the visible cap is filled");
            // Sample 0.15 pt inside the projected ellipse. A triangle fan with
            // seam strokes lets body colours intrude here between mesh corners.
            for step in 1..80 {
                let theta = std::f64::consts::PI * step as f64 / 80.0;
                let p = xf * Point::new(50.0 + 19.85 * theta.cos(), center_y + 19.85 * rotation.to_radians().abs().sin() * theta.sin());
                let pixel = live.pixel(p.x.round() as u32, p.y.round() as u32);
                assert!(
                    pixel.iter().zip(color).all(|(a, b)| a.abs_diff(b) <= 1),
                    "cap rim is continuous: rotation {rotation}, shade {shade}, theta {theta}, {pixel:?} vs {color:?}"
                );
            }
            assert_eq!(live.pixels, draw(&vectorcraft_effects::bake_document(&d).unwrap()).pixels);
        }
    }
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
    for (angle, rotation, perspective, opacity, shade) in [
        (360.0, 0.0, 0.0, 1.0, false),
        (360.0, -35.0, 55.0, 1.0, false),
        (150.0, -28.0, 40.0, 1.0, false),
        (360.0, -35.0, 55.0, 0.5, false),
        (360.0, -35.0, 55.0, 1.0, true),
        (150.0, -28.0, 40.0, 1.0, true),
        (360.0, -35.0, 55.0, 0.5, true),
    ] {
        n.opacity = opacity;
        n.appearance.effects = vec![fx(
            "threeD.revolve",
            json!({"angle":angle,"rotationX":rotation,"rotationY":25,"perspective":perspective,"segments":32,"expandVisibleOnly":true,"shade":shade}),
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
        NodeKind::Image(Box::new(ImageObject {
            key: "red".into(),
            width: 30,
            height: 30,
            xf: Affine::translate((20.0, 20.0)),
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
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

/// "IIII" in `style` with its baseline at (5, 60), its text space scaled or turned there by `xf`.
fn styled_text_doc(style: CharStyle, xf: Affine) -> Document {
    let mut t = TextObject::point(Point::new(5.0, 60.0), "IIII", style);
    t.xf *= xf;
    doc_with(Document::new(100.0, 100.0), Node::new(NodeId(0), NodeKind::Text(Box::new(t))))
}

/// Pixels whose color differs by more than 8 levels in a channel between `a` and `b`.
fn differing(a: &Rendered, b: &Rendered) -> usize {
    a.pixels
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.pixels.as_chunks::<4>().0)
        .filter(|(p, q)| p.iter().zip(q.iter()).any(|(x, y)| x.abs_diff(*y) > 8))
        .count()
}

/// Pixels that change by more than 8 levels in a channel when the first object of `d` gets Offset
/// Path at 0 pt, a geometry effect that returns its input: the object is then drawn through its
/// outlines.
fn changed_by_outlining(d: &Document) -> usize {
    let mut outlined = d.clone();
    let id = outlined.layers[0].children().unwrap()[0].id;
    outlined.node_mut(id).unwrap().appearance.effects.push(fx("path.offsetPath", json!({"offset": 0})));
    differing(&render(d), &render(&outlined))
}

#[test]
fn type_drawn_through_its_outlines_keeps_its_underline() {
    let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), underline: true, ..Default::default() };
    let d = styled_text_doc(style, Affine::IDENTITY);
    // "IIII" has no ink below its baseline: the bar is all that is drawn there.
    let plain = render(&d);
    let bar = (61..70).flat_map(|y| (0..100).map(move |x| (x, y))).filter(|&(x, y)| lum(plain.pixel(x, y)) < 300).count();
    assert!(bar > 50, "{bar} pixels of the bar");
    let changed = changed_by_outlining(&d);
    assert!(changed < 10, "{changed} pixels changed");
}

#[test]
fn type_drawn_through_its_outlines_keeps_its_stroke_weight_when_scaled() {
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let style = CharStyle { size: 20.0, fill: Paint::solid(Color::BLACK), stroke: red, stroke_width: 2.0, ..Default::default() };
    // The stroke is 2 pt wide in text space and 4 pt wide in the document.
    let changed = changed_by_outlining(&styled_text_doc(style, Affine::scale(2.0)));
    assert!(changed < 10, "{changed} pixels changed");
}

#[test]
fn type_drawn_through_its_outlines_keeps_its_gradients_in_place() {
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    g.angle = 90.0;
    let ramp = Paint::Gradient(Box::new(g));
    // On the characters, the gradient fits the layout box in text space and turns with the type.
    let characters = |xf| styled_text_doc(CharStyle { size: 40.0, fill: ramp.clone(), ..Default::default() }, xf);
    // On a fill of the type's own, it fits the bounding box of the layout box in the document.
    let mut own = styled_text_doc(CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() }, Affine::IDENTITY);
    let id = own.layers[0].children().unwrap()[0].id;
    own.node_mut(id).unwrap().appearance.items.push(AppearanceItem::Fill(FillLayer::new(ramp.clone())));
    let changed: Vec<usize> = [characters(Affine::IDENTITY), characters(Affine::rotate(0.3)), own].iter().map(changed_by_outlining).collect();
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
}

/// "IIII" at 40 pt with its baseline at (5, 60), its characters filled with a red-to-blue gradient
/// that runs left to right (0°) or bottom to top (90°).
fn gradient_text_doc(angle: f64) -> Document {
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut g = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    g.angle = angle;
    styled_text_doc(CharStyle { size: 40.0, fill: Paint::Gradient(Box::new(g)), ..Default::default() }, Affine::IDENTITY)
}

/// The Transform effects the gradient tests apply, each with the transform that does the same to
/// type `t`: 30 pt right, 20 pt down, and 50 % about the middle of its outlines.
fn transforms_of(t: &Node) -> [(Value, Affine); 3] {
    let middle = vectorcraft_effects::outline_text(t).unwrap().geometric_bounds().unwrap().center().to_vec2();
    [
        (json!({"moveH": 30}), Affine::translate((30.0, 0.0))),
        (json!({"moveV": 20}), Affine::translate((0.0, 20.0))),
        (json!({"scaleH": 50, "scaleV": 50}), Affine::translate(middle) * Affine::scale(0.5) * Affine::translate(-middle)),
    ]
}

/// A 100×100 document holding `content` in an envelope that leaves it as it is (an arc with no
/// bend), with Distort Appearance on or off.
fn envelope_doc(content: Node, distort_appearance: bool) -> Document {
    let kind = vectorcraft_doc::EnvelopeKind::Warp { style: "arc".into(), bend: 0.0, h: 0.0, v: 0.0, horizontal: true };
    let options = vectorcraft_doc::live::EnvelopeOptions { distort_appearance, ..Default::default() };
    let envelope = NodeKind::Envelope {
        content: vec![Arc::new(content)],
        kind: Box::new(kind),
        fidelity: 50.0,
        editing: false,
        options,
        frame: Box::new(Affine::IDENTITY),
    };
    doc_with(Document::new(100.0, 100.0), Node::new(NodeId(0), envelope))
}

#[test]
fn a_transform_effect_moves_and_scales_the_gradients_of_type_with_its_outlines() {
    let mut changed = vec![];
    for angle in [0.0, 90.0] {
        let d = gradient_text_doc(angle);
        let id = d.layers[0].children().unwrap()[0].id;
        // The type's own transform does the same as the Transform effect in the document it is
        // compared with.
        for (params, xf) in transforms_of(d.node(id).unwrap()) {
            let (mut effect, mut own) = (d.clone(), d.clone());
            effect.node_mut(id).unwrap().appearance.effects.push(fx("distort.transform", params));
            own.node_mut(id).unwrap().transform(xf, false);
            changed.push(differing(&render(&effect), &render(&own)));
        }
    }
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
}

#[test]
fn a_transform_effect_that_scales_type_out_of_proportion_scales_its_radial_and_diagonal_gradients() {
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let mut changed = vec![];
    // A radial gradient turns into an ellipse, and a linear one at 45° turns its isolines.
    for (kind, angle) in [(GradientKind::Radial, 0.0), (GradientKind::Linear, 45.0)] {
        let mut g = GradientPaint::new(Gradient::new(kind, stops.clone()));
        g.angle = angle;
        let d = styled_text_doc(CharStyle { size: 40.0, fill: Paint::Gradient(Box::new(g)), ..Default::default() }, Affine::IDENTITY);
        let id = d.layers[0].children().unwrap()[0].id;
        let middle = vectorcraft_effects::outline_text(d.node(id).unwrap()).unwrap().geometric_bounds().unwrap().center().to_vec2();
        // 50 % across and 150 % up and down, about the middle of the outlines.
        let xf = Affine::translate(middle) * Affine::scale_non_uniform(0.5, 1.5) * Affine::translate(-middle);
        let (mut effect, mut own) = (d.clone(), d.clone());
        effect.node_mut(id).unwrap().appearance.effects.push(fx("distort.transform", json!({"scaleH": 50, "scaleV": 150})));
        own.node_mut(id).unwrap().transform(xf, false);
        changed.push(differing(&render(&effect), &render(&own)));
    }
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
}

#[test]
fn a_transform_effect_on_a_group_symbol_or_envelope_moves_and_scales_the_gradients_of_its_type() {
    // The gradient runs left to right, so a move to the right goes along it and a move down
    // across it.
    let d = gradient_text_doc(0.0);
    let id = d.layers[0].children().unwrap()[0].id;
    let mut t = d.node(id).unwrap().clone();
    t.id = NodeId(500);
    // The type in a group, as the art of a symbol, and in an envelope that leaves it as it is.
    let mut symbols = Document::new(100.0, 100.0);
    symbols.symbols.push(Symbol { name: "Type".into(), art: Arc::new(t.clone()) });
    let holders = [
        doc_with(Document::new(100.0, 100.0), Node::group(NodeId(0), vec![Arc::new(t.clone())])),
        doc_with(symbols, Node::new(NodeId(0), NodeKind::SymbolInstance { symbol: "Type".into(), xf: Affine::IDENTITY })),
        envelope_doc(t.clone(), false),
    ];
    let mut changed = vec![];
    // The Transform effect is on the group, the symbol instance or the envelope. The type's own
    // transform does the same in the document it is compared with.
    for (params, xf) in transforms_of(&t) {
        let mut own = d.clone();
        own.node_mut(id).unwrap().transform(xf, false);
        let reference = render(&own);
        for h in &holders {
            let mut effect = h.clone();
            let holder = effect.layers[0].children().unwrap()[0].id;
            effect.node_mut(holder).unwrap().appearance.effects.push(fx("distort.transform", params.clone()));
            changed.push(differing(&render(&effect), &reference));
        }
    }
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
}

#[test]
fn type_in_an_envelope_moves_and_scales_its_gradients_with_its_own_transform_effect() {
    let d = gradient_text_doc(0.0);
    let id = d.layers[0].children().unwrap()[0].id;
    let mut changed = vec![];
    for (params, xf) in transforms_of(d.node(id).unwrap()) {
        let mut own = d.clone();
        own.node_mut(id).unwrap().transform(xf, false);
        let reference = render(&own);
        let mut t = d.node(id).unwrap().clone();
        t.id = NodeId(500);
        t.appearance.effects.push(fx("distort.transform", params));
        // The envelope leaves the type as it is, with Distort Appearance off and on.
        for distort in [false, true] {
            changed.push(differing(&render(&envelope_doc(t.clone(), distort)), &reference));
        }
    }
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
}

#[test]
fn type_drawn_through_its_outlines_keeps_its_pattern_tiles_on_the_type() {
    // Pattern `Checks` is a red 2 × 2 square in a 4 × 4 tile.
    let red = Appearance::basic(Paint::solid(Color::rgb(1.0, 0.0, 0.0)), Paint::None, 0.0);
    let mut def = PatternDef::new("Checks", vec![Arc::new(Node::path(NodeId(900), shapes::rectangle(Rect::new(0.0, 0.0, 2.0, 2.0)), red))]);
    def.tile = Rect::new(0.0, 0.0, 4.0, 4.0);
    // The type is 40 pt in the document: 20 pt type scaled 2×, or 40 pt type turned.
    let changed: Vec<usize> = [Affine::scale(2.0), Affine::rotate(0.3)]
        .into_iter()
        .map(|xf| {
            let mut d = styled_text_doc(CharStyle { size: 40.0 / xf.determinant().sqrt(), fill: pattern_paint("Checks"), ..Default::default() }, xf);
            d.patterns.push(def.clone());
            changed_by_outlining(&d)
        })
        .collect();
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
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
fn type_in_a_symbols_art_keeps_its_opacity_under_the_instances_effects() {
    // The symbol's art is 50 % black type. Offset Path 0 and a shadow at 0 % opacity on the
    // instance paint nothing of their own, so the instance is drawn the same with them as without
    // them.
    let doc = |effects| {
        let mut d = symbol_doc(effects);
        let style = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), ..Default::default() };
        let mut t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(-45.0, 10.0), "IIII", style))));
        t.opacity = 0.5;
        d.symbols[0].art = Arc::new(t);
        d
    };
    let plain = render(&doc(vec![]));
    let offset = fx("path.offsetPath", json!({"offset": 0}));
    let shadow = fx("stylize.dropShadow", json!({"x": 10, "y": 10, "blur": 0, "opacity": 0, "mode": "normal"}));
    for effect in [offset, shadow] {
        let img = render(&doc(vec![effect.clone()]));
        let worst = plain.pixels.iter().zip(&img.pixels).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst <= 4, "{}: max channel difference {worst}", effect.id);
    }
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
    let art = vectorcraft_effects::reshape(None, n, None).unwrap();
    let (mut text, mut black) = (0, 0);
    art.walk(&mut |m| {
        text += usize::from(matches!(m.kind, NodeKind::Text(_)));
        black += usize::from(m.appearance.fill_paint() == Paint::solid(Color::BLACK));
    });
    assert!(text == 0 && black > 0, "{art:?}");
}

/// Pixels that change by more than 8 levels in a channel when the first object of `d` gets
/// Outline Object.
fn changed_by_outline_object(d: &Document) -> usize {
    let mut outlined = d.clone();
    let id = outlined.layers[0].children().unwrap()[0].id;
    outlined.node_mut(id).unwrap().appearance.effects.push(fx("path.outlineObject", json!({})));
    differing(&render(d), &render(&outlined))
}

#[test]
fn outline_object_draws_type_as_its_outlines_and_leaves_its_look() {
    // The type is drawn through its outlines: plain paths painted with the text's fill.
    let d = text_doc(vec![fx("path.outlineObject", json!({}))]);
    let n = &d.layers[0].children().unwrap()[0];
    let art = vectorcraft_effects::reshape(None, n, None).unwrap();
    let (mut text, mut black) = (0, 0);
    art.walk(&mut |m| {
        text += usize::from(matches!(m.kind, NodeKind::Text(_)));
        black += usize::from(m.appearance.fill_paint() == Paint::solid(Color::BLACK));
    });
    assert!(text == 0 && black > 0, "{art:?}");
    // Plain, underlined and scaled stroked type, and 50 % type in a group that carries the effect,
    // look the same with it.
    let underlined = CharStyle { size: 40.0, fill: Paint::solid(Color::BLACK), underline: true, ..Default::default() };
    let red = Paint::solid(Color::rgb(1.0, 0.0, 0.0));
    let stroked = CharStyle { size: 20.0, fill: Paint::solid(Color::BLACK), stroke: red, stroke_width: 2.0, ..Default::default() };
    let mut t = text_doc(vec![]).layers[0].children().unwrap()[0].as_ref().clone();
    t.id = NodeId(500);
    t.opacity = 0.5;
    let docs = [
        text_doc(vec![]),
        styled_text_doc(underlined, Affine::IDENTITY),
        styled_text_doc(stroked, Affine::scale(2.0)),
        doc_with(Document::new(100.0, 100.0), Node::group(NodeId(0), vec![Arc::new(t)])),
    ];
    let changed: Vec<usize> = docs.iter().map(changed_by_outline_object).collect();
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
}

#[test]
fn outline_object_leaves_paths_and_images_as_they_are() {
    let ap = Appearance::basic(Paint::solid(Color::rgb(0.0, 0.0, 1.0)), Paint::solid(Color::BLACK), 4.0);
    let path = doc_with(Document::new(100.0, 100.0), Node::path(NodeId(0), shapes::ellipse(Rect::new(20.0, 20.0, 80.0, 70.0)), ap));
    let changed: Vec<usize> = [path, image_doc(vec![])].iter().map(changed_by_outline_object).collect();
    assert!(changed.iter().all(|&n| n < 10), "pixels changed: {changed:?}");
    // The image is drawn through its frame, as under any geometry effect.
    let d = image_doc(vec![fx("path.outlineObject", json!({}))]);
    assert!(vectorcraft_effects::reshape(None, &d.layers[0].children().unwrap()[0], None).is_some());
}

#[test]
fn a_rectangle_on_a_fill_below_type_with_outline_object_fits_the_glyphs() {
    // "IIII" with a red fill of its own below the characters that carries Convert to Shape ›
    // Rectangle (the default 18 pt extra width and height).
    let plain = render(&text_doc(vec![]));
    let inked = inked_columns(&plain);
    let gap = (inked[0]..*inked.last().unwrap()).find(|x| !inked.contains(x)).unwrap();
    let mut d = text_doc(vec![fx("path.outlineObject", json!({}))]);
    let id = d.layers[0].children().unwrap()[0].id;
    let mut fill = FillLayer::new(Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    fill.effects.push(fx("convertToShape.rectangle", json!({})));
    let ap = &mut d.node_mut(id).unwrap().appearance;
    ap.items.push(AppearanceItem::Fill(fill));
    ap.set_contents_at(1);
    let img = render(&d);
    // The rectangle is under the glyphs and fills the gap between two of them.
    assert_eq!(img.pixel(gap, 50), [255, 0, 0, 255], "column {gap}");
    assert_eq!(lum(img.pixel(inked[0] + 1, 50)), 0);
    // It reaches 9 pt past the glyph outlines on each side.
    let glyphs = vectorcraft_effects::outline_text(d.node(id).unwrap()).unwrap().geometric_bounds().unwrap();
    let (x, y) = (glyphs.center().x as u32, glyphs.center().y as u32);
    assert_eq!(img.pixel(glyphs.x1 as u32 + 6, y), [255, 0, 0, 255]);
    assert_eq!(lum(img.pixel(glyphs.x1 as u32 + 12, y)), WHITE);
    assert_eq!(img.pixel(x, glyphs.y1 as u32 + 6), [255, 0, 0, 255]);
    assert_eq!(lum(img.pixel(x, glyphs.y1 as u32 + 12)), WHITE);
}

/// [`image_doc`] with a black 4 pt stroke of its own that carries the effects `row`, below the
/// image when `below`.
fn stroked_image_doc(row: Vec<Effect>, below: bool) -> Document {
    let mut d = image_doc(vec![]);
    let id = d.layers[0].children().unwrap()[0].id;
    let mut stroke = StrokeLayer::new(Paint::solid(Color::BLACK), 4.0);
    stroke.effects = row;
    let ap = &mut d.node_mut(id).unwrap().appearance;
    ap.items.push(AppearanceItem::Stroke(stroke));
    ap.set_contents_at(usize::from(below));
    d
}

#[test]
fn an_images_stroke_draws_only_the_shape_convert_to_shape_or_outline_object_gives_it() {
    // The image's frame runs from 20 to 50. Without an effect the stroke draws nothing.
    for below in [false, true] {
        let img = render(&stroked_image_doc(vec![], below));
        assert_eq!((lum(img.pixel(18, 35)), img.pixel(21, 35)), (WHITE, [255, 0, 0, 255]));
    }
    let rectangle = fx("convertToShape.rectangle", json!({"relative": true, "extraW": 0, "extraH": 0}));
    let outline = fx("path.outlineObject", json!({}));
    for effect in [rectangle, outline] {
        // Below the image only the stroke's outer half shows, above it both halves.
        let img = render(&stroked_image_doc(vec![effect.clone()], true));
        assert_eq!((lum(img.pixel(17, 35)), lum(img.pixel(18, 35)), img.pixel(21, 35)), (WHITE, 0, [255, 0, 0, 255]), "{}", effect.id);
        let img = render(&stroked_image_doc(vec![effect.clone()], false));
        assert_eq!((lum(img.pixel(18, 35)), lum(img.pixel(21, 35)), img.pixel(23, 35)), (0, 0, [255, 0, 0, 255]), "{}", effect.id);
        // An effect before it is left out.
        let first = fx("path.offsetPath", json!({"offset": 10}));
        let img = render(&stroked_image_doc(vec![first, effect.clone()], true));
        assert_eq!((lum(img.pixel(10, 35)), lum(img.pixel(18, 35))), (WHITE, 0), "{}", effect.id);
    }
    // An Offset Path of 2 pt after Outline Object moves the stroke 2 pt out.
    let img = render(&stroked_image_doc(vec![fx("path.outlineObject", json!({})), fx("path.offsetPath", json!({"offset": 2}))], true));
    assert_eq!((lum(img.pixel(15, 35)), lum(img.pixel(16, 35)), lum(img.pixel(19, 35))), (WHITE, 0, 0));
}

#[test]
fn an_images_stroke_is_drawn_when_only_it_is_in_view() {
    // The stroke runs 8 pt outside the frame, from 10 to 14; the view shows x 15 and less.
    let row = vec![fx("path.outlineObject", json!({})), fx("path.offsetPath", json!({"offset": 8}))];
    let d = stroked_image_doc(row, true);
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let img = Renderer::new().render(&d, 100, 100, Affine::translate((85.0, 0.0)), &opts);
    assert_eq!((lum(img.pixel(96, 35)), lum(img.pixel(99, 35))), (0, WHITE));
}

#[test]
fn rasterize_on_an_image_takes_in_its_strokes_shape() {
    // The stroke runs 8 pt outside the frame, from 10 to 14; Rasterize at 72 ppi keeps it.
    let row = vec![fx("path.outlineObject", json!({})), fx("path.offsetPath", json!({"offset": 8}))];
    let mut d = stroked_image_doc(row, true);
    let id = d.layers[0].children().unwrap()[0].id;
    d.node_mut(id).unwrap().appearance.effects.push(fx("rasterize", json!({"useDocumentResolution": false, "resolution": 72})));
    let img = render(&d);
    assert_eq!((lum(img.pixel(8, 35)), lum(img.pixel(12, 35)), img.pixel(35, 35)), (WHITE, 0, [255, 0, 0, 255]));
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

#[test]
fn a_path_with_its_own_transform_paints_each_copy_with_its_own_gradient() {
    use vectorcraft_color::GradientGeom;
    let stops = vec![GradientStop::new(0.0, Color::rgb(1.0, 0.0, 0.0)), GradientStop::new(1.0, Color::rgb(0.0, 0.0, 1.0))];
    let ramp = GradientPaint::new(Gradient::new(GradientKind::Linear, stops));
    let geom = GradientGeom { start: Point::new(0.0, 50.0), end: Point::new(20.0, 50.0), aspect: 1.0, focal: None };
    // Fitted to the box on each render, and placed on the object.
    for paint in [ramp.clone(), GradientPaint { geom: Some(geom), ..ramp }] {
        let square = shapes::rectangle(vectorcraft_geom::Rect::new(0.0, 40.0, 20.0, 60.0));
        let mut n = Node::path(NodeId(0), square, Appearance::basic(Paint::Gradient(Box::new(paint)), Paint::None, 0.0));
        n.appearance.effects = vec![fx("distort.transform", json!({"moveH": 40, "copies": 1}))];
        let img = render(&doc_with(Document::new(100.0, 100.0), n));
        let at = |x: usize| img.pixels.chunks(4).nth(50 * 100 + x).map(|p| (p[0], p[2])).unwrap_or_default();
        // The left edge of each copy is red, its right edge blue.
        for (left, right) in [(1, 18), (41, 58)] {
            assert!(at(left).0 > 200 && at(left).1 < 60, "{:?} at {left}", at(left));
            assert!(at(right).1 > 200 && at(right).0 < 60, "{:?} at {right}", at(right));
        }
    }
}
