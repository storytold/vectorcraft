//! Raster effects in PDF on every kind of object: groups, layers, type, images, symbol instances,
//! live objects and single fills and strokes are written with images of their shadows, glows and
//! blurs (no "left out" warning), and the PDF reads back looking like the canvas.

use std::sync::Arc;

use serde_json::json;
use vectorcraft_color::{Color, Paint};
use vectorcraft_doc::{Appearance, CharStyle, Document, Effect, ImageBlob, ImageObject, Node, NodeKind, Symbol, TextObject};
use vectorcraft_geom::{Affine, Point, Rect, shapes};
use vectorcraft_render::{Rendered, Renderer};

use crate::*;

fn shadow() -> Effect {
    Effect { id: "stylize.dropShadow".into(), params: json!({"x": 6.0, "y": 6.0, "blur": 3.0, "opacity": 75.0}), visible: true }
}

fn blur() -> Effect {
    Effect { id: "blur.gaussian".into(), params: json!({"radius": 3.0}), visible: true }
}

fn rect(d: &mut Document, r: Rect, fill: Color) -> Node {
    Node::path(d.alloc_id(), shapes::rectangle(r), Appearance::basic(Paint::solid(fill), Paint::None, 0.0))
}

fn add(d: &mut Document, n: Node) {
    let l = d.layers[0].id;
    d.insert(Some(l), usize::MAX, n).unwrap();
}

fn with(mut n: Node, fx: Effect) -> Node {
    n.appearance.effects.push(fx);
    n
}

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = image::RgbaImage::from_fn(w, h, |x, _| image::Rgba([(x * 255 / w) as u8, 60, 160, 255]));
    let mut out = Vec::new();
    {
        let __dyn: image::DynamicImage = image::DynamicImage::from(img);
        let (__w, __h) = (__dyn.width(), __dyn.height());
        let (__layout, __sample, __raw) = match __dyn {
            image::DynamicImage::ImageLuma8(b) => (craft_image_codecs::ChannelLayout::Gray, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLumaA8(b) => (craft_image_codecs::ChannelLayout::GrayA, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgb8(b) => (craft_image_codecs::ChannelLayout::Rgb, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageRgba8(b) => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, b.into_raw()),
            image::DynamicImage::ImageLuma16(b) => (
                craft_image_codecs::ChannelLayout::Gray,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageLumaA16(b) => (
                craft_image_codecs::ChannelLayout::GrayA,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgb16(b) => (
                craft_image_codecs::ChannelLayout::Rgb,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            image::DynamicImage::ImageRgba16(b) => (
                craft_image_codecs::ChannelLayout::Rgba,
                craft_image_codecs::SampleType::U16,
                b.into_raw().into_iter().flat_map(|s| s.to_ne_bytes()).collect(),
            ),
            other => (craft_image_codecs::ChannelLayout::Rgba, craft_image_codecs::SampleType::U8, other.into_rgba8().into_raw()),
        };
        let __cimg = craft_image_codecs::Image::from_raw(__w, __h, __layout, __sample, __raw).unwrap();
        craft_image_codecs::encode(&__cimg, craft_image_codecs::Format::Png, &craft_image_codecs::EncodeOptions::default())
            .map(|__b| out.extend_from_slice(&__b))
    }
    .unwrap();
    out
}

fn render(d: &Document) -> Rendered {
    Renderer::new().render_region(d, Rect::new(0.0, 0.0, 200.0, 200.0), 1.0, true)
}

/// Mean difference of the colour channels of two renders, as a share of full scale.
fn mean_diff(a: &Rendered, b: &Rendered) -> f64 {
    let sum: u64 = a.pixels.iter().zip(&b.pixels).map(|(x, y)| x.abs_diff(*y) as u64).sum();
    sum as f64 / a.pixels.len() as f64 / 255.0
}

/// The PDF of `d` embeds images, warns about nothing and reads back within 2% of the canvas, much
/// closer than the art without its raster effects.
fn assert_written(what: &str, d: &Document) {
    let r = crate::cmd::rasterfx::export_pdf_with_report(d, &Default::default()).unwrap();
    assert!(r.warnings.is_empty(), "{what}: {:?}", r.warnings);
    assert!(String::from_utf8_lossy(&r.bytes).contains("/Subtype/Image"), "{what}: an image XObject");
    let canvas = render(d);
    let diff = mean_diff(&canvas, &render(&vectorcraft_pdf::import(&r.bytes).unwrap()));
    let mut plain = d.clone();
    for l in &mut plain.layers {
        crate::cmd::rasterfx::strip_raster(Arc::make_mut(l), true);
    }
    for s in &mut plain.symbols {
        crate::cmd::rasterfx::strip_raster(Arc::make_mut(&mut s.art), true);
    }
    let without = mean_diff(&canvas, &render(&plain));
    assert!(diff < 0.02 && diff < without / 4.0, "{what}: read back {:.2}% off ({:.2}% without the effects)", diff * 100.0, without * 100.0);
}

#[test]
fn groups_layers_type_images_and_symbols_keep_their_raster_effects() {
    // A group's shadow (one shadow of both members).
    let mut d = Document::new(200.0, 200.0);
    let (a, b) = (
        rect(&mut d, Rect::new(30.0, 30.0, 90.0, 90.0), Color::rgb(1.0, 0.0, 0.0)),
        rect(&mut d, Rect::new(70.0, 70.0, 150.0, 150.0), Color::rgb(0.0, 0.0, 1.0)),
    );
    let g = Node::group(d.alloc_id(), vec![Arc::new(a), Arc::new(b)]);
    add(&mut d, with(g, shadow()));
    assert_written("group", &d);

    // A layer's blur.
    let mut d = Document::new(200.0, 200.0);
    let r = rect(&mut d, Rect::new(40.0, 40.0, 160.0, 120.0), Color::rgb(0.1, 0.6, 0.2));
    add(&mut d, r);
    Arc::make_mut(&mut d.layers[0]).appearance.effects.push(blur());
    assert_written("layer", &d);

    // Type with a shadow.
    let mut d = Document::new(200.0, 200.0);
    let style = CharStyle { size: 60.0, fill: Paint::solid(Color::rgb(0.8, 0.1, 0.1)), ..Default::default() };
    let t = Node::new(d.alloc_id(), NodeKind::Text(Box::new(TextObject::point(Point::new(20.0, 120.0), "Ab", style))));
    add(&mut d, with(t, shadow()));
    assert_written("type", &d);

    // An image with a shadow.
    let mut d = Document::new(200.0, 200.0);
    d.images.insert("pic".into(), ImageBlob::new("image/png", png(40, 30)));
    let xf = Affine::translate((40.0, 50.0)) * Affine::scale(2.5);
    let im = Node::new(
        d.alloc_id(),
        NodeKind::Image(Box::new(ImageObject {
            key: "pic".into(),
            width: 40,
            height: 30,
            xf,
            link: None,
            placement: Default::default(),
            pixels: None,
        })),
    );
    add(&mut d, with(im, shadow()));
    assert_written("image", &d);

    // A symbol instance with a shadow, and a symbol whose art blurs.
    let mut d = Document::new(200.0, 200.0);
    let art = rect(&mut d, Rect::new(0.0, 0.0, 50.0, 50.0), Color::rgb(0.9, 0.6, 0.0));
    d.symbols.push(Symbol { dynamic: false, name: "Tile".into(), art: Arc::new(art) });
    let blurred = with(rect(&mut d, Rect::new(0.0, 0.0, 40.0, 40.0), Color::rgb(0.2, 0.2, 0.8)), blur());
    d.symbols.push(Symbol { dynamic: false, name: "Soft".into(), art: Arc::new(blurred) });
    let inst = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Tile".into(), xf: Affine::translate((30.0, 30.0)) });
    add(&mut d, with(inst, shadow()));
    let soft = Node::new(d.alloc_id(), NodeKind::SymbolInstance { symbol: "Soft".into(), xf: Affine::translate((120.0, 120.0)) });
    add(&mut d, soft);
    assert_written("symbols", &d);
}

#[test]
fn live_objects_and_single_fills_and_strokes_keep_their_raster_effects() {
    // A blend with a shadow.
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let a = s.execute("shape.rectangle", &json!({"x": 20, "y": 40, "width": 30, "height": 30})).unwrap()["id"].clone();
    let b = s.execute("shape.rectangle", &json!({"x": 140, "y": 120, "width": 30, "height": 30})).unwrap()["id"].clone();
    s.execute("select.set", &json!({"ids": [a, b]})).unwrap();
    let g = s.execute("object.blend.make", &json!({"steps": 3})).unwrap()["id"].clone();
    s.execute("effect.apply", &json!({"effect": "stylize.dropShadow", "ids": [g]})).unwrap();
    assert_written("blend", &s.doc().unwrap().doc);

    // A stroke's shadow falls on the fill below it; a fill's blur stays under a sharp stroke.
    let wide_blur = Effect { params: json!({"radius": 12.0}), ..blur() };
    for (item, fx) in [(1, shadow()), (0, wide_blur)] {
        let mut d = Document::new(200.0, 200.0);
        let mut n = Node::path(
            d.alloc_id(),
            shapes::rectangle(Rect::new(40.0, 40.0, 160.0, 160.0)),
            Appearance::basic(Paint::solid(Color::rgb(1.0, 0.9, 0.2)), Paint::solid(Color::rgb(0.1, 0.1, 0.6)), 6.0),
        );
        n.appearance.effects_mut(Some(item)).unwrap().push(fx);
        add(&mut d, n);
        assert_written(&format!("item {item}"), &d);
        let flat = crate::flatten_raster_effects(&d).unwrap();
        let pieces = flat.layers[0].children().unwrap()[0].children().map_or(0, |c| c.len());
        assert_eq!(pieces, 2, "item {item}: one piece per fill and stroke");
    }
}

#[test]
fn shadows_and_glows_keep_their_blend_modes() {
    // #827: a red Screen glow over grey can't darken it, as Normal would.
    let glow = Effect {
        id: "stylize.outerGlow".into(),
        params: json!({"mode": "screen", "color": "#ff0000", "opacity": 100.0, "blur": 20.0}),
        visible: true,
    };
    let mut d = Document::new(200.0, 200.0);
    let back = rect(&mut d, Rect::new(0.0, 0.0, 200.0, 200.0), Color::rgb(0.5, 0.5, 0.5));
    add(&mut d, back);
    let dot = rect(&mut d, Rect::new(70.0, 70.0, 130.0, 130.0), Color::rgb(0.12, 0.12, 0.12));
    add(&mut d, with(dot, glow.clone()));
    assert_written("screen glow", &d);
    let r = crate::cmd::rasterfx::export_pdf_with_report(&d, &Default::default()).unwrap();
    let back = render(&vectorcraft_pdf::import(&r.bytes).unwrap());
    for x in [133, 138, 145] {
        let i = (100 * back.width as usize + x) * 4;
        let (red, green, blue) = (back.pixels[i], back.pixels[i + 1], back.pixels[i + 2]);
        assert!(red > green && green >= 124 && blue >= 124, "x {x}: {:?}", (red, green, blue));
    }

    // A Multiply shadow and the Screen glow: an image each, composited with its mode.
    let mut d = Document::new(200.0, 200.0);
    let dot = rect(&mut d, Rect::new(70.0, 70.0, 130.0, 130.0), Color::rgb(0.12, 0.12, 0.12));
    add(&mut d, with(with(dot, shadow()), glow));
    let flat = crate::flatten_raster_effects(&d).unwrap();
    let modes: Vec<_> =
        flat.layers[0].children().unwrap()[0].children().unwrap().iter().filter(|c| matches!(c.kind, NodeKind::Image(_))).map(|c| c.blend).collect();
    assert_eq!(modes, [vectorcraft_color::BlendMode::Multiply, vectorcraft_color::BlendMode::Screen]);
}

/// Each copy of a group's Transform effect has its own image of the shadow listed before the
/// effect, under that copy: the squares' shadow (40..60) paints over their copy moved 30 pt right
/// (50..70), as on the canvas, and the copies' objects have ids of their own.
#[test]
fn each_copys_shadow_on_a_group_flattens_under_that_copy() {
    let mut d = Document::new(200.0, 200.0);
    let members = [20.0, 30.0].map(|x| Arc::new(rect(&mut d, Rect::new(x, 40.0, x + 10.0, 60.0), Color::rgb(1.0, 0.0, 0.0))));
    let mut g = Node::group(d.alloc_id(), members.to_vec());
    g.appearance.effects = vec![
        Effect {
            id: "stylize.dropShadow".into(),
            params: json!({"x": 20.0, "y": 0.0, "blur": 0.0, "opacity": 100.0, "mode": "normal", "color": "#00ff00"}),
            visible: true,
        },
        Effect { id: "distort.transform".into(), params: json!({"moveH": 30.0, "copies": 1}), visible: true },
    ];
    add(&mut d, g);
    let canvas = render(&d);
    let flat = crate::flatten_raster_effects(&d).unwrap();
    let back = render(&flat);
    let at = |r: &Rendered, x: usize| {
        let i = (50 * r.width as usize + x) * 4;
        (r.pixels[i], r.pixels[i + 1], r.pixels[i + 2])
    };
    for x in [55, 65, 85] {
        let (want, got) = (at(&canvas, x), at(&back, x));
        assert!(want.0.abs_diff(got.0) < 32 && want.1.abs_diff(got.1) < 32 && want.2.abs_diff(got.2) < 32, "x {x}: {want:?} {got:?}");
    }
    assert!(at(&back, 55).1 > 200 && at(&back, 55).0 < 60, "the squares' shadow over the copy: {:?}", at(&back, 55));
    let (mut seen, mut count) = (std::collections::HashSet::new(), 0);
    for l in &flat.layers {
        l.walk(&mut |n| {
            seen.insert(n.id);
            count += 1;
        });
    }
    assert_eq!(seen.len(), count, "each object has an id of its own");
}

#[test]
fn pdf_resolution_cap_warns_once_for_original_object_when_appearance_items_split() {
    let mut d = Document::new(300.0, 300.0);
    d.raster_effects_ppi = 2400.0;
    d.raster_effects.anti_alias = false;
    let mut n = Node::path(
        d.alloc_id(),
        shapes::rectangle(Rect::new(20.0, 20.0, 270.0, 270.0)),
        Appearance::basic(Paint::solid(Color::BLACK), Paint::solid(Color::BLACK), 20.0),
    );
    let id = n.id;
    n.name = Some("Large original".into());
    for item in &mut n.appearance.items {
        item.effects_mut().push(Effect { params: json!({"radius": 0.0}), ..blur() });
    }
    add(&mut d, n);

    let r = crate::cmd::rasterfx::export_pdf_with_report(&d, &Default::default()).unwrap();

    assert!(String::from_utf8_lossy(&r.bytes).contains("/Subtype/Image"));
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    let warning = &r.warnings[0];
    assert!(warning.contains(&format!("object {}", id.0)) && warning.contains("Large original"), "{warning}");
    assert!(warning.contains("requested 2400.0 ppi") && warning.contains("achieved 1268.7 ppi"), "{warning}");
}

#[test]
fn pdf_resolution_cap_stays_quiet_when_small_effect_meets_requested_ppi() {
    let mut d = Document::new(50.0, 50.0);
    d.raster_effects_ppi = 300.0;
    let n = with(rect(&mut d, Rect::new(10.0, 10.0, 30.0, 30.0), Color::BLACK), blur());
    add(&mut d, n);

    let r = crate::cmd::rasterfx::export_pdf_with_report(&d, &Default::default()).unwrap();

    assert!(String::from_utf8_lossy(&r.bytes).contains("/Subtype/Image"));
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}
