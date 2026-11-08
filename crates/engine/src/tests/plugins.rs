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

/// The manifest of a test live effect `id` that declares `paintOutput` (at `api` 2), or that
/// doesn't (`paints` off).
fn painting(id: &str, paints: bool) -> String {
    let declared = if paints { r#","api":2,"paintOutput":true"# } else { "" };
    format!(r#"{{"id":"{id}","name":"Test {id}","version":"1","kind":"effect"{declared},"params":{{}}}}"#)
}

/// A 20 × 20 square at `x`, 40.
fn square_at(x: f64) -> PathData {
    PathData::single(SubPath::polyline(&[Point::new(x, 40.0), Point::new(x + 20.0, 40.0), Point::new(x + 20.0, 60.0), Point::new(x, 60.0)], true))
}

fn solid(hex: &str) -> Value {
    serde_json::to_value(Paint::solid(Color::from_hex(hex).unwrap())).unwrap()
}

/// The 200 × 200 top left of the document, drawn on white.
fn drawn(s: &Session) -> vectorcraft_render::Rendered {
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    vectorcraft_render::Renderer::new().render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts)
}

const WHITE: [u8; 4] = [255, 255, 255, 255];
const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];

/// A live effect that declares `paintOutput` draws each piece it returns by itself, with the paints
/// it gives (a piece without `fills` keeps the object's), in place of the object; the same output
/// from an effect that doesn't declare it is one path painted by the object's fill. Drawing and
/// export leave the document alone, and Expand Appearance writes the pieces as ordinary paths in
/// one undo step.
#[test]
fn a_painted_live_effect_draws_its_pieces_with_their_own_paints() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}, {"path": square_at(80.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.paints", true), &out));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.geometry", false), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.geometry"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50), img.pixel(15, 15)), (RED, RED, WHITE), "without paintOutput the paint is ignored");
    run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 0}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.paints"}));
    let before = s.doc().unwrap().doc.clone();
    let undo = s.doc().unwrap().history.undo.len();
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50), img.pixel(15, 15)), (GREEN, RED, WHITE), "each piece with its own paint");
    // An upstream paint edit reaches the piece that keeps the object's fill; Undo brings it back.
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "color": "#0000ff"}));
    assert_eq!((drawn(&s).pixel(50, 50), drawn(&s).pixel(90, 50)), (GREEN, BLUE));
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(drawn(&s).pixel(90, 50), RED);
    run(&mut s, "edit.redo", json!({}));
    assert_eq!(drawn(&s).pixel(90, 50), BLUE);
    run(&mut s, "edit.undo", json!({}));
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.contains("#00ff00") && svg.contains("#ff0000"), "{svg}");
    assert_eq!(s.doc().unwrap().doc, before, "drawing and export don't change the document");
    assert_eq!(s.doc().unwrap().history.undo.len(), undo);
    // Expanded: two ordinary paths, green and red, and no live effect, as one undo step.
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "effect.expandAppearance", json!({}));
    assert_eq!(s.doc().unwrap().history.undo.len(), undo + 1);
    let mut paths = vec![];
    s.doc().unwrap().doc.walk(|n| {
        if let Some(p) = n.path_data() {
            assert!(n.appearance.effects.is_empty(), "no live effect left");
            paths.push((p.bounds(), n.appearance.fill().map(|f| f.paint.clone())));
        }
    });
    let fill = |hex: &str| Some(Paint::solid(Color::from_hex(hex).unwrap()));
    assert_eq!(paths, [(square_at(40.0).bounds(), fill("#00ff00")), (square_at(80.0).bounds(), fill("#ff0000"))]);
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(node(&s, id).appearance.effects[0].id, "plugin.test.engine.paints");
}

/// On a fill, a painted effect returns only fills: they replace that fill on each piece. A result
/// that breaks a rule (strokes from an effect on a fill, a swatch the document doesn't have) is
/// refused whole: the object is drawn as if the effect weren't there, and `plugin.info` names the
/// plug-in and the field.
#[test]
fn a_painted_effect_on_a_fill_and_refused_results() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let fills = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}]}).to_string();
    let strokes = json!({"objects": [{"path": square_at(40.0), "strokes": [{"paint": solid("#00ff00"), "width": 2}]}]}).to_string();
    let mut gone = solid("#00ff00");
    gone["swatch"] = json!("Not a swatch");
    let unknown = json!({"objects": [{"path": square_at(40.0), "fills": [gone]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.fillpaint", true), &fills));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.strokepaint", true), &strokes));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.swatchpaint", true), &unknown));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.fillpaint", "item": fill}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (GREEN, WHITE), "the fill's piece, green");
    assert_eq!(run(&mut s, "plugin.info", json!({"id": "test.engine.fillpaint"}))["lastError"], Value::Null);
    // A second painted effect on the same fill paints what it returns too.
    let blue = json!({"objects": [{"path": square_at(80.0), "fills": [solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.bluepaint", true), &blue));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.bluepaint", "item": fill}));
    let img = drawn(&s);
    assert_eq!((img.pixel(90, 50), img.pixel(50, 50), img.pixel(15, 15)), (BLUE, WHITE, WHITE), "the second effect's piece, blue");
    run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 1, "item": fill}));
    run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 0, "item": fill}));
    for (effect, item, field) in
        [("test.engine.strokepaint", json!(fill), "objects[0].strokes"), ("test.engine.swatchpaint", Value::Null, "objects[0].fills[0].swatch")]
    {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": format!("plugin.{effect}"), "item": item}));
        let img = drawn(&s);
        assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (WHITE, RED), "{effect}: drawn as it is");
        let e = run(&mut s, "plugin.info", json!({"id": effect}))["lastError"].as_str().unwrap_or_default().to_string();
        assert!(e.starts_with(&format!("Test {effect}: ")) && e.contains(field), "{effect}: {e}");
        let n = node(&s, id);
        let kept = if item.is_null() { n.appearance.effects.clone() } else { n.appearance.items[fill].effects().to_vec() };
        assert_eq!(kept.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(), [format!("plugin.{effect}")], "the effect record stays");
        run(&mut s, "effect.remove", json!({"ids": [id.0], "index": 0, "item": item}));
    }
}

/// A painted effect on a group may link its pieces to the document's swatches.
#[test]
fn a_painted_group_effect_links_to_the_document_s_swatches() {
    let mut s = session();
    rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00", "global": true}));
    run(&mut s, "select.all", json!({}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let mut leaf = solid("#00ff00");
    leaf["swatch"] = json!("Leaf");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [leaf]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.grouppaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "plugin.test.engine.grouppaint"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (GREEN, WHITE), "the group's piece, green");
}

/// The output of a painted effect that returns one green square linked to swatch `Leaf`.
fn leaf_square() -> String {
    let mut leaf = solid("#00ff00");
    leaf["swatch"] = json!("Leaf");
    json!({"objects": [{"path": square_at(40.0), "fills": [leaf]}]}).to_string()
}

/// A Pathfinder operation and baked appearance take a painted effect's pieces, swatch links
/// included, as the canvas draws them: a group with a painted effect inside a group with a live
/// Pathfinder effect draws the green piece, and baking a path's appearance gives the piece linked
/// to the swatch.
#[test]
fn pathfinder_and_baked_appearance_take_painted_pieces_with_swatch_links() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.leafpaint", true), &leaf_square()));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let inner = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [inner.0], "effect": "plugin.test.engine.leafpaint"}));
    let outer = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [outer.0], "effect": "pathfinder.add"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(15, 15)), (GREEN, WHITE), "the Pathfinder operation takes the green piece");

    let id = rect(&mut s, 120.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.leafpaint"}));
    let st = s.doc().unwrap();
    let baked = vectorcraft_render::effects::bake_appearance(Some(&st.doc), st.doc.node(id).unwrap()).unwrap();
    let mut fills = vec![];
    baked.walk(&mut |n| {
        if let Some(p) = n.path_data() {
            fills.push((p.bounds(), n.appearance.fill().map(|f| f.paint.clone())));
        }
    });
    let mut leaf = Paint::solid(Color::from_hex("#00ff00").unwrap());
    if let Paint::Solid { swatch, .. } = &mut leaf {
        *swatch = Some("Leaf".into());
    }
    assert_eq!(fills, [(square_at(40.0).bounds(), Some(leaf))]);
}

/// A swatch-linked piece on screen is drawn when the path it comes from is off screen: the
/// renderer's bounds of the path take in the piece.
#[test]
fn a_swatch_linked_piece_is_not_culled_with_its_off_screen_path() {
    let mut s = session();
    let id = rect(&mut s, 500.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.culled", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.culled"}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
}

/// A retained renderer's kept bounds follow the document's swatches: a swatch-linked piece on
/// screen appears when its swatch is added, though the path it comes from is off screen.
#[test]
fn a_retained_renderer_shows_a_piece_once_its_swatch_exists() {
    let mut s = session();
    let id = rect(&mut s, 500.0, "#ff0000");
    install_wat(&mut s, &tpl::returning(&painting("test.engine.lateleaf", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.lateleaf"}));
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut r = vectorcraft_render::Renderer::new();
    let at_piece = |r: &mut vectorcraft_render::Renderer, s: &Session| {
        r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(50, 50)
    };
    assert_eq!(at_piece(&mut r, &s), WHITE, "the piece is refused");
    run(&mut s, "select.none", json!({}));
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    assert_eq!(at_piece(&mut r, &s), GREEN, "the piece is drawn with its swatch");
}

/// Object › Expand of a gradient fill works piece by piece, for a painted effect on the object and
/// on the fill: a piece that keeps the gradient has it expanded, and a piece with a paint of its own,
/// here linked to the document's swatch, keeps that paint on its own path.
#[test]
fn expanding_a_gradient_fill_keeps_each_piece_s_own_paint() {
    for on_fill in [false, true] {
        let mut s = session();
        let id = rect(&mut s, 10.0, "#ff0000");
        run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": {"kind": "linear"}}));
        run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
        let mut leaf = solid("#00ff00");
        leaf["swatch"] = json!("Leaf");
        let out = json!({"objects": [{"path": square_at(40.0), "fills": [leaf]}, {"path": square_at(100.0)}]}).to_string();
        install_wat(&mut s, &tpl::returning(&painting("test.engine.expandref", true), &out));
        let item = if on_fill { json!(node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap()) } else { Value::Null };
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.expandref", "item": item}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "object.expand", json!({"object": false, "fill": true, "stroke": false}));
        let img = drawn(&s);
        assert_eq!((img.pixel(15, 15), img.pixel(50, 50)), (WHITE, GREEN), "on_fill {on_fill}: the input is gone and the piece keeps its paint");
        assert!(![WHITE, GREEN].contains(&img.pixel(110, 50)), "on_fill {on_fill}: the other piece shows the gradient");
        let mut own = Paint::solid(Color::from_hex("#00ff00").unwrap());
        if let Paint::Solid { swatch, .. } = &mut own {
            *swatch = Some("Leaf".into());
        }
        let mut kept = vec![];
        let mut plugins = 0;
        s.doc().unwrap().doc.walk(|n| {
            plugins += n
                .appearance
                .effects
                .iter()
                .chain(n.appearance.items.iter().flat_map(|i| i.effects()))
                .filter(|e| e.id.starts_with("plugin."))
                .count();
            if n.path_data().is_some_and(|p| p.bounds() == square_at(40.0).bounds()) {
                kept.push(n.appearance.fill().map(|f| f.paint.clone()));
            }
        });
        assert_eq!(kept, [Some(own)], "on_fill {on_fill}");
        assert_eq!(plugins, 0, "on_fill {on_fill}: no live effect is left");
    }
}

/// Object › Expand of a pattern fill works piece by piece: a piece that keeps the pattern has its
/// tiles expanded, and a piece with a paint of its own keeps that paint.
#[test]
fn expanding_a_pattern_fill_keeps_each_piece_s_own_paint() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let tile = rect(&mut s, 300.0, "#00ff00");
    run(&mut s, "object.pattern.make", json!({"ids": [tile.0], "name": "Tiles", "edit": false}));
    run(&mut s, "paint.setFill", json!({"ids": [id.0], "swatch": "Tiles"}));
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#0000ff")]}, {"path": square_at(100.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.expandtiles", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.expandtiles"}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "object.expand", json!({"object": false, "fill": true, "stroke": false}));
    let img = drawn(&s);
    assert_eq!((img.pixel(15, 15), img.pixel(50, 50)), (WHITE, BLUE), "the input is gone and the piece keeps its paint");
    assert!(near(img.pixel(110, 50), GREEN), "the other piece shows the tiles");
    let mut patterns = 0;
    s.doc().unwrap().doc.walk(|n| patterns += n.appearance.items.iter().filter(|i| matches!(i.paint(), Paint::Pattern { .. })).count());
    assert_eq!(patterns, 0, "the tiles are objects");
}

/// Two painted effects on one fill, the first returning a piece with two fills: the second replaces
/// the first of them and the other fill stays over it.
#[test]
fn a_second_painted_effect_on_a_fill_keeps_the_piece_s_other_fills() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let two = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#ff0000"), solid("#00ff00")]}]}).to_string();
    let one = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#0000ff")]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.twofills", true), &two));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.onefill", true), &one));
    let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.twofills", "item": fill}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.onefill", "item": fill}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN, "the other fill paints over the replaced one");
    let st = s.doc().unwrap();
    let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
    let mut paints = vec![];
    art.walk(&mut |n| {
        if n.path_data().is_some() {
            paints.push(n.appearance.items.iter().filter(|i| i.is_fill()).map(|i| i.paint().clone()).collect::<Vec<_>>());
        }
    });
    let fill = |hex: &str| Paint::solid(Color::from_hex(hex).unwrap());
    assert!(paints.contains(&vec![fill("#0000ff"), fill("#00ff00")]), "{paints:?}");
}

/// The geometry of a painted result that names a swatch counts only when the document has that
/// swatch: without a document, and with one that lacks it, the effect keeps its input.
#[test]
fn painted_geometry_is_used_only_when_its_references_are_in_the_document() {
    use vectorcraft_render::effects::{GeomContext, apply_geometry_with};
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    install_wat(&mut s, &tpl::returning(&painting("test.engine.geomref", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.geomref"}));
    let n = node(&s, id);
    let path = n.path_data().unwrap().clone();
    let input = path.bounds().unwrap();
    let geometry = |doc: Option<&vectorcraft_doc::Document>| {
        apply_geometry_with(&n.appearance.effects, &path, input, &GeomContext { doc, ..GeomContext::of(&n) }).bounds()
    };
    assert_eq!(geometry(None), Some(input), "no document");
    assert_eq!(geometry(Some(&s.doc().unwrap().doc)), Some(input), "no such swatch");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    assert_eq!(geometry(Some(&s.doc().unwrap().doc)), square_at(40.0).bounds(), "the piece's geometry");
}

/// Type with a painted effect, in a group with a live Pathfinder effect: the operation takes the
/// piece linked to the document's swatch.
#[test]
fn pathfinder_takes_the_swatch_linked_pieces_of_type() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 20}))["id"].as_u64().unwrap());
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.typepaint", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [t.0], "effect": "plugin.test.engine.typepaint"}));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.divide"}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
}

/// Type whose fill has a painted effect, in a group with a geometry effect inside a live Pathfinder
/// effect: the operation takes the piece linked to the document's swatch.
#[test]
fn pathfinder_takes_swatch_linked_pieces_of_type_in_a_reshaped_group() {
    let mut s = session();
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 150, "text": "Hi", "size": 20}))["id"].as_u64().unwrap());
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.memberpaint", true), &leaf_square()));
    run(&mut s, "select.set", json!({"ids": [t.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "plugin.test.engine.memberpaint", "item": 0}));
    let inner = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [inner.0], "effect": "path.offsetPath", "params": {"offset": 0}}));
    let outer = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [outer.0], "effect": "pathfinder.divide"}));
    assert_eq!(drawn(&s).pixel(50, 50), GREEN);
}

/// A painted effect on an image's own fill that links its piece to a swatch: hit tests and bounds
/// use the piece the canvas draws.
#[test]
fn an_image_s_painted_fill_is_hit_and_measured_as_drawn() {
    let mut s = session();
    let mut png = vec![];
    image::RgbaImage::from_pixel(30, 30, image::Rgba([255, 0, 0, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap();
    let r = run(&mut s, "file.place", json!({"name": "red.png", "dataBase64": vectorcraft_format::base64_encode(&png), "at": [65, 65]}));
    let im = NodeId(r["ids"][0].as_u64().unwrap());
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    let mut leaf = solid("#00ff00");
    leaf["swatch"] = json!("Leaf");
    let out = json!({"objects": [{"path": square_at(120.0), "fills": [leaf]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.imagepaint", true), &out));
    run(&mut s, "select.set", json!({"ids": [im.0]}));
    run(&mut s, "appearance.addFill", json!({}));
    run(&mut s, "paint.setFill", json!({"color": "#ff0000", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "convertToShape.rectangle", "item": 0}));
    run(&mut s, "effect.apply", json!({"effect": "plugin.test.engine.imagepaint", "item": 0}));
    assert_eq!(drawn(&s).pixel(130, 50), GREEN, "the canvas draws the piece");
    let doc = &s.doc().unwrap().doc;
    assert!(vectorcraft_doc::hit::hit_test(doc, Point::new(130.0, 50.0), Default::default()).is_some(), "a click on the piece hits the image");
    let boxes = doc.symbol_boxes();
    assert!(doc.node(im).unwrap().reach_bounds_with(&boxes).unwrap().x1 >= 140.0, "the bounds take in the piece");
}

/// A path with a painted effect takes part in a Pathfinder operation as its pieces, each with its
/// own paint and the path's opacity: Divide on a group holding it makes one face per piece,
/// painted as the piece is.
#[test]
fn a_path_with_a_painted_effect_takes_part_in_pathfinder_as_its_pieces() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [solid("#00ff00")]}, {"path": square_at(80.0), "fills": [solid("#0000ff")]}]})
        .to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.twopaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.twopaint"}));
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    run(&mut s, "effect.apply", json!({"ids": [g.0], "effect": "pathfinder.divide"}));
    let img = drawn(&s);
    assert_eq!((img.pixel(50, 50), img.pixel(90, 50), img.pixel(15, 15)), (GREEN, BLUE, WHITE), "one face per piece");
    run(&mut s, "select.set", json!({"ids": [id.0]}));
    run(&mut s, "transparency.set", json!({"item": null, "opacity": 50}));
    let st = s.doc().unwrap();
    let hook = |c: &vectorcraft_doc::Node| vectorcraft_render::effects::outline_object(Some(&st.doc), c);
    let faces = vectorcraft_render::effects::pathfinder_children(Some(&st.doc), st.doc.node(g).unwrap(), Some(&hook)).unwrap();
    let faces: Vec<_> = faces.iter().map(|f| (f.appearance.fill().map(|f| f.paint.clone()), f.opacity)).collect();
    let fill = |hex: &str| Some(Paint::solid(Color::from_hex(hex).unwrap()));
    assert_eq!(faces, [(fill("#0000ff"), 0.5), (fill("#00ff00"), 0.5)]);
}

/// Art a renderer keeps for an object with a painted effect is made again when a swatch its pieces
/// name leaves the document (the object is then drawn as if the effect weren't there) and when
/// Undo brings the swatch back.
#[test]
fn kept_art_follows_the_swatches_painted_pieces_name() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.keptpaint", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.keptpaint"}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| {
        let img = r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
        (img.pixel(50, 50), img.pixel(15, 15))
    };
    assert_eq!(draw(&s), (GREEN, WHITE), "the piece, green");
    run(&mut s, "swatch.delete", json!({"name": "Leaf"}));
    assert_eq!(draw(&s), (WHITE, RED), "without the swatch, the object as it is");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(draw(&s), (GREEN, WHITE), "the swatch back, the piece again");
}

/// Art a renderer keeps for an object whose plug-in starts to paint its pieces when it is replaced
/// follows the swatches those pieces name from then on.
#[test]
fn kept_art_follows_the_swatches_once_a_replaced_plugin_paints() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.laterpaint", false), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.laterpaint"}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| {
        let img = r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
        (img.pixel(50, 50), img.pixel(15, 15))
    };
    assert_eq!(draw(&s), (RED, WHITE), "the square in the object's paint");
    install_wat(&mut s, &tpl::returning(&painting("test.engine.laterpaint", true), &leaf_square()));
    assert_eq!(draw(&s), (GREEN, WHITE), "the piece, green");
    run(&mut s, "swatch.delete", json!({"name": "Leaf"}));
    assert_eq!(draw(&s), (WHITE, RED), "without the swatch, the object as it is");
}

/// Whether `px` is within 40 levels of `want` on every channel. A blur leaves the middle of a
/// shape close to its color.
fn near(px: [u8; 4], want: [u8; 4]) -> bool {
    px.iter().zip(want).all(|(a, b)| a.abs_diff(b) <= 40)
}

/// A kept raster of a blur over a painted effect is made again when a swatch its piece names
/// leaves the document (the object is then drawn as if the effect weren't there) and when Undo
/// brings the swatch back.
#[test]
fn a_kept_raster_follows_the_swatches_painted_pieces_name() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00"}));
    install_wat(&mut s, &tpl::returning(&painting("test.engine.blurpaint", true), &leaf_square()));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.blurpaint"}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "blur.gaussian", "params": {"radius": 1}}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts);
    let img = draw(&s);
    assert!(near(img.pixel(50, 50), GREEN) && near(img.pixel(15, 15), WHITE), "the piece, green and blurred");
    run(&mut s, "swatch.delete", json!({"name": "Leaf"}));
    let img = draw(&s);
    assert!(near(img.pixel(50, 50), WHITE) && near(img.pixel(15, 15), RED), "without the swatch, the object as it is: {:?}", img.pixel(50, 50));
    run(&mut s, "edit.undo", json!({}));
    let img = draw(&s);
    assert!(near(img.pixel(50, 50), GREEN) && near(img.pixel(15, 15), WHITE), "the swatch back, the piece again");
}

/// A kept raster of a blur over a painted effect is made again when the tile of a pattern its
/// piece names changes and the pattern keeps its name.
#[test]
fn a_kept_raster_follows_the_tiles_of_patterns_painted_pieces_name() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let green = rect(&mut s, 300.0, "#00ff00");
    let blue = rect(&mut s, 320.0, "#0000ff");
    let tile = |r: f64, art: vectorcraft_doc::Node| vectorcraft_doc::pattern::PatternDef {
        tile: vectorcraft_geom::Rect::new(r, 10.0, r + 10.0, 20.0),
        ..vectorcraft_doc::pattern::PatternDef::new("Tiles", vec![std::sync::Arc::new(art)])
    };
    let (first, second) = (tile(300.0, node(&s, green)), tile(320.0, node(&s, blue)));
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).patterns.push(first);
    let paint = serde_json::to_value(Paint::Pattern { pattern: "Tiles".into(), xf: vectorcraft_geom::Affine::IDENTITY }).unwrap();
    let out = json!({"objects": [{"path": square_at(40.0), "fills": [paint]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.tilepaint", true), &out));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.tilepaint"}));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "blur.gaussian", "params": {"radius": 1}}));
    let mut r = vectorcraft_render::Renderer::new();
    let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
    let mut draw = |s: &Session| r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(50, 50);
    assert!(near(draw(&s), GREEN), "the piece in the green tile");
    std::sync::Arc::make_mut(&mut s.doc_mut().unwrap().doc).patterns[0] = second;
    let px = draw(&s);
    assert!(near(px, BLUE), "the piece in the new tile: {px:?}");
}

/// Every painted piece counts against the effects' budget, however small its path: two painted
/// effects in a stack that each return 350 tiny triangles make no more than 100,000 objects, not
/// 350 × 350 of them.
#[test]
fn painted_pieces_keep_to_the_effects_budget() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let tiny = PathData::single(SubPath::polyline(&[Point::new(0.0, 0.0), Point::new(0.1, 0.0), Point::new(0.0, 0.1)], true));
    let out = json!({"objects": vec![json!({"path": tiny}); 350]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.many", true), &out));
    for _ in 0..2 {
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.many"}));
    }
    let st = s.doc().unwrap();
    let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
    let mut paths = 0;
    art.walk(&mut |n| paths += usize::from(matches!(n.kind, vectorcraft_doc::NodeKind::Path { .. })));
    assert!((350..=100_000).contains(&paths), "{paths} paths");
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
/// hash. The grant is recorded with that hash: replacing the plug-in with one that lists the same
/// permissions needs no new grant, and one that lists another needs one. Removing it forgets the
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
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&other)}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, vectorcraft_plugins::permissions::sha256_hex(&other));
    // One that asks for more is refused until it is granted all of them.
    let more = data(r#"["doc.read","doc.write"]"#);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": more}))["needsApproval"], true);
    let e = s.execute("plugin.install", &json!({"dataBase64": more})).unwrap_err().to_string();
    assert!(e.contains("doc.read, doc.write") && e.contains("`grant`"), "{e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().manifest().permissions().len(), 1, "the one installed stays");
    run(&mut s, "plugin.install", json!({"dataBase64": more, "grant": ["doc.write", "doc.read", "state"]}));
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
        run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(wasm), "grant": perms}));
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
