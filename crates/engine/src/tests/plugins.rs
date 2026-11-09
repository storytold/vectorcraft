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

/// Installs `wasm`, in place of any module installed with its id (`replace: true`).
fn install(s: &mut Session, wasm: &[u8]) -> Value {
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "replace": true}))
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

/// `plugin.run` refuses an `ids` list with an entry that is not an object id, with an error naming
/// the key, the entry and its value, and changes nothing. With `ids` left out or null the filter
/// runs on the selection, as it does on the list of the selection.
#[test]
fn object_filter_ids_entries_must_be_ids_and_ids_left_out_take_the_selection() {
    // A session with the plug-in, a red and a blue rectangle, and the red one selected. The
    // document has no creation date, so documents made in different seconds compare equal.
    let fresh = || {
        let mut s = Session::new();
        run(&mut s, "file.new", json!({"width": 400, "height": 200, "created": null}));
        install(&mut s, DESATURATE);
        let a = rect(&mut s, 10.0, "#ff0000");
        let b = rect(&mut s, 40.0, "#0000ff");
        run(&mut s, "select.set", json!({"ids": [a.0]}));
        (s, a, b)
    };
    let (_, a, b) = fresh();
    for (bad, entry) in [(json!(["abc"]), r#"entry 0 is "abc""#), (json!([a.0, true]), "entry 1 is true"), (json!([{}]), "entry 0 is {}")] {
        let (mut s, ..) = fresh();
        let st = s.doc().unwrap();
        let before = (st.doc.clone(), st.selection.clone(), st.history.undo.len());
        let r = s.execute("plugin.run", &json!({"id": DESATURATE_ID, "amount": 100, "ids": bad}));
        let want = format!("`ids` {entry}, not a non-negative integer object id");
        assert!(matches!(&r, Err(EngineError::BadParams { cmd, msg }) if cmd == "plugin.run" && *msg == want), "{bad}: {r:?}");
        let st = s.doc().unwrap();
        assert!((st.doc.clone(), st.selection.clone(), st.history.undo.len()) == before, "{bad}: something changed");
    }
    let gray = |s: &Session, id: NodeId| {
        let [r, g, b] = node(s, id).appearance.fill_paint().color().unwrap().to_rgb_uncalibrated();
        (r - g).abs() < 1e-6 && (g - b).abs() < 1e-6
    };
    let mut docs = vec![];
    for ids in [None, Some(Value::Null), Some(json!([a.0]))] {
        let (mut s, ..) = fresh();
        let mut p = json!({"id": DESATURATE_ID, "amount": 100});
        if let Some(ids) = &ids {
            p["ids"] = ids.clone();
        }
        let r = run(&mut s, "plugin.run", p);
        assert_eq!(r["ids"], json!([a.0]), "{ids:?}");
        assert!(gray(&s, a) && !gray(&s, b), "{ids:?}: only the selected rectangle turns gray");
        docs.push(s.doc().unwrap().doc.clone());
    }
    assert!(docs.windows(2).all(|w| w[0] == w[1]), "left out, null and the selection's list give the same document");
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

/// A generator can make guides and lock what it adds, in its one undo step; those objects are left
/// out of the selection that follows, as Make Guides and Object › Lock leave them.
#[test]
fn filters_make_guides_and_locked_objects_left_unselected() {
    let mut s = session();
    let out = json!({"objects": [
        {"path": triangle(), "name": "Guide", "guide": true},
        {"path": triangle(), "name": "Locked", "locked": true},
        {"path": triangle(), "name": "Plain"}]});
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.guides", "filter", "{}"), &out.to_string()));
    let undo = s.doc().unwrap().history.undo.len();
    let r = run(&mut s, "plugin.run", json!({"id": "test.engine.guides"}));
    let ids: Vec<NodeId> = r["ids"].as_array().unwrap().iter().map(|v| NodeId(v.as_u64().unwrap())).collect();
    assert!(matches!(node(&s, ids[0]).kind, vectorcraft_doc::NodeKind::Path { guide: true, .. }));
    assert!(node(&s, ids[1]).locked);
    let st = s.doc().unwrap();
    assert_eq!(st.selection.objects, [ids[2]]);
    assert_eq!(st.history.undo.len(), undo + 1);
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

#[test]
fn invalid_install_flags_are_refused() {
    let mut s = session();
    let id = "test.engine.heavy.flags";
    let data = vectorcraft_format::base64_encode(&heavy(id, 1));
    for allow in [json!("true"), json!(1)] {
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": allow})).unwrap_err().to_string();
        assert!(e.contains("`allowHeavy` must be true or false"), "allowHeavy {allow}: {e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "installed with allowHeavy {allow}");
    }
    // Null, as for every command, is the same as leaving the flag out.
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": null})).unwrap_err().to_string();
    assert!(e.contains("heavy budget"), "{e}");
    run(&mut s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true}));
    for replace in [json!("false"), json!(0)] {
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": true, "replace": replace})).unwrap_err().to_string();
        assert!(e.contains("`replace` must be true or false"), "replace {replace}: {e}");
    }
    run(&mut s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true, "replace": null}));
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    // A plug-in with the usual budget is refused too.
    let light = "test.engine.light.flags";
    let manifest = format!(r#"{{"id":"{light}","name":"Test {light}","version":"1","kind":"filter"}}"#);
    let output = json!({"objects": [{"path": triangle(), "name": "Plug-in"}]});
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::spinning(&manifest, 1, &output.to_string())).unwrap());
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "allowHeavy": "false"})).unwrap_err().to_string();
    assert!(e.contains("`allowHeavy` must be true or false"), "{e}");
    assert!(vectorcraft_plugins::registry::get(light).is_none());
}

/// A test filter `id` (named "Test `id`") whose manifest lists `permissions` (a JSON list) and that
/// returns `output`.
fn asking(id: &str, permissions: &str, output: &Value) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","version":"2.1","kind":"filter","permissions":{permissions}}}"#);
    wat::parse_str(tpl::returning(&manifest, &output.to_string())).unwrap()
}

/// A plug-in whose manifest lists permissions installs only once they are all granted (`grant`),
/// which `plugin.inspect` tells beforehand (`needsApproval`) with the permissions and the module's
/// hash. The grant is recorded with that hash: replacing the plug-in (`replace: true`) with one
/// that lists the same permissions needs no new grant, and one that lists another needs one. Removing it forgets the
/// grant. A plug-in without `permissions` installs as before, with the ones its kind implies.
#[test]
fn a_plugin_that_lists_permissions_installs_once_they_are_granted() {
    let mut s = session();
    let id = "test.engine.permissions.install";
    let empty = json!({"objects": []});
    let data = |perms: &str| vectorcraft_format::base64_encode(&asking(id, perms, &empty));
    let reads = data(r#"["doc.read"]"#);
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": reads}));
    assert_eq!((&inspected["needsApproval"], &inspected["permissions"]), (&json!(true), &json!(["doc.read"])));
    let hash = vectorcraft_plugins::permissions::sha256_hex(&asking(id, r#"["doc.read"]"#, &empty));
    assert_eq!(inspected["hash"], hash);
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "inspecting installs nothing");
    // Refused without a grant that holds every permission it lists, naming them.
    for (grant, says) in [
        (Value::Null, "asks for the permissions doc.read"),
        (json!([]), "asks for the permissions doc.read"),
        (json!(["doc.write"]), "asks for the permissions doc.read"),
        (json!(["doc.read", "net.fetch"]), "unknown permission `net.fetch`"),
        (json!("doc.read"), "`grant` must be a list"),
        (json!([1]), "`grant` must be a list"),
    ] {
        let e = s.execute("plugin.install", &json!({"dataBase64": reads, "grant": grant})).unwrap_err().to_string();
        assert!(e.contains(says), "grant {grant}: {e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "installed with grant {grant}");
    }
    // `hash` pins the module the user was shown.
    for wrong in [json!("0".repeat(64)), json!(1)] {
        let e = s.execute("plugin.install", &json!({"dataBase64": reads, "grant": ["doc.read"], "hash": wrong})).unwrap_err().to_string();
        assert!(e.contains("SHA-256 `hash`"), "hash {wrong}: {e}");
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "installed with hash {wrong}");
    }
    let installed = run(&mut s, "plugin.install", json!({"dataBase64": reads, "grant": ["doc.read"], "hash": hash}));
    assert_eq!(installed["permissions"], json!(["doc.read"]));
    let g = vectorcraft_plugins::registry::granted(id).unwrap();
    assert_eq!((g.hash.as_str(), g.permissions.as_slice()), (hash.as_str(), &[vectorcraft_plugins::permissions::Permission::DocRead][..]));
    // Replaced with another module that lists the same permissions: no new grant needed, and the
    // grant now records the new module's hash.
    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&other)}))["needsApproval"], false);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&other), "replace": true}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, vectorcraft_plugins::permissions::sha256_hex(&other));
    // One that asks for more is refused until it is granted all of them.
    let more = data(r#"["doc.read","doc.write"]"#);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": more}))["needsApproval"], true);
    let e = s.execute("plugin.install", &json!({"dataBase64": more, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("doc.read, doc.write") && e.contains("`grant`"), "{e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().manifest().permissions().len(), 1, "the one installed stays");
    run(&mut s, "plugin.install", json!({"dataBase64": more, "grant": ["doc.write", "doc.read", "state"], "replace": true}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().permissions.len(), 2, "only what it lists is recorded");
    // Removed, it is asked again.
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": reads}))["needsApproval"], true);
    // A plug-in without `permissions` installs as before, with its kind's.
    let plain = install(&mut s, DESATURATE);
    assert_eq!(plain["permissions"], json!(["doc.read", "doc.write", "state"]));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(DESATURATE)}))["needsApproval"], false);
}

/// The host enforces what a filter was granted: without `doc.write` an output that changes the
/// document is refused and nothing changes, without `doc.read` it is given no objects, and without
/// `state` it is given no data kept with the document and may not write any.
#[test]
fn a_filter_does_only_what_its_permissions_allow() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let filter = |s: &mut Session, pid: &str| s.execute("plugin.run", &json!({"id": pid, "ids": [id.0]}));
    let grant = |s: &mut Session, wasm: &[u8], perms: Value| {
        run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "grant": perms, "replace": true}));
    };
    // Denied `doc.write`: renaming the object is refused, and the document and history stay.
    let reader = "test.engine.permissions.reader";
    grant(&mut s, &asking(reader, r#"["doc.read"]"#, &json!({"objects": [{"id": id.0, "name": "Changed"}]})), json!(["doc.read"]));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = filter(&mut s, reader).unwrap_err().to_string();
    assert!(e.contains("wasn't granted `doc.write`"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Deleting it (leaving it out of the output) and adding objects are changes too.
    for out in [json!({"objects": []}), json!({"objects": [{"id": id.0}, {"path": triangle()}]})] {
        grant(&mut s, &asking(reader, r#"["doc.read"]"#, &out), json!(["doc.read"]));
        assert!(filter(&mut s, reader).is_err(), "{out}");
        assert_eq!(s.doc().unwrap().doc, before, "{out}");
    }
    // Returning the object unchanged changes nothing, so it may.
    grant(&mut s, &asking(reader, r#"["doc.read"]"#, &json!({"objects": [{"id": id.0}]})), json!(["doc.read"]));
    filter(&mut s, reader).unwrap();
    assert_eq!(s.doc().unwrap().doc.layers, before.layers);
    // Granted `doc.write` alone: it is given no objects, so its triangle is added beside the
    // rectangle rather than replacing it.
    let writer = "test.engine.permissions.writer";
    let manifest = format!(r#"{{"id":"{writer}","name":"W","kind":"filter","permissions":["doc.write"]}}"#);
    let added = json!({"objects": [{"path": triangle(), "name": "Given nothing"}]});
    let src = tpl::when_given(&manifest, tpl::Given::Input, r#"{"objects":[]}"#, &added.to_string());
    grant(&mut s, &wat::parse_str(src).unwrap(), json!(["doc.write"]));
    let made = filter(&mut s, writer).unwrap();
    let new = NodeId(made["ids"][0].as_u64().unwrap());
    assert_eq!(node(&s, new).name.as_deref(), Some("Given nothing"));
    assert!(s.doc().unwrap().doc.node(id).is_some(), "the rectangle it wasn't given stays");
    // Without `state`: kept data is not handed over, and writing some is refused.
    run(
        &mut s,
        "plugin.install",
        json!({"dataBase64": vectorcraft_format::base64_encode(&asking("test.engine.permissions.keeper", "null", &json!({"objects": [{"id": id.0}], "documentData": {"pen": 1}})))}),
    );
    filter(&mut s, "test.engine.permissions.keeper").unwrap();
    let stateless = "test.engine.permissions.keeper";
    let manifest = format!(r#"{{"id":"{stateless}","name":"K","kind":"filter","permissions":["doc.read","doc.write"]}}"#);
    let src =
        tpl::when_given(&manifest, tpl::Given::Params, r#""documentData":null"#, &json!({"objects": [{"id": id.0, "name": "No data"}]}).to_string());
    grant(&mut s, &wat::parse_str(src).unwrap(), json!(["doc.read", "doc.write"]));
    filter(&mut s, stateless).unwrap();
    assert_eq!(node(&s, id).name.as_deref(), Some("No data"), "given no kept data");
    let before = s.doc().unwrap().doc.clone();
    grant(
        &mut s,
        &asking(stateless, r#"["doc.read","doc.write"]"#, &json!({"objects": [{"id": id.0}], "documentData": {"pen": 2}})),
        json!(["doc.read", "doc.write"]),
    );
    let e = filter(&mut s, stateless).unwrap_err().to_string();
    assert!(e.contains("wasn't granted `state`"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().doc.plugin_data(stateless), Some(&json!({"pen": 1})));
}

/// What the user granted is kept in the plug-in store with the module: the next session installs
/// the plug-in again without asking. A stored plug-in that lists permissions nobody granted it is
/// skipped, even when another store granted it, and so is one in the Additional Plug-ins Folder
/// until it is granted on its own.
#[test]
fn granted_permissions_are_kept_and_folders_skip_the_rest() {
    let folder = crate::tests_links::Folder::new("plugin-store-grants");
    let id = "test.engine.permissions.stored";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    assert!(folder.0.join("grants.json").is_file());
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again from the store, as granted");
    // Only a store's own grants count: the same module in another store without a grant is
    // skipped, and a grant given elsewhere is not written to that store.
    let other = crate::tests_links::Folder::new("plugin-store-grants-other");
    std::fs::write(other.0.join("copy.wasm"), &wasm).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut elsewhere = session();
    elsewhere.attach_plugin_store(other.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "granted in another store only");
    let e = elsewhere.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm)})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "a grant kept in another store doesn't count here: {e}");
    let lone = "test.engine.permissions.lone";
    let lone_wasm = asking(lone, r#"["doc.read"]"#, &json!({"objects": []}));
    run(&mut elsewhere, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&lone_wasm), "grant": ["doc.read"]}));
    let kept = std::fs::read_to_string(other.0.join("grants.json")).unwrap();
    assert!(kept.contains(lone) && !kept.contains(id), "{kept}");
    run(&mut elsewhere, "plugin.remove", json!({"id": lone}));
    assert!(!other.0.join("grants.json").exists(), "the last grant removed takes the file with it");
    // Without the grant it is skipped, and the file stays.
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    std::fs::remove_file(folder.0.join("grants.json")).unwrap();
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "nobody granted it");
    assert_eq!(std::fs::read_dir(&folder.0).unwrap().count(), 1, "the module stays in the store");

    // The Additional Plug-ins Folder skips it with a note until it is granted.
    let dir = crate::tests_links::Folder::new("plugin-folder-grants");
    std::fs::write(dir.0.join("asks.wasm"), &wasm).unwrap();
    let r = run(&mut third, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([]));
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval") && why.contains("doc.read"), "{why}");
    run(&mut third, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    let r = run(&mut third, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([id]));
    run(&mut third, "plugin.remove", json!({"id": id}));
    // With a store attached, only that store's grants load it from the folder: a grant given in a
    // session without a store doesn't count.
    let mut storeless = session();
    run(&mut storeless, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    vectorcraft_plugins::registry::remove(id);
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "granted in the process");
    let empty = crate::tests_links::Folder::new("plugin-store-grants-empty");
    let mut scoped = session();
    scoped.attach_plugin_store(empty.0.clone());
    let r = run(&mut scoped, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    vectorcraft_plugins::registry::revoke(id);
    assert_eq!(r["loaded"], json!([]), "a grant outside this store doesn't load it");
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval"), "{why}");
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "not installed");
    // The preference's folder is loaded again for a session whose store holds other grants.
    let pref_dir = crate::tests_links::Folder::new("plugin-folder-pref-grants");
    std::fs::write(pref_dir.0.join("asks.wasm"), &wasm).unwrap();
    let pref = json!({"key": "pluginsFolder", "value": pref_dir.0.display().to_string()});
    run(&mut scoped, "prefs.set", pref.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "nothing granted in this store");
    let granting = crate::tests_links::Folder::new("plugin-store-grants-pref");
    let mut granted = session();
    granted.attach_plugin_store(granting.0.clone());
    run(&mut granted, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    run(&mut granted, "prefs.set", pref);
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "loaded under this store's grant");
    run(&mut granted, "plugin.remove", json!({"id": id}));
}

/// A grant covers the module it was given to: another module under the id, listing the same
/// permissions, isn't installed from the store at the next start, a folder load reports it as
/// needing approval, and `plugin.install` asks for `grant` again, even with `replace: true`. It
/// doesn't get the settings kept for the id until the user grants it. A kept grant without a hash
/// covers no module.
#[test]
fn a_grant_covers_only_the_module_it_was_given_to() {
    let folder = crate::tests_links::Folder::new("plugin-store-grant-hash");
    let id = "test.engine.granthash";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    // The stored module is replaced on disk by one that reads the settings.
    let other = storing(id, READS);
    let stored: Vec<_> =
        std::fs::read_dir(&folder.0).unwrap().map(|e| e.unwrap().path()).filter(|p| p.extension().is_some_and(|x| x == "wasm")).collect();
    assert_eq!(stored.len(), 1, "{stored:?}");
    std::fs::write(&stored[0], &other).unwrap();
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "another module isn't loaded under the grant");
    let dir = crate::tests_links::Folder::new("plugin-folder-grant-hash");
    std::fs::write(dir.0.join("other.wasm"), &other).unwrap();
    let r = run(&mut next, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([]));
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval"), "{why}");
    let data = vectorcraft_format::base64_encode(&other);
    let e = next.execute("plugin.install", &json!({"dataBase64": data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "{e}");
    // Granted, it reads the settings the first module set.
    run(&mut next, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}));
    run(&mut next, "plugin.run", json!({"id": id, "ids": []}));

    // A kept grant that records no hash, or an empty one, covers no module: the stored module isn't
    // installed at the next start and `plugin.inspect` asks for approval.
    for entry in [json!({"id": id, "permissions": ["settings", "cache"]}), json!({"id": id, "hash": "", "permissions": ["settings", "cache"]})] {
        std::fs::write(folder.0.join("grants.json"), serde_json::to_vec(&json!([entry])).unwrap()).unwrap();
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
        let mut again = session();
        again.attach_plugin_store(folder.0.clone());
        assert!(vectorcraft_plugins::registry::get(id).is_none(), "a grant without a hash doesn't load the module: {entry}");
        assert_eq!(run(&mut again, "plugin.inspect", json!({"dataBase64": data}))["needsApproval"], json!(true), "{entry}");
        next = again;
    }
    run(&mut next, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}));
    run(&mut next, "plugin.remove", json!({"id": id}));
}

/// A grants file entry this version can't use (a permission it doesn't know, an invalid id) is
/// skipped without dropping the other grants, and recording a grant writes it back as it was,
/// except an entry for the plug-in being granted or removed. A grants file that doesn't parse as a
/// list is never overwritten.
#[test]
fn grants_this_version_cant_read_are_kept() {
    let folder = crate::tests_links::Folder::new("plugin-store-grants-unknown");
    let kept = "test.engine.permissions.kept";
    let added = "test.engine.permissions.added";
    let later = "test.engine.permissions.later";
    let empty = json!({"objects": []});
    let kept_wasm = asking(kept, r#"["doc.read"]"#, &empty);
    let added_wasm = asking(added, r#"["doc.read"]"#, &empty);
    let later_wasm = asking(later, r#"["doc.read"]"#, &empty);
    let future = json!({"id": later, "hash": "00", "permissions": ["doc.read", "ui.panel"]});
    let invalid = json!({"id": "bad id/", "hash": "00", "permissions": ["doc.read"]});
    let grant = json!({"id": kept, "hash": vectorcraft_plugins::permissions::sha256_hex(&kept_wasm), "permissions": ["doc.read"]});
    let grants = folder.0.join("grants.json");
    std::fs::write(&grants, serde_json::to_vec(&json!([future, grant, invalid])).unwrap()).unwrap();
    std::fs::write(folder.0.join("kept.wasm"), &kept_wasm).unwrap();
    let entries = || serde_json::from_slice::<Vec<Value>>(&std::fs::read(&grants).unwrap()).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(kept).is_some(), "the entries this version can't read don't drop the grant next to them");
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
    let now = entries();
    assert_eq!(now.len(), 4, "{now:?}");
    assert!(now.contains(&future) && now.contains(&invalid), "written back as they were: {now:?}");
    assert!(now.iter().any(|e| e["id"] == added), "{now:?}");
    run(&mut s, "plugin.remove", json!({"id": added}));
    assert_eq!(entries().len(), 3);
    // Granting the plug-in an entry names replaces that entry.
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&later_wasm), "grant": ["doc.read"]}));
    let now = entries();
    assert!(!now.contains(&future), "{now:?}");
    assert_eq!(now.iter().filter(|e| e["id"] == later).count(), 1, "{now:?}");
    run(&mut s, "plugin.remove", json!({"id": later}));
    run(&mut s, "plugin.remove", json!({"id": kept}));
    assert_eq!(entries(), vec![invalid.clone()], "the last usable grant removed leaves the entry it can't read");

    // A file that doesn't hold a list is left as it is: installing still works in this session.
    for broken in [&b"[{\"id\": "[..], &b"{\"id\": \"x\"}"[..]] {
        std::fs::write(&grants, broken).unwrap();
        run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
        assert!(vectorcraft_plugins::registry::get(added).is_some());
        assert_eq!(std::fs::read(&grants).unwrap(), broken, "not overwritten");
        run(&mut s, "plugin.remove", json!({"id": added}));
        assert_eq!(std::fs::read(&grants).unwrap(), broken, "not overwritten by a removal either");
    }

    // The file stays within the 1 MiB it is read up to: compact when pretty-printing would pass it,
    // and left as it is when even compact would.
    let cap = 1 << 20;
    let many: Vec<Value> = (0..14_000).map(|i| json!({"id": format!("x.{i}"), "hash": "00", "permissions": ["ui.panel"]})).collect();
    let compact = serde_json::to_vec(&many).unwrap();
    assert!(compact.len() < cap && serde_json::to_vec_pretty(&many).unwrap().len() > cap);
    std::fs::write(&grants, &compact).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
    let written = std::fs::read(&grants).unwrap();
    assert!(written.len() <= cap, "{}", written.len());
    assert_eq!(entries().len(), many.len() + 1);
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm)}));
    assert_eq!(inspected["needsApproval"], json!(false), "the compact file is read back");
    run(&mut s, "plugin.remove", json!({"id": added}));
    let full = serde_json::to_vec(&json!([{"id": "x.full", "hash": "0".repeat(cap - 100), "permissions": ["ui.panel"]}])).unwrap();
    std::fs::write(&grants, &full).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&added_wasm), "grant": ["doc.read"]}));
    assert_eq!(std::fs::read(&grants).unwrap(), full, "not written past the cap");
    run(&mut s, "plugin.remove", json!({"id": added}));
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
        run(s, "plugin.install", json!({"dataBase64": data, "allowHeavy": true, "replace": true}));
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

/// A plug-in of API level 2 that uses the host imports lists `progress`, which the user grants like
/// any permission; a filter run made elsewhere then reports its progress, as a share, to the
/// progress the caller shows, and sees the caller's Cancel.
#[test]
fn a_level_2_filter_installs_with_progress_granted_and_reports_to_the_caller() {
    use crate::cmd::fileio::progress::{self, Progress};
    use crate::cmd::plugin::FilterRun;
    let mut s = session();
    let id = "test.engine.level2";
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read","doc.write","progress"]}}"#);
    let wasm = wat::parse_str(tpl::cooperating(&manifest, 1000, r#"{"objects":[]}"#, r#"{"objects":[]}"#)).unwrap();
    let data = vectorcraft_format::base64_encode(&wasm);
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": data}));
    assert_eq!((&inspected["needsApproval"], &inspected["permissions"]), (&json!(true), &json!(["doc.read", "doc.write", "progress"])));
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read", "doc.write"]})).unwrap_err().to_string();
    assert!(e.contains("asks for the permissions doc.read, doc.write, progress"), "{e}");
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write", "progress"]}));
    let shown = Progress::new();
    let r = FilterRun::prepare(&s, &json!({"id": id})).unwrap();
    assert!(progress::scope(&shown, || r.run()).is_ok());
    assert_eq!((shown.counts(), shown.counts_share()), ((999, 1000), true));
    // Cancelled, it stops on its first step, and what it returns then is discarded.
    shown.cancel();
    let cancelled = progress::scope(&shown, || r.run());
    assert!(matches!(cancelled, Err(EngineError::Cancelled)), "{cancelled:?}");
    assert_eq!(shown.counts(), (0, 1000));
    run(&mut s, "plugin.remove", json!({"id": id}));
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

/// The manifest of a filter `id` named "Cmd Test" of `kind` with `commands` (a JSON list).
fn with_commands(id: &str, kind: &str, commands: &str) -> String {
    format!(r#"{{"id":"{id}","name":"Cmd Test","version":"1","kind":"{kind}","commands":{commands}}}"#)
}

/// Two commands: Grow, with a parameter, and Tidy, whose `menu` names another menu.
const GROW_AND_TIDY: &str = r#"[{"id":"grow","label":"Grow","params":{"by":{"type":"number","min":0,"max":10,"default":2}}},
    {"id":"tidy","label":"Tidy","menu":"File"}]"#;

/// A plug-in that adds a triangle when it runs as its command `grow` (`_context.command`) and
/// returns its input unchanged otherwise.
fn grower(id: &str) -> String {
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    tpl::when_given(&with_commands(id, "filter", GROW_AND_TIDY), tpl::Given::Params, r#""command":"grow""#, triangle)
}

/// The command listing's entry for `id`.
fn listed(s: &Session, id: &str) -> Option<crate::cmd::CommandInfo> {
    s.commands().into_iter().find(|c| c.id == id)
}

fn objects(s: &Session) -> usize {
    s.doc().unwrap().doc.layers.iter().map(|l| l.children().map_or(0, |c| c.len())).sum()
}

/// A plug-in's commands are commands of their own while it is installed: listed under Object ›
/// Plug-ins › its name whatever menu the manifest names, run with their checked parameters and
/// `_context.command` as one undo step named after them, and gone once it is removed or replaced
/// by a module without them.
#[test]
fn plugin_commands_are_registered_run_and_removed() {
    let mut s = session();
    let id = "test.engine.cmds";
    let (grow, tidy) = ("plugin.test.engine.cmds.grow", "plugin.test.engine.cmds.tidy");
    let info = install_wat(&mut s, &grower(id));
    assert_eq!(info["commands"][0]["command"], grow);
    assert_eq!(info["commands"][1]["label"], "Tidy");
    for (cmd, label) in [(grow, "Grow"), (tidy, "Tidy")] {
        let c = listed(&s, cmd).unwrap_or_else(|| panic!("{cmd} is listed"));
        assert_eq!((c.label, c.menu.as_slice()), (label, &["Object", "Plug-ins", "Cmd Test"][..]), "{cmd}");
        assert!(c.enabled, "{cmd}");
    }
    assert!(crate::cmd::find_command(grow).is_some());

    run(&mut s, "select.none", json!({}));
    let (before, undo) = (objects(&s), s.doc().unwrap().history.undo.len());
    let r = run(&mut s, grow, json!({"by": 3}));
    assert_eq!((r["plugin"].as_str(), r["command"].as_str()), (Some(id), Some("grow")));
    assert_eq!(objects(&s), before + 1, "the command added its triangle");
    let st = s.doc().unwrap();
    assert_eq!((st.history.undo.len(), st.history.undo.last().unwrap().label.as_str()), (undo + 1, "Grow"));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(objects(&s), before);
    // Its parameters are checked before it runs.
    let e = s.execute(grow, &json!({"by": "lots"})).unwrap_err();
    assert!(matches!(&e, EngineError::BadParams { cmd, .. } if cmd == grow), "{e}");
    assert_eq!(objects(&s), before);
    // Another command of the same plug-in is told apart by its id.
    run(&mut s, tidy, json!({}));
    assert_eq!(objects(&s), before, "tidy returns its input unchanged");

    // Replaced by a module without commands, its commands are gone; installed again, back.
    install_wat(&mut s, &tpl::module(&tpl::manifest(id, "filter", "{}"), tpl::ECHO, "", 1));
    assert!(listed(&s, grow).is_none() && crate::cmd::find_command(grow).is_none());
    install_wat(&mut s, &grower(id));
    assert!(listed(&s, grow).is_some());
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(listed(&s, grow).is_none() && listed(&s, tidy).is_none());
    assert!(s.execute(grow, &json!({})).is_err(), "removed, it no longer runs");
}

/// A command run is journaled like any other, so an action records it with its parameters and
/// plays it back.
#[test]
fn actions_record_and_play_plugin_commands() {
    let mut s = session();
    let id = "test.engine.cmds.actions";
    let grow = crate::cmd::plugin::command_id(id, "grow");
    install_wat(&mut s, &grower(id));
    run(&mut s, "select.none", json!({}));
    let before = objects(&s);
    let start = s.journal.len();
    run(&mut s, &grow, json!({"by": 4}));
    let steps = s.journal_for_action(start);
    assert_eq!(steps, [(grow.clone(), json!({"by": 4}))]);
    run(&mut s, "edit.undo", json!({}));
    let commands: Vec<Value> = steps.iter().map(|(c, p)| json!({"command": c, "params": p})).collect();
    // The triangle a run adds is selected, and the next run would be given it: deselect between.
    let none = json!({"command": "select.none", "params": {}});
    run(&mut s, "command.batch", json!({"label": "Grow Twice", "commands": [commands[0].clone(), none, commands[0].clone()]}));
    assert_eq!(objects(&s), before + 2, "both steps ran");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A command that traps leaves the document and its history as they were, and the error names
/// the plug-in and the command.
#[test]
fn a_failing_plugin_command_changes_nothing_and_names_itself() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let id = "test.engine.cmds.trap";
    install_wat(&mut s, &tpl::module(&with_commands(id, "filter", r#"[{"id":"boom","label":"Boom"}]"#), "unreachable", "", 1));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute(&crate::cmd::plugin::command_id(id, "boom"), &json!({})).unwrap_err().to_string();
    assert!(e.contains("“Cmd Test” › Boom: ") && e.contains("trapped"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A command that runs out of its instruction budget leaves the document and its history as they
/// were, and the error names the plug-in and the command.
#[test]
fn a_plugin_command_out_of_budget_changes_nothing_and_names_itself() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let id = "test.engine.cmds.spin";
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    // More steps than a light budget pays for (as the plug-in crate's budget test counts them).
    install_wat(&mut s, &tpl::spinning(&with_commands(id, "filter", r#"[{"id":"spin","label":"Spin"}]"#), 100_000_000, triangle));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute(&crate::cmd::plugin::command_id(id, "spin"), &json!({})).unwrap_err().to_string();
    assert!(e.contains("“Cmd Test” › Spin: ") && e.contains("instruction budget"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// An `ids` that is not a list, or that holds an entry that is not an object id, is an error naming
/// `ids` for a plug-in command and for `plugin.run`, and is never read as the selection; left out
/// or null, `ids` is the selection.
#[test]
fn a_malformed_ids_is_refused_not_read_as_the_selection() {
    let mut s = session();
    let id = "test.engine.cmds.ids";
    let grow = crate::cmd::plugin::command_id(id, "grow");
    install_wat(&mut s, &grower(id));
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "select.all", json!({}));
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let cases = [
        (grow.as_str(), json!(5)),
        (grow.as_str(), json!([1, "x"])),
        (grow.as_str(), json!([-1])),
        ("plugin.run", json!("all")),
        ("plugin.run", json!([1.5])),
    ];
    for (cmd, ids) in cases {
        let mut p = json!({"ids": ids});
        if cmd == "plugin.run" {
            p["id"] = json!(id);
        }
        let e = s.execute(cmd, &p).unwrap_err();
        assert!(matches!(&e, EngineError::BadParams { cmd: c, .. } if c == cmd), "{cmd} {ids}: {e}");
        let said = e.to_string();
        assert!(
            said.contains("ids must be an array of non-negative integer object ids") || said.contains("not a non-negative integer object id"),
            "{cmd} {ids}: {e}"
        );
        assert_eq!(s.doc().unwrap().doc, before, "{cmd} {ids}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo, "{cmd} {ids}");
    }
    // Left out or null, the command runs on the selection.
    for p in [json!({}), json!({"ids": null})] {
        let r = run(&mut s, &grow, p.clone());
        assert_eq!(r["command"], "grow", "{p}");
        assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1, "{p}");
        run(&mut s, "edit.undo", json!({}));
        assert_eq!(s.doc().unwrap().doc, before, "{p}");
    }
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A live effect's commands are listed under Effect › Plug-ins and run under its permissions (with
/// commands it gets `doc.write` unless its list leaves it out), and labels show bidirectional
/// characters escaped. A malformed
/// `commands` entry is refused at install, and the error names it.
#[test]
fn effect_commands_labels_and_malformed_entries() {
    let mut s = session();
    let id = "test.engine.cmds.effect";
    let triangle = r#"{"objects":[{"path":{"subpaths":[{"closed":true,"anchors":[{"p":[0,0]},{"p":[5,0]},{"p":[5,5]}]}]}}]}"#;
    install_wat(&mut s, &tpl::returning(&with_commands(id, "effect", "[{\"id\":\"flip\",\"label\":\"Flip\u{202E}txt\"}]"), triangle));
    let c = listed(&s, &crate::cmd::plugin::command_id(id, "flip")).expect("listed");
    assert_eq!((c.label, c.menu.as_slice()), ("Flip\\u{202E}txt", &["Effect", "Plug-ins", "Cmd Test"][..]));
    run(&mut s, "select.none", json!({}));
    // Without `permissions`, a live effect with commands may change the document: its command adds
    // the triangle, as one undo step.
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let out = run(&mut s, &crate::cmd::plugin::command_id(id, "flip"), json!({}));
    assert_eq!(out["ids"].as_array().map(Vec::len), Some(1), "{out}");
    assert_ne!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    run(&mut s, "plugin.remove", json!({"id": id}));
    // One that lists its permissions without `doc.write` is refused the change, and the document
    // stays as it was.
    let listed = format!(
        r#"{{"id":"{id}","name":"Cmd Test","version":"1","kind":"effect","permissions":["doc.read","effect.render"],"commands":[{{"id":"flip","label":"Flip"}}]}}"#
    );
    let wasm = wat::parse_str(tpl::returning(&listed, triangle)).unwrap();
    let hash = vectorcraft_plugins::permissions::sha256_hex(&wasm);
    let grant = json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read", "effect.render"], "hash": hash});
    run(&mut s, "plugin.install", grant);
    let (before, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    let e = s.execute(&crate::cmd::plugin::command_id(id, "flip"), &json!({})).unwrap_err().to_string();
    assert!(e.contains("wasn't granted `doc.write`"), "{e}");
    assert_eq!(s.doc().unwrap().doc, before);
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    run(&mut s, "plugin.remove", json!({"id": id}));

    let bad = id.replace("effect", "bad");
    let wasm = wat::parse_str(tpl::module(&with_commands(&bad, "filter", r#"[{"id":"ok","label":"A"},{"id":"Bad","label":"B"}]"#), tpl::ECHO, "", 1));
    let e = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&wasm.unwrap())})).unwrap_err().to_string();
    assert!(e.contains("command `Bad`"), "{e}");
    assert!(vectorcraft_plugins::registry::get(&bad).is_none());
}

/// A filter `id` of API level 2 that lists `settings` and `cache`, whose `vc_run` runs `first`
/// (instructions that trap unless each storage call returns what it should), with the keys "zoom"
/// at 0x2000 and "tile" at 0x2004 and the value "hello" at 0x2100.
fn storing(id: &str, first: &str) -> Vec<u8> {
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["settings","cache"]}}"#);
    let data = format!(r#"{} (data (i32.const 0x2000) "zoomtile") (data (i32.const 0x2100) "hello")"#, tpl::STORAGE_IMPORTS);
    wat::parse_str(tpl::with_imports(&manifest, &data, first, r#"{"objects":[]}"#)).unwrap()
}

/// Sets "zoom" to "hello" and writes "hel" to the cache entry "tile".
const WRITES: &str = "(if (call $vc_settings_set (i32.const 0x2000) (i32.const 4) (i32.const 0x2100) (i32.const 5)) (then unreachable))
    (if (call $vc_cache_write (i32.const 0x2004) (i32.const 4) (i32.const 0x2100) (i32.const 3)) (then unreachable))";

/// Traps unless "zoom" is "hello".
const READS: &str =
    "(if (i32.ne (call $vc_settings_get (i32.const 0x2000) (i32.const 4) (i32.const 0x3000) (i32.const 16)) (i32.const 5)) (then unreachable))
    (if (i32.ne (i32.load (i32.const 0x3000)) (i32.load (i32.const 0x2100))) (then unreachable))";

fn install_storing(s: &mut Session, id: &str, first: &str) -> Value {
    let data = vectorcraft_format::base64_encode(&storing(id, first));
    run(s, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"]}))
}

/// [`install_storing`] in place of another module with the same id (`replace: true`).
fn replace_storing(s: &mut Session, id: &str, first: &str) -> Value {
    let data = vectorcraft_format::base64_encode(&storing(id, first));
    run(s, "plugin.install", json!({"dataBase64": data, "grant": ["settings", "cache"], "replace": true}))
}

/// A plug-in's settings and cache are kept in the plug-in store under its id: the next session
/// has them, replacing the plug-in keeps them, another plug-in can't read them, `plugin.clearCache`
/// with its id empties its cache alone and `plugin.remove` deletes both. The caps and the bytes
/// each call is charged follow ([`plugin_storage_is_capped_and_survives_a_damaged_settings_file`],
/// [`plugin_storage_calls_are_charged_the_files_they_read_and_write`]) in the same test.
/// `plugin.clearCache` without an id empties the cache of every plug-in installed in the process,
/// so it is tested in a binary of its own (`tests/plugin_clear_cache.rs`).
#[test]
fn plugins_keep_settings_and_a_cache_of_their_own_in_the_store() {
    let folder = crate::tests_links::Folder::new("plugin-storage");
    let (id, other) = ("test.engine.storing", "test.engine.storing.other");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    let dir = crate::store::plugin_data_dir(&folder.0, id);
    assert!(dir.join("settings.json").is_file());
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));

    // A restart: installed again from the store, and replaced by a module that reads.
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    let listed = run(&mut next, "plugin.list", json!({}));
    let entry = listed["plugins"].as_array().unwrap().iter().find(|p| p["id"] == id).cloned().unwrap();
    assert_eq!(entry["cacheBytes"], json!(3));
    assert_eq!(replace_storing(&mut next, id, READS)["cacheBytes"], json!(3));
    run(&mut next, "plugin.run", json!({"id": id, "ids": []}));

    // Another plug-in has settings of its own: "zoom" isn't set there.
    install_storing(&mut next, other, READS);
    assert!(next.execute("plugin.run", &json!({"id": other, "ids": []})).is_err());

    // Clearing the cache leaves the settings, and the other plug-in's cache.
    replace_storing(&mut next, other, WRITES);
    run(&mut next, "plugin.run", json!({"id": other, "ids": []}));
    assert_eq!(run(&mut next, "plugin.clearCache", json!({"id": id})), json!({"cleared": [id], "bytesFreed": 3}));
    assert_eq!(run(&mut next, "plugin.info", json!({"id": id}))["cacheBytes"], json!(0));
    assert_eq!(run(&mut next, "plugin.info", json!({"id": other}))["cacheBytes"], json!(3));
    run(&mut next, "plugin.run", json!({"id": id, "ids": []}));
    assert!(next.execute("plugin.clearCache", &json!({"id": "test.engine.storing.missing"})).is_err());

    run(&mut next, "plugin.remove", json!({"id": id}));
    assert!(!dir.exists(), "removed with the plug-in");
    run(&mut next, "plugin.remove", json!({"id": other}));
    plugin_storage_is_capped_and_survives_a_damaged_settings_file();
    plugin_storage_calls_are_charged_the_files_they_read_and_write();
}

/// A `storage` folder in the plug-in store that is a link leads out of the store: the plug-in
/// installs without `replace` although a folder for its id is there, its storage calls fail as
/// unavailable, and removing the plug-in deletes nothing the link leads to.
#[test]
fn plugin_storage_behind_a_link_is_left_alone() {
    let folder = crate::tests_links::Folder::new("plugin-storage-link");
    let outside = crate::tests_links::Folder::new("plugin-storage-outside");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&outside.0, folder.0.join("storage")).is_ok();
    // Without Developer Mode or elevation Windows makes no symbolic link, but a junction.
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&outside.0, folder.0.join("storage")).is_ok()
        || std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(folder.0.join("storage"))
            .arg(&outside.0)
            .output()
            .is_ok_and(|o| o.status.success());
    #[cfg(not(any(unix, windows)))]
    let made = false;
    if !made {
        eprintln!("skipped: symbolic links can't be made here");
        return;
    }
    let id = "test.engine.storagelink";
    let kept = crate::store::plugin_data_dir(&folder.0, id).join("settings.json");
    std::fs::create_dir_all(kept.parent().unwrap()).unwrap();
    std::fs::write(&kept, b"{}").unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, "");
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let m = &mut vectorcraft_plugins::storage::Meter::unlimited();
    assert_eq!(st.set_setting("zoom", b"hello", m), Err(vectorcraft_plugins::storage::StorageError::Unavailable));
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(kept.is_file(), "left where the link leads");
}

/// `plugin.remove` with the id of a plug-in that isn't installed deletes the settings and cache the
/// plug-in store still keeps for it; with neither, it fails as before.
#[test]
fn removing_an_id_that_is_not_installed_deletes_its_storage() {
    let folder = crate::tests_links::Folder::new("plugin-storage-orphan");
    let id = "test.engine.storageorphan";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let dir = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::write(dir.join("settings.json"), b"{}").unwrap();
    std::fs::write(dir.join("cache").join("tile.bin"), b"abc").unwrap();
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert_eq!(run(&mut s, "plugin.remove", json!({"id": id})), json!({"removed": id}));
    assert!(!dir.exists(), "the storage of an id that isn't installed is deleted");
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err(), "nothing left to remove");
}

/// The settings file and the cache are checked: a file that doesn't read is moved aside and the
/// settings start empty; a write past a cap fails and changes nothing; cache entries whose names
/// differ only in case are two files; a removed plug-in's storage takes no more writes.
fn plugin_storage_is_capped_and_survives_a_damaged_settings_file() {
    use vectorcraft_plugins::storage::{self, Meter, StorageError};
    let folder = crate::tests_links::Folder::new("plugin-storage-caps");
    let id = "test.engine.storagecaps";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, "");
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let dir = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(&dir).unwrap();
    let m = &mut Meter::unlimited();

    // A truncated settings file.
    std::fs::write(dir.join("settings.json"), br#"{"zoom": "aGVsbG"#).unwrap();
    assert_eq!(st.setting("zoom", m).unwrap(), None);
    assert!(dir.join("settings.unreadable.json").is_file() && !dir.join("settings.json").exists());
    st.set_setting("zoom", b"hello", m).unwrap();
    assert_eq!(st.setting("zoom", m).unwrap(), Some(b"hello".to_vec()));
    // Damaged again, it replaces the file moved aside before.
    std::fs::write(dir.join("settings.json"), b"[").unwrap();
    assert_eq!(st.setting("zoom", m).unwrap(), None);
    assert_eq!(std::fs::read(dir.join("settings.unreadable.json")).unwrap(), b"[");
    st.set_setting("zoom", b"hello", m).unwrap();
    plugin_storage_writes_are_atomic(&*st, &dir, &folder.0);

    // At most 256 keys: a new one is refused, an existing one still changes.
    let keys: std::collections::BTreeMap<String, String> = (0..storage::MAX_SETTINGS).map(|i| (format!("k{i}"), "eA==".to_string())).collect();
    std::fs::write(dir.join("settings.json"), serde_json::to_vec(&keys).unwrap()).unwrap();
    assert_eq!(st.set_setting("one-more", b"x", m), Err(StorageError::Full));
    st.set_setting("k0", b"y", m).unwrap();
    assert_eq!(st.setting("k0", m).unwrap(), Some(b"y".to_vec()));
    plugin_storage_skips_oversize_files(&*st, &dir);

    // At most 1 MiB in all: eleven values of 64 KiB fit, a twelfth doesn't, and nothing changes.
    let big = vec![7_u8; storage::MAX_SETTING_BYTES];
    let keys: std::collections::BTreeMap<String, String> = (0..11).map(|i| (format!("k{i}"), vectorcraft_format::base64_encode(&big))).collect();
    std::fs::write(dir.join("settings.json"), serde_json::to_vec(&keys).unwrap()).unwrap();
    assert_eq!(st.set_setting("k11", &big, m), Err(StorageError::Full));
    assert_eq!((st.setting("k11", m).unwrap(), st.setting("k0", m).unwrap()), (None, Some(big.clone())));

    // The cache: entry size, then the total, here nearly reached by a file already there.
    assert_eq!(st.set_cache_entry("big", &vec![0; storage::MAX_CACHE_ENTRY + 1], m), Err(StorageError::Full));
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    std::fs::File::create(dir.join("cache").join("filler.bin")).unwrap().set_len(storage::MAX_CACHE_BYTES - 2).unwrap();
    // A temporary file a failed write left behind counts toward no cap, and clearing removes it.
    let leftover = dir.join("cache").join(".tile.bin.1-0.tmp");
    std::fs::write(&leftover, b"tmp").unwrap();
    assert_eq!(st.set_cache_entry("tile", b"abc", m), Err(StorageError::Full));
    assert_eq!(st.cache_entry("tile", m).unwrap(), None);
    st.set_cache_entry("tile", b"ab", m).unwrap();
    assert_eq!(st.cache_bytes(), storage::MAX_CACHE_BYTES);
    assert_eq!(st.clear_cache().unwrap(), storage::MAX_CACHE_BYTES + 3);
    assert!(!leftover.exists());

    // Names differing in case are two entries.
    st.set_cache_entry("Tile", b"1", m).unwrap();
    st.set_cache_entry("tile", b"22", m).unwrap();
    assert_eq!((st.cache_entry("Tile", m).unwrap(), st.cache_entry("tile", m).unwrap()), (Some(b"1".to_vec()), Some(b"22".to_vec())));
    assert_eq!(st.cache_bytes(), 3);

    // More items in the cache folder than a write looks at: the write is refused whatever the
    // items it looked at hold, and clearing still reaches every entry.
    let cache = dir.join("cache");
    for i in 0..crate::store::MAX_CACHE_SCAN {
        std::fs::create_dir(cache.join(format!("d{i:05}"))).unwrap();
    }
    std::fs::File::create(cache.join("z.bin")).unwrap().set_len(storage::MAX_CACHE_BYTES).unwrap();
    assert_eq!(st.set_cache_entry("more", b"1", m), Err(StorageError::Full));
    assert_eq!(st.clear_cache().unwrap(), storage::MAX_CACHE_BYTES + 3);
    assert!(!cache.join("z.bin").exists());

    // An id that differs only in case has storage of its own, which removing this one leaves.
    let upper_id = "test.engine.StorageCaps";
    s.attach_plugin_storage(upper_id);
    let upper = vectorcraft_plugins::registry::storage(upper_id).unwrap();
    assert_eq!(upper.setting("k0", m).unwrap(), None);
    upper.set_setting("k0", b"upper", m).unwrap();
    assert_eq!(st.setting("k0", m).unwrap(), Some(big));
    // So do ids whose `_` and `.` could be read as each other's escapes.
    let folders: std::collections::BTreeSet<_> =
        ["a.b", "a_0b", "a_b", "A.b", "a__b", "a._b", "a_.b"].iter().map(|id| crate::store::plugin_data_dir(&folder.0, id)).collect();
    assert_eq!(folders.len(), 7, "{folders:?}");

    // Removing the plug-in closes the storage a run still holds, even one from before the plug-in
    // was replaced, so nothing is written back.
    s.attach_plugin_storage(id);
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert_eq!(st.set_setting("zoom", b"again", m), Err(StorageError::Unavailable));
    assert_eq!(st.set_cache_entry("tile", b"again", m), Err(StorageError::Unavailable));
    assert!(!dir.exists());
    assert_eq!(upper.setting("k0", m).unwrap(), Some(b"upper".to_vec()));
    s.remove_plugin_storage(upper_id).unwrap();
}

/// Files larger than the storage reads are not read: a settings file larger than 1 MiB, although
/// each of its values would fit, is moved aside and the settings start empty; a cache entry file
/// larger than an entry may be fails as unavailable. `st` is the storage in `dir`.
fn plugin_storage_skips_oversize_files(st: &dyn vectorcraft_plugins::storage::Storage, dir: &std::path::Path) {
    use vectorcraft_plugins::storage::{self, Meter, StorageError};
    let m = &mut Meter::unlimited();
    let value = vectorcraft_format::base64_encode(&vec![7_u8; storage::MAX_SETTING_BYTES]);
    let keys: std::collections::BTreeMap<String, String> = (0..13).map(|i| (format!("k{i}"), value.clone())).collect();
    let oversize = serde_json::to_vec(&keys).unwrap();
    assert!(oversize.len() > storage::MAX_SETTINGS_BYTES);
    std::fs::write(dir.join("settings.json"), &oversize).unwrap();
    assert_eq!(st.setting("k0", m).unwrap(), None);
    assert!(dir.join("settings.unreadable.json").is_file() && !dir.join("settings.json").exists());
    std::fs::create_dir_all(dir.join("cache")).unwrap();
    let huge = dir.join("cache").join("huge.bin");
    std::fs::File::create(&huge).unwrap().set_len(storage::MAX_CACHE_ENTRY as u64 + 1).unwrap();
    assert_eq!(st.cache_entry("huge", m), Err(StorageError::Unavailable));
    std::fs::remove_file(&huge).unwrap();
}

/// Writes are atomic: a new file takes the old one's name, so another name for the old file (a hard
/// link) keeps its content. `st` is the storage in `dir` and holds the setting "zoom"; `outside` is
/// a folder outside it.
fn plugin_storage_writes_are_atomic(st: &dyn vectorcraft_plugins::storage::Storage, dir: &std::path::Path, outside: &std::path::Path) {
    let m = &mut vectorcraft_plugins::storage::Meter::unlimited();
    let settings_copy = outside.join("settings-copy.json");
    let before = std::fs::read(dir.join("settings.json")).unwrap();
    std::fs::hard_link(dir.join("settings.json"), &settings_copy).unwrap();
    st.set_setting("zoom", b"world", m).unwrap();
    assert_eq!(std::fs::read(&settings_copy).unwrap(), before);
    assert_eq!(st.setting("zoom", m).unwrap(), Some(b"world".to_vec()));
    st.set_cache_entry("tile", b"abc", m).unwrap();
    let entry_copy = outside.join("tile-copy.bin");
    let entry = std::fs::read_dir(dir.join("cache")).unwrap().map(|e| e.unwrap().path()).find(|p| p.is_file()).unwrap();
    std::fs::hard_link(&entry, &entry_copy).unwrap();
    st.set_cache_entry("tile", b"xyz", m).unwrap();
    assert_eq!(std::fs::read(&entry_copy).unwrap(), b"abc");
    assert_eq!(st.cache_entry("tile", m).unwrap(), Some(b"xyz".to_vec()));
    assert!(st.remove_cache_entry("tile").unwrap());
    std::fs::remove_file(&settings_copy).unwrap();
    std::fs::remove_file(&entry_copy).unwrap();
}

/// Gives the file at `path` the modified time `when`, whatever the file system's clock says.
fn set_modified(path: &std::path::Path, when: std::time::SystemTime) {
    std::fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
}

/// Each storage call is charged the bytes it reads and writes, before it does: a settings call the
/// whole settings file when it reads the file and the whole file when it writes it, a cache call the
/// entry it reads or writes. A call that would pass what its meter has left fails and changes nothing.
fn plugin_storage_calls_are_charged_the_files_they_read_and_write() {
    use vectorcraft_plugins::storage::{Meter, StorageError};
    let folder = crate::tests_links::Folder::new("plugin-storage-meter");
    let id = "test.engine.storagemeter";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, "");
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let file = crate::store::plugin_data_dir(&folder.0, id).join("settings.json");
    let size = || std::fs::metadata(&file).unwrap().len();
    st.set_setting("big", &[7; 60_000], &mut Meter::unlimited()).unwrap();
    st.set_setting("small", b"x", &mut Meter::unlimited()).unwrap();

    // A file another window changed is read whole, even for a one-byte setting.
    let touch = || set_modified(&file, std::fs::metadata(&file).unwrap().modified().unwrap() + std::time::Duration::from_secs(1));
    touch();
    let before = size();
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("small", &mut m).unwrap(), Some(b"x".to_vec()));
    assert_eq!(m.used(), before);
    // Setting it writes the file again, and so does deleting one.
    let mut m = Meter::unlimited();
    st.set_setting("small", b"yz", &mut m).unwrap();
    assert_eq!(m.used(), size());
    let mut m = Meter::unlimited();
    assert!(st.delete_setting("small", &mut m).unwrap());
    assert_eq!(m.used(), size());
    st.set_setting("small", b"y", &mut Meter::unlimited()).unwrap();

    // Less left than the file: nothing is read. Enough to read it but not to write it again:
    // nothing changes.
    touch();
    let mut m = Meter::new(size() - 1);
    assert_eq!(st.setting("small", &mut m), Err(StorageError::Budget));
    assert_eq!(m.used(), 0);
    assert_eq!(st.set_setting("small", b"z", &mut Meter::new(size())), Err(StorageError::Budget));
    assert_eq!(st.setting("small", &mut Meter::unlimited()).unwrap(), Some(b"y".to_vec()));

    // A cache write is charged the entry it writes, a read the entry it reads.
    let mut m = Meter::unlimited();
    st.set_cache_entry("tile", &[1; 1000], &mut m).unwrap();
    assert_eq!(m.used(), 1000);
    let mut m = Meter::unlimited();
    assert_eq!(st.cache_entry("tile", &mut m).unwrap().map(|v| v.len()), Some(1000));
    assert_eq!(m.used(), 1000);
    assert_eq!(st.cache_entry("tile", &mut Meter::new(999)), Err(StorageError::Budget));
    assert_eq!(st.set_cache_entry("tile", &[2; 10], &mut Meter::new(9)), Err(StorageError::Budget));
    assert_eq!(st.cache_entry("tile", &mut Meter::unlimited()).unwrap().map(|v| v.len()), Some(1000));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A session with the plug-in `id` installed in a new store → its storage and the path of its
/// settings file.
fn storage_in(
    s: &mut Session,
    folder: &crate::tests_links::Folder,
    id: &str,
) -> (std::sync::Arc<dyn vectorcraft_plugins::storage::Storage>, std::path::PathBuf) {
    s.attach_plugin_store(folder.0.clone());
    install_storing(s, id, "");
    (vectorcraft_plugins::registry::storage(id).unwrap(), crate::store::plugin_data_dir(&folder.0, id).join("settings.json"))
}

/// The settings stay in memory while the file keeps its size and modified time: a read doesn't open
/// the file, a set to the value already there doesn't write it, a real change is in the file when
/// the call returns, and a file with another time is read again.
#[test]
fn plugin_settings_are_read_from_memory_until_the_file_changes() {
    use vectorcraft_plugins::storage::{Meter, StorageError};
    let folder = crate::tests_links::Folder::new("plugin-storage-kept");
    let id = "test.engine.storagekept";
    let mut s = session();
    let (st, file) = storage_in(&mut s, &folder, id);
    st.set_setting("zoom", b"hello", &mut Meter::unlimited()).unwrap();
    let written = std::fs::metadata(&file).unwrap().modified().unwrap();

    // Other bytes of the same size and the same time: the storage can't tell, so what it answers
    // shows that it did not read the file.
    let other =
        std::fs::read_to_string(&file).unwrap().replace(&vectorcraft_format::base64_encode(b"hello"), &vectorcraft_format::base64_encode(b"world"));
    std::fs::write(&file, &other).unwrap();
    set_modified(&file, written);
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("zoom", &mut m).unwrap(), Some(b"hello".to_vec()));
    assert_eq!(m.used(), 0, "answered from memory");

    // Setting the value it has writes nothing: the file still holds the other bytes.
    let mut m = Meter::unlimited();
    st.set_setting("zoom", b"hello", &mut m).unwrap();
    assert_eq!(m.used(), 0, "nothing to write");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), other);

    // Another time: read again.
    set_modified(&file, written + std::time::Duration::from_secs(1));
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("zoom", &mut m).unwrap(), Some(b"world".to_vec()));
    assert_eq!(m.used(), other.len() as u64);

    // A change reaches the file before the call returns, and a failed one is not kept.
    st.set_setting("tile", b"x", &mut Meter::unlimited()).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"tile\""));
    assert_eq!(st.set_setting("tile", b"y", &mut Meter::new(0)), Err(StorageError::Budget));
    assert_eq!(st.setting("tile", &mut Meter::unlimited()).unwrap(), Some(b"x".to_vec()));
    assert!(st.delete_setting("tile", &mut Meter::unlimited()).unwrap());
    assert!(!std::fs::read_to_string(&file).unwrap().contains("\"tile\""));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// `plugin.remove` drops what was kept: a file put back with the removed settings' size and time is
/// read from disk by the storage of the id installed again.
#[test]
fn removing_a_plugin_drops_its_settings_from_memory() {
    use vectorcraft_plugins::storage::Meter;
    let folder = crate::tests_links::Folder::new("plugin-storage-kept-removed");
    let id = "test.engine.storagekeptremoved";
    let mut s = session();
    let (st, file) = storage_in(&mut s, &folder, id);
    st.set_setting("zoom", b"hello", &mut Meter::unlimited()).unwrap();
    let (bytes, written) = (std::fs::read(&file).unwrap(), std::fs::metadata(&file).unwrap().modified().unwrap());
    run(&mut s, "plugin.remove", json!({"id": id}));

    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, &bytes).unwrap();
    set_modified(&file, written);
    s.attach_plugin_storage(id);
    let st = vectorcraft_plugins::registry::storage(id).unwrap();
    let mut m = Meter::unlimited();
    assert_eq!(st.setting("zoom", &mut m).unwrap(), Some(b"hello".to_vec()));
    assert_eq!(m.used(), bytes.len() as u64, "read from the file");
    s.remove_plugin_storage(id).unwrap();
}

/// The cache's total stays in memory while the cache folder keeps its time; a write and a clear
/// update it, and removing an entry drops it. A file whose length changes behind the storage leaves
/// the folder's time alone, so the total it reports shows that it did not list the folder.
#[test]
fn plugin_cache_total_is_kept_and_follows_writes_removals_and_clears() {
    use vectorcraft_plugins::storage::Meter;
    let folder = crate::tests_links::Folder::new("plugin-storage-kept-cache");
    let id = "test.engine.storagekeptcache";
    let mut s = session();
    let (st, file) = storage_in(&mut s, &folder, id);
    let cache = file.parent().unwrap().join("cache");
    let m = &mut Meter::unlimited();
    let grow = |name: &str, len: u64| std::fs::File::options().write(true).open(cache.join(name)).unwrap().set_len(len).unwrap();

    st.set_cache_entry("tile", b"abc", m).unwrap();
    grow("tile.bin", 10);
    assert_eq!(st.cache_bytes(), 3, "the write set the total");
    // A write lists the folder for its caps, and starts from what it finds.
    st.set_cache_entry("more", b"12345", m).unwrap();
    grow("more.bin", 5);
    assert_eq!(st.cache_bytes(), 15);
    assert!(st.remove_cache_entry("more").unwrap());
    assert_eq!(st.cache_bytes(), 10, "after a removal the total comes from the folder");
    grow("tile.bin", 20);
    assert_eq!(st.cache_bytes(), 10, "and is kept again");
    assert_eq!(st.clear_cache().unwrap(), 20);
    assert_eq!(st.cache_bytes(), 0);

    // A folder deleted behind the storage is noticed.
    st.set_cache_entry("tile", b"abc", m).unwrap();
    std::fs::remove_dir_all(&cache).unwrap();
    assert_eq!(st.cache_bytes(), 0);
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A module with other bytes than the plug-in installed with its id, or, with none installed, one
/// whose id has settings and a cache in the plug-in store, installs only with `replace: true`, and
/// then keeps them; the same module installs again without it, and `replace` must be true or false.
#[test]
fn another_module_under_an_id_with_settings_needs_replace() {
    let folder = crate::tests_links::Folder::new("plugin-storage-replace");
    let id = "test.engine.storingreplace";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    let hash = run(&mut s, "plugin.info", json!({"id": id}))["hash"].clone();
    // The same module again.
    install_storing(&mut s, id, WRITES);

    let reads = vectorcraft_format::base64_encode(&storing(id, READS));
    let install = |s: &mut Session, extra: Value| {
        let mut p = json!({"dataBase64": reads, "grant": ["settings", "cache"]});
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        s.execute("plugin.install", &p)
    };
    let Err(e) = install(&mut s, json!({})) else { panic!("another module took the plug-in's place") };
    assert!(e.to_string().contains(&format!("“Test {id}” is another module than the plug-in with id \"{id}\"")), "{e}");
    assert!(e.to_string().contains("`replace: true`"), "{e}");
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": reads}))["needsReplace"], json!(true));
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["hash"], hash);
    for bad in [json!({"replace": "yes"}), json!({"replace": 1})] {
        let Err(e) = install(&mut s, bad) else { panic!("a bad `replace` was taken") };
        assert!(e.to_string().contains("`replace` must be true or false"), "{e}");
    }
    assert!(install(&mut s, json!({"replace": false})).is_err());
    install(&mut s, json!({"replace": true})).unwrap();
    // It reads what the one it replaced set.
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));

    // Not installed, but with settings in the store: another module needs `replace` too.
    vectorcraft_plugins::registry::remove(id);
    let writes = vectorcraft_format::base64_encode(&storing(id, WRITES));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": writes}))["needsReplace"], json!(true));
    assert!(s.execute("plugin.install", &json!({"dataBase64": writes, "grant": ["settings", "cache"]})).is_err());
    run(&mut s, "plugin.install", json!({"dataBase64": writes, "grant": ["settings", "cache"], "replace": true}));
    run(&mut s, "plugin.remove", json!({"id": id}));
    // Once removed, with its settings and cache, it installs as a first install does.
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": reads}))["needsReplace"], json!(false));
    install(&mut s, json!({})).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}
