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

/// Held by the tests that set the Additional Plug-ins Folder preference or run `plugin.reload`:
/// the folder last loaded and its report are kept for the whole process, so another test loading
/// its own folder between two steps would change what the next step loads or lists.
static PREFS_FOLDER: std::sync::Mutex<()> = std::sync::Mutex::new(());

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

/// A session started to take `grant` and `allowHeavy` from requests, as most tests here install.
fn session() -> Session {
    let mut s = strict_session();
    s.allow_plugin_grants_from_requests();
    s
}

/// A session as MCP and the CLI have by default: `grant` and `allowHeavy` need an approval.
fn strict_session() -> Session {
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

/// A result refused for a swatch the document doesn't have stays cached: once the document has the
/// swatch, the next check of that result clears the error.
#[test]
fn a_refused_swatch_s_error_clears_once_the_document_has_the_swatch() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let mut later = solid("#00ff00");
    later["swatch"] = json!("Later");
    let output = json!({"objects": [{"path": square_at(40.0), "fills": [later]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.laterswatch", true), &output));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.laterswatch"}));
    drawn(&s);
    assert!(run(&mut s, "plugin.info", json!({"id": "test.engine.laterswatch"}))["lastError"].is_string());
    run(&mut s, "swatch.new", json!({"name": "Later", "color": "#00ff00", "global": true}));
    drawn(&s);
    assert_eq!(run(&mut s, "plugin.info", json!({"id": "test.engine.laterswatch"}))["lastError"], Value::Null);
}

/// An effect that paints nothing has no references to check, so replacing a refused painted effect
/// with one clears the refusal's error once it runs.
#[test]
fn a_replacement_that_paints_nothing_clears_a_refused_swatch_s_error() {
    let mut s = session();
    let id = rect(&mut s, 10.0, "#ff0000");
    let mut gone = solid("#00ff00");
    gone["swatch"] = json!("Absent");
    let output = json!({"objects": [{"path": square_at(40.0), "fills": [gone]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.replaced", true), &output));
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.replaced"}));
    drawn(&s);
    assert!(run(&mut s, "plugin.info", json!({"id": "test.engine.replaced"}))["lastError"].is_string());
    install_wat(&mut s, &tpl::returning(&painting("test.engine.replaced", false), &output));
    drawn(&s);
    assert_eq!(run(&mut s, "plugin.info", json!({"id": "test.engine.replaced"}))["lastError"], Value::Null);
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

/// The renderer measures a painted piece with its own strokes: a piece off screen whose wide stroke
/// reaches the view is drawn, and the bounds raster effects work over take in that stroke, on a
/// path with no stroke of its own and with a blur on the stroke the effect is on.
#[test]
fn a_piece_s_own_stroke_counts_in_the_bounds_of_what_is_drawn() {
    let mut s = session();
    let wide = json!({"objects": [{"path": square_at(-120.0), "strokes": [{"paint": solid("#00ff00"), "width": 300}]}]}).to_string();
    install_wat(&mut s, &tpl::returning(&painting("test.engine.widestroke", true), &wide));
    let id = rect(&mut s, -300.0, "#ff0000");
    run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.widestroke"}));
    assert_eq!(drawn(&s).pixel(30, 50), GREEN, "only the piece's stroke is on screen");
    let bounds = |s: &Session, id: NodeId| vectorcraft_render::drawn_bounds(&s.doc().unwrap().doc, &node(s, id)).unwrap();
    assert!(bounds(&s, id).x1 >= 50.0, "{:?}", bounds(&s, id));

    let blurred = rect(&mut s, -300.0, "#ff0000");
    run(&mut s, "paint.setStroke", json!({"ids": [blurred.0], "color": "#0000ff", "width": 1}));
    let stroke = node(&s, blurred).appearance.items.iter().position(|i| !i.is_fill()).unwrap();
    run(&mut s, "effect.apply", json!({"ids": [blurred.0], "effect": "plugin.test.engine.widestroke", "item": stroke}));
    run(&mut s, "effect.apply", json!({"ids": [blurred.0], "effect": "blur.gaussian", "params": {"radius": 10}, "item": stroke}));
    assert!(bounds(&s, blurred).x1 > 55.0, "the blur spreads the piece's stroke: {:?}", bounds(&s, blurred));
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

/// A Transform effect before a painted effect on a fill moves the fill's placed gradient: a piece
/// that keeps the fill, and the fill drawn on the moved shape after a refused result, both have the
/// gradient at its moved position.
#[test]
fn a_painted_effect_on_a_fill_keeps_a_placed_gradient_with_its_moved_shape() {
    let mut gone = solid("#00ff00");
    gone["swatch"] = json!("Not a swatch");
    for (name, out, want) in [
        ("keeps", json!({"objects": [{"path": square_at(40.0)}]}).to_string(), square_at(40.0).bounds()),
        ("refused", json!({"objects": [{"path": square_at(40.0), "fills": [gone]}]}).to_string(), square_at(110.0).bounds()),
    ] {
        let mut s = session();
        let id = NodeId(run(&mut s, "shape.rectangle", json!({"x": 10, "y": 40, "width": 20, "height": 20}))["id"].as_u64().unwrap());
        run(&mut s, "paint.setStroke", json!({"ids": [id.0], "none": true}));
        let stops = json!([{"offset": 0, "color": "#ffffff"}, {"offset": 1, "color": "#000000"}]);
        run(&mut s, "paint.setFill", json!({"ids": [id.0], "gradient": {"kind": "linear", "stops": stops}}));
        run(&mut s, "select.set", json!({"ids": [id.0]}));
        run(&mut s, "paint.setGradientGeom", json!({"start": [14, 50], "end": [26, 50], "stroke": false}));
        let plugin = format!("test.engine.carry{name}");
        install_wat(&mut s, &tpl::returning(&painting(&plugin, true), &out));
        let fill = node(&s, id).appearance.items.iter().position(|i| i.is_fill()).unwrap();
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "distort.transform", "params": {"moveH": 100}, "item": fill}));
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": format!("plugin.{plugin}"), "item": fill}));
        let st = s.doc().unwrap();
        let art = vectorcraft_render::effects::reshape(Some(&st.doc), st.doc.node(id).unwrap(), None).unwrap();
        let mut seen = vec![];
        art.walk(&mut |n| {
            if let (Some(p), Some(Paint::Gradient(g))) = (n.path_data(), n.appearance.items.iter().find(|i| i.is_fill()).map(|i| i.paint())) {
                seen.push((p.bounds(), g.geom.map(|g| (g.start.x, g.end.x))));
            }
        });
        assert_eq!(seen, [(want, Some((114.0, 126.0)))], "{name}");
    }
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

/// A piece linked to a global swatch shows the swatch's edited color, in art a renderer keeps and
/// in a kept raster of a blur over it, and its earlier color after Undo.
#[test]
fn a_piece_linked_to_a_global_swatch_takes_its_edited_color() {
    for blur in [false, true] {
        let mut s = session();
        let id = rect(&mut s, 10.0, "#ff0000");
        install_wat(&mut s, &tpl::returning(&painting("test.engine.globalleaf", true), &leaf_square()));
        run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "plugin.test.engine.globalleaf"}));
        if blur {
            run(&mut s, "effect.apply", json!({"ids": [id.0], "effect": "blur.gaussian", "params": {"radius": 1}}));
        }
        run(&mut s, "select.none", json!({}));
        run(&mut s, "swatch.new", json!({"name": "Leaf", "color": "#00ff00", "global": true}));
        let mut r = vectorcraft_render::Renderer::new();
        let opts = vectorcraft_render::RenderOptions { background: Some([255, 255, 255, 255]), ..Default::default() };
        let mut at_piece = |s: &Session| r.render(&s.doc().unwrap().doc, 200, 200, vectorcraft_geom::Affine::IDENTITY, &opts).pixel(50, 50);
        assert!(near(at_piece(&s), GREEN), "the piece, in the swatch's color (blur {blur})");
        run(&mut s, "swatch.edit", json!({"name": "Leaf", "color": "#0000ff"}));
        let px = at_piece(&s);
        assert!(near(px, BLUE), "the swatch's new color (blur {blur}): {px:?}");
        run(&mut s, "edit.undo", json!({}));
        assert!(near(at_piece(&s), GREEN), "the swatch's color after Undo (blur {blur})");
    }
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

/// A clock for a document's deferrals that reads `step` milliseconds later each time it is read;
/// the test changes the step as it goes.
fn stepping_clock(step: &std::sync::Arc<std::sync::atomic::AtomicU64>) -> vectorcraft_plugins::deferred::Clock {
    let (now, step) = (std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0)), step.clone());
    std::sync::Arc::new(move || Some(now.fetch_add(step.load(std::sync::atomic::Ordering::Relaxed), std::sync::atomic::Ordering::Relaxed)))
}

/// While a document is drawn on screen, the plug-in effects of its first view share a time budget.
/// A stage left out draws its input art, is listed by `effect.deferredStatus`, and stays left out
/// on later drawings; Effect › Refresh Deferred Effects runs it again with a fresh budget, and an
/// export draws every stage. Another document has a budget of its own.
#[test]
fn the_first_view_defers_plug_in_effects_past_its_budget() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 30.0, "#ff0000");
    for (id, x) in [("test.engine.deferred.a", 100.0), ("test.engine.deferred.b", 140.0)] {
        let out = json!({"objects": [{"path": square_at(x)}]}).to_string();
        install_wat(&mut s, &tpl::returning(&tpl::manifest(id, "effect", "{}"), &out));
    }
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.deferred.a"}));
    run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": "plugin.test.engine.deferred.b"}));
    // Each reading is 3 s later: the first run's result comes back after the budget, and the
    // second run doesn't start.
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    let img = fx.view(|| drawn(&s));
    fx.first_view_done();
    assert_eq!((img.pixel(15, 15), img.pixel(35, 15), img.pixel(110, 50), img.pixel(150, 50)), (RED, RED, WHITE, WHITE), "the input art");
    assert_eq!(fx.take_report(), Some(2));
    let status = run(&mut s, "effect.deferredStatus", json!({}));
    let listed = |id: NodeId, plugin: &str| json!({"objectId": id.0, "effectIndex": 0, "pluginId": plugin, "reason": "budget"});
    assert_eq!(status, json!({"budgetMs": 2000, "effects": [listed(a, "test.engine.deferred.a"), listed(b, "test.engine.deferred.b")]}));
    assert_eq!(fx.view(|| drawn(&s)).pixel(110, 50), WHITE, "a later drawing leaves it deferred");

    // Refreshed with a fresh budget, in which the clock stands still.
    assert!(s.execute("effect.refreshDeferred", &json!({"ids": ["a"]})).is_err());
    assert!(s.execute("effect.refreshDeferred", &json!({"ids": [999_999]})).is_err(), "not an object of the document");
    step.store(0, std::sync::atomic::Ordering::Relaxed);
    let refreshed = run(&mut s, "effect.refreshDeferred", json!({"ids": [a.0]}));
    assert_eq!(refreshed, json!({"refreshed": 1, "stillDeferred": 0, "failed": 0}));
    let img = fx.view(|| drawn(&s));
    assert_eq!((img.pixel(110, 50), img.pixel(150, 50)), (RED, WHITE));
    assert_eq!(run(&mut s, "effect.deferredStatus", json!({}))["effects"], json!([listed(b, "test.engine.deferred.b")]));

    // An export draws every stage; the view then uses the result.
    let svg = s.execute("document.export", &json!({"format": "svg"})).unwrap();
    let svg = String::from_utf8(vectorcraft_format::base64_decode(svg["dataBase64"].as_str().unwrap()).unwrap()).unwrap();
    assert!(svg.contains(r#"<rect x="140" y="40" width="20" height="20""#), "the square at 140: {svg}");
    assert_eq!(fx.view(|| drawn(&s)).pixel(150, 50), RED);
    assert_eq!(run(&mut s, "effect.deferredStatus", json!({}))["effects"], json!([]));
    assert!(s.execute("effect.refreshDeferred", &json!({})).is_err(), "nothing is deferred");

    // Another document: its first view has a budget of its own.
    let mut other = session();
    let c = rect(&mut other, 50.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(170.0)}]}).to_string();
    install_wat(&mut other, &tpl::returning(&tpl::manifest("test.engine.deferred.c", "effect", "{}"), &out));
    run(&mut other, "effect.apply", json!({"ids": [c.0], "effect": "plugin.test.engine.deferred.c"}));
    let still = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let fx_c = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&still));
    other.doc_mut().unwrap().plugin_effects = fx_c.clone();
    assert_eq!(fx_c.view(|| drawn(&other)).pixel(180, 50), RED);
    fx_c.first_view_done();
    assert_eq!(fx_c.take_report(), Some(0));
}

/// Deferred stages are listed where their effects are: after an effect that changes no geometry,
/// and on a fill after a Transform effect that reaches its appearance, each of two instances of
/// one plug-in at its own place.
#[test]
fn deferred_stages_are_listed_at_their_effects_places() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 60.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    // An effect that paints its pieces: the object is drawn from its reshaped art.
    install_wat(&mut s, &tpl::returning(&painting("test.engine.deferred.at", true), &out));
    const FX: &str = "plugin.test.engine.deferred.at";
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "stylize.dropShadow"}));
    for _ in 0..2 {
        run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": FX}));
    }
    run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": "distort.transform", "params": {"strokes": true}, "item": 0}));
    for _ in 0..2 {
        run(&mut s, "effect.apply", json!({"ids": [b.0], "effect": FX, "item": 0}));
    }
    // No run has time to give a result.
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    let places = run(&mut s, "effect.deferredStatus", json!({}))["effects"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| (e["objectId"].as_u64().unwrap(), e["item"].as_u64(), e["effectIndex"].as_u64().unwrap()))
        .collect::<Vec<_>>();
    assert_eq!(places, [(a.0, None, 1), (a.0, None, 2), (b.0, Some(0), 1), (b.0, Some(0), 2)]);
}

/// A refresh that does not draw a stage (its object, or a group holding it, is hidden) leaves it
/// deferred, and a stage whose effect has been removed or hidden, or whose object has been removed,
/// is no longer listed and does not enable Effect › Refresh Deferred Effects.
#[test]
fn refresh_keeps_stages_it_did_not_draw_and_drops_removed_ones() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let b = rect(&mut s, 30.0, "#ff0000");
    let c = rect(&mut s, 50.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.deferred.kept", "effect", "{}"), &out));
    run(&mut s, "effect.apply", json!({"ids": [a.0, b.0, c.0], "effect": "plugin.test.engine.deferred.kept"}));
    run(&mut s, "select.set", json!({"ids": [c.0]}));
    let g = NodeId(run(&mut s, "object.group", json!({}))["id"].as_u64().unwrap());
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    fx.first_view_done();
    let listed = |s: &mut Session| {
        run(s, "effect.deferredStatus", json!({}))["effects"].as_array().unwrap().iter().map(|e| e["objectId"].as_u64().unwrap()).collect::<Vec<_>>()
    };
    assert_eq!(listed(&mut s), [a.0, b.0, c.0]);

    s.execute("object.hide", &json!({"ids": [a.0, g.0]})).unwrap();
    step.store(0, std::sync::atomic::Ordering::Relaxed);
    let refreshed = run(&mut s, "effect.refreshDeferred", json!({"ids": [a.0, c.0]}));
    assert_eq!(refreshed, json!({"refreshed": 0, "stillDeferred": 2, "failed": 0}), "a hidden object is not drawn");
    assert_eq!(listed(&mut s), [a.0, b.0, c.0]);

    run(&mut s, "effect.remove", json!({"ids": [a.0], "index": 0}));
    assert_eq!(listed(&mut s), [b.0, c.0], "its effect was removed");
    run(&mut s, "effect.setParams", json!({"ids": [b.0], "index": 0, "visible": false}));
    assert_eq!(listed(&mut s), [c.0], "its effect was hidden");
    s.execute("edit.clear", &json!({"ids": [b.0, g.0]})).unwrap();
    assert_eq!(listed(&mut s), Vec::<u64>::new(), "its object was deleted");
    assert!(s.execute("effect.refreshDeferred", &json!({})).is_err(), "nothing is deferred");
}

/// A deferred stage in a symbol's art is listed by the object in that art, and a refresh runs it
/// where the symbol is placed.
#[test]
fn deferred_stages_in_symbol_art_are_listed_and_refreshed() {
    let mut s = session();
    let a = rect(&mut s, 10.0, "#ff0000");
    let out = json!({"objects": [{"path": square_at(110.0)}]}).to_string();
    install_wat(&mut s, &tpl::returning(&tpl::manifest("test.engine.deferred.symbol", "effect", "{}"), &out));
    run(&mut s, "effect.apply", json!({"ids": [a.0], "effect": "plugin.test.engine.deferred.symbol"}));
    run(&mut s, "select.set", json!({"ids": [a.0]}));
    let instance = run(&mut s, "symbol.new", json!({"name": "Deferred"}))["id"].as_u64().unwrap();
    let step = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(3_000));
    let fx = vectorcraft_plugins::deferred::Deferrals::with_clock(stepping_clock(&step));
    s.doc_mut().unwrap().plugin_effects = fx.clone();
    fx.view(|| drawn(&s));
    fx.first_view_done();
    assert_eq!(fx.take_report(), Some(1));
    let effects = run(&mut s, "effect.deferredStatus", json!({}))["effects"].clone();
    let object = effects[0]["objectId"].as_u64().unwrap();
    assert_eq!(effects.as_array().map(Vec::len), Some(1), "{effects}");
    assert!(object != instance && s.doc().unwrap().doc.node(NodeId(object)).is_none(), "an object of the symbol's art: {effects}");

    step.store(0, std::sync::atomic::Ordering::Relaxed);
    assert_eq!(run(&mut s, "effect.refreshDeferred", json!({})), json!({"refreshed": 1, "stillDeferred": 0, "failed": 0}));
    assert_eq!(run(&mut s, "effect.deferredStatus", json!({}))["effects"], json!([]));
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
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
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

/// Clearing the Additional Plug-ins Folder forgets the folder: setting the same folder again
/// installs what is in it, as setting any other folder does.
#[test]
fn plugins_folder_set_again_after_clearing_installs_its_plugins() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let dir = crate::tests_links::Folder::new("plugin-folder-cleared");
    let id = "test.engine.foldercleared";
    let src = tpl::returning(&tpl::manifest(id, "filter", "{}"), r#"{"objects":[]}"#);
    std::fs::write(dir.0.join("cleared.wasm"), wat::parse_str(src).unwrap()).unwrap();
    let mut s = session();
    let folder = json!({"key": "pluginsFolder", "value": dir.0.display().to_string()});
    run(&mut s, "prefs.set", folder.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed from the folder");
    run(&mut s, "plugin.remove", json!({"id": id}));
    run(&mut s, "prefs.set", json!({"key": "pluginsFolder", "value": ""}));
    run(&mut s, "prefs.set", folder);
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again when the folder is set again");
    run(&mut s, "plugin.remove", json!({"id": id}));
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
/// hash. The grant is recorded with that hash and covers only that module: replacing the plug-in
/// (`replace: true`) with one with other bytes needs a new grant. Removing it forgets the grant. A
/// plug-in without `permissions` installs as before, with the ones its kind implies.
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
    // Replaced with another module that lists the same permissions: granted again, the grant now
    // records the new module's hash.
    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&other)}))["needsApproval"], true);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&other), "grant": ["doc.read"], "replace": true}));
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

/// A module with other bytes under an id that was granted is not covered, even when it lists the
/// same permissions: `plugin.inspect` says so, `plugin.install` refuses it and leaves the installed
/// module and its grant alone, and a new `grant` installs it and records its hash. The same bytes
/// install again without asking, and a module that lists no permissions never asks.
#[test]
fn a_grant_covers_only_the_module_that_was_approved() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let mut s = session();
    let id = "test.engine.permissions.bytes";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    let (first_data, changed_data) = (vectorcraft_format::base64_encode(&first), vectorcraft_format::base64_encode(&changed));
    let (first_hash, changed_hash) = (sha256_hex(&first), sha256_hex(&changed));
    run(&mut s, "plugin.install", json!({"dataBase64": first_data, "grant": ["doc.read"]}));
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": changed_data}))["needsApproval"], true);
    let e = s.execute("plugin.install", &json!({"dataBase64": changed_data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("doc.read") && e.contains("`grant`"), "{e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": changed_data, "replace": true, "hash": changed_hash})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "`hash` is not an approval: {e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().hash(), first_hash, "the installed module stays");
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, first_hash, "its grant stays");
    // The same bytes need no new grant.
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": first_data}))["needsApproval"], false);
    run(&mut s, "plugin.install", json!({"dataBase64": first_data}));
    // A new grant installs the changed module, and the old bytes are no longer covered.
    run(&mut s, "plugin.install", json!({"dataBase64": changed_data, "grant": ["doc.read"], "hash": changed_hash, "replace": true}));
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().hash(), changed_hash);
    assert_eq!(vectorcraft_plugins::registry::granted(id).unwrap().hash, changed_hash);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": first_data}))["needsApproval"], true);
    run(&mut s, "plugin.remove", json!({"id": id}));

    // A manifest without a list, or with an empty one, asks for nothing, whatever its bytes.
    let exempt = "test.engine.permissions.exempt";
    for (permissions, out) in [("null", json!({"objects": []})), ("null", json!({"objects": [], "error": null})), ("[]", json!({"objects": []}))] {
        let data = vectorcraft_format::base64_encode(&asking(exempt, permissions, &out));
        assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": data}))["needsApproval"], false, "{permissions}");
        run(&mut s, "plugin.install", json!({"dataBase64": data, "replace": true}));
    }
    run(&mut s, "plugin.remove", json!({"id": exempt}));
}

/// Installing a module that lists no permissions under a granted id forgets the grant, so the
/// formerly approved bytes need a new approval. A refused install leaves the grant, and a module
/// that lists permissions keeps replacing it as before.
#[test]
fn a_module_without_permissions_ends_the_grant_of_its_id() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let mut s = session();
    let id = "test.engine.permissions.replaced";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let granted = || vectorcraft_plugins::registry::granted(id).map(|g| g.hash);
    for permissions in ["null", "[]"] {
        let plain = vectorcraft_format::base64_encode(&asking(id, permissions, &empty));
        run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
        // Refused installs change nothing.
        let e = s.execute("plugin.install", &json!({"dataBase64": plain, "hash": "0".repeat(64), "replace": true})).unwrap_err().to_string();
        assert!(e.contains("SHA-256 `hash`"), "{e}");
        let e = s.execute("plugin.install", &json!({"dataBase64": plain, "replace": false})).unwrap_err().to_string();
        assert!(e.contains("already installed"), "{e}");
        let e = s.execute("plugin.install", &json!({"dataBase64": plain})).unwrap_err().to_string();
        assert!(e.contains("`replace: true`"), "{e}");
        assert_eq!(granted(), Some(sha256_hex(&approved)), "{permissions}: a refused install leaves the grant");

        run(&mut s, "plugin.install", json!({"dataBase64": plain, "replace": true}));
        assert_eq!(granted(), None, "{permissions}");
        assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true, "{permissions}");
        let e = s.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
        assert!(e.contains("`grant`"), "{permissions}: {e}");
        assert!(!vectorcraft_plugins::registry::get(id).unwrap().manifest().asks_permissions(), "{permissions}: the module installed stays");
    }
    // With no module installed under the id, the grant ends the same way.
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
    vectorcraft_plugins::registry::remove(id);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&asking(id, "null", &empty))}));
    assert_eq!(granted(), None);
    // A module that lists permissions records its own grant.
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
    assert_eq!(granted(), Some(sha256_hex(&approved)));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// In a plug-in store, a module that lists no permissions removes only its own id's entry from
/// `grants.json`, and the formerly approved bytes need a new approval after a restart. A store
/// that holds such a module and a grant for its id has that grant forgotten when it is attached.
#[test]
fn a_module_without_permissions_ends_the_grant_kept_in_the_store() {
    use vectorcraft_plugins::permissions::{Grant, GrantToken, Permission, sha256_hex};
    let folder = crate::tests_links::Folder::new("plugin-store-stale-grant");
    let id = "test.engine.permissions.storedstale";
    let other = "test.engine.permissions.storedother";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let other_wasm = asking(other, r#"["doc.read"]"#, &empty);
    let grants = folder.0.join("grants.json");
    let ids = || -> Vec<String> {
        serde_json::from_slice::<Vec<Value>>(&std::fs::read(&grants).unwrap())
            .unwrap()
            .iter()
            .map(|e| e["id"].as_str().unwrap().to_string())
            .collect()
    };
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&other_wasm), "grant": ["doc.read"]}));
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    assert_eq!(ids().len(), 2);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert_eq!(ids(), vec![other.to_string()], "only this id's entry goes");
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    assert!(vectorcraft_plugins::registry::granted(other).is_some(), "the other plug-in's grant stays");

    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut next, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true);

    // A grant the store already holds for an id whose stored module lists no permissions.
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let other_entry = json!({"id": other, "hash": sha256_hex(&other_wasm), "permissions": ["doc.read"]});
    std::fs::write(&grants, serde_json::to_vec(&json!([entry, other_entry])).unwrap()).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the module that lists nothing still loads");
    assert_eq!(ids(), vec![other.to_string()]);
    assert!(vectorcraft_plugins::registry::get(other).is_some());

    // So does an entry this version can't read, and a grant held only by the process.
    let future = json!({"id": id, "hash": "00", "permissions": ["doc.read", "ui.panel"]});
    std::fs::write(&grants, serde_json::to_vec(&json!([future, other_entry])).unwrap()).unwrap();
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::grant(GrantToken::after_engine_approval(Grant {
        id: id.into(),
        hash: sha256_hex(&approved),
        permissions: vec![Permission::DocRead],
    }));
    let mut fourth = session();
    fourth.attach_plugin_store(folder.0.clone());
    assert_eq!(ids(), vec![other.to_string()]);
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    run(&mut fourth, "plugin.remove", json!({"id": id}));
    run(&mut fourth, "plugin.remove", json!({"id": other}));
}

/// A module listing no permissions that can't be written to the plug-in store leaves the stored
/// module in place, so `grants.json` keeps the grant that module needs at the next launch.
#[test]
fn a_failed_store_write_of_a_module_without_permissions_keeps_the_stored_grant() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-write-plain");
    let id = "test.engine.permissions.storefail";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let before = std::fs::read(&grants).unwrap();
    let file = s.stored_plugin_file(id).unwrap();
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert_eq!(std::fs::read(&grants).unwrap(), before, "the grant stays with the module left in the store");
    assert!(vectorcraft_plugins::registry::granted(id).is_none(), "the process runs the module that lists nothing");

    std::fs::remove_dir(&file).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A module listing no permissions whose id's grant can't be removed from `grants.json` doesn't stay
/// in the store, so the grant and the module it covers stay together. It does stay when the module
/// before it can't be read back, and when the grants file is too large or doesn't hold a list and so
/// approves nothing.
#[test]
fn a_grant_that_cannot_be_removed_keeps_the_stored_module() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-forget");
    let id = "test.engine.permissions.forgetfail";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let replace = json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true});
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    // An entry this version can't use is written back as it was read: `1e9` as `1000000000.0`, which
    // makes the rewritten file larger than the 1 MiB that is read.
    let entry = serde_json::from_slice::<Vec<Value>>(&std::fs::read(&grants).unwrap()).unwrap().remove(0);
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();

    run(&mut s, "plugin.install", replace.clone());
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert!(vectorcraft_plugins::registry::granted(id).is_none(), "the process runs the module that lists nothing");

    let oversized = vectorcraft_plugins::Limits::default().max_module_bytes as u64 + 1;
    std::fs::File::options().write(true).open(&file).unwrap().set_len(oversized).unwrap();
    run(&mut s, "plugin.install", replace.clone());
    assert_eq!(std::fs::read(&file).unwrap(), plain, "a module that can't be read back can't be put back");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    let inspect = json!({"dataBase64": vectorcraft_format::base64_encode(&approved)});
    assert_eq!(run(&mut s, "plugin.inspect", inspect)["needsApproval"], true, "the grant left behind stops counting");

    std::fs::write(&file, &approved).unwrap();
    std::fs::write(&grants, b"not a list").unwrap();
    run(&mut s, "plugin.install", replace.clone());
    assert_eq!(std::fs::read(&file).unwrap(), plain, "a grants file that isn't a list approves nothing");

    std::fs::write(&file, &approved).unwrap();
    std::fs::write(&grants, vec![b' '; (1 << 20) + 1]).unwrap();
    run(&mut s, "plugin.install", replace);
    assert_eq!(std::fs::read(&file).unwrap(), plain, "a grants file over its size limit approves nothing");

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grant that can't be removed stops counting for the session when the module it covers can't be
/// written back either: the store holds the new module, and the old bytes would otherwise install
/// without approval.
#[test]
fn a_grant_left_behind_when_its_module_cannot_go_back_stops_counting() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-put-back");
    let id = "test.engine.permissions.failedputback";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = serde_json::from_slice::<Vec<Value>>(&std::fs::read(&grants).unwrap()).unwrap().remove(0);
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();

    crate::store::PUT_BACK_FAILS.set(true);
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    crate::store::PUT_BACK_FAILS.set(false);
    assert_eq!(std::fs::read(&file).unwrap(), plain, "the store keeps the new module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true, "the grant left behind stops counting");
    let refused = s.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
    assert!(refused.contains("doc.read"), "{refused}");

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// An entry of `grants.json` that this version can't use is kept for a version that can, so when it
/// can't be removed, neither `plugin.remove` nor a replacing module gets past it.
#[test]
fn an_unusable_entry_that_cannot_be_removed_stops_plugin_remove() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-remove-other");
    let id = "test.engine.permissions.removeother";
    let empty = json!({"objects": []});
    let first = asking(id, "null", &empty);
    let second = asking(id, "[]", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first)}));
    let file = s.stored_plugin_file(id).unwrap();
    let entry = json!({"id": id, "hash": "0".repeat(64), "permissions": ["no.such.permission"]});
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();

    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert_eq!(std::fs::read(&file).unwrap(), first, "the store keeps the module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&second), "replace": true}));
    assert_eq!(std::fs::read(&file).unwrap(), first, "the store gets its module back");

    std::fs::write(&grants, serde_json::to_vec(&json!([entry])).unwrap()).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!grants.exists(), "the removal takes the entry out of the file");
}

/// A grant that can't be removed when no module is stored for its id has no module to go with, so
/// it stops counting for the session.
#[test]
fn a_grant_that_cannot_be_removed_and_has_no_stored_module_stops_counting() {
    let folder = crate::tests_links::Folder::new("plugin-store-grant-no-module");
    let id = "test.engine.permissions.forgetnomodule";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let entry = serde_json::from_slice::<Vec<Value>>(&std::fs::read(&grants).unwrap()).unwrap().remove(0);
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();
    std::fs::remove_file(&file).unwrap();

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert!(!std::path::Path::new(&file).exists(), "the store keeps no module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true);

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A grant that a load of the plug-in store should forget and can't stays out of the approvals in
/// the process: both when `grants.json` can't be rewritten and when it can't be read at the load.
/// A grant recorded later counts again, and another store's grant for the id counts.
#[test]
fn a_grant_the_load_cannot_remove_stops_counting_in_the_process() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-load-forget");
    let elsewhere = crate::tests_links::Folder::new("plugin-store-load-forget-other");
    let id = "test.engine.permissions.loadforget";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let needs_approval = json!({"dataBase64": approved_data});
    let grants = folder.0.join("grants.json");
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let recorded = serde_json::to_vec(&json!([entry])).unwrap();
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain)}));

    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert_eq!(std::fs::read(&grants).unwrap(), full, "the load can't rewrite the file");
    assert_eq!(run(&mut next, "plugin.inspect", needs_approval.clone())["needsApproval"], true);
    let refused = next.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
    assert!(refused.contains("doc.read"), "{refused}");

    std::fs::write(elsewhere.0.join("grants.json"), &recorded).unwrap();
    next.attach_plugin_store(elsewhere.0.clone());
    assert_eq!(run(&mut next, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "that store's grant counts");

    // A load that can rewrite the file forgets the grant, and the store has nothing ignored again.
    std::fs::write(&grants, &recorded).unwrap();
    next.attach_plugin_store(folder.0.clone());
    assert!(!grants.exists(), "the load removes the grant");

    // A directory at the file's path gives a read error on every platform.
    std::fs::create_dir(&grants).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut third = session();
    third.attach_plugin_store(folder.0.clone());
    std::fs::remove_dir(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    assert_eq!(run(&mut third, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the file reads again, and the grant doesn't count");

    // Attaching the same store again, with no stored module to try the removal for, keeps it out.
    let file = third.stored_plugin_file(id).unwrap();
    std::fs::remove_file(&file).unwrap();
    third.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut third, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the same store's grant stays out");

    // A later load in the session doesn't count it either, for the bytes it covers.
    std::fs::write(&file, &approved).unwrap();
    vectorcraft_plugins::registry::remove(id);
    third.load_stored_plugins();
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "the formerly approved module isn't loaded");

    run(&mut third, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"], "replace": true}));
    assert_eq!(run(&mut third, "plugin.inspect", needs_approval)["needsApproval"], false, "the user granted it again");
    run(&mut third, "plugin.remove", json!({"id": id}));
}

/// A grant one session stops counting stops counting for every session in the process attached to
/// the same folder, by any path to it: one attached before, one attached after with no stored module
/// to try the removal for, and one that comes back from another folder. A successful write of the
/// grant in any of them makes it count in all of them again.
#[test]
fn an_ignored_stored_grant_is_shared_by_sessions_on_the_same_folder() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-shared-ignore");
    let elsewhere = crate::tests_links::Folder::new("plugin-store-shared-ignore-other");
    let id = "test.engine.permissions.sharedignore";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let needs_approval = json!({"dataBase64": approved_data});
    let grants = folder.0.join("grants.json");
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let recorded = serde_json::to_vec(&json!([entry])).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    let mut first = session();
    first.attach_plugin_store(folder.0.clone());
    let inside = folder.0.join("inside");
    std::fs::create_dir(&inside).unwrap();
    let mut other = session();
    other.attach_plugin_store(inside.join(".."));
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "the recorded grant counts");

    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();
    run(&mut first, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain)}));
    let file = first.stored_plugin_file(id).unwrap();
    assert!(!std::path::Path::new(&file).exists(), "the store keeps no module");
    assert_eq!(std::fs::read(&grants).unwrap(), full);

    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the other session doesn't count it");
    let refused = other.execute("plugin.install", &json!({"dataBase64": approved_data, "replace": true})).unwrap_err().to_string();
    assert!(refused.contains("doc.read"), "{refused}");
    std::fs::write(&file, &approved).unwrap();
    vectorcraft_plugins::registry::remove(id);
    other.load_stored_plugins();
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "the formerly approved module isn't loaded");
    std::fs::remove_file(&file).unwrap();

    let mut later = session();
    later.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut later, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "a session attached later doesn't count it");

    std::fs::write(elsewhere.0.join("grants.json"), &recorded).unwrap();
    other.attach_plugin_store(elsewhere.0.clone());
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "that store's grant counts");
    other.attach_plugin_store(folder.0.clone());
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "back on the folder, it doesn't");

    std::fs::write(&grants, &recorded).unwrap();
    assert!(later.store_grant(id, Some(serde_json::from_value(entry).unwrap())), "the file can be written");
    for s in [&mut first, &mut other, &mut later] {
        assert_eq!(run(s, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "it counts again");
    }
    std::fs::remove_file(&grants).unwrap();
}

/// A plug-in folder that doesn't exist yet gets the key it gets once it is made, and every path to a
/// folder gets the same key.
#[test]
fn a_plugin_folder_has_one_key_for_every_path_to_it() {
    use crate::store::plugin_store_key;
    let folder = crate::tests_links::Folder::new("plugin-store-key");
    let dir = folder.0.join("Plug-ins");
    let before = plugin_store_key(&dir);
    assert_eq!(plugin_store_key(&dir.join("sub").join("..")), before);
    assert_eq!(plugin_store_key(&folder.0.join(".").join("Plug-ins")), before);
    std::fs::create_dir(&dir).unwrap();
    assert_eq!(plugin_store_key(&dir), before, "the key stays when the folder is made");
    assert_eq!(plugin_store_key(&dir.join(".")), before);
    #[cfg(windows)]
    {
        let plain = std::path::absolute(&dir).unwrap();
        let verbatim = std::path::PathBuf::from(format!(r"\\?\{}", plain.display()));
        assert_eq!(plugin_store_key(&verbatim), before, "a verbatim path names the same folder");
    }
}

/// Sessions attached to a plug-in folder before it exists, by paths that only name the same folder
/// once it is made, share the grants the process doesn't count for it. A grant that one session
/// writes while another fails to remove it keeps counting.
#[test]
fn sessions_attached_before_the_folder_exists_share_its_ignored_grants() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-late-folder");
    let id = "test.engine.permissions.latefolder";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let needs_approval = json!({"dataBase64": vectorcraft_format::base64_encode(&approved)});
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    std::fs::create_dir_all(folder.0.join("a").join("b")).unwrap();
    let dir = folder.0.join("a").join("Plug-ins");
    // On disk `link/..` is `a`, so `link/../Plug-ins` is the folder; by name it is a `Plug-ins` beside `a`.
    // On Windows the folder is named in other letter case.
    #[cfg(unix)]
    let alias = {
        std::os::unix::fs::symlink(folder.0.join("a").join("b"), folder.0.join("link")).unwrap();
        folder.0.join("link").join("..").join("Plug-ins")
    };
    #[cfg(windows)]
    let alias = folder.0.join("A").join("PLUG-INS");
    #[cfg(not(any(unix, windows)))]
    let alias = dir.clone();
    let mut first = session();
    first.attach_plugin_store(dir.clone());
    let mut other = session();
    other.attach_plugin_store(alias);
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("grants.json"), serde_json::to_vec(&json!([entry])).unwrap()).unwrap();
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "the recorded grant counts");

    first.ignore_stored_grant(id, first.grant_writes());
    assert_eq!(run(&mut other, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the other session doesn't count it");
    assert!(other.store_grant(id, Some(serde_json::from_value(entry.clone()).unwrap())), "the file can be written");
    assert_eq!(run(&mut first, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "it counts again");

    // A write that fails in one session while another writes the grant for the id leaves the grant
    // counting.
    let before = first.grant_writes();
    assert!(other.store_grant(id, Some(serde_json::from_value(entry).unwrap())), "the other session writes the grant");
    first.ignore_stored_grant(id, before);
    assert_eq!(run(&mut first, "plugin.inspect", needs_approval)["needsApproval"], false, "the written grant counts");
    std::fs::remove_file(dir.join("grants.json")).unwrap();
}

/// Grant writes that change nothing, each for another id, don't grow what the process keeps past its
/// limit.
#[test]
fn the_process_keeps_a_bounded_number_of_grant_writes() {
    let folder = crate::tests_links::Folder::new("plugin-store-writes-kept");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    for n in 0..300 {
        assert!(s.store_grant(&format!("test.engine.permissions.unknown{n}"), None), "nothing to remove is a success");
    }
    assert!(Session::grant_writes_kept() <= 256, "{}", Session::grant_writes_kept());
}

/// A session attached by a link to a plug-in folder matches the grants the process doesn't count
/// with the folder the link leads to now, not the one it led to when the session was attached.
#[cfg(unix)]
#[test]
fn a_linked_plugin_folder_uses_the_ignored_grants_of_where_the_link_leads() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-relinked");
    let id = "test.engine.permissions.relinked";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let needs_approval = json!({"dataBase64": vectorcraft_format::base64_encode(&approved)});
    let entry = json!({"id": id, "hash": sha256_hex(&approved), "permissions": ["doc.read"]});
    let recorded = serde_json::to_vec(&json!([entry])).unwrap();
    let (old, new, link) = (folder.0.join("old"), folder.0.join("new"), folder.0.join("Plug-ins"));
    for dir in [&old, &new] {
        std::fs::create_dir(dir).unwrap();
        std::fs::write(dir.join("grants.json"), &recorded).unwrap();
    }
    std::os::unix::fs::symlink(&old, &link).unwrap();
    let mut linked = session();
    linked.attach_plugin_store(link.clone());
    assert_eq!(run(&mut linked, "plugin.inspect", needs_approval.clone())["needsApproval"], false, "the recorded grant counts");

    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink(&new, &link).unwrap();
    let mut direct = session();
    direct.attach_plugin_store(new.clone());
    direct.ignore_stored_grant(id, direct.grant_writes());
    assert_eq!(run(&mut linked, "plugin.inspect", needs_approval.clone())["needsApproval"], true, "the link leads to the ignored grant");
    assert!(linked.store_grant(id, Some(serde_json::from_value(entry).unwrap())), "the file can be written");
    assert_eq!(run(&mut direct, "plugin.inspect", needs_approval)["needsApproval"], false, "it counts again");
}

/// A grants file that can't be opened or read may name the old module once it reads again, so
/// neither a module listing no permissions nor `plugin.remove` gets past it. A directory at the
/// file's path gives a read error on every platform.
#[test]
fn a_grants_file_that_cannot_be_read_keeps_the_grant_and_the_module_it_covers() {
    let folder = crate::tests_links::Folder::new("plugin-store-unread-grants");
    let id = "test.engine.permissions.unreadgrants";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let recorded = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir(&grants).unwrap();

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"], "replace": true}));
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store keeps the module");

    std::fs::remove_dir(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!grants.exists(), "the file reads again and the grant goes");
}

/// `plugin.remove` returns an error and keeps the plug-in when its grant can't be removed from
/// `grants.json`; a second one tries again. A grants file that doesn't hold a list doesn't stop it.
#[test]
fn a_grant_that_cannot_be_removed_stops_plugin_remove() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-remove");
    let id = "test.engine.permissions.removefail";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();
    let recorded = std::fs::read(&grants).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = serde_json::from_slice::<Vec<Value>>(&recorded).unwrap().remove(0);
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();
    let data = crate::store::plugin_data_dir(&folder.0, id);
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), b"{}").unwrap();

    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "it keeps its grant");
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store keeps the module the grant covers");
    assert_eq!(std::fs::read(&grants).unwrap(), full);
    assert!(data.join("settings.json").is_file(), "it keeps its settings");

    // Settings that can't be moved aside leave the grant recorded. A file at the aside path stops
    // the move on Unix, and a file held open in the folder stops it on Windows.
    std::fs::write(&grants, &recorded).unwrap();
    let mut aside = data.clone().into_os_string();
    aside.push(".removed");
    let aside = std::path::PathBuf::from(aside);
    std::fs::write(&aside, b"in the way").unwrap();
    #[cfg(windows)]
    let held = {
        use std::os::windows::fs::OpenOptionsExt;
        std::fs::File::options().read(true).share_mode(1).open(data.join("settings.json")).unwrap()
    };
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    #[cfg(windows)]
    drop(held);
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the grant stays while the settings do");
    assert!(vectorcraft_plugins::registry::get(id).is_some());
    let _ = std::fs::remove_file(&aside);

    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!grants.exists(), "the second removal forgets the grant");

    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    std::fs::write(&grants, b"not a list").unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "a grants file that isn't a list approves nothing");
    assert!(!std::path::Path::new(&file).exists());
}

/// When `plugin.remove` can't remove the grant and can't move the settings back either, its error
/// says where the settings are, and the plug-in gets no storage until they are back. A second
/// removal that can't remove the grant moves them back rather than deleting them, and so does the
/// next load of the plug-in.
#[test]
fn settings_that_cannot_be_moved_back_are_reported_and_moved_back_later() {
    let folder = crate::tests_links::Folder::new("plugin-store-failed-move-back");
    let id = "test.engine.permissions.movebackfail";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let recorded = std::fs::read(&grants).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = serde_json::from_slice::<Vec<Value>>(&recorded).unwrap().remove(0);
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();
    let data = crate::store::plugin_data_dir(&folder.0, id);
    let settings = br#"{"zoom":"aGVsbG8="}"#;
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), settings).unwrap();
    let mut aside = data.clone().into_os_string();
    aside.push(".removed");
    let aside = std::path::PathBuf::from(aside);

    crate::store::MOVE_BACK_FAILS.set(true);
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    crate::store::MOVE_BACK_FAILS.set(false);
    assert!(refused.contains(crate::store::GRANT_NOT_REMOVED), "{refused}");
    assert!(refused.contains(&aside.display().to_string()) && refused.contains("the folder is in use"), "{refused}");
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in stays installed");
    assert!(!data.exists());
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings);
    assert!(vectorcraft_plugins::registry::storage(id).is_none(), "no empty settings take their place");

    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert_eq!(refused, crate::store::GRANT_NOT_REMOVED);
    assert_eq!(std::fs::read(data.join("settings.json")).unwrap(), settings, "the second removal moves the settings back");
    assert!(!aside.exists());

    crate::store::MOVE_BACK_FAILS.set(true);
    assert!(s.execute("plugin.remove", &json!({"id": id})).is_err());
    crate::store::MOVE_BACK_FAILS.set(false);
    assert!(!data.exists());

    // A folder of the plug-in's own made meanwhile is used, and a removal that can't remove the
    // grant leaves both.
    let newer = br#"{"zoom":"bmV3"}"#;
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), newer).unwrap();
    let refused = s.execute("plugin.remove", &json!({"id": id})).unwrap_err().to_string();
    assert_eq!(refused, crate::store::GRANT_NOT_REMOVED);
    assert_eq!(std::fs::read(data.join("settings.json")).unwrap(), newer);
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings, "the kept settings stay");
    std::fs::remove_dir_all(&data).unwrap();

    // A restart loads the plug-in again with its settings.
    std::fs::write(&grants, &recorded).unwrap();
    vectorcraft_plugins::registry::remove(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "the plug-in loads");
    assert_eq!(std::fs::read(data.join("settings.json")).unwrap(), settings, "the load moves the settings back");
    assert!(!aside.exists());

    run(&mut next, "plugin.remove", json!({"id": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert!(!data.exists() && !aside.exists(), "the removal deletes the settings");
    assert_eq!(std::fs::read_dir(data.parent().unwrap()).unwrap().count(), 0, "nothing is left beside them");
}

/// What a deletion left in a plug-in's renamed settings folder, marked by the `.delete` file beside
/// it, isn't moved back. When that file can't be written, nothing is deleted, so the folder stays
/// whole and counts as kept settings.
#[test]
fn a_removal_deletes_only_folders_it_marked() {
    let folder = crate::tests_links::Folder::new("plugin-store-delete-mark");
    let id = "test.engine.permissions.deletemark";
    let approved = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let install = json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]});
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    let data = crate::store::plugin_data_dir(&folder.0, id);
    let settings = br#"{"zoom":"aGVsbG8="}"#;
    let mut aside = data.clone().into_os_string();
    aside.push(".removed");
    let aside = std::path::PathBuf::from(aside);
    let mut marker = aside.clone().into_os_string();
    marker.push(".delete");

    // The next install starts with no settings, and its removal deletes the leftover and the mark.
    std::fs::create_dir_all(&aside).unwrap();
    std::fs::write(aside.join("settings.json"), settings).unwrap();
    std::fs::write(&marker, b"").unwrap();
    run(&mut next, "plugin.install", install.clone());
    assert!(!data.exists(), "a leftover of a deletion isn't moved back");
    run(&mut next, "plugin.remove", json!({"id": id}));
    assert_eq!(std::fs::read_dir(data.parent().unwrap()).unwrap().count(), 0, "the leftover and its mark are deleted");

    // When the mark can't be written, the removal deletes nothing: installing the id again needs
    // `replace`, and a removal of the id that isn't installed deletes the folder.
    run(&mut next, "plugin.install", install.clone());
    std::fs::create_dir_all(&data).unwrap();
    std::fs::write(data.join("settings.json"), settings).unwrap();
    crate::store::MARKER_WRITE_FAILS.set(true);
    let removed = next.execute("plugin.remove", &json!({"id": id}));
    crate::store::MARKER_WRITE_FAILS.set(false);
    assert_eq!(removed.unwrap(), json!({"removed": id}));
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings, "nothing is deleted without the mark");
    assert!(!std::path::Path::new(&marker).exists());
    assert!(next.execute("plugin.install", &install).is_err(), "the kept settings go to a new install only with `replace`");
    assert!(vectorcraft_plugins::registry::get(id).is_none());
    assert_eq!(std::fs::read(aside.join("settings.json")).unwrap(), settings);
    run(&mut next, "plugin.remove", json!({"id": id}));
    assert_eq!(std::fs::read_dir(data.parent().unwrap()).unwrap().count(), 0, "the removal of the id deletes the kept folder");
}

/// Windows keeps a file another program holds open without delete sharing, so the last grant can't
/// be removed by deleting `grants.json`.
#[cfg(windows)]
#[test]
fn a_grants_file_that_cannot_be_deleted_keeps_the_stored_module() {
    use std::os::windows::fs::OpenOptionsExt;
    const FILE_SHARE_READ: u32 = 1;
    let folder = crate::tests_links::Folder::new("plugin-store-held-grants");
    let id = "test.engine.permissions.heldgrants";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    let file = s.stored_plugin_file(id).unwrap();

    let held = std::fs::File::options().read(true).share_mode(FILE_SHARE_READ).open(&grants).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    drop(held);
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store gets the module the grant covers back");
    assert!(grants.exists());

    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A plug-in folder that leaves a module listing no permissions under an id forgets the process's
/// grant for it, unless a later file in the folder leaves a granted module there. A grant kept in
/// the plug-in store stays, with the stored module it covers.
#[test]
fn a_plugin_folder_ends_the_process_grant_of_an_id_it_replaces() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let id = "test.engine.permissions.folderstale";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let approved_data = vectorcraft_format::base64_encode(&approved);
    let plain = asking(id, "null", &empty);
    let dir = crate::tests_links::Folder::new("plugin-folder-stale-grant");
    std::fs::write(dir.0.join("a-plain.wasm"), &plain).unwrap();
    std::fs::write(dir.0.join("b-approved.wasm"), &approved).unwrap();
    let path = json!({"path": dir.0.display().to_string()});
    run(&mut s, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));

    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!(r["failed"], json!([]), "{r}");
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "the granted module is the one left");

    std::fs::remove_file(dir.0.join("b-approved.wasm")).unwrap();
    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!(r["loaded"], json!([id]), "{r}");
    assert!(vectorcraft_plugins::registry::granted(id).is_none());
    assert_eq!(run(&mut s, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], true);
    run(&mut s, "plugin.remove", json!({"id": id}));

    let store = crate::tests_links::Folder::new("plugin-folder-stale-grant-store");
    let mut stored = session();
    stored.attach_plugin_store(store.0.clone());
    run(&mut stored, "plugin.install", json!({"dataBase64": approved_data, "grant": ["doc.read"]}));
    let grants = store.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    run(&mut stored, "plugin.reload", path);
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the store still holds the module its grant covers");
    assert_eq!(run(&mut stored, "plugin.inspect", json!({"dataBase64": approved_data}))["needsApproval"], false);
    run(&mut stored, "plugin.remove", json!({"id": id}));
}

/// A stored module whose bytes changed is skipped at the next start and stays in the store with its
/// grant, and the original bytes load again. A stored grant whose hash is malformed covers nothing.
/// With a store attached, a matching grant held only by the process doesn't count.
#[test]
fn a_stored_grant_covers_only_the_stored_module_it_recorded() {
    use vectorcraft_plugins::permissions::{Grant, GrantToken, Permission, sha256_hex};
    let folder = crate::tests_links::Folder::new("plugin-store-recorded-module");
    let id = "test.engine.permissions.storedbytes";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    let module = std::fs::read_dir(&folder.0)
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.extension().is_some_and(|x| x == "wasm"))
        .expect("the stored module");
    let forget = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    };
    let starts = || {
        forget();
        let mut next = session();
        next.attach_plugin_store(folder.0.clone());
        vectorcraft_plugins::registry::get(id).is_some()
    };
    assert!(starts(), "the module that was approved loads");
    std::fs::write(&module, &changed).unwrap();
    assert!(!starts(), "other bytes under the old grant are skipped");
    assert_eq!(std::fs::read(&module).unwrap(), changed, "the module stays in the store");
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "its grant stays");
    std::fs::write(&module, &first).unwrap();
    assert!(starts(), "the approved bytes load again");

    // A malformed hash is no wildcard, even for the module on disk.
    let upper = sha256_hex(&first).to_uppercase();
    for hash in ["", "00", upper.as_str()] {
        std::fs::write(&grants, serde_json::to_vec(&json!([{"id": id, "hash": hash, "permissions": ["doc.read"]}])).unwrap()).unwrap();
        assert!(!starts(), "hash {hash:?}");
    }

    // The store's grants are the ones that count, even when the process holds one for these bytes.
    std::fs::write(&grants, &recorded).unwrap();
    assert!(starts());
    let mut attached = session();
    attached.attach_plugin_store(folder.0.clone());
    vectorcraft_plugins::registry::grant(GrantToken::after_engine_approval(Grant {
        id: id.into(),
        hash: sha256_hex(&changed),
        permissions: vec![Permission::DocRead],
    }));
    let inspected = run(&mut attached, "plugin.inspect", json!({"dataBase64": vectorcraft_format::base64_encode(&changed)}));
    assert_eq!(inspected["needsApproval"], true, "a grant held only by the process doesn't count with a store attached");
    run(&mut attached, "plugin.remove", json!({"id": id}));
}

/// A write to the plug-in store that fails leaves the store's module beside the grant that approved
/// it, so the old module loads at the next start: a module that can't be written doesn't update the
/// grant, and a grant that can't be written puts the old module back. On a first install, it takes
/// the new module out again.
#[test]
fn a_failed_store_write_keeps_the_stored_module_with_its_grant() {
    use vectorcraft_plugins::permissions::sha256_hex;
    let folder = crate::tests_links::Folder::new("plugin-store-failed-write");
    let id = "test.engine.permissions.failedwrite";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read","doc.write"]"#, &json!({"objects": []}));
    let first_data = vectorcraft_format::base64_encode(&first);
    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read", "doc.write"], "replace": true});
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": first_data, "grant": ["doc.read"]}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    let module = std::path::PathBuf::from(s.stored_plugin_file(id).unwrap());
    let starts = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
        let mut next = session();
        next.attach_plugin_store(folder.0.clone());
        vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string())
    };
    let block = |path: &std::path::Path| {
        std::fs::remove_file(path).unwrap();
        std::fs::create_dir_all(path.join("block")).unwrap();
    };

    // The module can't be written: the grant stays the old module's.
    block(&module);
    run(&mut s, "plugin.install", update.clone());
    assert_eq!(std::fs::read(&grants).unwrap(), recorded, "the grant isn't updated for a module that wasn't stored");
    std::fs::remove_dir_all(&module).unwrap();
    std::fs::write(&module, &first).unwrap();
    assert_eq!(starts(), Some(sha256_hex(&first)), "the old module loads after a failed module write");

    // The grants file is full: the new grant, which is longer, doesn't fit.
    let cap = 1 << 20;
    let mut entries: Vec<Value> = serde_json::from_slice(&recorded).unwrap();
    entries.push(json!({"id": "", "pad": ""}));
    let room = cap - serde_json::to_vec(&entries).unwrap().len();
    *entries.last_mut().unwrap() = json!({"id": "", "pad": "x".repeat(room)});
    let full = serde_json::to_vec(&entries).unwrap();
    assert_eq!(full.len(), cap);
    std::fs::write(&grants, &full).unwrap();
    run(&mut s, "plugin.install", update.clone());
    assert_eq!(std::fs::read(&grants).unwrap(), full, "not written past the cap");
    assert_eq!(std::fs::read(&module).unwrap(), first, "the old module is put back");
    assert_eq!(starts(), Some(sha256_hex(&first)), "the old module loads after a failed grant write");

    // The grants file can't be read, so nothing can be recorded in it.
    block(&grants);
    run(&mut s, "plugin.install", update.clone());
    assert_eq!(std::fs::read(&module).unwrap(), first, "the old module is put back");
    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    assert_eq!(starts(), Some(sha256_hex(&first)), "the old module loads once the grants are readable");

    // A stored module that can't be read back isn't replaced, so a failed grant write can't lose it.
    let oversized = vectorcraft_plugins::Limits::default().max_module_bytes as u64 + 1;
    std::fs::File::options().write(true).open(&module).unwrap().set_len(oversized).unwrap();
    block(&grants);
    run(&mut s, "plugin.install", update);
    assert_eq!(std::fs::metadata(&module).unwrap().len(), oversized, "the stored module stays");
    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    std::fs::write(&module, &first).unwrap();

    // A first install whose grant can't be written leaves no module without a grant in the store.
    run(&mut s, "plugin.remove", json!({"id": id}));
    assert!(!module.exists() && !grants.exists());
    std::fs::create_dir_all(grants.join("block")).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": first_data, "grant": ["doc.read"]}));
    assert!(!module.exists(), "the store holds no module its grants don't cover");
    vectorcraft_plugins::registry::remove(id);
    vectorcraft_plugins::registry::revoke(id);
}

/// Runs the test `name` of this module again in a process of its own → whether this is that
/// process. The list of disabled plug-ins is kept for the whole process, and the other tests here
/// read it again each time they attach a plug-in store.
fn in_own_process(name: &str) -> bool {
    const CHILD: &str = "VECTORCRAFT_TEST_PLUGIN_DISABLED_LIST";
    if std::env::var_os(CHILD).is_some() {
        return true;
    }
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &format!("tests::plugins::{name}"), "--nocapture"])
        .env(CHILD, "1")
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("test result: ok. 1 passed; 0 failed;"),
        "{name} failed in its own process:\n{stdout}\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    false
}

/// An update whose grant can't be written, and whose old module can't be put back either, stays in
/// the store: a disabled plug-in is turned on, as for any update the store holds. The old grant
/// doesn't cover the new module, so the next start skips it until it is approved.
#[test]
fn an_update_left_in_the_store_by_a_failed_put_back_turns_a_disabled_plugin_on() {
    if !in_own_process("an_update_left_in_the_store_by_a_failed_put_back_turns_a_disabled_plugin_on") {
        return;
    }
    use vectorcraft_plugins::{permissions::sha256_hex, registry, safe_mode};
    let folder = crate::tests_links::Folder::new("plugin-store-failed-put-back-update");
    let id = "test.engine.permissions.putbackupdate";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read","doc.write"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    run(&mut s, "plugin.setEnabled", json!({"id": id, "enabled": false}));
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir_all(grants.join("block")).unwrap();

    crate::store::PUT_BACK_FAILS.set(true);
    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read", "doc.write"], "replace": true});
    let installed = s.execute("plugin.install", &update);
    crate::store::PUT_BACK_FAILS.set(false);
    assert_eq!(installed.unwrap()["hash"], sha256_hex(&changed), "the new module runs in this session");
    assert_eq!(registry::get(id).map(|p| p.hash().to_string()), Some(sha256_hex(&changed)));
    assert_eq!(std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap(), changed, "the store holds the new module");
    assert!(safe_mode::disabled().is_empty(), "the module the store holds is turned on");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(safe_mode::parse_disabled_file(&saved).is_empty(), "nothing disabled is saved");

    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    registry::remove(id);
    registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(registry::get(id).is_none(), "the old grant doesn't cover the new module");
    safe_mode::set_disabled(vec![]);
}

/// An update whose grant can't be written, and whose put-back writes the old module and then
/// reports a failure, leaves the old module in the store: a disabled plug-in stays disabled, and the
/// next start doesn't load it.
#[test]
fn an_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off() {
    if !in_own_process("an_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off") {
        return;
    }
    use vectorcraft_plugins::{registry, safe_mode};
    let folder = crate::tests_links::Folder::new("plugin-store-put-back-fails-after-write");
    let id = "test.engine.permissions.putbacklate";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read","doc.write"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    run(&mut s, "plugin.setEnabled", json!({"id": id, "enabled": false}));
    let disabled = safe_mode::disabled();
    assert!(!disabled.is_empty());
    let grants = folder.0.join("grants.json");
    let recorded = std::fs::read(&grants).unwrap();
    std::fs::remove_file(&grants).unwrap();
    std::fs::create_dir_all(grants.join("block")).unwrap();

    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(true);
    let update = json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read", "doc.write"], "replace": true});
    let installed = s.execute("plugin.install", &update);
    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(false);
    installed.unwrap();
    assert_eq!(std::fs::read(s.stored_plugin_file(id).unwrap()).unwrap(), first, "the store holds the old module");
    assert_eq!(safe_mode::disabled(), disabled, "the plug-in stays disabled");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(!safe_mode::parse_disabled_file(&saved).is_empty(), "the disabled list on disk is kept");

    std::fs::remove_dir_all(&grants).unwrap();
    std::fs::write(&grants, &recorded).unwrap();
    registry::remove(id);
    registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(registry::get(id).is_none(), "the disabled old module doesn't load");
    safe_mode::set_disabled(vec![]);
}

/// A module without permissions whose install can't remove the old grant, and whose put-back writes
/// the old module and then reports a failure, leaves the old module in the store: a disabled plug-in
/// stays disabled.
#[test]
fn a_plain_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off() {
    if !in_own_process("a_plain_update_put_back_that_fails_after_writing_keeps_a_disabled_plugin_off") {
        return;
    }
    use vectorcraft_plugins::safe_mode;
    let folder = crate::tests_links::Folder::new("plugin-store-plain-put-back-fails-after-write");
    let id = "test.engine.permissions.plainputbacklate";
    let empty = json!({"objects": []});
    let approved = asking(id, r#"["doc.read"]"#, &empty);
    let plain = asking(id, "null", &empty);
    let grants = folder.0.join("grants.json");
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&approved), "grant": ["doc.read"]}));
    run(&mut s, "plugin.setEnabled", json!({"id": id, "enabled": false}));
    let disabled = safe_mode::disabled();
    assert!(!disabled.is_empty());
    let file = s.stored_plugin_file(id).unwrap();
    // `1e9` is written back as `1000000000.0`, which makes the rewritten file larger than the 1 MiB that is read.
    let entry = serde_json::from_slice::<Vec<Value>>(&std::fs::read(&grants).unwrap()).unwrap().remove(0);
    let full = format!(r#"[{entry},{{"n":[{}]}}]"#, vec!["1e9"; 200_000].join(",")).into_bytes();
    std::fs::write(&grants, &full).unwrap();

    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(true);
    let installed = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&plain), "replace": true}));
    crate::store::PUT_BACK_FAILS_AFTER_WRITE.set(false);
    installed.unwrap();
    assert_eq!(std::fs::read(&file).unwrap(), approved, "the store holds the old module");
    assert_eq!(safe_mode::disabled(), disabled, "the plug-in stays disabled");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(!safe_mode::parse_disabled_file(&saved).is_empty(), "the disabled list on disk is kept");

    std::fs::remove_file(&grants).unwrap();
    run(&mut s, "plugin.remove", json!({"id": id}));
    safe_mode::set_disabled(vec![]);
}

/// A first install whose grant can't be written, and whose module can't be taken out of the store
/// again, stays in the store: the plug-in's file is turned on. No grant covers the module, so the
/// next start skips it until it is approved.
#[test]
fn a_first_install_left_in_the_store_by_a_failed_put_back_turns_its_file_on() {
    if !in_own_process("a_first_install_left_in_the_store_by_a_failed_put_back_turns_its_file_on") {
        return;
    }
    use vectorcraft_plugins::{registry, safe_mode};
    let folder = crate::tests_links::Folder::new("plugin-store-failed-put-back-first");
    let id = "test.engine.permissions.putbackfirst";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    let module = s.stored_plugin_file(id).unwrap();
    run(&mut s, "plugin.setEnabled", json!({"file": module, "enabled": false}));
    let grants = folder.0.join("grants.json");
    std::fs::create_dir_all(grants.join("block")).unwrap();

    crate::store::PUT_BACK_FAILS.set(true);
    let installed = s.execute("plugin.install", &json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    crate::store::PUT_BACK_FAILS.set(false);
    installed.unwrap();
    assert!(registry::get(id).is_some(), "the module runs in this session");
    assert_eq!(std::fs::read(&module).unwrap(), first, "the store holds the module");
    assert!(safe_mode::disabled().is_empty(), "the file the store holds is turned on");
    let saved = std::fs::read(folder.0.join(crate::store::DISABLED_FILE)).unwrap();
    assert!(safe_mode::parse_disabled_file(&saved).is_empty(), "nothing disabled is saved");

    std::fs::remove_dir_all(&grants).unwrap();
    registry::remove(id);
    registry::revoke(id);
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(registry::get(id).is_none(), "no grant covers the module");
    safe_mode::set_disabled(vec![]);
}

/// A plug-in folder reports a changed module as needing approval and still loads the other files;
/// granted on its own, the changed module loads from the folder. A session without a store uses the
/// grants held by the process.
#[test]
fn a_plugin_folder_skips_a_changed_module_until_it_is_approved() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let mut s = session();
    let id = "test.engine.permissions.folderbytes";
    let plain = "test.engine.permissions.folderplain";
    let first = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let changed = asking(id, r#"["doc.read"]"#, &json!({"objects": [], "error": null}));
    let dir = crate::tests_links::Folder::new("plugin-folder-changed-module");
    let file = dir.0.join("asks.wasm");
    std::fs::write(&file, &first).unwrap();
    std::fs::write(dir.0.join("plain.wasm"), asking(plain, "null", &json!({"objects": []}))).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&first), "grant": ["doc.read"]}));
    let path = json!({"path": dir.0.display().to_string()});
    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!((r["loaded"].as_array().unwrap().len(), r["failed"].as_array().unwrap().len()), (2, 0), "{r}");

    std::fs::write(&file, &changed).unwrap();
    let r = run(&mut s, "plugin.reload", path.clone());
    assert_eq!(r["loaded"], json!([plain]), "the other file still loads");
    let failed = r["failed"].as_array().unwrap();
    assert_eq!(failed.len(), 1, "{r}");
    assert!(failed[0]["file"].as_str().unwrap().ends_with("asks.wasm"), "{r}");
    let why = failed[0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval") && why.contains("doc.read"), "{why}");
    let hash = vectorcraft_plugins::permissions::sha256_hex(&first);
    assert_eq!(vectorcraft_plugins::registry::get(id).unwrap().hash(), hash, "the module installed earlier stays");

    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&changed), "grant": ["doc.read"], "replace": true}));
    let r = run(&mut s, "plugin.reload", path);
    assert_eq!(r["failed"], json!([]), "approved on its own, it loads: {r}");
    run(&mut s, "plugin.remove", json!({"id": id}));
    run(&mut s, "plugin.remove", json!({"id": plain}));
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

/// A filter whose manifest lists `inputs` is also given the selected type, read-only: an echo
/// changes nothing, and an output that leaves the type out or changes it is refused with the
/// document and undo history as they were, handed in from a run made elsewhere too. A filter
/// without `inputs` is not given the type and so never deletes it. Installing one that lists
/// `inputs` needs the user's grant.
#[test]
fn read_only_kind_field_and_omission_refuse_without_undo() {
    use cmd::plugin::FilterRun;
    let mut s = session();
    let r = rect(&mut s, 10.0, "#ff0000");
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Hi"}))["id"].as_u64().unwrap());
    let reads =
        |id: &str| format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":["doc.read","doc.write"],"inputs":["type"]}}"#);
    // Each case installs another module under the same id, in place of the last (`replace: true`).
    let install_wat = |s: &mut Session, src: &str| {
        let data = vectorcraft_format::base64_encode(&wat::parse_str(src).unwrap());
        run(s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read", "doc.write"], "replace": true}))
    };
    let echo = "test.engine.inputs.echo";
    let module = tpl::module(&reads(echo), tpl::ECHO, "", 1);
    let data = vectorcraft_format::base64_encode(&wat::parse_str(&module).unwrap());
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("asks for the permissions doc.read, doc.write"), "{e}");
    let listed = install_wat(&mut s, &module);
    assert_eq!(listed["inputs"], json!(["type"]));
    let p = |id: &str| json!({"id": id, "ids": [r.0, t.0]});
    let before = s.doc().unwrap().doc.clone();
    assert_eq!(run(&mut s, "plugin.run", p(echo))["ids"], json!([r.0, t.0]));
    assert_eq!(s.doc().unwrap().doc.layers, before.layers, "an echo changes nothing");
    for (output, says) in [
        (json!({"objects": [{"id": r.0}]}), format!("objects (id {}): a read-only object was left out", t.0)),
        (json!({"objects": [{"id": r.0}, {"id": t.0, "name": "Renamed"}]}), format!("objects[1] (id {}): read-only field 'name' changed", t.0)),
        (json!({"objects": [{"id": r.0}, {"id": t.0}, {"type": "type", "path": triangle()}]}), "`type` must be \"path\" or \"compound\"".into()),
    ] {
        let id = "test.engine.inputs.bad";
        install_wat(&mut s, &tpl::returning(&reads(id), &output.to_string()));
        let (doc, undo) = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
        let e = s.execute("plugin.run", &p(id)).unwrap_err().to_string();
        assert!(e.contains(&says), "{says}: {e}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo), "{output}");
        // Handed in from a run made away from plugin.run, it is refused the same way.
        let made = FilterRun::prepare(&s, &p(id)).unwrap();
        let out = made.run().unwrap();
        made.hand_in(&mut s, out);
        let e = s.execute("plugin.run", &p(id)).unwrap_err().to_string();
        assert!(e.contains(&says), "handed in: {e}");
        assert_eq!((&s.doc().unwrap().doc, s.doc().unwrap().history.undo.len()), (&doc, undo), "handed in: {output}");
    }
    // Without `inputs` the type isn't given, so leaving it out keeps it.
    let legacy = "test.engine.inputs.legacy";
    install_wat(&mut s, &tpl::returning(&tpl::manifest(legacy, "filter", "{}"), &json!({"objects": [{"id": r.0, "name": "Kept"}]}).to_string()));
    assert_eq!(run(&mut s, "plugin.run", p(legacy))["ids"], json!([r.0]));
    assert_eq!(node(&s, r).name.as_deref(), Some("Kept"));
    assert!(matches!(node(&s, t).kind, vectorcraft_doc::NodeKind::Text(_)), "the type stays");
    // A manifest that lists `inputs` below `api` 2 doesn't install.
    let old = format!(r#"{{"id":"{echo}","name":"Old","kind":"filter","inputs":["type"]}}"#);
    let data = vectorcraft_format::base64_encode(&wat::parse_str(tpl::module(&old, tpl::ECHO, "", 1)).unwrap());
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("`inputs` needs `api` 2"), "{e}");
}

/// A run past its memory budget fails before the plug-in starts, with the document and undo
/// history as they were: a read-only object whose other fields don't fit, a path and a compound
/// path that don't fit, and parameters past the input's half of the cap, with objects selected or
/// with none. The module has one page of memory, the most it may have, so a run that started would
/// report that the plug-in ran out of memory instead.
#[test]
fn a_run_past_the_memory_budget_fails_before_the_plugin_starts() {
    use std::sync::Arc;
    use vectorcraft_doc::{Appearance, Node, NodeKind};
    use vectorcraft_geom::FillRule;
    use vectorcraft_plugins::{Limits, Plugin, registry};
    let id = "test.engine.inputs.budget";
    let params: Vec<String> = (0..9).map(|i| format!(r#""p{i}":{{"type":"string","maxLength":4096}}"#)).collect();
    let manifest = format!(
        r#"{{"id":"{id}","name":"Budget","kind":"filter","api":2,"permissions":["doc.read","doc.write"],"inputs":["type"],"params":{{{}}}}}"#,
        params.join(",")
    );
    let module = wat::parse_str(tpl::module(&manifest, "unreachable", "", 1)).unwrap();
    // One page of memory: the input and the parameters share 32 KiB. The module lists permissions,
    // so its grant is recorded first.
    let plugin = Plugin::load(&module, Limits { max_memory_bytes: 1 << 16, ..Limits::default() }).unwrap();
    registry::grant(vectorcraft_plugins::permissions::GrantToken::after_engine_approval(vectorcraft_plugins::permissions::Grant {
        id: id.into(),
        hash: plugin.hash().to_string(),
        permissions: plugin.manifest().permissions(),
    }));
    registry::install(plugin).unwrap();
    let mut s = session();
    let small = rect(&mut s, 10.0, "#ff0000");
    let long = rect(&mut s, 30.0, "#00ff00");
    let t = NodeId(run(&mut s, "text.create", json!({"x": 10, "y": 80, "text": "Hi"}))["id"].as_u64().unwrap());
    let mut compound = NodeId(0);
    s.edit("Set up", |d, _| {
        d.node_mut(t).unwrap().name = Some("n".repeat(40_000));
        let points: Vec<Point> = (0..4_000).map(|i| Point::new(f64::from(i), 0.0)).collect();
        let outline = PathData::single(SubPath::polyline(&points, false));
        if let NodeKind::Path { path, .. } = &mut d.node_mut(long).unwrap().kind {
            *path = outline.clone();
        }
        let member = Node::path(d.alloc_id(), outline, Appearance::default());
        let n = Node::new(d.alloc_id(), NodeKind::Compound { children: vec![Arc::new(member)], rule: FillRule::NonZero });
        let layer = d.layers[0].id;
        compound = d.insert(Some(layer), usize::MAX, n).unwrap();
        Ok(())
    })
    .unwrap();
    let padding: Value = (0..9).map(|i| (format!("p{i}"), json!("x".repeat(4096)))).collect::<serde_json::Map<_, _>>().into();
    let too_large = "memory budget (the selection is too large to hand over)";
    let named = format!("memory budget (object {} needs", t.0);
    let with_padding = |ids: Vec<u64>| {
        let mut p = padding.clone();
        p["id"] = json!(id);
        p["ids"] = json!(ids);
        p
    };
    let before = (s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len());
    for (p, says) in [
        (json!({"id": id, "ids": [t.0]}), named.as_str()),
        (json!({"id": id, "ids": [long.0]}), too_large),
        (json!({"id": id, "ids": [compound.0]}), too_large),
        (with_padding(vec![small.0]), too_large),
        (with_padding(vec![]), too_large),
    ] {
        let e = s.execute("plugin.run", &p).unwrap_err().to_string();
        assert!(e.contains(says), "{says}: {e}");
        assert_eq!((s.doc().unwrap().doc.clone(), s.doc().unwrap().history.undo.len()), before, "{says}");
    }
    assert!(registry::remove(id) && registry::revoke(id));
}

/// What the user granted is kept in the plug-in store with the module: the next session installs
/// the plug-in again without asking. A stored plug-in that lists permissions nobody granted it is
/// skipped, even when another store granted it, and so is one in the Additional Plug-ins Folder
/// until it is granted on its own.
#[test]
fn granted_permissions_are_kept_and_folders_skip_the_rest() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
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

/// The registry installs a module that lists permissions only under a grant the process holds, so
/// a start and a `plugin.reload` record the store's grant for the module they load. A module in the
/// folder that nobody granted is reported and skipped, and records nothing.
#[test]
fn stored_and_reload_grants_still_install() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
    let store = crate::tests_links::Folder::new("plugin-store-grants-reload");
    let id = "test.engine.permissions.reload";
    let lone = "test.engine.permissions.reload.lone";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let mut s = session();
    s.attach_plugin_store(store.0.clone());
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": ["doc.read"]}));
    let forget = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    };
    forget();
    let mut next = session();
    next.attach_plugin_store(store.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_some(), "installed again from the store");
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));
    forget();
    let dir = crate::tests_links::Folder::new("plugin-folder-grants-reload");
    std::fs::write(dir.0.join("granted.wasm"), &wasm).unwrap();
    std::fs::write(dir.0.join("lone.wasm"), asking(lone, r#"["doc.read"]"#, &json!({"objects": []}))).unwrap();
    let r = run(&mut next, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([id]), "{r}");
    assert!(vectorcraft_plugins::registry::granted(id).is_some(), "the store's grant is the process's too");
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval") && r["failed"][0]["file"].as_str().unwrap().ends_with("lone.wasm"), "{r}");
    assert!(vectorcraft_plugins::registry::get(lone).is_none() && vectorcraft_plugins::registry::granted(lone).is_none());
    run(&mut next, "plugin.remove", json!({"id": id}));
}

/// A grant covers the module it was given to: another module under the id, listing the same
/// permissions, isn't installed from the store at the next start, a folder load reports it as
/// needing approval, and `plugin.install` asks for `grant` again, even with `replace: true`. It
/// doesn't get the settings kept for the id until the user grants it. A kept grant without a hash
/// covers no module.
#[test]
fn a_grant_covers_only_the_module_it_was_given_to() {
    let _folder = PREFS_FOLDER.lock().unwrap_or_else(|e| e.into_inner());
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
    vectorcraft_plugins::registry::set_storage(id, None);
    let no_storage = |how: &str| {
        assert!(vectorcraft_plugins::registry::storage(id).is_none(), "{how} gives another module the id's settings and cache");
    };
    let mut next = session();
    next.attach_plugin_store(folder.0.clone());
    assert!(vectorcraft_plugins::registry::get(id).is_none(), "another module isn't loaded under the grant");
    no_storage("the next start");
    let dir = crate::tests_links::Folder::new("plugin-folder-grant-hash");
    std::fs::write(dir.0.join("other.wasm"), &other).unwrap();
    let r = run(&mut next, "plugin.reload", json!({"path": dir.0.display().to_string()}));
    assert_eq!(r["loaded"], json!([]));
    let why = r["failed"][0]["error"].as_str().unwrap();
    assert!(why.contains("needs approval"), "{why}");
    no_storage("a folder load");
    let data = vectorcraft_format::base64_encode(&other);
    let e = next.execute("plugin.install", &json!({"dataBase64": data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("`grant`"), "{e}");
    no_storage("a refused install");
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

/// An approval of the module `wasm` with id `id`, its `permissions` and, if `heavy`, its budget.
fn approval(id: &str, wasm: &[u8], permissions: &[&str], heavy: bool) -> crate::cmd::plugin::InstallApproval {
    crate::cmd::plugin::InstallApproval {
        id: id.into(),
        hash: vectorcraft_plugins::permissions::sha256_hex(wasm),
        permissions: permissions.iter().map(|p| p.to_string()).collect(),
        heavy,
    }
}

/// In a session that wasn't started to accept them, `grant` and `allowHeavy` are refused whichever
/// way the command runs, a batch included, until an approval covers the module, its id, SHA-256 and
/// permissions. An approval is spent by the first install, whatever it does. A hash already approved
/// still installs.
#[test]
fn a_request_cannot_grant_what_only_a_person_approves() {
    let mut s = strict_session();
    let id = "test.engine.approval";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let refused = "take effect only in a session started with --allow-plugin-grants";
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": data})).unwrap_err().to_string();
    assert!(e.contains("approve them in the desktop app") && e.contains("--allow-plugin-grants"), "{e}");
    let batch = json!({"commands": [{"command": "plugin.install", "params": {"dataBase64": data, "grant": ["doc.read"]}}]});
    let e = s.execute("command.batch", &batch).unwrap_err().to_string();
    assert!(e.contains(refused), "a batch can't grant: {e}");
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    // Approved for other bytes, for another id or for other permissions: refused, and spent.
    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [{"name": "x"}]}));
    for wrong in [
        approval(id, &other, &["doc.read"], false),
        approval("test.engine.approval.other", &wasm, &["doc.read"], false),
        approval(id, &wasm, &["doc.read", "state"], false),
    ] {
        s.approve_plugin_install(wrong);
        let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
        assert!(e.contains(refused), "{e}");
    }
    assert!(vectorcraft_plugins::registry::get(id).is_none());

    // The approval for this module installs it once.
    s.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert_eq!(vectorcraft_plugins::registry::granted(id).map(|g| g.hash), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "the approval was spent: {e}");
    // A hash already approved installs without one.
    run(&mut s, "plugin.install", json!({"dataBase64": data}));
    run(&mut s, "plugin.remove", json!({"id": id}));

    // A refused install spends the approval too.
    s.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"], "hash": "0"})).unwrap_err().to_string();
    assert!(e.contains("SHA-256"), "{e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");
    // So does one refused for its params.
    s.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    s.execute("plugin.install", &json!({"dataBase64": data, "grant": "doc.read"})).unwrap_err();
    let e = s.execute("plugin.install", &json!({"dataBase64": data, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");

    // The heavy budget needs its own agreement.
    let heavy_id = "test.engine.approval.heavy";
    let heavy_wasm = heavy(heavy_id, 1);
    let heavy_data = vectorcraft_format::base64_encode(&heavy_wasm);
    let e = s.execute("plugin.install", &json!({"dataBase64": heavy_data, "allowHeavy": true})).unwrap_err().to_string();
    assert!(e.contains(refused), "{e}");
    // Without a `permissions` list it has the ones its kind implies.
    let inspected = run(&mut s, "plugin.inspect", json!({"dataBase64": heavy_data}));
    let implied: Vec<&str> = inspected["permissions"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    s.approve_plugin_install(approval(heavy_id, &heavy_wasm, &implied, false));
    let e = s.execute("plugin.install", &json!({"dataBase64": heavy_data, "allowHeavy": true})).unwrap_err().to_string();
    assert!(e.contains(refused), "an approval without the heavy budget doesn't cover it: {e}");
    let e = s.execute("plugin.install", &json!({"dataBase64": heavy_data})).unwrap_err().to_string();
    assert!(e.contains("approve it in the desktop app"), "{e}");
    s.approve_plugin_install(approval(heavy_id, &heavy_wasm, &implied, true));
    run(&mut s, "plugin.install", json!({"dataBase64": heavy_data, "allowHeavy": true}));
    run(&mut s, "plugin.remove", json!({"id": heavy_id}));

    // A session started to accept them takes them from requests.
    s.allow_plugin_grants_from_requests();
    run(&mut s, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A module a person approved in the desktop app, recorded in the store's `grants.json`, installs
/// from another session with that store and no flag and no `grant`. Other bytes under its id are
/// refused there, with or without `grant`.
#[test]
fn a_module_approved_in_the_app_installs_from_a_strict_session_with_its_store() {
    let folder = crate::tests_links::Folder::new("plugin-approval-store");
    let id = "test.engine.approval.store";
    let wasm = asking(id, r#"["doc.read"]"#, &json!({"objects": []}));
    let data = vectorcraft_format::base64_encode(&wasm);
    let mut desktop = strict_session();
    desktop.attach_plugin_store(folder.0.clone());
    desktop.approve_plugin_install(approval(id, &wasm, &["doc.read"], false));
    run(&mut desktop, "plugin.install", json!({"dataBase64": data, "grant": ["doc.read"]}));
    assert!(std::fs::read_to_string(folder.0.join("grants.json")).unwrap().contains(id), "recorded in the store");

    // Another process starts with an empty registry and the store's grants.
    let forget = || {
        vectorcraft_plugins::registry::remove(id);
        vectorcraft_plugins::registry::revoke(id);
    };
    forget();
    let mut headless = strict_session();
    headless.attach_plugin_store(folder.0.clone());
    forget();
    run(&mut headless, "plugin.install", json!({"dataBase64": data}));
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));

    let other = asking(id, r#"["doc.read"]"#, &json!({"objects": [{"name": "x"}]}));
    let other_data = vectorcraft_format::base64_encode(&other);
    let e = headless.execute("plugin.install", &json!({"dataBase64": other_data, "replace": true})).unwrap_err().to_string();
    assert!(e.contains("approve them in the desktop app"), "{e}");
    let e = headless.execute("plugin.install", &json!({"dataBase64": other_data, "replace": true, "grant": ["doc.read"]})).unwrap_err().to_string();
    assert!(e.contains("take effect only in a session started with --allow-plugin-grants"), "{e}");
    assert_eq!(vectorcraft_plugins::registry::get(id).map(|p| p.hash().to_string()), Some(vectorcraft_plugins::permissions::sha256_hex(&wasm)));
    run(&mut headless, "plugin.remove", json!({"id": id}));
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

/// The `plugin.*` commands that the tests running every command with `{}` run
/// ([`crate::runs_with_empty_params`]) leave the cache of an installed plug-in as it is, so
/// those tests can run beside the ones here that check a cache. Each command runs with `{}` in a
/// document with two rectangles, one or both selected, as `every_command_survives_empty_params`
/// runs it.
#[test]
fn plugin_commands_run_with_empty_params_keep_other_plugins_caches() {
    let folder = crate::tests_links::Folder::new("plugin-storage-empty-params");
    let id = "test.engine.storageemptyparams";
    let mut s = session();
    s.attach_plugin_store(folder.0.clone());
    install_storing(&mut s, id, WRITES);
    run(&mut s, "plugin.run", json!({"id": id, "ids": []}));
    assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3));
    let mut ran = vec![];
    for all in [false, true] {
        for c in crate::cmd::command_specs().iter().filter(|c| c.id.starts_with("plugin.") && crate::runs_with_empty_params(c.id)) {
            let mut other = session();
            let a = rect(&mut other, 10.0, "#ff0000");
            rect(&mut other, 40.0, "#0000ff");
            if all {
                run(&mut other, "select.all", json!({}));
            } else {
                run(&mut other, "select.set", json!({"ids": [a.0]}));
            }
            let _ = other.execute(c.id, &json!({}));
            ran.push(c.id);
            assert_eq!(run(&mut s, "plugin.info", json!({"id": id}))["cacheBytes"], json!(3), "after {}", c.id);
        }
    }
    assert!(ran.contains(&"plugin.list"), "{ran:?}");
    run(&mut s, "plugin.remove", json!({"id": id}));
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
    s.remove_plugin_storage(upper_id, || true).unwrap();
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
    s.remove_plugin_storage(id, || true).unwrap();
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

/// A filter that calls commands runs on the document with the edit in progress kept first: a move
/// still being previewed is its own undo step, and the filter is given the moved rectangle, which
/// its empty output then removes.
#[test]
fn a_filter_that_calls_commands_is_given_the_edit_in_progress() {
    let id = "test.engine.callsettle";
    let mut s = session();
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"]}}"#);
    let wasm = wat::parse_str(tpl::returning(&manifest, r#"{"objects":[]}"#)).unwrap();
    run(&mut s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true}));
    let paths = |s: &Session| s.doc().unwrap().doc.layers.first().and_then(|l| l.children()).map_or(0, |c| c.len());
    run(&mut s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}));
    s.begin_interaction("Move").unwrap();
    s.preview("object.move", &json!({"dx": 7, "dy": 0})).unwrap();
    run(&mut s, "plugin.run", json!({"id": id}));
    assert!(!s.in_interaction());
    assert_eq!(paths(&s), 0, "the filter was given the moved rectangle and removed it");
    let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
    assert_eq!(labels, [format!("Test {id}").as_str(), "Move"]);
    run(&mut s, "edit.undo", json!({}));
    let x = run(&mut s, "document.inspect", json!({}))["layers"][0]["children"][0]["bounds"]["x"].clone();
    assert_eq!(x, json!(17.0), "undoing the filter brings back the moved rectangle");
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// Installs a filter `id` that calls commands and hands back what it is given, with `commands` (a
/// JSON list of command specs) in its manifest.
fn install_echo_calling(s: &mut Session, id: &str, commands: &str) {
    let grant = json!(["doc.read", "doc.write", "commands"]);
    let manifest = format!(
        r#"{{"id":"{id}","name":"Test {id}","kind":"filter","api":2,"permissions":{grant},"commandAllowlist":["select.all"],"commands":{commands}}}"#
    );
    let wasm = wat::parse_str(tpl::module(&manifest, tpl::ECHO, "", 1)).unwrap();
    run(s, "plugin.install", json!({"dataBase64": vectorcraft_format::base64_encode(&wasm), "grant": grant, "replace": true}));
}

/// With Real-time Drawing off, a 50 × 50 rectangle at (10, 10) whose move by 7 pt is still being
/// dragged: the tool has stored the move, which the document does not hold yet.
fn deferred_move(s: &mut Session) -> NodeId {
    let r = NodeId(run(s, "shape.rectangle", json!({"x": 10, "y": 10, "width": 50, "height": 50}))["id"].as_u64().unwrap());
    run(s, "prefs.set", json!({"key": "realTimeDrawing", "value": false}));
    let move_by = vectorcraft_tools::Action::Preview("object.move".into(), json!({"dx": 7, "dy": 0}));
    s.apply_actions(vec![vectorcraft_tools::Action::Begin("Move".into()), move_by]).unwrap();
    assert_eq!(node(s, r).geometric_bounds().unwrap().x0, 10.0, "the move is not applied while it is dragged");
    r
}

/// The rectangle a run named `label` during [`deferred_move`] left: moved, with the interaction
/// ended and the move an undo step of its own before the run's.
fn assert_moved_once(s: &Session, r: NodeId, label: &str) {
    assert!(!s.in_interaction());
    assert_eq!(node(s, r).geometric_bounds().unwrap().x0, 17.0, "the filter was given the moved rectangle");
    let labels: Vec<&str> = s.doc().unwrap().history.undo.iter().rev().take(2).map(|e| e.label.as_str()).collect();
    assert_eq!(labels, [label, "Move"]);
}

/// With Real-time Drawing off, `plugin.run` of a filter that calls commands during a drag applies
/// the dragged move first and gives the filter the moved rectangle, which it hands back as it was.
#[test]
fn a_filter_that_calls_commands_is_given_a_deferred_move() {
    let id = "test.engine.callsdeferred";
    let mut s = session();
    install_echo_calling(&mut s, id, "[]");
    let r = deferred_move(&mut s);
    run(&mut s, "plugin.run", json!({"id": id}));
    assert_moved_once(&s, r, &format!("Test {id}"));
    run(&mut s, "plugin.remove", json!({"id": id}));
}

/// A plug-in command that calls commands, run with Real-time Drawing off during a drag, is given
/// the moved rectangle as `plugin.run` is.
#[test]
fn a_plugin_command_that_calls_commands_is_given_a_deferred_move() {
    let id = "test.engine.callsdeferredcmd";
    let mut s = session();
    install_echo_calling(&mut s, id, r#"[{"id":"same","label":"Same"}]"#);
    let r = deferred_move(&mut s);
    run(&mut s, &format!("plugin.{id}.same"), json!({}));
    assert_moved_once(&s, r, "Same");
    run(&mut s, "plugin.remove", json!({"id": id}));
}
