//! Colored 3D lighting remains a live command and survives native and vector output.
use serde_json::{Value, json};
use vectorcraft_doc::{Document, Node, NodeId};
use vectorcraft_engine::Session;

const EFFECTS: [&str; 4] = ["threeD.revolve", "threeD.extrude", "threeD.inflate", "threeD.rotate"];

fn setup(effect: &str) -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let r = s.execute("path.create", &json!({"d":"M100 100L200 100L200 200L100 200Z"})).unwrap();
    s.execute("paint.setFill", &json!({"color":"#ffffff"})).unwrap();
    s.execute("paint.setStroke", &json!({"none":true})).unwrap();
    let p = json!({"effect":effect,"item":null,"params":{"rotationX":0,"rotationY":0,"rotationZ":0,"lightHeight":90}});
    s.check_params("effect.apply", &p).unwrap();
    s.execute("effect.apply", &p).unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn doc(s: &Session) -> &Document {
    &s.active().unwrap().doc
}
fn params(s: &Session, id: NodeId) -> &Value {
    &doc(s).node(id).unwrap().appearance.effects[0].params
}
fn contains_color(n: &Node, hex: &str) -> bool {
    n.appearance.items.iter().any(|i| i.paint().color().is_some_and(|c| c.to_hex() == hex))
        || n.children().is_some_and(|ch| ch.iter().any(|n| contains_color(n, hex)))
}

#[test]
fn all_four_light_colors_save_reopen_undo_and_expand_without_editing_source() {
    for effect in EFFECTS {
        let (mut s, id) = setup(effect);
        let original = doc(&s).clone();
        let path = doc(&s).node(id).unwrap().path_data().unwrap().clone();
        assert_eq!(params(&s, id)["lightColor"], json!("#ffffff"));
        let change = json!({"index":0,"item":null,"params":{"lightColor":"#ff0000"}});
        s.check_params("effect.setParams", &change).unwrap();
        s.execute("effect.setParams", &change).unwrap();
        assert_eq!(doc(&s).node(id).unwrap().path_data(), Some(&path));
        let red = doc(&s).clone();
        let reopened = vectorcraft_format::load(&vectorcraft_format::save(&red, false)).unwrap();
        assert_eq!(reopened.node(id).unwrap().appearance.effects[0].params["lightColor"], json!("#ff0000"));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), &original);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(doc(&s), &red);
        let svg = s.execute("document.serialize", &json!({"format":"svg"})).unwrap();
        let text = svg["text"].as_str().unwrap();
        assert!(vectorcraft_testkit::svg_shapes(text) > 0 && !text.contains("<image"), "{effect}");
        if effect == "threeD.rotate" {
            assert!(text.contains("#ff4040"), "front plane is white under red light and neutral 25% ambient: {text}");
        }
        let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
        let bytes = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
        assert!(!bytes.windows(b"/Subtype /Image".len()).any(|s| s == b"/Subtype /Image"), "{effect}");
        assert_eq!(doc(&s), &red, "exports leave the live document intact");
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        assert!(doc(&s).node(id).unwrap().appearance.effects.is_empty());
        if effect == "threeD.rotate" {
            assert!(contains_color(doc(&s).node(id).unwrap(), "#ff4040"));
        }
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), &red);
    }
}

#[test]
fn invalid_light_color_is_rejected_without_mutating_any_effect() {
    for effect in EFFECTS {
        let (mut s, _) = setup(effect);
        let before = doc(&s).clone();
        for color in [json!("#ff0"), json!("#ffffff00"), json!("invalid"), json!(42), json!(null), json!([1, 0, 0])] {
            let p = json!({"index":0,"item":null,"params":{"lightColor":color}});
            assert!(s.execute("effect.setParams", &p).is_err(), "{effect}: {p}");
            assert_eq!(doc(&s), &before);
        }
        let p = json!({"index":0,"params":{"lightColour":"#ff0000"}});
        assert!(s.check_params("effect.setParams", &p).is_err());
    }
}

fn capped_profile() -> (Session, NodeId) {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    let r = s.execute("path.create", &json!({"d":"M200 120L200 280"})).unwrap();
    s.execute("paint.setFill", &json!({"color":"#ffffff"})).unwrap();
    s.execute("paint.setStroke", &json!({"none":true})).unwrap();
    s.execute(
        "effect.apply",
        &json!({"effect":"threeD.revolve","item":null,"params":{
            "offset":70,"rotationX":-25,"segments":24,"ambient":20,"lightIntensity":65,"lightColor":"#ff0000"
        }}),
    )
    .unwrap();
    (s, NodeId(r["id"].as_u64().unwrap()))
}

fn render(d: &Document) -> vectorcraft_render::Rendered {
    vectorcraft_render::Renderer::new().render(
        d,
        400,
        400,
        kurbo::Affine::IDENTITY,
        &vectorcraft_render::RenderOptions { background: None, artboards: false, ..Default::default() },
    )
}

fn vector_exports(s: &mut Session) -> String {
    let svg = s.execute("document.serialize", &json!({"format":"svg"})).unwrap()["text"].as_str().unwrap().to_owned();
    assert!(svg.contains("<linearGradient") && !svg.contains("<image"));
    assert!(
        svg.split(" d=\"").skip(1).any(|d| d.split('"').next().is_some_and(|d| d.contains('C'))),
        "continuous curved cap boundaries survive export"
    );
    let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
    let bytes = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
    assert!(!bytes.windows(b"/Subtype /Image".len()).any(|s| s == b"/Subtype /Image"));
    svg
}

#[test]
fn colored_revolve_smooth_shading_and_default_caps_work_together_and_expand() {
    let (mut s, id) = capped_profile();
    assert_eq!(params(&s, id)["cap"], true);
    assert!(params(&s, id).get("smoothShading").is_none());
    let before = render(doc(&s));
    let source = doc(&s).node(id).unwrap().path_data().unwrap().clone();
    vector_exports(&mut s);
    s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"cap":false}})).unwrap();
    assert_ne!(render(doc(&s)).pixels, before.pixels, "Cap changes the open profile's ends");
    s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"cap":true}})).unwrap();
    assert_eq!(doc(&s).node(id).unwrap().path_data(), Some(&source));
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    let expanded = render(doc(&s));
    let mut shades = std::collections::BTreeSet::new();
    let mut samples = 0;
    for y in (4..396).step_by(3) {
        for x in (4..396).step_by(3) {
            let p = before.pixel(x, y);
            let q = expanded.pixel(x, y);
            if p[3] == 255 && q[3] == 255 {
                assert!(p[1].abs_diff(51) <= 1 && p[2].abs_diff(51) <= 1, "neutral ambient {p:?}");
                assert!(p.into_iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 3), "expand ({x},{y}): {p:?} vs {q:?}");
                shades.insert(p[0]);
                samples += 1;
            }
        }
    }
    assert!(samples > 500 && shades.len() > 40, "smooth colored shading at only 24 segments");
    vector_exports(&mut s);
}

#[test]
fn colored_smooth_source_gradients_keep_alpha_and_vector_output() {
    for opacity in [1.0, 0.45] {
        let (mut s, id) = capped_profile();
        let solid = render(doc(&s));
        s.execute(
            "paint.setFill",
            &json!({"gradient":{"kind":"linear","start":[80,200],"end":[240,200],"stops":[
                {"offset":0,"color":"#ffffff","opacity":opacity,"midpoint":0.3},
                    {"offset":1,"color":"#808080","opacity":if opacity == 1.0 { 1.0 } else { opacity*0.8 }}
            ]}}),
        )
        .unwrap();
        let red = render(doc(&s));
        if opacity == 1.0 {
            let vectorcraft_doc::color::Paint::Gradient(source) = doc(&s).node(id).unwrap().appearance.items[0].paint() else {
                panic!("source gradient")
            };
            let mut checked = 0;
            // Compare the body away from the cap/body antialiasing boundary. This crosses
            // every radial seam and the shared triangle diagonals in the smooth surface.
            for y in (170..230).step_by(2) {
                for x in (65..195).step_by(2) {
                    let white = solid.pixel(x, y);
                    if white[3] == 255 && solid.pixel(x - 1, y)[3] == 255 && solid.pixel(x + 1, y)[3] == 255 {
                        let p = red.pixel(x, y);
                        let t = source.geom.unwrap().param_at(source.gradient.kind, kurbo::Point::new(x as f64 + 0.5, y as f64 + 0.5));
                        let (color, _) = source.gradient.sample(t as f32);
                        let expected: Vec<_> = white[..3].iter().zip(color.to_rgb()).map(|(v, c)| (*v as f32 * c).round() as u8).collect();
                        assert_eq!(p[3], 255, "source and light share one antialiased coverage");
                        assert!(
                            p.into_iter().zip(&expected).all(|(a, b)| a.abs_diff(*b) <= 3),
                            "gradient seam at ({x},{y}): {p:?}, expected {expected:?}, solid {white:?}"
                        );
                        checked += 1;
                    }
                }
            }
            assert!(checked > 1000);
        }
        s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"lightColor":"#0000ff"}})).unwrap();
        let blue = render(doc(&s));
        let mut translucent = 0;
        for y in (4..396).step_by(7) {
            for x in (4..396).step_by(7) {
                let r = red.pixel(x, y);
                let b = blue.pixel(x, y);
                assert_eq!(r[3], b[3], "light color preserves source coverage");
                assert!(
                    r[0].abs_diff(b[2]) <= 2 && r[1].abs_diff(b[1]) <= 2 && r[2].abs_diff(b[0]) <= 2,
                    "RGB channels at ({x},{y}): {r:?} vs {b:?}"
                );
                translucent += usize::from(r[3] > 30 && r[3] < 245);
            }
        }
        if opacity < 1.0 {
            assert!(translucent > 100, "gradient stop opacity is preserved");
        }
        let svg = vector_exports(&mut s);
        assert!(svg.contains("multiply"));
        if opacity < 1.0 {
            assert!(svg.contains("<mask"));
        } else {
            assert!(!svg.contains("<mask"), "opaque faces use vector clipping without opacity-mask buffers");
        }
        let imported = vectorcraft_svg::import(&svg).unwrap();
        let svg_image = render(&imported);
        let mut checked = 0;
        for y in (110..290).step_by(11) {
            for x in (80..225).step_by(11) {
                let p = blue.pixel(x, y);
                let q = svg_image.pixel(x, y);
                if p[3] > 50 {
                    assert!(p.into_iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 8), "SVG ({x},{y}): {p:?} vs {q:?}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 50);
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        assert_eq!(render(doc(&s)).pixels, blue.pixels, "gradient expansion keeps the live composition");
        vector_exports(&mut s);
    }
}

#[test]
fn multiple_lights_survive_commands_history_native_save_and_vector_expansion() {
    let lights = json!([
        {"color":"#ff0000","intensity":60,"height":90,"rotation":0,"softness":0,"behind":false},
        {"color":"#0000ff","intensity":40,"height":90,"rotation":0,"softness":0,"behind":false}
    ]);
    for effect in EFFECTS {
        let (mut s, id) = setup(effect);
        let before = doc(&s).clone();
        let source = doc(&s).node(id).unwrap().path_data().unwrap().clone();
        let change = json!({"index":0,"item":null,"params":{"ambient":20,"lights":lights}});
        s.check_params("effect.setParams", &change).unwrap();
        s.execute("effect.setParams", &change).unwrap();
        let lit = doc(&s).clone();
        assert_eq!(params(&s, id)["lights"], lights);
        let restored = vectorcraft_format::load(&vectorcraft_format::save(&lit, false)).unwrap();
        assert_eq!(restored.node(id).unwrap().appearance.effects[0].params["lights"], lights);
        assert_eq!(doc(&s).node(id).unwrap().path_data(), Some(&source));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), &before);
        s.execute("edit.redo", &json!({})).unwrap();
        assert_eq!(doc(&s), &lit);
        let live = render(doc(&s));
        if effect == "threeD.rotate" {
            assert_eq!(live.pixel(150, 150), [204, 51, 153, 255]);
        }
        let svg = s.execute("document.serialize", &json!({"format":"svg"})).unwrap()["text"].as_str().unwrap().to_owned();
        assert!(!svg.contains("<image"));
        let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
        let bytes = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
        assert!(!bytes.windows(b"/Subtype /Image".len()).any(|s| s == b"/Subtype /Image"));
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let expanded = render(doc(&s));
        let mut checked = 0;
        for y in (100..200).step_by(4) {
            for x in (100..200).step_by(4) {
                let p = live.pixel(x, y);
                let q = expanded.pixel(x, y);
                if p[3] == 255 && q[3] == 255 {
                    assert!(p.into_iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 3), "{effect}: {p:?} vs {q:?}");
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "{effect}");
    }
}

#[test]
fn malformed_light_lists_are_rejected_atomically_and_null_restores_legacy_controls() {
    for effect in EFFECTS {
        let (mut s, id) = setup(effect);
        let before = doc(&s).clone();
        for lights in [
            json!([]),
            json!(vec![json!({}); 11]),
            json!(false),
            json!([null]),
            json!([{"color":"#f00"}]),
            json!([{"height":91}]),
            json!([{"intensity":-1}]),
            json!([{"softness":"soft"}]),
            json!([{"behind":1}]),
            json!([{"colour":"#ffffff"}]),
        ] {
            let change = json!({"index":0,"item":null,"params":{"lights":lights}});
            assert!(s.execute("effect.setParams", &change).is_err(), "{effect}: {lights}");
            assert_eq!(doc(&s), &before);
        }
        s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"lights":[{},{}]}})).unwrap();
        s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"lights":null}})).unwrap();
        assert!(params(&s, id)["lights"].is_null());
        assert_eq!(render(doc(&s)).pixels, render(&before).pixels);
    }
}

#[test]
fn smooth_multi_light_color_varies_in_two_directions_and_stays_vector_with_source_gradients() {
    let (mut s, id) = capped_profile();
    s.execute(
        "effect.setParams",
        &json!({"index":0,"item":null,"params":{"ambient":10,"lights":[
            {"color":"#ff0000","rotation":0,"height":25,"intensity":70},
            {"color":"#0000ff","rotation":180,"height":50,"intensity":65}
        ]}}),
    )
    .unwrap();
    let solid = render(doc(&s));
    assert!(solid.pixels.as_chunks::<4>().0.iter().any(|p| u16::from(p[0]) > u16::from(p[2]) + 20 && p[3] == 255));
    assert!(solid.pixels.as_chunks::<4>().0.iter().any(|p| u16::from(p[2]) > u16::from(p[0]) + 20 && p[3] == 255));
    for opacity in [1.0, 0.5] {
        s.execute(
            "paint.setFill",
            &json!({"gradient":{"kind":"linear","start":[80,200],"end":[240,200],"stops":[
                {"offset":0,"color":"#ffffff","opacity":opacity},{"offset":1,"color":"#808080","opacity":opacity}
            ]}}),
        )
        .unwrap();
        let live = render(doc(&s));
        if opacity == 1.0 {
            for y in (175..225).step_by(5) {
                for x in (75..190).step_by(5) {
                    let a = solid.pixel(x, y);
                    let b = live.pixel(x, y);
                    if a[3] == 255 {
                        let factor = 1.0 - ((f64::from(x) + 0.5 - 80.0) / 160.0).clamp(0.0, 1.0) * (1.0 - 128.0 / 255.0);
                        for c in 0..3 {
                            let expected = (f64::from(a[c]) * factor).round() as u8;
                            assert!(b[c].abs_diff(expected) <= 4, "({x},{y}) {a:?} vs {b:?}");
                        }
                    }
                }
            }
        } else {
            assert!(live.pixels.as_chunks::<4>().0.iter().filter(|p| p[3] > 30 && p[3] < 220).count() > 1000);
        }
        let svg = vector_exports(&mut s);
        assert!(svg.contains("screen") && svg.contains("multiply"));
        let imported = vectorcraft_svg::import(&svg).unwrap();
        let roundtrip = render(&imported);
        for y in (150..250).step_by(13) {
            for x in (90..185).step_by(13) {
                let a = live.pixel(x, y);
                let b = roundtrip.pixel(x, y);
                assert!(a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 8), "SVG {a:?} {b:?}");
            }
        }
        let before = doc(&s).clone();
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        assert!(doc(&s).node(id).unwrap().appearance.effects.is_empty());
        if opacity < 1.0 {
            assert_eq!(live.pixels, render(doc(&s)).pixels);
        }
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), &before);
    }
}

#[test]
fn inflate_lighting_has_no_polygon_steps_and_stays_vector_after_expansion() {
    // Check the export pipeline's continuous shading. The 8-bit canvas is also reviewed
    // natively at high zoom; it can quantize a short gradient ramp differently at 1:1.
    let render = |d: &Document| {
        vectorcraft_render::Renderer::new().render(
            d,
            400,
            400,
            kurbo::Affine::IDENTITY,
            &vectorcraft_render::RenderOptions { background: None, artboards: false, precise: true, supersample: true, ..Default::default() },
        )
    };
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    s.execute("path.create", &json!({"d":"M300 200C300 255.228475 255.228475 300 200 300C144.771525 300 100 255.228475 100 200C100 144.771525 144.771525 100 200 100C255.228475 100 300 144.771525 300 200Z"})).unwrap();
    s.execute("paint.setFill", &json!({"color":"#ffffff"})).unwrap();
    s.execute("paint.setStroke", &json!({"none":true})).unwrap();
    s.execute(
        "effect.apply",
        &json!({"effect":"threeD.inflate","item":null,"params":{
            "rotationX":0,"rotationY":0,"rotationZ":0,"ambient":10,"volume":50,"lights":[
                {"color":"#ff0000","rotation":135,"height":35,"intensity":70},
                {"color":"#0000ff","rotation":-35,"height":25,"intensity":65}
            ]
        }}),
    )
    .unwrap();
    let live = render(doc(&s));
    for y in [175, 200, 225] {
        for x in 145..255 {
            let a = live.pixel(x, y);
            let b = live.pixel(x + 1, y);
            assert_eq!(a[3], 255, "interior seam at ({x},{y})");
            assert!(a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 3), "polygon band at ({x},{y}): {a:?} vs {b:?}");
        }
    }
    let svg = s.execute("document.serialize", &json!({"format":"svg"})).unwrap()["text"].as_str().unwrap().to_owned();
    assert!(svg.contains("linearGradient") && svg.contains("screen") && !svg.contains("<image"));
    let roundtrip = render(&vectorcraft_svg::import(&svg).unwrap());
    for y in (110..290).step_by(9) {
        for x in (110..290).step_by(9) {
            let a = live.pixel(x, y);
            let b = roundtrip.pixel(x, y);
            assert!(a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 6), "batched SVG ({x},{y}): {a:?} vs {b:?}");
        }
    }
    let before = doc(&s).clone();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(render(doc(&s)).pixels, live.pixels);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(doc(&s), &before);
    // A colored source gradient still multiplies the light, keeping stop transparency.
    s.execute(
        "paint.setFill",
        &json!({"gradient":{"kind":"linear","start":[100,200],"end":[300,200],"stops":[
            {"offset":0,"color":"#ffffff","opacity":0.6},{"offset":1,"color":"#808080","opacity":0.6}
        ]}}),
    )
    .unwrap();
    let live = render(doc(&s));
    assert!((150..=156).contains(&live.pixel(200, 200)[3]), "surface opacity is applied once: {:?}", live.pixel(200, 200));
    let svg = vector_exports(&mut s);
    assert!(svg.contains("linearGradient") && svg.contains("screen") && svg.contains("multiply"));
    let imported = vectorcraft_svg::import(&svg).unwrap();
    let roundtrip = render(&imported);
    for y in (155..245).step_by(9) {
        for x in (155..245).step_by(9) {
            let a = live.pixel(x, y);
            let b = roundtrip.pixel(x, y);
            assert!(a.into_iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 6), "SVG ({x},{y}): {a:?} vs {b:?}");
        }
    }
    let before = doc(&s).clone();
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(render(doc(&s)).pixels, live.pixels);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(doc(&s), &before);
}

#[test]
fn solid_gradient_opacity_is_applied_once_even_without_shading() {
    for effect in &EFFECTS[1..] {
        let (mut s, _) = setup(effect);
        s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"shade":false,"lightColor":"#ff8040"}})).unwrap();
        let opaque = render(doc(&s));
        s.execute(
            "paint.setFill",
            &json!({"gradient":{"kind":"linear","start":[100,150],"end":[200,150],"stops":[
                {"offset":0,"color":"#ffffff","opacity":0.6},{"offset":1,"color":"#808080","opacity":0.6}
            ]}}),
        )
        .unwrap();
        let image = render(doc(&s));
        for y in (120..180).step_by(5) {
            for x in (120..180).step_by(5) {
                let alpha = image.pixel(x, y)[3];
                // Preserve the canvas pipeline's edge coverage, then apply source opacity
                // once. Subpixel seams can have less than full coverage in the 8-bit canvas.
                let expected = (f32::from(opaque.pixel(x, y)[3]) * 0.6).round() as u8;
                assert!(alpha.abs_diff(expected) <= 3, "{effect} ({x},{y}): alpha {alpha}, expected {expected}");
            }
        }
    }
}

/// How `image`, `d` drawn at 1:1 on 400 × 400 pixels, differs from `d` drawn at 3x and averaged
/// down, on the pixels inside the art: those whose 3x average has some alpha, the same as every
/// pixel within 3 of them. Left out too are pixels where the 3x average bends sharply (a channel's
/// second difference across the pixel, along x or y, over 4 levels), such as the rim where a cap
/// meets a body: a gradient sampled at each pixel's center matches an area average only where the
/// light changes close to linearly across the pixel. Returns (pixels measured, premultiplied RGB
/// channels more than 3 levels off, the largest RGB difference, alphas off: below 255 where the 3x
/// average is opaque, otherwise more than 3 levels off, the first few pixels off).
fn seams_against_supersampled(d: &Document, image: &vectorcraft_render::Rendered) -> (usize, usize, u8, usize, Vec<String>) {
    let fine = vectorcraft_render::Renderer::new().render(
        d,
        1200,
        1200,
        kurbo::Affine::scale(3.0),
        &vectorcraft_render::RenderOptions { background: None, artboards: false, ..Default::default() },
    );
    let average = fine.area_resized(400, 400).unwrap();
    let (mut measured, mut color, mut max, mut holes) = (0, 0, 0, 0);
    let mut worst = vec![];
    for y in 3..397 {
        for x in 3..397 {
            let q = average.pixel(x, y);
            if q[3] == 0 || !(y - 3..=y + 3).all(|y| (x - 3..=x + 3).all(|x| average.pixel(x, y)[3] == q[3])) {
                continue;
            }
            let bend = |a: [u8; 4], b: [u8; 4]| (0..3).any(|c| (i32::from(a[c]) + i32::from(b[c]) - 2 * i32::from(q[c])).abs() > 4);
            if bend(average.pixel(x - 1, y), average.pixel(x + 1, y)) || bend(average.pixel(x, y - 1), average.pixel(x, y + 1)) {
                continue;
            }
            measured += 1;
            let p = image.pixel(x, y);
            for c in 0..3 {
                let diff = p[c].abs_diff(q[c]);
                max = max.max(diff);
                color += usize::from(diff > 3);
            }
            let hole = if q[3] == 255 { p[3] < 255 } else { p[3].abs_diff(q[3]) > 3 };
            holes += usize::from(hole);
            if (p.into_iter().zip(q).any(|(a, b)| a.abs_diff(b) > 3) || hole) && worst.len() < 4 {
                worst.push(format!("({x},{y}) {p:?} vs {q:?}"));
            }
        }
    }
    (measured, color, max, holes, worst)
}

/// Colored lights on smooth 3D surfaces, compared at 1:1 with a 3x render averaged down, on every
/// pixel inside the art ([`seams_against_supersampled`]). A solid fill draws as one batched surface;
/// a source gradient composes each facet on its own, and one with transparent stops applies their
/// opacity once over the surface. Inflate seen straight on, one layer of facets, has no alpha off, and
/// in opaque fills no RGB channel more than 3 levels off. Inflate turned so facets narrow to slivers
/// at the edges, and Revolve with a cap over its body, draw facets over other facets, and have no
/// alpha off.
#[test]
fn colored_light_surfaces_match_supersampling_at_internal_facet_edges() {
    let lights = json!([
        {"color":"#ff0000","rotation":135,"height":35,"intensity":70},
        {"color":"#0000ff","rotation":-35,"height":25,"intensity":65}
    ]);
    let fills = |start: [u32; 2], end: [u32; 2]| {
        [
            json!({"color":"#ffcc66"}),
            json!({"gradient":{"kind":"linear","start":start,"end":end,"stops":[
                {"offset":0,"color":"#ffffff"},{"offset":1,"color":"#808080"}
            ]}}),
            json!({"gradient":{"kind":"linear","start":start,"end":end,"stops":[
                {"offset":0,"color":"#ffffff","opacity":0.6},{"offset":1,"color":"#808080","opacity":0.6}
            ]}}),
        ]
    };
    let mut cases = vec![];
    for rotation in [(0, 0), (50, 35)] {
        let (mut s, _) = setup("threeD.inflate");
        let change = json!({"index":0,"item":null,"params":{"ambient":10,"lights":lights,"rotationX":rotation.0,"rotationY":rotation.1}});
        s.execute("effect.setParams", &change).unwrap();
        cases.push((format!("inflate {rotation:?}"), s, fills([100, 150], [200, 150]), rotation == (0, 0)));
    }
    let (mut s, _) = capped_profile();
    s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"ambient":10,"lights":lights}})).unwrap();
    cases.push(("revolve".into(), s, fills([80, 200], [240, 200]), false));
    let mut failures = vec![];
    for (name, mut s, fills, one_layer) in cases {
        for fill in fills {
            s.execute("paint.setFill", &fill).unwrap();
            let image = render(doc(&s));
            let (measured, color, max, holes, worst) = seams_against_supersampled(doc(&s), &image);
            assert!(measured > 2_000, "{name} {fill}: {measured} pixels inside");
            let opaque = !fill.to_string().contains("opacity");
            if holes > 0 || (one_layer && opaque && color > 0) {
                failures.push(format!(
                    "{name} {fill}: of {measured} pixels, {color} channels off by more than 3 (at most {max}), {holes} alphas off: {worst:?}"
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// The closed facet edges of colored-light surfaces survive SVG and PDF, read back and drawn at
/// 1:1, and Expand Appearance: inside the art, every pixel stays opaque and within 6 levels of the
/// live art.
#[test]
fn colored_light_seams_stay_closed_in_svg_pdf_and_expansion() {
    let lights = json!([
        {"color":"#ff0000","rotation":135,"height":35,"intensity":70},
        {"color":"#0000ff","rotation":-35,"height":25,"intensity":65}
    ]);
    for fill in [
        json!({"color":"#ffcc66"}),
        json!({"gradient":{"kind":"linear","start":[100,150],"end":[200,150],"stops":[
            {"offset":0,"color":"#ffffff"},{"offset":1,"color":"#808080"}
        ]}}),
    ] {
        let (mut s, _) = setup("threeD.inflate");
        s.execute("effect.setParams", &json!({"index":0,"item":null,"params":{"ambient":10,"lights":lights}})).unwrap();
        s.execute("paint.setFill", &fill).unwrap();
        let live = render(doc(&s));
        let svg = s.execute("document.serialize", &json!({"format":"svg"})).unwrap()["text"].as_str().unwrap().to_owned();
        assert!(svg.contains("<clipPath") && svg.contains("linearGradient") && svg.contains("screen") && !svg.contains("<image"), "{fill}");
        let pdf = s.execute("document.serialize", &json!({"format":"pdf"})).unwrap();
        let bytes = vectorcraft_format::base64_decode(pdf["dataBase64"].as_str().unwrap()).unwrap();
        assert!(!bytes.windows(b"/Subtype /Image".len()).any(|w| w == b"/Subtype /Image"), "{fill}");
        let before = doc(&s).clone();
        s.execute("effect.expandAppearance", &json!({})).unwrap();
        let expanded = render(doc(&s));
        s.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(doc(&s), &before);
        for (name, image) in [
            ("SVG", render(&vectorcraft_svg::import(&svg).unwrap())),
            ("PDF", render(&vectorcraft_pdf::import(&bytes).unwrap())),
            ("expanded", expanded),
        ] {
            let mut checked = 0;
            for y in 103..197 {
                for x in 103..197 {
                    let (p, q) = (live.pixel(x, y), image.pixel(x, y));
                    assert!(q[3] == 255 && p.into_iter().zip(q).all(|(a, b)| a.abs_diff(b) <= 6), "{name} {fill} ({x},{y}): {p:?} vs {q:?}");
                    checked += 1;
                }
            }
            assert_eq!(checked, 94 * 94);
        }
    }
}

#[test]
fn batched_colored_inflate_preserves_compound_holes_and_surface_opacity() {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width":400,"height":400})).unwrap();
    s.execute("path.create", &json!({"d":"M80 80H320V320H80ZM150 150V250H250V150Z"})).unwrap();
    s.execute("paint.setFill", &json!({"color":"#ffffff"})).unwrap();
    s.execute("transparency.set", &json!({"item":0,"opacity":60})).unwrap();
    s.execute("paint.setStroke", &json!({"none":true})).unwrap();
    s.execute(
        "effect.apply",
        &json!({"effect":"threeD.inflate","item":null,"params":{
            "rotationX":0,"rotationY":0,"rotationZ":0,"lights":[{"color":"#ff0000"},{"color":"#0000ff"}]
        }}),
    )
    .unwrap();
    let image = render(doc(&s));
    assert_eq!(image.pixel(200, 200)[3], 0, "the compound coverage clip preserves holes");
    assert!((150..=156).contains(&image.pixel(110, 200)[3]), "surface opacity applies once: {:?}", image.pixel(110, 200));
    s.execute("effect.expandAppearance", &json!({})).unwrap();
    assert_eq!(image.pixels, render(doc(&s)).pixels);
}
