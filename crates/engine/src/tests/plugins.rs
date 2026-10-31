//! WebAssembly plug-ins (#121): object filters through `plugin.run` (one undo step,
//! transactional), live-effect plug-ins in appearance stacks, and the `plugin.*` management
//! commands.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_plugins::wat as tpl;

use crate::*;

/// The example Desaturate plug-in (built from `crates/plugins/example`).
const DESATURATE: &[u8] = include_bytes!("../../../plugins/tests/fixtures/desaturate.wasm");
const DESATURATE_ID: &str = "org.vectorcraft.example.desaturate";

fn run(s: &mut Session, cmd: &str, p: Value) -> Value {
    s.execute(cmd, &p).unwrap_or_else(|e| panic!("{cmd}: {e}"))
}

fn install(s: &mut Session, wasm: &[u8]) -> Value {
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm)}))
}

fn install_wat(s: &mut Session, src: &str) -> Value {
    install(s, &wat::parse_str(src).unwrap())
}

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", &json!({"width": 400, "height": 200})).unwrap();
    s
}

/// A 10 × 10 rectangle at `x` filled with `fill` and no stroke.
fn rect(s: &mut Session, x: f64, fill: &str) -> NodeId {
    let id = NodeId(run(s, "shape.rectangle", json!({"x": x, "y": 10, "width": 10, "height": 10}))["id"].as_u64().unwrap());
    run(s, "paint.setFill", json!({"ids": [id.0], "color": fill}));
    run(s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
    id
}

fn node(s: &Session, id: NodeId) -> vectorcraft_doc::Node {
    s.doc().unwrap().doc.node(id).unwrap().clone()
}

fn triangle() -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(30.0, 0.0), Point::new(15.0, 20.0)], true))
}

#[test]
fn object_filter_recolours_the_selection_as_one_undo_step() {
    let mut s = session();
    let info = install(&mut s, DESATURATE);
    assert_eq!((info["id"].as_str(), info["kind"].as_str()), (Some(DESATURATE_ID), Some("filter")));
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#0000ff");
    run(&mut s, "select.all", json!({}));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "plugin.run", json!({"id": DESATURATE_ID, "params": {"amount": 100}}));
    assert_eq!(r["ids"], json!([a.0, b.0]));
    for id in [a, b] {
        let [r, g, b] = node(&s, id).appearance.fill_paint().color().unwrap().to_rgb_uncalibrated();
        assert!((r - g).abs() < 1e-6 && (g - b).abs() < 1e-6, "grey: {r} {g} {b}");
    }
    let st = s.doc().unwrap();
    assert_eq!(st.history.undo.len(), undo + 1);
    assert_eq!(st.history.undo.last().unwrap().label, "Desaturate");
    assert_eq!(st.selection.objects, [a, b]);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    // Plug-in parameters may also be top-level keys; wrong types are refused before it runs.
    run(&mut s, "plugin.run", json!({"id": DESATURATE_ID, "amount": 0}));
    assert_eq!(node(&s, a).appearance.fill_paint(), Paint::solid(Color::rgb(1.0, 0.0, 0.0)));
    assert!(matches!(s.execute("plugin.run", &json!({"id": DESATURATE_ID, "amount": "all"})), Err(EngineError::BadParams { .. })));
}

#[test]
fn object_filters_add_replace_and_generate_objects() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 40.0, "#00ff00");
    run(&mut s, "select.all", json!({}));
    // Keeps A, drops B and adds a triangle above A.
    let out =
        json!({"objects": [{"id": a.0}, {"path": triangle(), "fills": [{"type": "solid", "color": {"model": "gray", "k": 1}}], "name": "Tri"}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.replace", "filter", "{}"), &out.to_string()));
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.replace"}));
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert_eq!(ids[0], a);
    let st = s.doc().unwrap();
    assert!(st.doc.node(b).is_none());
    let layer: Vec<NodeId> = st.doc.layers[0].children().unwrap().iter().map(|n| n.id).collect();
    assert_eq!(layer, [a, ids[1]]);
    assert_eq!(st.doc.node(ids[1]).unwrap().name.as_deref(), Some("Tri"));
    assert_eq!(st.selection.objects, ids);
    // A generator with nothing selected adds to the current layer.
    let generated = json!({"objects": [{"path": triangle()}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.generate", "filter", "{}"), &generated.to_string()));
    run(&mut s, "select.none", json!({}));
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.generate"}));
    let made = NodeId(r["ids"][0].as_u64().unwrap());
    assert_eq!(s.doc().unwrap().doc.layers[0].children().unwrap().last().unwrap().id, made);
}

#[test]
fn failing_plugins_leave_the_document_and_history_alone() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let man = |id: &str| tpl::manifest(id, "filter", "{}");
    install_wat(&mut s, &tpl::module(&man("test.engine.trap"), "unreachable", "", 1));
    install_wat(&mut s, &tpl::returning(&man("test.engine.far"), r#"{"objects":[{"path":{"subpaths":[{"anchors":[{"p":[0,0]},{"p":[1e9,0]}]}]}}]}"#));
    install_wat(&mut s, &tpl::returning(&man("test.engine.declines"), r#"{"error":"Select two paths."}"#));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    for id in ["test.engine.trap", "test.engine.far", "test.engine.declines", "test.engine.missing"] {
        let e = s.execute("plugin.run", &json!({"id": id})).unwrap_err();
        if id == "test.engine.declines" {
            assert!(e.to_string().contains("Select two paths."), "{e}");
        }
        assert_eq!(s.doc().unwrap().doc, before, "{id}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{id}");
    }
}

#[test]
fn live_effect_plugins_reshape_editable_art_and_expand() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    // An effect that draws a triangle, and one that hands the geometry back.
    let tri = json!({"objects": [{"path": triangle()}]});
    install_wat(
        &mut s,
        &tpl::returning(&tpl::manifest("test.engine.tri", "effect", r#"{"size":{"type":"number","min":0,"max":10,"default":3}}"#), &tri.to_string()),
    );
    install_wat(&mut s, &tpl::module(&tpl::manifest("test.engine.echo", "effect", "{}"), tpl::ECHO, "", 1));
    let catalog = run(&mut s, "effect.list", json!({}))["catalog"].clone();
    let entry = catalog.as_array().unwrap().iter().find(|e| e["id"] == "plugin.test.engine.tri").expect("listed");
    assert_eq!((entry["label"].as_str(), entry["menu"].clone()), (Some("Test test.engine.tri…"), json!(["Effect", "Plug-ins"])));
    assert_eq!(entry["defaults"], json!({"size": 3.0}));
    // A filter can't be applied as an effect, nor an effect run as a filter.
    install(&mut s, DESATURATE);
    assert!(s.execute("effect.apply", &json!({"ids": [a.0], "effect": format!("plugin.{DESATURATE_ID}")})).is_err());
    assert!(s.execute("plugin.run", &json!({"id": "test.engine.tri"})).is_err());

    let effected = |s: &Session| {
        let n = node(s, a);
        let path = n.path_data().unwrap().clone();
        let b = path.bounds().unwrap();
        vectorcraft_render::effects::apply_geometry(&n.appearance.effects, &path, b)
    };
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.echo"}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap(), "identity");
    // Re-evaluated when the object changes (non-destructive).
    run(&mut s, "select.all", json!({}));
    run(&mut s, "object.move", json!({"dx": 50, "dy": 0}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.tri", "params": {"size": 4}}));
    assert_eq!(effected(&s), triangle());
    assert_eq!(node(&s, a).appearance.effects[1].params, json!({"size": 4}));
    // Uninstalled: the geometry is drawn as it is.
    run(&mut s, "plugin.remove", json!({"id": "test.engine.tri"}));
    assert_eq!(effected(&s), *node(&s, a).path_data().unwrap());
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.tri", "effect", "{}"), &tri.to_string()));
    // Expand Appearance bakes it.
    run(&mut s, "select.all", json!({}));
    run(&mut s, "effect.expandAppearance", json!({}));
    let mut baked = None;
    s.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            baked = Some(p.clone());
        }
    });
    assert_eq!(baked.unwrap().bounds(), triangle().bounds());
}

/// A live effect is given its object's fills and strokes, read only, and which of them it sits on;
/// a change of paint runs it again.
#[test]
fn live_effects_are_given_their_object_s_paints() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "paint.setStroke", json!({"ids": [id.0], "color": "#000000"}));
    run(&mut s, "stroke.set", json!({"ids": [id.0], "weight": 5}));
    // The red fill as the host writes paints: a JSON value, its keys sorted.
    let red = serde_json::to_value(&node(&s, id).appearance.fill().unwrap().paint).unwrap().to_string();
    let tri = json!({"objects": [{"path": triangle()}]}).to_string();
    // Each draws the triangle while it is given what it looks for, else hands the geometry back.
    let effect = |id: &str, given, needle: &str| tpl::when_given(&tpl::manifest(id, "effect", "{}"), given, needle, &tri);
    install_wat(&mut s, &effect("test.engine.wide", tpl::Given::Input, r#""width":5.0"#));
    install_wat(&mut s, &effect("test.engine.red", tpl::Given::Input, &format!(r#""fills":[{red}]"#)));
    install_wat(&mut s, &effect("test.engine.second", tpl::Given::Params, r#""item":{"stroke":1}"#));
    let effected = |s: &Session, item: Option<usize>| {
        let n = node(s, id);
        let path = n.path_data().unwrap().clone();
        let ctx = vectorcraft_render::effects::GeomContext::of(&n);
        let (effects, ctx) = match item {
            Some(i) => match &n.appearance.items[i] {
                vectorcraft_doc::AppearanceItem::Stroke(st) => (st.effects.clone(), ctx.item(&n.appearance.items[i])),
                other => panic!("not a stroke: {other:?}"),
            },
            None => (n.appearance.effects.clone(), ctx),
        };
        vectorcraft_render::effects::apply_geometry_with(&effects, &path, path.bounds().unwrap(), &ctx)
    };
    let own = node(&s, id).path_data().unwrap().clone();
    let remove = |s: &mut Session| run(s, "effect.remove", json!({"ids": [id.0], "index": 0, "item": null}));
    for effect in ["plugin.test.engine.wide", "plugin.test.engine.red"] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": effect, "item": null}));
        assert_eq!(effected(&s, None), triangle(), "{effect}: given the 5 pt stroke and the red fill");
        remove(&mut s);
    }
    // Run again once the stroke is thinner.
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.wide", "item": null}));
    run(&mut s, "stroke.set", json!({"ids": [id.0], "weight": 3, "item": 1}));
    assert_eq!(effected(&s, None), own, "a 3 pt stroke");
    remove(&mut s);
    // On the second of two strokes, and not on the first.
    run(&mut s, "appearance.addStroke", json!({"ids": [id.0]}));
    let top = node(&s, id).appearance.items.len() - 1;
    for item in [1, top] {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.second", "item": item}));
    }
    assert_eq!(effected(&s, Some(1)), own, "the first stroke");
    assert_eq!(effected(&s, Some(top)), triangle(), "the second stroke");
}

/// Expand gives a plug-in effect on a fill the context drawing gives it (`_context.item`), so a
/// gradient fill is expanded into the shape the canvas fills.
#[test]
fn expanding_a_fill_gives_its_plug_in_effect_the_fill_it_sits_on() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": {"kind": "linear"}}));
    let tri = json!({"objects": [{"path": triangle()}]}).to_string();
    let on_fill = tpl::when_given(&tpl::manifest("test.engine.onfill", "effect", "{}"), tpl::Given::Params, r#""item":{"fill":0}"#, &tri);
    install_wat(&mut s, &on_fill);
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.onfill", "item": fill}));
    // Drawn, the gradient fills the triangle: a point of it outside the rectangle is painted.
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let drawn = vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 40, 40, vectorcraft_geom::Affine::IDENTITY, &opts);
    assert_ne!(drawn.pixel(15, 1), [255, 255, 255, 255], "drawn as a triangle");
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.expand", json!({"steps": 2}));
    // The gradient's strips sit in a clip group whose clipping path is the shape filled.
    let group = s.doc().unwrap().selection.objects[0];
    let clip = node(&s, group).children().unwrap()[0].clone();
    assert_eq!(clip.path_data().unwrap().bounds(), triangle().bounds(), "the triangle the canvas fills");
}

/// `CRAFT_PLUGIN_PAINTS_WASM=<folder>` saves a test live effect to try in the app
/// (`stroke-width-test-effect.wasm`): it draws a triangle over the area 100,80–300,230 while its
/// object's stroke is 5 pt wide, and hands the geometry back otherwise.
#[test]
fn stroke_width_test_effect() {
    let Some(dir) = std::env::var_os("CRAFT_PLUGIN_PAINTS_WASM") else { return };
    let triangle = PathData::single(SubPath::polyline(&[Point::new(100.0, 230.0), Point::new(300.0, 230.0), Point::new(200.0, 80.0)], true));
    let manifest = r#"{"id":"org.example.strokewidth","name":"Stroke Width (test plug-in)","version":"1","kind":"effect",
        "description":"Draws a triangle while the object's stroke is 5 pt wide.","params":{}}"#;
    let src = tpl::when_given(manifest, tpl::Given::Input, r#""width":5.0"#, &json!({"objects": [{"path": triangle}]}).to_string());
    std::fs::write(std::path::Path::new(&dir).join("stroke-width-test-effect.wasm"), wat::parse_str(src).unwrap()).unwrap();
}

/// A filter is given its objects' effect records and their strokes' join, cap and miter limit, and
/// may return them. The records are admitted as effect.apply would admit them; one that isn't
/// refuses the whole output, and nothing changes.
#[test]
fn filters_set_effects_and_stroke_styles() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    install_wat(
        &mut s,
        &tpl::module(&tpl::manifest("test.engine.fx", "effect", r#"{"n":{"type":"int","min":0,"max":3,"default":1}}"#), tpl::ECHO, "", 1),
    );
    let black = json!({"type": "solid", "color": {"model": "gray", "k": 1.0}});
    let out = |fx: Value| json!({"objects": [{"id": id.0, "strokes": [{"paint": black, "width": 4, "join": "round", "cap": "square", "miterLimit": 4}], "effects": fx}]});
    let filter = |s: &mut Session, name: &str, fx: Value| {
        install_wat(s, &tpl::returning(&tpl::manifest(name, "filter", "{}"), &out(fx).to_string()));
        s.execute("plugin.run", &json!({"id": name, "ids": [id.0]}))
    };
    let fx = json!([{"id": "distort.roughen", "params": {"size": 3}}, {"id": "plugin.test.engine.fx", "params": {"n": 2}, "visible": false}]);
    filter(&mut s, "test.engine.setfx", fx).unwrap();
    let n = node(&s, id);
    let st = n.appearance.stroke().unwrap();
    assert_eq!((st.width, st.join, st.cap, st.miter_limit), (4.0, vectorcraft_doc::LineJoin::Round, vectorcraft_doc::LineCap::Square, 4.0));
    let effects = &n.appearance.effects;
    assert_eq!(effects.iter().map(|e| (e.id.as_str(), e.visible)).collect::<Vec<_>>(), [("distort.roughen", true), ("plugin.test.engine.fx", false)]);
    assert_eq!(effects[0].params, vectorcraft_render::effects::merged_params("distort.roughen", &json!({"size": 3})), "filled in");
    assert_eq!(effects[1].params, json!({"n": 2}));
    // Handed back unchanged (an echo), nothing changes, not even the records' parameters.
    install_wat(&mut s, &tpl::module(&tpl::manifest("test.engine.same", "filter", "{}"), tpl::ECHO, "", 1));
    let before = s.doc().unwrap().doc.clone();
    run(&mut s, "plugin.run", json!({"id": "test.engine.same", "ids": [id.0]}));
    assert_eq!(s.doc().unwrap().doc, before);
    // Refused: what effect.apply refuses, a live effect that isn't installed, parameters its
    // manifest refuses, a texture to read from a file, two 3D effects.
    for (name, fx, why) in [
        ("test.engine.unknown", json!([{"id": "distort.nothing"}]), "no such effect"),
        ("test.engine.key", json!([{"id": "distort.roughen", "params": {"bogus": 1}}]), "bogus"),
        ("test.engine.gone", json!([{"id": "plugin.test.engine.gone"}]), "no such effect"),
        ("test.engine.badn", json!([{"id": "plugin.test.engine.fx", "params": {"n": "two"}}]), "`n`"),
        ("test.engine.file", json!([{"id": "distort.glass", "params": {"textureImage": {"path": "/etc/hosts"}}}]), "textureImage"),
        ("test.engine.file2", json!([{"id": "texture.texturizer", "params": {"textureImage": {"path": "/etc/hosts"}}}]), "textureImage"),
        ("test.engine.two3d", json!([{"id": "threeD.extrude"}, {"id": "threeD.extrude"}]), "one 3D effect"),
    ] {
        let e = filter(&mut s, name, fx).unwrap_err().to_string();
        assert!(e.contains("objects[0].effects") && e.contains(why), "{name}: {e}");
        assert_eq!(s.doc().unwrap().doc, before, "{name}");
    }
}

/// `CRAFT_PLUGIN_FRAME_WASM=<folder>` saves a test filter to try in the app
/// (`rough-frame-test-filter.wasm`): it adds a frame around 60,40–340,260 with a 6 pt stroke with
/// round joins and caps, roughened by the Roughen effect.
#[test]
fn rough_frame_test_filter() {
    let Some(dir) = std::env::var_os("CRAFT_PLUGIN_FRAME_WASM") else { return };
    let frame = PathData::single(SubPath::polyline(
        &[Point::new(60.0, 40.0), Point::new(340.0, 40.0), Point::new(340.0, 260.0), Point::new(60.0, 260.0)],
        true,
    ));
    let ink = json!({"type": "solid", "color": {"model": "rgb", "r": 0.15, "g": 0.2, "b": 0.45}});
    let out = json!({"objects": [{"name": "Frame", "path": frame, "fills": [],
        "strokes": [{"paint": ink, "width": 6, "join": "round", "cap": "round"}],
        "effects": [{"id": "distort.roughen", "params": {"size": 2, "detail": 8, "points": "smooth"}}]}]});
    let manifest = r#"{"id":"org.example.frame","name":"Rough Frame (test plug-in)","version":"1","kind":"filter",
        "description":"Adds a frame whose stroke is roughened by the Roughen effect.","params":{}}"#;
    let src = tpl::returning(manifest, &out.to_string());
    std::fs::write(std::path::Path::new(&dir).join("rough-frame-test-filter.wasm"), wat::parse_str(src).unwrap()).unwrap();
}

/// A filter keeps data with the document: it writes it with `documentData` in the same undo step as
/// its objects (`null` removes it) and reads what it last wrote in `_context.documentData`, which
/// no other filter is given. It is the document's foreign data, so the native file keeps it with
/// no section of its own. More than the limit is refused.
#[test]
fn filters_keep_their_data_with_the_document() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let pid = "test.engine.data";
    let returning = |out: Value| tpl::returning(&tpl::manifest(pid, "filter", "{}"), &out.to_string());
    let keep = |data: Value| json!({"objects": [{"id": id.0}], "documentData": data});
    let data = |s: &Session| s.doc().unwrap().doc.plugin_data(pid).cloned();
    let filter = |s: &mut Session, pid: &str| s.execute("plugin.run", &json!({"id": pid, "ids": [id.0]}));
    install_wat(&mut s, &returning(keep(json!({"pen": 2}))));
    filter(&mut s, pid).unwrap();
    assert_eq!(data(&s), Some(json!({"pen": 2})));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(data(&s), None, "one undo step with the objects");
    run(&mut s, "edit.redo", json!({}));
    // Read back: these name the object only when they are given what they look for.
    let named = |name: &str| json!({"objects": [{"id": id.0, "name": name}]}).to_string();
    let when_given =
        |pid: &str, needle: &str, name: &str| tpl::when_given(&tpl::manifest(pid, "filter", "{}"), tpl::Given::Params, needle, &named(name));
    install_wat(&mut s, &when_given(pid, r#""documentData":{"pen":2}"#, "Pen 2"));
    filter(&mut s, pid).unwrap();
    assert_eq!(node(&s, id).name.as_deref(), Some("Pen 2"));
    // Another filter is given nothing: what a filter keeps is its own.
    let other = "test.engine.other";
    install_wat(&mut s, &when_given(other, r#""documentData":null"#, "Nothing kept"));
    filter(&mut s, other).unwrap();
    assert_eq!(node(&s, id).name.as_deref(), Some("Nothing kept"));
    // Saved with the document, among its foreign data.
    let saved = vectorcraft_format::load(&vectorcraft_format::save(&s.doc().unwrap().doc, false)).unwrap();
    assert_eq!(saved.plugin_data(pid), Some(&json!({"pen": 2})));
    assert_eq!(saved.unknown.get(vectorcraft_doc::plugin_data::PLUGIN_DATA), Some(&json!({pid: {"pen": 2}})));
    // Too much is refused, and the document stays as it was; null removes the data.
    let before = s.doc().unwrap().doc.clone();
    let big = "x".repeat(vectorcraft_plugins::objects::MAX_DOCUMENT_DATA);
    install_wat(&mut s, &returning(keep(json!(big))));
    let e = filter(&mut s, pid).unwrap_err().to_string();
    assert!(e.contains("documentData"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    install_wat(&mut s, &returning(keep(Value::Null)));
    filter(&mut s, pid).unwrap();
    assert_eq!(data(&s), None);
}

#[test]
fn plugin_management_commands() {
    let mut s = session();
    install(&mut s, DESATURATE);
    let list = run(&mut s, "plugin.list", json!({}));
    assert!(list["plugins"].as_array().unwrap().iter().any(|p| p["id"] == DESATURATE_ID));
    let info = run(&mut s, "plugin.info", json!({"id": DESATURATE_ID}));
    assert_eq!(info["defaults"], json!({"amount": 100.0}));
    assert_eq!(info["params"], "{amount: 0..100 (100)}");
    assert_eq!(info["manifest"]["kind"], "filter");
    // Refused installs.
    for bad in [
        json!({}),
        json!({"dataBase64": "!!"}),
        json!({"dataBase64": "AGFzbQ=="}),
        json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE), "replace": false}),
    ] {
        assert!(s.execute("plugin.install", &bad).is_err(), "{bad}");
    }
    assert!(s.execute("plugin.info", &json!({"id": "test.engine.nothing"})).is_err());
    assert!(s.execute("plugin.remove", &json!({"id": "test.engine.nothing"})).is_err());
    assert!(s.execute("plugin.reload", &json!({})).is_err(), "no folder set");

    // The Additional Plug-ins Folder preference installs what's in it.
    let dir = std::env::temp_dir().join(format!("vectorcraft-engine-plugins-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = tpl::returning(&tpl::manifest("test.engine.folder", "filter", "{}"), r#"{"objects":[]}"#);
    std::fs::write(dir.join("folder.wasm"), wat::parse_str(src).unwrap()).unwrap();
    std::fs::write(dir.join("broken.wasm"), b"\0asm?").unwrap();
    run(&mut s, "prefs.set", json!({"key": "pluginsFolder", "value": dir.display().to_string()}));
    assert!(vectorcraft_plugins::registry::get("test.engine.folder").is_some());
    run(&mut s, "plugin.remove", json!({"id": "test.engine.folder"}));
    let r = run(&mut s, "plugin.reload", json!({}));
    assert_eq!(r["loaded"], json!(["test.engine.folder"]));
    assert_eq!(r["failed"].as_array().unwrap().len(), 1);
    assert_eq!(run(&mut s, "plugin.list", json!({}))["folder"]["loaded"], json!(["test.engine.folder"]));
    std::fs::remove_dir_all(&dir).ok();
}

/// #2541: a plug-in installed in a session with a plug-in store is installed again in the next
/// session that attaches the store, and Remove Plug-in takes it out of the store.
#[test]
fn installed_plugins_are_kept_in_the_store() {
    let folder = crate::tests_links::Folder::new("plugin-store");
    let id = "test.engine.stored";
    let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_wat(&mut s, &src);
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1, "one file in the store");
    vectorcraft_plugins::registry::remove(id);

    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again from the store");
    run(&mut next, "plugin.remove", json!({"id": id}));
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 0, "removed from the store");

    // A file that isn't a plug-in is skipped; the store still attaches.
    std::fs::write(folder.0.join("broken.wasm"), b"\0asm?").unwrap();
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none());
}

/// A heavy test filter `id` that counts `steps` down, then adds a triangle named "Plug-in".
fn heavy(id: &str, steps: u64) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"filter","budget":"heavy"}}"#);
    let output = json!({"objects": [{"path": triangle(), "name": "Plug-in"}]});
    wat::parse_str(tpl::spinning(&manifest, steps, &output.to_string())).unwrap()
}

/// A filter that asks for a heavy budget installs only once the user agrees (`allowHeavy`);
/// `plugin.inspect` tells whoever installs it so beforehand, and installs nothing.
#[test]
fn a_heavy_filter_installs_once_the_user_agrees() {
    let mut s = session();
    let id = "test.engine.heavy";
    let data = vectorcraft_format::base64_encode(&heavy(id, 1));
    // With the file's name, as the app passes it.
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": data, "name": "route.wasm"}));
    assert_eq!((inspected["id"].as_str(), inspected["budget"].as_str()), (Some(id), Some("heavy")));
    assert_eq!(inspected["source"], "route.wasm");
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "inspecting installs nothing");
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("heavy budget") && e.contains("5 minutes") && e.contains("allowHeavy"), "{e}");
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    let info = run(&mut s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true}));
    assert_eq!(info["budget"], "heavy");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().limits().fuel_base, vectorcraft_plugins::HEAVY_FUEL);
    let listed = run(&mut s, "plugin.list", json!({}));
    assert!(listed["plugins"].as_array().unwrap().iter().any(|p| p["id"] == id && p["budget"] == "heavy"));
    // A light plug-in's entry says nothing of budgets.
    assert!(install(&mut s, DESATURATE).get("budget").is_none());
}

/// The user agreed once: a heavy filter kept in the plug-in store is installed again at the next
/// start without asking, with its budget.
#[test]
fn a_stored_heavy_filter_keeps_its_budget() {
    let folder = crate::tests_links::Folder::new("plugin-store-heavy");
    let id = "test.engine.storedheavy";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1)), "allowHeavy": true}));
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    let plugin = vectorcraft_plugins::registry::get(id).expect("installed again from the store");
    assert_eq!(plugin.limits().fuel_base, vectorcraft_plugins::HEAVY_FUEL);
    run(&mut next, "plugin.remove", json!({"id": id}));
}

/// The app runs a heavy filter away from the session ([`cmd::plugin::FilterRun`]) and hands its
/// output in: `plugin.run` with the same params applies it, as one undo step, without running the
/// plug-in again. Only once, and only for the same run: objects that changed since, or the plug-in
/// installed again, run the plug-in instead.
#[test]
fn a_filter_run_made_elsewhere_is_applied_by_plugin_run() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.handin";
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1)), "allowHeavy": true}));
    let p = json!({"id": id, "ids": []});
    let named = |s: &Session, ids: &Value| s.doc().unwrap().doc.node(NodeId(ids[0].as_u64().unwrap())).unwrap().name.clone();
    // What it computed, as the app hands it in; here marked, so the test sees which output applied.
    let r = FilterRun::prepare(&s, &p).unwrap();
    assert!(r.current(&s));
    let output = String::from_utf8(r.run().unwrap()).unwrap().replace("Plug-in", "Handed in");
    r.hand_in(&mut s, output.clone().into_bytes());
    let undo = s.doc().unwrap().history.undo.len();
    let made = run(&mut s, "plugin.run", p.clone())["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Handed in"));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    // Once only: the next run runs the plug-in.
    let made = run(&mut s, "plugin.run", p.clone())["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Plug-in"));
    // Made for objects that changed since, it isn't theirs.
    let obj = rect(&mut s, 10.0, "#ff0000");
    let on = json!({"id": id, "ids": [obj.0]});
    let stale = FilterRun::prepare(&s, &on).unwrap();
    run(&mut s, "paint.setFill", json!({"ids": [obj.0], "color": "#00ff00"}));
    assert!(!stale.current(&s), "the document changed");
    stale.hand_in(&mut s, output.clone().into_bytes());
    let made = run(&mut s, "plugin.run", on)["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Plug-in"), "made for objects that changed since");
    // A plug-in installed again is another one.
    let again = FilterRun::prepare(&s, &p).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1)), "allowHeavy": true}));
    assert!(!again.current(&s));
    again.hand_in(&mut s, output.into_bytes());
    let made = run(&mut s, "plugin.run", p)["ids"].clone();
    assert_eq!(named(&s, &made).as_deref(), Some("Plug-in"), "made by the plug-in installed before");
}

/// A heavy filter run away from the session reads what it keeps with the document
/// (`_context.documentData`) and hands its `documentData` back with its output: `plugin.run`
/// applies both as one undo step.
#[test]
fn a_filter_run_made_elsewhere_keeps_its_document_data() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.handindata";
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"filter","budget":"heavy"}}"#);
    let install = |s: &mut Session, src: String| {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        run(s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true}));
    };
    // It first keeps the pens it found with the document.
    install(&mut s, tpl::returning(&manifest, &json!({"objects": [], "documentData": {"pens": 2}}).to_string()));
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    // Given those pens, it adds one, in the background; the output handed in is marked, so the
    // test sees it is the one that applied.
    let next = json!({"objects": [{"path": triangle(), "name": "Pen 3"}], "documentData": {"pens": 3}}).to_string();
    install(&mut s, tpl::when_given(&manifest, tpl::Given::Params, r#""documentData":{"pens":2}"#, &next));
    let p = json!({"id": id, "ids": []});
    let r = FilterRun::prepare(&s, &p).unwrap();
    let output = String::from_utf8(r.run().unwrap()).unwrap().replace("Pen 3", "Handed in");
    r.hand_in(&mut s, output.into_bytes());
    let undo = s.doc().unwrap().history.undo.len();
    let made = run(&mut s, "plugin.run", p)["ids"].clone();
    let st = s.doc().unwrap();
    assert_eq!(st.doc.node(NodeId(made[0].as_u64().unwrap())).unwrap().name.as_deref(), Some("Handed in"));
    assert_eq!(st.doc.plugin_data(id), Some(&json!({"pens": 3})));
    assert_eq!(st.history.undo.len(), undo + 1, "one undo step");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(s.doc().unwrap().doc.plugin_data(id), Some(&json!({"pens": 2})), "undone with its objects");
}

/// A filter run the caller cancels (MCP's `notifications/cancelled`, the app's Cancel) stops between
/// two slices of its budget and changes nothing.
#[test]
fn a_cancelled_filter_run_changes_nothing() {
    let mut s = session();
    let id = "test.engine.cancelled";
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&heavy(id, 1 << 40)), "allowHeavy": true}));
    let before = s.doc().unwrap().doc.clone();
    let token = vectorcraft_doc::cancel::CancelToken::new();
    token.cancel();
    let r = vectorcraft_doc::cancel::scope(&token, None, || s.execute("plugin.run", &json!({"id": id})));
    assert!(matches!(r, Err(EngineError::Cancelled)), "{r:?}");
    assert_eq!(s.doc().unwrap().doc, before);
}
