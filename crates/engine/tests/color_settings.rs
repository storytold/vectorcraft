//! Edit → Color Settings and Assign Profile with non-default profiles. The Color Settings are
//! process-wide and every colour shown or rendered goes through them, so these tests run in their
//! own test binary (process) instead of next to the engine's unit tests, which would see the
//! switched working spaces mid-test; here they take turns.

use std::sync::{Mutex, MutexGuard};

use serde_json::json;
use vectorcraft_color::cms;
use vectorcraft_engine::Session;
use vectorcraft_engine::cmd::colormgmt::{PROFILES_KEY, doc_profiles};
use vectorcraft_geom::Affine;
use vectorcraft_render::{RenderOptions, Renderer};
use vectorcraft_testkit::fixtures::{exec, id_of, rect, select, session};

/// The active Color Settings: one test at a time.
fn settings() -> MutexGuard<'static, ()> {
    static SETTINGS: Mutex<()> = Mutex::new(());
    SETTINGS.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn color_settings_lists_profiles_and_validates() {
    let _g = settings();
    let mut s = session();
    let r = s.execute("edit.colorSettings", &json!({})).unwrap();
    let names: Vec<&str> = r["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&cms::GENERIC_CMYK) && names.contains(&cms::SRGB) && names.contains(&cms::DEVICE_CMYK));
    assert!(s.execute("edit.colorSettings", &json!({"cmyk": "Missing.icc"})).is_err());
    assert_eq!(cms::active_settings().cmyk, r["cmyk"].as_str().unwrap(), "failed change leaves settings alone");
    let before = r["bpc"].as_bool().unwrap();
    let r2 = s.execute("edit.colorSettings", &json!({"bpc": !before})).unwrap();
    assert_eq!(r2["bpc"], !before);
    s.execute("edit.colorSettings", &json!({"bpc": before})).unwrap();
    assert!(s.execute("color.loadProfile", &json!({"path": "/nonexistent/x.icc"})).is_err());
}

#[test]
fn legacy_profile_name_resolves_in_commands_and_files() {
    let _g = settings();
    let (old, _) = cms::LEGACY_NAMES[0];
    let mut s = session();
    let before = cms::active_settings();
    let r = s.execute("edit.colorSettings", &json!({"rgb": old})).unwrap();
    assert_eq!(r["rgb"], cms::WIDE_GAMUT_RGB);
    assert_eq!(cms::active_settings().rgb, cms::WIDE_GAMUT_RGB);
    let names: Vec<&str> = r["profiles"].as_array().unwrap().iter().map(|p| p["name"].as_str().unwrap()).collect();
    assert!(names.contains(&cms::WIDE_GAMUT_RGB) && !names.contains(&old));
    let r = s.execute("edit.assignProfile", &json!({"rgb": old})).unwrap();
    assert_eq!(r["rgb"], cms::WIDE_GAMUT_RGB, "stored under the current name");
    // A document saved by an older version names the profile the old way.
    let mut d = (*s.doc().unwrap().doc).clone();
    d.unknown.insert(PROFILES_KEY.into(), json!({"rgb": old, "cmyk": null}));
    let back = vectorcraft_format::load(&vectorcraft_format::save(&d, false)).unwrap();
    assert_eq!(doc_profiles(&back), (Some(cms::WIDE_GAMUT_RGB.to_string()), None));
    cms::set_active(&before).unwrap();
}

/// Separations Preview follows the Color Settings and the profiles loaded: a renderer draws a gray
/// square on the Black plate through the new working CMYK profile, and the four process plates
/// through a proof profile loaded again under the same name, as a new renderer draws them.
#[test]
fn separations_preview_draws_through_new_color_settings() {
    let _g = settings();
    let mut s = session();
    let gray = rect(&mut s, 20.0, 20.0, 160.0, 120.0);
    exec(&mut s, "paint.setFill", json!({"ids": [gray.0], "color": "#808080"}));
    // The proof and Overprint Preview options the canvas takes from the view state the
    // commands set.
    let canvas = || RenderOptions {
        proof: vectorcraft_render::proof::active_proof(),
        overprint_preview: vectorcraft_render::proof::overprint_preview_on(),
        background: Some([255, 255, 255, 255]),
        ..Default::default()
    };
    let frame = |r: &mut Renderer, s: &Session| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &canvas()).pixels;
    exec(&mut s, "view.separationsPreview", json!({"only": "Black"}));
    let mut r = Renderer::new();
    let before = frame(&mut r, &s);
    let old = cms::active_settings();
    s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let fresh = frame(&mut Renderer::new(), &s);
    let warm = frame(&mut r, &s);
    cms::set_active(&old).unwrap();
    assert!(fresh != before, "the new profile changes the plate");
    assert!(warm == fresh);
    let name = "Separations Proof CMYK";
    cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some(name.into())).unwrap();
    exec(&mut s, "view.proofSetup", json!({"target": format!("cmyk:{name}")}));
    exec(&mut s, "view.separationsPreview", json!({"plates": ["Cyan", "Magenta", "Yellow", "Black"]}));
    let before = frame(&mut r, &s);
    cms::register_icc(&cms::icc_bytes(cms::DEVICE_CMYK).unwrap(), Some(name.into())).unwrap();
    let fresh = frame(&mut Renderer::new(), &s);
    let warm = frame(&mut r, &s);
    exec(&mut s, "view.proofSetup", json!({"target": "workingCmyk"}));
    exec(&mut s, "view.separationsPreview", json!({"on": false}));
    assert!(fresh != before, "the profile loaded again changes the plates");
    assert!(warm == fresh);
}

/// Raster effects follow the Color Settings: a blurred CMYK fill, and a white square's drop shadow
/// in a CMYK color, are drawn again through the new working CMYK profile, in a frame drawn while
/// interacting (as during a dialog's preview) and after it.
#[test]
fn raster_effects_draw_through_new_color_settings() {
    let _g = settings();
    let mut s = session();
    let blurred = rect(&mut s, 20.0, 20.0, 160.0, 120.0);
    exec(&mut s, "paint.setFill", json!({"ids": [blurred.0], "color": {"c": 0, "m": 60, "y": 100, "k": 0}}));
    exec(&mut s, "effect.apply", json!({"ids": [blurred.0], "effect": "blur.gaussian", "params": {"radius": 2}}));
    let lit = rect(&mut s, 220.0, 20.0, 120.0, 120.0);
    exec(&mut s, "paint.setFill", json!({"ids": [lit.0], "color": "#ffffff"}));
    let cyan = json!({"model": "cmyk", "c": 1.0, "m": 0.0, "y": 0.0, "k": 0.0});
    let shadow = json!({"x": 8, "y": 8, "blur": 1, "opacity": 100, "mode": "normal", "color": cyan});
    exec(&mut s, "effect.apply", json!({"ids": [lit.0], "effect": "stylize.dropShadow", "params": shadow}));
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let live = RenderOptions { interacting: true, ..opts.clone() };
    let frame = |r: &mut Renderer, s: &Session, o: &RenderOptions| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, o).pixels;
    let mut r = Renderer::new();
    let before = frame(&mut r, &s, &opts);
    let old = cms::active_settings();
    s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let fresh = frame(&mut Renderer::new(), &s, &opts);
    assert!(fresh != before, "the new profile changes the frame");
    assert!(frame(&mut r, &s, &live) == fresh, "while interacting");
    assert!(frame(&mut r, &s, &opts) == fresh, "after it");
    cms::set_active(&old).unwrap();
}

/// Pattern tiles follow the Color Settings: a fill of a pattern in a CMYK color is drawn through
/// the new working CMYK profile in the next frame, as a renderer on a new thread (with no tiles of
/// its own yet) draws it.
#[test]
fn pattern_tiles_draw_through_new_color_settings() {
    use std::sync::Arc;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::pattern::{PatternDef, pattern_paint};
    use vectorcraft_doc::{Appearance, Document, Node};
    use vectorcraft_geom::Rect;
    use vectorcraft_geom::shapes::rectangle;
    let _g = settings();
    let old = cms::active_settings();
    let mut d = Document::new(200.0, 100.0);
    let cyan = Appearance::basic(Paint::solid(Color::cmyk(1.0, 0.0, 0.0, 0.0)), Paint::None, 0.0);
    let square = Node::path(d.alloc_id(), rectangle(Rect::new(0.0, 0.0, 10.0, 10.0)), cyan);
    let mut def = PatternDef::new("Checks", vec![Arc::new(square)]);
    def.tile = Rect::new(0.0, 0.0, 20.0, 20.0);
    d.patterns.push(def);
    let checks = Appearance::basic(pattern_paint("Checks"), Paint::None, 0.0);
    let filled = Node::path(d.alloc_id(), rectangle(Rect::new(10.0, 10.0, 190.0, 90.0)), checks);
    let layer = d.layers[0].id;
    d.insert(Some(layer), 0, filled).unwrap();
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let frame = |r: &mut Renderer, d: &Document| r.render(d, 200, 100, Affine::IDENTITY, &opts).pixels;
    let mut r = Renderer::new();
    let before = frame(&mut r, &d);
    session().execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
    let warm = frame(&mut r, &d);
    let fresh = std::thread::scope(|t| t.spawn(|| frame(&mut Renderer::new(), &d)).join().unwrap());
    cms::set_active(&old).unwrap();
    assert!(fresh != before, "the new profile changes the frame");
    assert!(warm == fresh, "drawn as a renderer with no tiles draws it");
}

/// Live objects follow the Color Settings: in the next frame, a blend of an RGB red and a CMYK cyan,
/// the same blend in an envelope, an envelope that distorts the appearance of a CMYK cyan rectangle
/// with Revolve, and a gradient mesh in CMYK colors draw through the new working CMYK profile, and
/// so do the blend and the mesh with a drop shadow.
#[test]
fn live_objects_draw_through_new_color_settings() {
    let _g = settings();
    let old = cms::active_settings();
    let cyan = json!({"c": 100, "m": 0, "y": 0, "k": 0});
    let blend = |s: &mut Session| {
        let a = rect(s, 20.0, 40.0, 60.0, 120.0);
        exec(s, "paint.setFill", json!({"ids": [a.0], "color": "#ff0000"}));
        let b = rect(s, 300.0, 40.0, 60.0, 120.0);
        exec(s, "paint.setFill", json!({"ids": [b.0], "color": cyan}));
        id_of(&exec(s, "object.blend.make", json!({"ids": [a.0, b.0], "steps": 5})))
    };
    let mut blended = session();
    blend(&mut blended);
    let mut enveloped = session();
    let b = blend(&mut enveloped);
    exec(&mut enveloped, "object.envelope.makeWithWarp", json!({"ids": [b.0], "style": "arc", "bend": 30}));
    let mut revolved = session();
    let profile = rect(&mut revolved, 150.0, 60.0, 60.0, 120.0);
    exec(&mut revolved, "paint.setFill", json!({"ids": [profile.0], "color": cyan}));
    exec(&mut revolved, "effect.apply", json!({"ids": [profile.0], "effect": "threeD.revolve"}));
    let e = id_of(&exec(&mut revolved, "object.envelope.makeWithWarp", json!({"ids": [profile.0], "style": "arc", "bend": 30})));
    select(&mut revolved, &[e]);
    exec(&mut revolved, "object.envelope.options", json!({"distortAppearance": true}));
    let mesh = |s: &mut Session| {
        let m = rect(s, 20.0, 20.0, 360.0, 260.0);
        exec(s, "paint.setFill", json!({"ids": [m.0], "color": cyan}));
        exec(s, "object.mesh.create", json!({"ids": [m.0], "rows": 2, "cols": 2}));
        exec(s, "object.mesh.setPointColor", json!({"id": m.0, "index": 4, "color": {"c": 0, "m": 100, "y": 0, "k": 0}}));
        m
    };
    let mut meshed = session();
    mesh(&mut meshed);
    let shadow = json!({"x": 4, "y": 4, "blur": 1, "opacity": 100});
    let mut shadowed_blend = session();
    let b = blend(&mut shadowed_blend);
    exec(&mut shadowed_blend, "effect.apply", json!({"ids": [b.0], "effect": "stylize.dropShadow", "params": shadow}));
    let mut shadowed_mesh = session();
    let m = mesh(&mut shadowed_mesh);
    exec(&mut shadowed_mesh, "effect.apply", json!({"ids": [m.0], "effect": "stylize.dropShadow", "params": shadow}));
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let frame = |r: &mut Renderer, s: &Session| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &opts).pixels;
    for (what, mut s) in [
        ("the blend", blended),
        ("the envelope", enveloped),
        ("the revolved rectangle", revolved),
        ("the mesh", meshed),
        ("the blend with a drop shadow", shadowed_blend),
        ("the mesh with a drop shadow", shadowed_mesh),
    ] {
        let mut r = Renderer::new();
        let before = frame(&mut r, &s);
        s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
        let fresh = frame(&mut Renderer::new(), &s);
        let warm = frame(&mut r, &s);
        cms::set_active(&old).unwrap();
        assert!(fresh != before, "{what}: the new profile changes the frame");
        assert!(warm == fresh, "{what}: drawn as a new renderer draws it");
    }
}

/// The art drawn for 3D effects and color adjustments follows the Color Settings: in the next
/// frame, a CMYK cyan rectangle with Revolve, with Extrude & Bevel, with Inflate, with Rotate, with
/// Brightness/Contrast, and a group holding it with Brightness/Contrast, draw through the new
/// working CMYK profile.
#[test]
fn effect_art_draws_through_new_color_settings() {
    let _g = settings();
    let old = cms::active_settings();
    let cyan = json!({"c": 100, "m": 0, "y": 0, "k": 0});
    let brighter = json!({"brightness": 30});
    let cases = [
        ("the revolved rectangle", "threeD.revolve", json!({}), false),
        ("the extruded rectangle", "threeD.extrude", json!({}), false),
        ("the inflated rectangle", "threeD.inflate", json!({}), false),
        ("the rotated rectangle", "threeD.rotate", json!({}), false),
        ("the adjusted rectangle", "adjust.brightnessContrast", brighter.clone(), false),
        ("the adjusted group", "adjust.brightnessContrast", brighter, true),
    ];
    let opts = RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let frame = |r: &mut Renderer, s: &Session| r.render(&s.doc().unwrap().doc, 400, 300, Affine::IDENTITY, &opts).pixels;
    for (what, effect, params, grouped) in cases {
        let mut s = session();
        let shape = rect(&mut s, 150.0, 60.0, 60.0, 120.0);
        exec(&mut s, "paint.setFill", json!({"ids": [shape.0], "color": cyan}));
        let target = if grouped { id_of(&exec(&mut s, "object.group", json!({"ids": [shape.0]}))) } else { shape };
        exec(&mut s, "effect.apply", json!({"ids": [target.0], "effect": effect, "params": params}));
        let mut r = Renderer::new();
        let before = frame(&mut r, &s);
        s.execute("edit.colorSettings", &json!({"cmyk": cms::DEVICE_CMYK})).unwrap();
        let fresh = frame(&mut Renderer::new(), &s);
        let warm = frame(&mut r, &s);
        cms::set_active(&old).unwrap();
        assert!(fresh != before, "{what}: the new profile changes the frame");
        assert!(warm == fresh, "{what}: drawn as a new renderer draws it");
    }
}

/// #2276: with a working RGB space other than sRGB, the Eyedropper takes an image's sRGB pixel
/// into the working space, so the sampled fill shows as the pixel it was picked from.
#[test]
fn the_eyedropper_takes_an_image_pixel_into_the_working_rgb_space() {
    use vectorcraft_color::Color;
    use vectorcraft_doc::{ImageBlob, ImageObject, Node, NodeKind};
    use vectorcraft_engine::ViewInfo;
    use vectorcraft_tools::{PointerEvent, PointerKind};
    let _g = settings();
    let old = cms::active_settings();
    let mut s = session();
    let img = image::RgbaImage::from_raw(1, 1, vec![83, 170, 223, 255]).unwrap();
    let mut png = vec![];
    img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
    s.edit("Place", |d, _| {
        d.images.insert("px".into(), ImageBlob::png(png));
        let l = d.layers[0].id;
        let id = d.alloc_id();
        let xf = Affine::translate((300.0, 0.0)) * Affine::scale(100.0);
        let im = ImageObject { key: "px".into(), width: 1, height: 1, xf, link: None, placement: Default::default(), pixels: None };
        d.insert(Some(l), 0, Node::new(id, NodeKind::Image(Box::new(im)))).map_err(|e| vectorcraft_engine::EngineError::Other(e.to_string()))
    })
    .unwrap();
    let dst = rect(&mut s, 10.0, 10.0, 50.0, 50.0);
    s.execute("edit.colorSettings", &json!({"rgb": cms::WIDE_GAMUT_RGB})).unwrap();
    s.select_tool("eyedropper", ViewInfo::default()).unwrap();
    s.pointer(&PointerEvent::new(PointerKind::Down, 350.0, 50.0), ViewInfo::default()).unwrap();
    let got = s.doc().unwrap().doc.node(dst).unwrap().appearance.fill_paint().color();
    let shown = got.map(|c| c.to_rgba8(1.0));
    cms::set_active(&old).unwrap();
    assert!(matches!(got, Some(Color::Rgb { .. })), "{got:?}");
    let [r, g, b, _] = shown.unwrap_or_default();
    assert!(r.abs_diff(83) <= 1 && g.abs_diff(170) <= 1 && b.abs_diff(223) <= 1, "{shown:?} for 83/170/223");
}

/// With a working RGB space other than sRGB, a freeform gradient of RGB points expands to mesh
/// points with the points' own RGB values, and points in different models expand to mesh points
/// that show the colors the gradient shows there.
#[test]
fn a_freeform_gradient_expands_with_its_rgb_values_in_the_working_rgb_space() {
    use vectorcraft_color::freeform::spread_scale;
    use vectorcraft_color::{Color, Paint};
    use vectorcraft_doc::NodeKind;
    use vectorcraft_geom::Rect;
    let _g = settings();
    let old = cms::active_settings();
    cms::set_active(&cms::ColorSettings { rgb: cms::WIDE_GAMUT_RGB.into(), ..old.clone() }).unwrap();
    // Each mesh point's color, the display color it shows and the display color the gradient
    // shows at its place.
    let expand = |colors: [serde_json::Value; 4]| {
        let mut s = session();
        let id = rect(&mut s, 20.0, 20.0, 160.0, 160.0);
        select(&mut s, &[id]);
        exec(&mut s, "paint.setFill", json!({"gradient": {"kind": "freeform"}}));
        exec(&mut s, "paint.setStroke", json!({"none": true}));
        for (i, c) in colors.into_iter().enumerate() {
            exec(&mut s, "paint.freeform.setPoint", json!({"index": i, "color": c, "stroke": false}));
        }
        let b = Rect::new(20.0, 20.0, 180.0, 180.0);
        let field = match s.doc().unwrap().doc.node(id).map(|n| n.appearance.fill_paint()) {
            Some(Paint::Gradient(g)) => g.freeform_on(b).field(spread_scale(b)),
            p => panic!("a gradient fill: {p:?}"),
        };
        exec(&mut s, "object.expand", json!({}));
        let d = &s.doc().unwrap();
        let mesh = d.selection.objects.first().and_then(|id| d.doc.node(*id)).map(|n| n.kind.clone());
        let Some(NodeKind::Mesh(m)) = mesh else { panic!("a mesh: {mesh:?}") };
        m.points.iter().map(|p| (p.color, p.color.to_rgb(), field.sample(p.p).0)).collect::<Vec<_>>()
    };
    let rgb = expand([json!("#3399e6"), json!("#3399e6"), json!("#3399e6"), json!("#3399e6")]);
    let mixed = expand([json!("#3399e6"), json!("#3399e6"), json!({"gray": 0.5}), json!({"gray": 0.5})]);
    cms::set_active(&old).unwrap();
    let want = Color::rgb8(0x33, 0x99, 0xe6);
    for (c, ..) in &rgb {
        let (Color::Rgb { r, g, b }, Color::Rgb { r: r2, g: g2, b: b2 }) = (*c, want) else { panic!("an RGB point: {c:?}") };
        assert!((r - r2).abs() < 1e-5 && (g - g2).abs() < 1e-5 && (b - b2).abs() < 1e-5, "{c:?} for {want:?}");
    }
    for (c, shown, field) in &mixed {
        let close = shown.iter().zip(field).all(|(a, b)| (a - b).abs() < 2.0 / 255.0);
        assert!(matches!(c, Color::Rgb { .. }) && close, "{c:?} shows {shown:?} for {field:?}");
    }
}

/// Loading a new file of the working RGB profile (named after its description, as before) takes
/// effect at once: CMYK colors convert into the new file and the canvas redraws, without choosing
/// the working spaces again.
#[test]
fn loading_the_working_rgb_profile_again_takes_effect_at_once() {
    let _g = settings();
    let old = cms::active_settings();
    // A built-in RGB profile's file renamed `name`: padded with spaces, which names drop.
    let renamed = |builtin: &str, name: &str| {
        let utf16 = |s: &str| s.encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>();
        let mut bytes = cms::icc_bytes(builtin).unwrap().to_vec();
        let (from, to) = (utf16(builtin), utf16(&format!("{name:<width$}", width = builtin.len())));
        let at = bytes.windows(from.len()).position(|w| w == from).unwrap();
        bytes[at..at + to.len()].copy_from_slice(&to);
        bytes
    };
    let name = "Reload RGB";
    let dir = vectorcraft_testkit::temp_dir("reload-working-rgb");
    let load = |s: &mut Session, builtin: &str| {
        let path = dir.join(format!("{}.icc", builtin.len()));
        std::fs::write(&path, renamed(builtin, name)).unwrap();
        let r = s.execute("color.loadProfile", &json!({"path": path.to_str().unwrap()})).unwrap();
        assert_eq!(r["name"], name);
    };
    let cmyk = cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some("Reload Working CMYK".into())).unwrap().name;
    let cyan = [1.0, 0.0, 0.0, 0.0];
    let convert = || cms::active().cmyk_to_rgb(cyan, old.intent);
    let mut s = session();
    load(&mut s, cms::DISPLAY_P3);
    s.execute("edit.colorSettings", &json!({"rgb": name, "cmyk": cmyk})).unwrap();
    let in_p3 = convert();
    let revision = s.doc().unwrap().revision;
    load(&mut s, cms::PROPHOTO_RGB);
    let after = convert();
    let redrawn = s.doc().unwrap().revision > revision;
    // What a CMYK profile loaded afresh gives in the new file.
    let fresh = cms::register_icc(&cms::icc_bytes(cms::GENERIC_CMYK).unwrap(), Some("Reload Working CMYK 2".into())).unwrap().name;
    let want = cms::Cms::new(&cms::ColorSettings { cmyk: fresh, ..cms::active_settings() }).unwrap().cmyk_to_rgb(cyan, old.intent);
    cms::set_active(&old).unwrap();
    assert!(in_p3.iter().zip(want).any(|(a, b)| (a - b).abs() > 0.05), "Display P3 {in_p3:?} vs ProPhoto {want:?}");
    assert!(after.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-4), "after the reload {after:?}, want {want:?}");
    assert!(redrawn, "the canvas redraws");
}
