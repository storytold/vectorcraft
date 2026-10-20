//! WebAssembly plug-ins (#121): object filters through `plugin.run` (one undo step,
//! transactional), live-effect plug-ins in appearance stacks, and the `plugin.*` management
//! commands.

use serde_json::{Value, json};
use vectorcraft_color::{Color, Paint};
use vectorcraft_geom::{PathData, Point, SubPath};
use vectorcraft_plugins::wat as tpl;

use super::*;

/// The example Desaturate plug-in (built from `crates/plugins/example`).
const DESATURATE: &[u8] = include_bytes!("../../plugins/tests/fixtures/desaturate.wasm");
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
