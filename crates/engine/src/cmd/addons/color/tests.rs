//! Color Effects: its live effects through effect.apply, and Apply as Filter.

use serde_json::json;
use vectorcraft_doc::NodeId;
use vectorcraft_geom::Rect;

use crate::Session;

/// A document with a selected red rectangle, unstroked.
fn red_rect() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 20, "y": 20, "width": 60, "height": 60})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"color": "#ff0000"})).unwrap();
    s.execute("paint.setStroke", &json!({"none": true})).unwrap();
    (s, id)
}

fn fill_hex(s: &Session, id: NodeId) -> String {
    s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().color().unwrap().to_hex()
}

fn pixel(s: &Session) -> [u8; 4] {
    vectorcraft_render::Renderer::new().render_region(&s.doc().unwrap().doc, Rect::new(0.0, 0.0, 100.0, 100.0), 1.0, true).pixel(50, 50)
}

#[test]
fn the_live_effects_draw_without_changing_the_colours() {
    for (effect, params, drawn) in [
        ("addon.color.invert", json!({}), [0, 255, 255, 255]),
        ("addon.color.desaturate", json!({}), [128, 128, 128, 255]),
        ("addon.color.exposure", json!({"exposure": -20}), [0, 0, 0, 255]),
    ] {
        let (mut s, id) = red_rect();
        s.execute("effect.apply", &json!({"effect": effect, "params": params})).unwrap();
        assert_eq!(fill_hex(&s, id), "#ff0000", "{effect}");
        assert_eq!(pixel(&s), drawn, "{effect}");
    }
}

#[test]
fn a_filter_recolours_the_object_as_one_undo_step() {
    let (mut s, id) = red_rect();
    let undo = s.doc().unwrap().history.undo.len();
    let v = s.execute("addon.color.filter", &json!({"effect": "addon.color.invert"})).unwrap();
    assert_eq!(v["changed"], 1);
    assert_eq!(fill_hex(&s, id), "#00ffff");
    let n = s.doc().unwrap().doc.node(id).unwrap().clone();
    assert!(n.appearance.effects.is_empty(), "no effect is added");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(fill_hex(&s, id), "#ff0000");
}

#[test]
fn a_filter_takes_any_colour_adjustment_with_its_params() {
    let (mut s, id) = red_rect();
    s.execute("addon.color.filter", &json!({"effect": "adjust.hueSaturation", "params": {"hue": 120}})).unwrap();
    assert_eq!(fill_hex(&s, id), "#00ff00");
    // Live effects already on the object stay live, on top of the new colours.
    s.execute("effect.apply", &json!({"effect": "addon.color.invert"})).unwrap();
    s.execute("addon.color.filter", &json!({"effect": "addon.color.desaturate", "ids": [id.0]})).unwrap();
    assert_eq!(fill_hex(&s, id), "#808080");
    assert_eq!(s.doc().unwrap().doc.node(id).unwrap().appearance.effects.len(), 1);
    // The stroke only: the fill stays.
    s.execute("addon.color.filter", &json!({"effect": "addon.color.invert", "fill": false})).unwrap();
    assert_eq!(fill_hex(&s, id), "#808080");
}

#[test]
fn bad_requests_are_refused() {
    let (mut s, id) = red_rect();
    let undo = s.doc().unwrap().history.undo.len();
    for p in [
        json!({}),
        json!({"effect": 3}),
        json!({"effect": "distort.roughen"}),
        json!({"effect": "adjust.nothing"}),
        json!({"effect": "addon.color.invert", "params": [1]}),
        json!({"effect": "addon.color.invert", "ids": [999_999]}),
        json!({"effect": "addon.color.invert", "ids": ["x"]}),
    ] {
        assert!(s.execute("addon.color.filter", &p).is_err(), "{p}");
    }
    assert_eq!(fill_hex(&s, id), "#ff0000");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    s.execute("select.none", &json!({})).unwrap();
    assert!(s.execute("addon.color.filter", &json!({"effect": "addon.color.invert"})).is_err(), "nothing selected");
}

/// An embedded image object of `blob`, selected.
fn image(s: &mut Session, key: &str, blob: vectorcraft_doc::ImageBlob, x: f64) -> NodeId {
    s.edit("Place", |d, sel| {
        d.images.insert(key.into(), blob);
        let id = d.alloc_id();
        let im = vectorcraft_doc::ImageObject {
            key: key.into(),
            width: 4,
            height: 4,
            xf: vectorcraft_geom::Affine::translate((x, 0.0)) * vectorcraft_geom::Affine::scale(10.0),
            link: None,
            placement: Default::default(),
            pixels: None,
        };
        d.insert(d.default_layer(), 0, vectorcraft_doc::Node::new(id, vectorcraft_doc::NodeKind::Image(Box::new(im))))?;
        sel.set([id]);
        Ok(id)
    })
    .unwrap()
}

fn png(img: image::DynamicImage) -> vectorcraft_doc::ImageBlob {
    let mut bytes = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
    vectorcraft_doc::ImageBlob::png(bytes)
}

fn selected(s: &Session) -> Vec<NodeId> {
    s.doc().unwrap().selection.objects.clone()
}

#[test]
fn select_by_color_kind_finds_spot_colours_effects_and_image_models() {
    let (mut s, plain) = red_rect();
    // A spot colour, inside a group: the path is taken, not the group.
    s.execute("swatch.new", &json!({"name": "Gold", "color": {"model": "cmyk", "c": 0.0, "m": 0.2, "y": 0.8, "k": 0.1}, "spot": true})).unwrap();
    let spot = NodeId(s.execute("shape.rectangle", &json!({"x": 100, "y": 20, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    s.execute("paint.setFill", &json!({"swatch": "Gold"})).unwrap();
    s.execute("object.group", &json!({})).unwrap();
    // A live colour adjustment.
    let adjusted = NodeId(s.execute("shape.rectangle", &json!({"x": 140, "y": 20, "width": 30, "height": 30})).unwrap()["id"].as_u64().unwrap());
    // Its own colour: setting the spot fill made it the default for new art too.
    s.execute("paint.setFill", &json!({"color": "#00ff00"})).unwrap();
    s.execute("effect.apply", &json!({"effect": "addon.color.invert"})).unwrap();
    let rgb = image(&mut s, "rgb", png(image::DynamicImage::ImageRgb8(image::RgbImage::new(4, 4))), 0.0);
    let gray = image(&mut s, "gray", png(image::DynamicImage::ImageLuma8(image::GrayImage::new(4, 4))), 50.0);
    let inks = vectorcraft_doc::cmyk::Inks::new(4, 4, vec![0; 64]).unwrap();
    let cmyk = image(&mut s, "cmyk", vectorcraft_doc::ImageBlob::cmyk_tiff(&inks).unwrap(), 100.0);
    for (kind, ids) in [
        ("spotColors", vec![spot]),
        ("colorEffects", vec![adjusted]),
        ("rgbImages", vec![rgb]),
        ("grayImages", vec![gray]),
        ("cmykImages", vec![cmyk]),
    ] {
        let v = s.execute("addon.color.select", &json!({"kind": kind})).unwrap();
        assert_eq!((selected(&s), v["count"].clone()), (ids.clone(), json!(ids.len())), "{kind}");
    }
    // Locked objects are left out; none found selects nothing.
    s.execute("select.set", &json!({"ids": [spot.0]})).unwrap();
    s.execute("object.lock", &json!({})).unwrap();
    assert_eq!(s.execute("addon.color.select", &json!({"kind": "spotColors"})).unwrap()["count"], 0);
    assert!(!selected(&s).contains(&plain));
    assert!(s.execute("addon.color.select", &json!({"kind": "pantone"})).is_err());
    assert!(s.execute("addon.color.select", &json!({})).is_err());
}

/// Vector Halftone, the live effect.
const HALFTONE: &str = "addon.color.halftone";

/// The dots in group `g`: each path's geometry, ink and blend mode.
fn dots(g: &vectorcraft_doc::Node) -> Vec<(vectorcraft_geom::PathData, String, vectorcraft_color::BlendMode)> {
    let ink = |c: &vectorcraft_doc::Node| c.appearance.fill_paint().color().unwrap().to_hex();
    g.children().unwrap().iter().map(|c| (c.path_data().unwrap().clone(), ink(c), c.blend)).collect()
}

/// `format` as `document.export` writes the active document.
fn exported(s: &mut Session, format: &str) -> Vec<u8> {
    let v = s.execute("document.export", &json!({"format": format})).unwrap();
    vectorcraft_format::base64_decode(v["dataBase64"].as_str().unwrap()).unwrap()
}

/// How many subpaths the path data of `svg` starts.
fn svg_moves(svg: &[u8]) -> usize {
    let svg = String::from_utf8_lossy(svg);
    svg.split(" d=\"").skip(1).map(|d| d.split('"').next().unwrap_or("").matches(['M', 'm']).count()).sum()
}

#[test]
fn expand_appearance_gives_the_dots_object_vector_halftone_makes() {
    for params in [
        json!({"frequency": 18}),
        json!({"frequency": 12, "mode": "cmyk", "shape": "square", "angle": 15}),
        json!({"shape": "line", "invert": true, "clip": false}),
        json!({"shape": "ellipse", "color": "#336699", "angle": -30}),
    ] {
        let (mut live, id) = red_rect();
        live.execute("effect.apply", &json!({"effect": HALFTONE, "params": params})).unwrap();
        assert_eq!(fill_hex(&live, id), "#ff0000", "the effect leaves the object as it is");
        live.execute("effect.expandAppearance", &json!({})).unwrap();
        let expanded = live.doc().unwrap().doc.node(id).cloned().unwrap();
        assert!(expanded.appearance.effects.is_empty());
        let (mut made, _) = red_rect();
        let r = made.execute("object.vectorHalftone", &params).unwrap();
        let g = made.doc().unwrap().doc.node(NodeId(r["id"].as_u64().unwrap())).cloned().unwrap();
        assert!(!dots(&g).is_empty(), "{params}");
        assert_eq!(dots(&expanded), dots(&g), "{params}");
        // Ids of their own.
        let ids: std::collections::HashSet<_> = expanded.children().unwrap().iter().map(|c| c.id).chain([id]).collect();
        assert_eq!(ids.len(), expanded.children().unwrap().len() + 1);
    }
}

#[test]
fn exports_write_the_dots_as_paths() {
    let (mut s, _) = red_rect();
    let plain: Vec<usize> = ["pdf", "eps", "ai"].map(|f| exported(&mut s, f).len()).to_vec();
    s.execute("effect.apply", &json!({"effect": HALFTONE, "params": {"frequency": 18}})).unwrap();
    // The exporters' copy of the document has the dots in the object's place.
    let baked = vectorcraft_render::effects::bake_document(&s.doc().unwrap().doc).unwrap();
    let mut paths = 0;
    baked.walk(|n| {
        assert!(n.appearance.effects.iter().all(|e| e.id != HALFTONE));
        paths += n.path_data().map_or(0, |p| p.subpaths.len());
    });
    assert!(paths > 100, "{paths} subpaths");
    // SVG: path data of a subpath per dot.
    let moves = svg_moves(&exported(&mut s, "svg"));
    assert!(moves > 100, "{moves} subpaths");
    // PDF, EPS and AI: the dots' paths make each file larger.
    for (f, plain) in ["pdf", "eps", "ai"].into_iter().zip(plain) {
        let with = exported(&mut s, f).len();
        assert!(with > plain + 1000, "{f}: {with} bytes, {plain} without the effect");
    }
}

#[test]
fn a_halftone_past_the_caps_leaves_the_object_as_it_is() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 200, "height": 200})).unwrap();
    let id = NodeId(s.execute("shape.rectangle", &json!({"x": 0, "y": 0, "width": 9000, "height": 9000})).unwrap()["id"].as_u64().unwrap());
    s.execute("effect.apply", &json!({"effect": HALFTONE, "params": {"frequency": 300}})).unwrap();
    // Drawn as it is, exported and expanded without dots and without a panic.
    pixel(&s);
    assert!(svg_moves(&exported(&mut s, "svg")) < 5);
    let r = s.execute("effect.expandAppearance", &json!({}));
    assert!(!matches!(r, Err(crate::EngineError::Internal { .. })), "{r:?}");
    let n = s.doc().unwrap().doc.node(id).cloned().unwrap();
    assert!(n.children().is_none_or(|c| c.len() < 5), "no dots");
}

/// Duotone, the live effect.
const DUOTONE: &str = "addon.color.duotone";

/// A document with spot swatch "Ink" and a selected grey (50 %) rectangle, unstroked.
fn grey_rect() -> (Session, NodeId) {
    let (mut s, id) = red_rect();
    s.execute("swatch.new", &json!({"name": "Ink", "color": {"model": "cmyk", "c": 0.0, "m": 0.6, "y": 1.0, "k": 0.0}, "spot": true})).unwrap();
    s.execute("select.set", &json!({"ids": [id.0]})).unwrap();
    s.execute("paint.setFill", &json!({"color": {"gray": 0.5}})).unwrap();
    (s, id)
}

fn ink_tone(curve: &str) -> serde_json::Value {
    json!({"tones": [{"color": {"model": "cmyk", "c": 0.0, "m": 0.6, "y": 1.0, "k": 0.0}, "swatch": "Ink", "curve": curve}]})
}

fn fill_paint(s: &Session, id: NodeId) -> vectorcraft_color::Paint {
    s.doc().unwrap().doc.node(id).unwrap().appearance.fill_paint().clone()
}

#[test]
fn duotone_draws_a_tint_of_its_spot_tone_and_expands_to_it() {
    let (mut s, id) = grey_rect();
    let grey = pixel(&s);
    s.execute("effect.apply", &json!({"effect": DUOTONE, "params": ink_tone("0,0 100,100")})).unwrap();
    assert_eq!(fill_paint(&s, id).color().unwrap(), vectorcraft_color::Color::gray(0.5), "the colours stay");
    let drawn = pixel(&s);
    assert!(drawn != grey && drawn[0] > drawn[2], "drawn in the ink: {drawn:?}");
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let vectorcraft_color::Paint::Solid { swatch, tint, .. } = fill_paint(&s, id) else { panic!("a solid fill") };
    assert_eq!(swatch.as_deref(), Some("Ink"), "linked to the spot swatch, so it separates on its plate");
    assert!((tint - 0.5).abs() < 0.01, "{tint}");
}

#[test]
fn duotone_as_a_filter_equals_the_live_effect_expanded() {
    for params in [
        ink_tone("0,0 100,100"),
        ink_tone("0,0 40,70 100,100"),
        json!({"tones": [{"color": "#00ffff", "swatch": "Ink"}, {"color": {"model": "cmyk", "c": 0.0, "m": 0.0, "y": 0.0, "k": 1.0}, "curve": "0,0 100,40"}]}),
        json!({"tones": [{"color": "#000000"}], "skipSpot": true}),
    ] {
        let (mut live, id) = grey_rect();
        live.execute("effect.apply", &json!({"effect": DUOTONE, "params": params})).unwrap();
        live.execute("effect.expandAppearance", &json!({})).unwrap();
        let (mut filtered, _) = grey_rect();
        let undo = filtered.doc().unwrap().history.undo.len();
        assert_eq!(filtered.execute("addon.color.filter", &json!({"effect": DUOTONE, "params": params})).unwrap()["changed"], 1);
        assert_eq!(filtered.doc().unwrap().history.undo.len(), undo + 1, "one undo step");
        assert_eq!(fill_paint(&filtered, id), fill_paint(&live, id), "{params}");
    }
    // The params round-trip: the effect keeps what it was given, and the filter reads the same.
    let (mut s, id) = grey_rect();
    let params = ink_tone("0,0 40,70 100,100");
    s.execute("effect.apply", &json!({"effect": DUOTONE, "params": params})).unwrap();
    let kept = s.doc().unwrap().doc.node(id).unwrap().appearance.effects[0].params.clone();
    assert_eq!(kept["tones"], params["tones"]);
    s.execute("addon.color.filter", &json!({"effect": DUOTONE, "params": kept})).unwrap();
    let vectorcraft_color::Paint::Solid { swatch, tint, .. } = fill_paint(&s, id) else { panic!("a solid fill") };
    assert_eq!(swatch.as_deref(), Some("Ink"));
    let fx = vectorcraft_render::effects::addons::duotone::tone_curve_points(Some(&json!("0,0 40,70 100,100")));
    let want = vectorcraft_render::effects::curve_at(&fx, 0.5);
    assert!(want > 0.7 && (tint - want).abs() < 0.01, "the curve lifts the mid tone: {tint}, {want}");
}

#[test]
fn duotone_skips_spot_colours_when_asked() {
    let (mut s, id) = grey_rect();
    s.execute("paint.setFill", &json!({"swatch": "Ink"})).unwrap();
    let spot = fill_paint(&s, id);
    s.execute("addon.color.filter", &json!({"effect": DUOTONE, "params": {"tones": [{"color": "#000000"}], "skipSpot": true}})).unwrap();
    assert_eq!(fill_paint(&s, id), spot);
    s.execute("addon.color.filter", &json!({"effect": DUOTONE, "params": {"tones": [{"color": "#000000"}]}})).unwrap();
    assert_ne!(fill_paint(&s, id), spot);
}
